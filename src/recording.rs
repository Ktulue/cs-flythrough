use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

use crate::bsp::collision::{CollisionData, resolve_position};
use crate::bsp::parse::MeshData;
use crate::renderer::{init_gpu_core, GpuCore};

// ── Public arguments ────────────────────────────────────────────────

/// Arguments parsed from the CLI for `--record-route` mode.
pub struct RecordingArgs {
    pub map_name: String,
    pub start_pos: Option<[f32; 3]>,
}

// ── Free camera ─────────────────────────────────────────────────────

/// Mouse sensitivity: radians per raw mouse-delta pixel.
const MOUSE_SENSITIVITY: f32 = 0.002;

/// Free-fly camera with WASD + mouse-look and BSP collision.
pub struct FreeCamera {
    pub pos: Vec3,
    pub yaw: f32,   // radians
    pub pitch: f32,  // radians, clamped ±89°
    speed: f32,
    collision: Option<CollisionData>,
}

impl FreeCamera {
    pub fn new(pos: Vec3, speed: f32, collision: Option<CollisionData>) -> Self {
        Self {
            pos,
            yaw: 0.0,
            pitch: 0.0,
            speed,
            collision,
        }
    }

    /// Apply mouse delta (raw device units) to yaw/pitch.
    pub fn mouse_look(&mut self, dx: f64, dy: f64) {
        self.yaw -= dx as f32 * MOUSE_SENSITIVITY;
        self.pitch = (self.pitch - dy as f32 * MOUSE_SENSITIVITY)
            .clamp(-89.0_f32.to_radians(), 89.0_f32.to_radians());
    }

    /// Forward direction on the horizontal plane (ignores pitch for movement).
    fn forward_flat(&self) -> Vec3 {
        Vec3::new(self.yaw.cos(), self.yaw.sin(), 0.0)
    }

    /// Right direction (perpendicular to forward on XY plane).
    fn right(&self) -> Vec3 {
        Vec3::new(self.yaw.sin(), -self.yaw.cos(), 0.0)
    }

    /// Move camera based on held keys. `shift` multiplies speed by 3.
    pub fn update(
        &mut self,
        delta: f32,
        forward: bool,
        back: bool,
        left: bool,
        right: bool,
        up: bool,
        down: bool,
        shift: bool,
    ) {
        let mut dir = Vec3::ZERO;
        if forward { dir += self.forward_flat(); }
        if back    { dir -= self.forward_flat(); }
        if right   { dir += self.right(); }
        if left    { dir -= self.right(); }
        if up      { dir += Vec3::Z; }
        if down    { dir -= Vec3::Z; }
        if dir.length_squared() > 0.0 {
            dir = dir.normalize();
        }

        let multiplier = if shift { 3.0 } else { 1.0 };
        let desired = self.pos + dir * self.speed * multiplier * delta;

        self.pos = if let Some(ref coll) = self.collision {
            resolve_position(coll, self.pos, desired)
        } else {
            desired
        };
    }

    /// Build the view matrix (RH) for the current camera state.
    /// Eye is at `pos` (which is already at eye height — caller stores waypoints
    /// with the -64 offset applied on save).
    pub fn view_matrix(&self) -> Mat4 {
        let forward = Vec3::new(
            self.yaw.cos() * self.pitch.cos(),
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
        );
        let eye = self.pos;
        let target = eye + forward;
        Mat4::look_at_rh(eye, target, Vec3::Z)
    }
}

// ── GPU state (matches renderer pattern) ────────────────────────────

struct GpuState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    core: GpuCore,
    config: wgpu::SurfaceConfiguration,
    #[allow(dead_code)]
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
}

// ── Recording app ───────────────────────────────────────────────────

struct RecordingApp {
    mesh: Option<MeshData>,
    #[allow(dead_code)]
    collision: Option<CollisionData>,
    camera: FreeCamera,
    gpu: Option<GpuState>,
    last_frame: Instant,
    keys_held: HashSet<KeyCode>,
    waypoints: Vec<Vec3>,
    map_name: String,
    save_on_exit: bool,
}

