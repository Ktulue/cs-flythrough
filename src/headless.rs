use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use std::path::PathBuf;
use wgpu;

use crate::camera::{Camera, CameraPose};
use crate::capture::{self, FrameEntry};
use crate::config::Config;
use crate::renderer::init_gpu_core;
use crate::{bsp, camera as cam_mod, maplist};

pub struct HeadlessArgs {
    pub walkthrough: bool,
    pub output_dir: PathBuf,
    pub camera_pos: Option<[f32; 3]>,
    pub camera_angle_deg: Option<[f32; 2]>, // degrees; converted to radians internally
    pub frame_count: u32,
    pub frame_step: u32,
    pub width: u32,
    pub height: u32,
    pub map: Option<String>,
}

pub fn run(args: HeadlessArgs, cfg: Config) -> Result<()> {
    pollster::block_on(run_async(args, cfg))
}

async fn run_async(args: HeadlessArgs, cfg: Config) -> Result<()> {
    // Validate frame_count and frame_step
    if args.frame_count == 0 {
        capture::print_error_json("frame-count must be at least 1");
        std::process::exit(1);
    }
    if args.frame_step == 0 {
        capture::print_error_json("frame-step must be at least 1");
        std::process::exit(1);
    }

    // Warn about conflicting flags
    if args.camera_pos.is_some() && args.walkthrough {
        eprintln!("warning: --walkthrough ignored when --camera-pos is set");
    }

    // Create output directory
    std::fs::create_dir_all(&args.output_dir)
        .with_context(|| format!("creating output dir {}", args.output_dir.display()))?;

    // Resolve map name
    let map_name = args
        .map
        .as_deref()
        .or(cfg.map.as_deref())
        .unwrap_or("de_dust2")
        .to_string();

    // Load BSP
    let bsp_path = maplist::resolve_bsp(&cfg.cs_install_path, &map_name).map_err(|e| {
        capture::print_error_json(&format!("failed to load map '{map_name}': {e:#}"));
        e
    })?;

    let mesh = bsp::load(&bsp_path, &cfg.cs_install_path).map_err(|e| {
        capture::print_error_json(&format!("failed to load map '{map_name}': {e:#}"));
        e
    })?;

    // Extract entity origins before mesh is consumed by init_gpu_core
    let entity_origins = mesh.entity_origins.clone();

    // Build Camera for spline modes (not needed for fixed-pose)
    let mut camera: Option<Camera> = if args.camera_pos.is_none() {
        let waypoints = if let Some(route) = cfg.find_route(&map_name) {
            // Custom hand-designed route takes priority.
            eprintln!("[cs-flythrough] using {} custom waypoints for '{map_name}'", route.waypoints.len());
            route.waypoints.iter().map(|&[x, y, z]| glam::Vec3::new(x, y, z)).collect()
        } else if let Some(nav_path) = maplist::resolve_nav(&cfg.cs_install_path, &map_name) {
            match bsp::nav::load_waypoints(&nav_path, 250.0, -64.0) {
                Ok(pts) => {
                    eprintln!("[cs-flythrough] loaded {} NAV waypoints from {}", pts.len(), nav_path.display());
                    let sorted = cam_mod::nearest_neighbor_sort(pts);
                    let decimated = cam_mod::decimate_waypoints(sorted, 250.0);
                    cam_mod::smooth_waypoints(decimated, 3)
                }
                Err(e) => {
                    eprintln!("[cs-flythrough] NAV load failed ({e:#}), falling back to entity origins");
                    cam_mod::smooth_waypoints(cam_mod::nearest_neighbor_sort(entity_origins), 3)
                }
            }
        } else {
            eprintln!("[cs-flythrough] no NAV file found for '{map_name}', using entity origins");
            cam_mod::smooth_waypoints(cam_mod::nearest_neighbor_sort(entity_origins), 3)
        };
        let n = waypoints.len();
        Some(
            Camera::new(waypoints, cfg.camera_speed, cfg.bob_amplitude, cfg.bob_frequency)
                .map_err(|_| {
                    let msg = format!(
                        "not enough waypoints for spline camera: need >= 4, got {n}"
                    );
                    capture::print_error_json(&msg);
                    anyhow::anyhow!("{msg}")
                })?,
        )
    } else {
        None
    };

    // Init GPU without a surface
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12 | wgpu::Backends::VULKAN,
        ..Default::default()
    });
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        })
        .await
        .context("no GPU adapter available for headless rendering")?;

    let core = init_gpu_core(&adapter, &mesh, wgpu::TextureFormat::Rgba8UnormSrgb).await?;

    // Staging buffer: allocated once, reused across all frames
    let unpadded_bpr = args.width * 4;
    let padded_bpr = (unpadded_bpr + 255) & !255;
    let staging_buf = core.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staging"),
        size: (padded_bpr * args.height) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // Depth texture: allocated once (resolution is fixed for the whole run)
    let depth_tex = core.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless_depth"),
        size: wgpu::Extent3d {
            width: args.width,
            height: args.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth_tex.create_view(&Default::default());

    let mut manifest_frames: Vec<FrameEntry> = Vec::new();

    for frame_idx in 0..args.frame_count {
        // Advance spline between frames (frame 0 is at t=0, no advance before it)
        if frame_idx > 0 {
            if let Some(ref mut cam) = camera {
                for _ in 0..args.frame_step {
                    cam.update(1.0 / 60.0);
                }
            }
        }

        // Obtain camera pose for this frame
        let pose = if let Some(pos) = args.camera_pos {
            fixed_pose(pos, args.camera_angle_deg)
        } else {
            camera.as_mut().unwrap().update(0.0)
        };

        // Compute VP matrix
        let aspect = args.width as f32 / args.height as f32;
        let fov_y = 2.0 * (1.0_f32 / aspect).atan();
        let proj = Mat4::perspective_rh(fov_y, aspect, 4.0, 4096.0);
        let vp: [[f32; 4]; 4] = (proj * pose.view).to_cols_array_2d();
        core.queue.write_buffer(&core.vp_buf, 0, bytemuck::cast_slice(&vp));

        // Allocate fresh color texture for this frame (discarded after readback)
        let color_tex = core.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless_color"),
            size: wgpu::Extent3d {
                width: args.width,
                height: args.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color_tex.create_view(&Default::default());

        // Render
        let mut encoder = core
            .device
            .create_command_encoder(&Default::default());
        core.encode_frame(&mut encoder, &color_view, &depth_view);

        // Copy rendered texture to staging buffer
        encoder.copy_texture_to_buffer(
            color_tex.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &staging_buf,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bpr),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width: args.width,
                height: args.height,
                depth_or_array_layers: 1,
            },
        );
        core.queue.submit(std::iter::once(encoder.finish()));

        // GPU readback sequence (mandatory order: map_async -> poll -> recv -> get_mapped_range)
        let staging_slice = staging_buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        staging_slice.map_async(wgpu::MapMode::Read, move |r| {
            tx.send(r).unwrap();
        });
        let _ = core.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        rx.recv().unwrap().unwrap();

        // Strip alignment padding and write PNG
        let filename = format!("frame_{frame_idx:04}.png");
        let out_path = args.output_dir.join(&filename);
        {
            let raw = staging_slice.get_mapped_range();
            let pixels = capture::strip_padding(&raw, args.width, args.height, padded_bpr);
            capture::write_png(&out_path, &pixels, args.width, args.height)?;
        }
        staging_buf.unmap();

        // Emit JSON line to stdout
        let yaw_deg = pose.yaw.to_degrees();
        let pitch_deg = pose.pitch.to_degrees();
        capture::print_frame_json(frame_idx, pose.eye, yaw_deg, pitch_deg, &out_path);

        manifest_frames.push(FrameEntry {
            frame: frame_idx,
            pos: [pose.eye.x, pose.eye.y, pose.eye.z],
            angle: [yaw_deg, pitch_deg],
            file: filename,
        });
    }

    // Write walkthrough manifest (only for spline walkthrough mode)
    if args.camera_pos.is_none() && args.walkthrough {
        let manifest_path = args.output_dir.join("walkthrough.json");
        capture::write_manifest(
            &manifest_path,
            &map_name,
            args.width,
            args.height,
            args.frame_step,
            &manifest_frames,
        )?;
    }

    Ok(())
}

/// Build a CameraPose directly from fixed-pose CLI args.
/// `pos` is used as the exact eye position (no eye-height offset added — the user controls
/// the position via --camera-pos, unlike spline mode where pos is a waypoint + 64 units up).
/// `angle_deg` is [yaw, pitch] in degrees; defaults to looking +X if omitted.
fn fixed_pose(pos: [f32; 3], angle_deg: Option<[f32; 2]>) -> CameraPose {
    let [x, y, z] = pos;
    let eye = Vec3::new(x, y, z);

    let (yaw, pitch) = if let Some([yaw_d, pitch_d]) = angle_deg {
        (yaw_d.to_radians(), pitch_d.to_radians())
    } else {
        (0.0_f32, 0.0_f32)
    };

    let forward = Vec3::new(yaw.cos() * pitch.cos(), yaw.sin() * pitch.cos(), pitch.sin());

    let view = Mat4::look_at_rh(eye, eye + forward, Vec3::Z);
    CameraPose { view, eye, yaw, pitch }
}