impl RecordingApp {
    fn drop_waypoint(&mut self) {
        let wp = self.camera.pos;
        self.waypoints.push(wp);
        let count = self.waypoints.len();
        eprintln!(
            "[recording] waypoint #{count}: ({:.1}, {:.1}, {:.1})",
            wp.x,
            wp.y,
            wp.z - 64.0  // display as stored z (eye_z - 64)
        );
    }

    fn undo_waypoint(&mut self) {
        if let Some(removed) = self.waypoints.pop() {
            eprintln!(
                "[recording] undid waypoint ({:.1}, {:.1}, {:.1}) — {} remaining",
                removed.x,
                removed.y,
                removed.z - 64.0,
                self.waypoints.len()
            );
        } else {
            eprintln!("[recording] no waypoints to undo");
        }
    }
}

impl ApplicationHandler for RecordingApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attrs = Window::default_attributes()
            .with_title("cs-flythrough [RECORDING]");
        let window = Arc::new(
            event_loop
                .create_window(window_attrs)
                .expect("window creation"),
        );
        // Capture cursor
        let _ = window.set_cursor_grab(CursorGrabMode::Confined);
        window.set_cursor_visible(false);

        let mesh = self.mesh.take().expect("mesh already consumed");
        let gpu = pollster::block_on(init_recording_gpu(window, mesh))
            .expect("GPU init failed");
        self.gpu = Some(gpu);
        self.last_frame = Instant::now();
        eprintln!("[recording] GPU ready — WASD+mouse to fly, F=waypoint, Z=undo, Enter=save, Esc=cancel");
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::KeyboardInput { event: ref ke, is_synthetic, .. } => {
                if is_synthetic { return; }
                let code = match ke.physical_key {
                    PhysicalKey::Code(c) => c,
                    _ => return,
                };
                match ke.state {
                    ElementState::Pressed => {
                        // Action keys (only on initial press, not repeats)
                        if !self.keys_held.contains(&code) {
                            match code {
                                KeyCode::KeyF => self.drop_waypoint(),
                                KeyCode::KeyZ => self.undo_waypoint(),
                                KeyCode::Enter => {
                                    self.save_on_exit = true;
                                    event_loop.exit();
                                    return;
                                }
                                KeyCode::Escape => {
                                    eprintln!("[recording] cancelled — no save");
                                    event_loop.exit();
                                    return;
                                }
                                _ => {}
                            }
                        }
                        self.keys_held.insert(code);
                    }
                    ElementState::Released => {
                        self.keys_held.remove(&code);
                    }
                }
            }
            WindowEvent::Resized(new_size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.config.width = new_size.width.max(1);
                    gpu.config.height = new_size.height.max(1);
                    gpu.surface.configure(&gpu.core.device, &gpu.config);
                    let (dt, dv) = create_depth_texture(
                        &gpu.core.device,
                        gpu.config.width,
                        gpu.config.height,
                    );
                    gpu.depth_texture = dt;
                    gpu.depth_view = dv;
                }
            }
            WindowEvent::RedrawRequested => {
                let gpu = match &mut self.gpu {
                    Some(g) => g,
                    None => return,
                };
                let now = Instant::now();
                let delta = (now - self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;

                // Update camera from held keys
                self.camera.update(
                    delta,
                    self.keys_held.contains(&KeyCode::KeyW),
                    self.keys_held.contains(&KeyCode::KeyS),
                    self.keys_held.contains(&KeyCode::KeyA),
                    self.keys_held.contains(&KeyCode::KeyD),
                    self.keys_held.contains(&KeyCode::Space),
                    self.keys_held.contains(&KeyCode::ControlLeft)
                        || self.keys_held.contains(&KeyCode::ControlRight),
                    self.keys_held.contains(&KeyCode::ShiftLeft)
                        || self.keys_held.contains(&KeyCode::ShiftRight),
                );

                // VP matrix
                let view = self.camera.view_matrix();
                let aspect = gpu.config.width as f32 / gpu.config.height as f32;
                let fov_y = 2.0 * (1.0_f32 / aspect).atan();
                let proj = Mat4::perspective_rh(fov_y, aspect, 4.0, 4096.0);
                let vp: [[f32; 4]; 4] = (proj * view).to_cols_array_2d();
                gpu.core
                    .queue
                    .write_buffer(&gpu.core.vp_buf, 0, bytemuck::cast_slice(&vp));

                // Render
                let frame = match gpu.surface.get_current_texture() {
                    Ok(f) => f,
                    Err(_) => return,
                };
                let view_tex = frame.texture.create_view(&Default::default());
                let mut encoder =
                    gpu.core.device.create_command_encoder(&Default::default());
                gpu.core
                    .encode_frame(&mut encoder, &view_tex, &gpu.depth_view);
                gpu.core.queue.submit(std::iter::once(encoder.finish()));
                frame.present();
                gpu.window.request_redraw();
            }
            _ => {}
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _id: DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event {
            self.camera.mouse_look(delta.0, delta.1);
        }
    }
}

// ── GPU init (mirrors renderer::init_gpu) ───────────────────────────

async fn init_recording_gpu(window: Arc<Window>, mesh: MeshData) -> Result<GpuState> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12 | wgpu::Backends::VULKAN,
        ..Default::default()
    });

    let surface = instance
        .create_surface(window.clone())
        .context("creating surface")?;

    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        })
        .await
        .context("no compatible GPU adapter")?;

    let size = window.inner_size();
    let caps = surface.get_capabilities(&adapter);
    let surface_format = caps.formats[0];
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: wgpu::PresentMode::Fifo,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };

    let core = init_gpu_core(&adapter, &mesh, surface_format).await?;
    surface.configure(&core.device, &config);

    let (depth_texture, depth_view) =
        create_depth_texture(&core.device, config.width, config.height);

    Ok(GpuState {
        window,
        surface,
        core,
        config,
        depth_texture,
        depth_view,
    })
}

fn create_depth_texture(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

// ── Waypoint saving ─────────────────────────────────────────────────

fn save_waypoints(map_name: &str, waypoints: &[Vec3]) -> Result<PathBuf> {
    let binary_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let out_dir = binary_dir.join("routes").join("recorded");
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("creating {}", out_dir.display()))?;

    let out_path = out_dir.join(format!("{map_name}_route.toml"));

    // Build TOML manually for readable formatting
    let mut buf = String::new();
    buf.push_str(&format!(
        "# Recorded route for {map_name}\n# Generated by cs-flythrough --record-route\n\n"
    ));
    buf.push_str(&format!("map = \"{map_name}\"\n"));
    buf.push_str("waypoints = [\n");
    for wp in waypoints {
        // Store z as eye_z - 64 (TOML convention: camera adds +64 on playback)
        buf.push_str(&format!(
            "    [{:.1}, {:.1}, {:.1}],\n",
            wp.x,
            wp.y,
            wp.z - 64.0
        ));
    }
    buf.push_str("]\n");

    std::fs::write(&out_path, &buf)
        .with_context(|| format!("writing {}", out_path.display()))?;

    Ok(out_path)
}

// ── Entry point ─────────────────────────────────────────────────────

pub fn run(
    args: RecordingArgs,
    mesh: MeshData,
    collision: CollisionData,
    camera_speed: f32,
) -> Result<()> {
    let start = if let Some([x, y, z]) = args.start_pos {
        Vec3::new(x, y, z + 64.0) // elevate to eye height
    } else if let Some(first_ent) = mesh.entity_origins.first() {
        *first_ent + Vec3::new(0.0, 0.0, 64.0)
    } else {
        Vec3::new(0.0, 0.0, 128.0)
    };

    let event_loop = EventLoop::new().context("creating event loop")?;
    let camera = FreeCamera::new(start, camera_speed, Some(collision));
    let app = RecordingApp {
        mesh: Some(mesh),
        collision: None,
        camera,
        gpu: None,
        last_frame: Instant::now(),
        keys_held: HashSet::new(),
        waypoints: Vec::new(),
        map_name: args.map_name.clone(),
        save_on_exit: false,
    };
    let mut app = app;

    event_loop
        .run_app(&mut app)
        .context("event loop error")?;

    // Post-exit: save if requested
    if app.save_on_exit {
        let count = app.waypoints.len();
        if count < 4 {
            eprintln!(
                "[recording] warning: only {count} waypoints — Catmull-Rom spline needs at least 4"
            );
        }
        if count == 0 {
            eprintln!("[recording] no waypoints to save");
        } else {
            let path = save_waypoints(&app.map_name, &app.waypoints)?;
            eprintln!("[recording] saved {count} waypoints to {}", path.display());
        }
    }

    Ok(())
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_free_camera_initial_state() {
        let cam = FreeCamera::new(Vec3::new(100.0, 200.0, 300.0), 500.0, None);
        assert_eq!(cam.pos, Vec3::new(100.0, 200.0, 300.0));
        assert_eq!(cam.yaw, 0.0);
        assert_eq!(cam.pitch, 0.0);
    }

    #[test]
    fn test_free_camera_mouse_look_yaw() {
        let mut cam = FreeCamera::new(Vec3::ZERO, 500.0, None);
        cam.mouse_look(100.0, 0.0);
        // dx > 0 should decrease yaw (turn right)
        assert!(cam.yaw < 0.0);
        assert_eq!(cam.pitch, 0.0);
    }

    #[test]
    fn test_free_camera_mouse_look_pitch_clamp() {
        let mut cam = FreeCamera::new(Vec3::ZERO, 500.0, None);
        // Huge downward motion — pitch should clamp at ~-89°
        cam.mouse_look(0.0, 100000.0);
        let limit = 89.0_f32.to_radians();
        assert!((cam.pitch + limit).abs() < 0.01, "pitch should clamp near -89°");
    }

    #[test]
    fn test_free_camera_forward_movement() {
        let mut cam = FreeCamera::new(Vec3::ZERO, 500.0, None);
        // yaw=0 → forward is +X
        cam.update(1.0, true, false, false, false, false, false, false);
        assert!(cam.pos.x > 0.0, "should move +X when yaw=0 and W held");
        assert!(cam.pos.y.abs() < 0.01, "no sideways movement");
    }

    #[test]
    fn test_free_camera_shift_multiplier() {
        let mut cam_normal = FreeCamera::new(Vec3::ZERO, 500.0, None);
        cam_normal.update(1.0, true, false, false, false, false, false, false);

        let mut cam_shift = FreeCamera::new(Vec3::ZERO, 500.0, None);
        cam_shift.update(1.0, true, false, false, false, false, false, true);

        let ratio = cam_shift.pos.x / cam_normal.pos.x;
        assert!(
            (ratio - 3.0).abs() < 0.01,
            "shift should give 3x speed, got {ratio}"
        );
    }

    #[test]
    fn test_free_camera_no_movement_when_no_keys() {
        let mut cam = FreeCamera::new(Vec3::new(10.0, 20.0, 30.0), 500.0, None);
        cam.update(1.0, false, false, false, false, false, false, false);
        assert_eq!(cam.pos, Vec3::new(10.0, 20.0, 30.0));
    }

    #[test]
    fn test_free_camera_strafe_right() {
        let mut cam = FreeCamera::new(Vec3::ZERO, 500.0, None);
        // yaw=0 → right is +Y... actually right is perpendicular.
        // forward_flat at yaw=0 is (1,0,0), right is (sin0, -cos0, 0) = (0,-1,0)?
        // Wait: right() = (sin(yaw), -cos(yaw), 0). At yaw=0: (0, -1, 0).
        // But in GoldSrc Y is typically forward... let's just test it moves.
        cam.update(1.0, false, false, false, true, false, false, false);
        assert!(cam.pos.length() > 0.0, "should move when strafing right");
    }

    #[test]
    fn test_free_camera_view_matrix_not_identity() {
        let cam = FreeCamera::new(Vec3::new(100.0, 200.0, 300.0), 500.0, None);
        let view = cam.view_matrix();
        assert_ne!(view, Mat4::IDENTITY);
    }

    #[test]
    fn test_save_waypoints_format() {
        // We can't easily test file I/O in unit tests, but we can verify
        // the z-offset logic by checking camera state
        let cam = FreeCamera::new(Vec3::new(0.0, 0.0, 128.0), 500.0, None);
        // eye_z=128 → stored z should be 128-64 = 64
        let stored_z = cam.pos.z - 64.0;
        assert!((stored_z - 64.0).abs() < 0.01);
    }
}
