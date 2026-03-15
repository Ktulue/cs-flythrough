use anyhow::{Context, Result};
use glam::Mat4;
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    window::{Fullscreen, Window, WindowId},
};

use crate::bsp::parse::{MeshData, Vertex};
use crate::camera::Camera;
use crate::input::should_exit_on_mouse;

pub fn run(mesh: MeshData, camera: Camera) -> Result<()> {
    let event_loop = EventLoop::new().context("creating event loop")?;
    let mut app = App::new(mesh, camera);
    event_loop.run_app(&mut app).context("event loop error")?;
    Ok(())
}

/// Surface-agnostic GPU state. Shared between windowed and headless modes.
/// Pipelines are compiled against `output_format` at construction time — a pipeline
/// compiled for one format cannot be used with a render pass targeting a different format.
pub(crate) struct GpuCore {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    geo_index_count: u32,
    sky_index_count: u32,
    sky_index_offset: u32,
    pub vp_buf: wgpu::Buffer,
    geo_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    geo_bind_group: wgpu::BindGroup,
    sky_bind_group: wgpu::BindGroup,
}

impl GpuCore {
    /// Record the geometry and sky render passes into `encoder`.
    /// `color_view` is the render target (surface texture or offscreen texture).
    /// `depth_view` must match the resolution of `color_view`.
    pub(crate) fn encode_frame(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
    ) {
        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // Desert-sky blue: matches de_dust2's horizon colour and hides
                    // BSP void (below-floor gaps) without a jarring black border.
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.47,
                        g: 0.63,
                        b: 0.78,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        rpass.set_vertex_buffer(0, self.vertex_buf.slice(..));
        rpass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint32);
        rpass.set_pipeline(&self.geo_pipeline);
        rpass.set_bind_group(0, &self.geo_bind_group, &[]);
        rpass.draw_indexed(0..self.geo_index_count, 0, 0..1);
        rpass.set_pipeline(&self.sky_pipeline);
        rpass.set_bind_group(0, &self.sky_bind_group, &[]);
        rpass.draw_indexed(
            self.sky_index_offset..self.sky_index_offset + self.sky_index_count,
            0,
            0..1,
        );
    }
}

struct GpuState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    core: GpuCore,
    config: wgpu::SurfaceConfiguration,
    #[allow(dead_code)]
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
}

struct App {
    mesh: Option<MeshData>,
    camera: Camera,
    gpu: Option<GpuState>,
    last_frame: std::time::Instant,
    shutdown: bool,
    mouse_grace_until: std::time::Instant,
}

impl App {
    fn new(mesh: MeshData, camera: Camera) -> Self {
        Self {
            mesh: Some(mesh),
            camera,
            gpu: None,
            last_frame: std::time::Instant::now(),
            shutdown: false,
            mouse_grace_until: std::time::Instant::now(),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // resumed requires no field-access changes — it only constructs GpuState by
        // calling init_gpu, which now returns the new struct layout.
        let window_attrs = Window::default_attributes()
            .with_title("cs-flythrough")
            .with_fullscreen(Some(Fullscreen::Borderless(None)));
        let window = Arc::new(
            event_loop
                .create_window(window_attrs)
                .expect("window creation"),
        );
        window.set_cursor_visible(false);

        let mesh = self.mesh.take().expect("mesh already consumed");
        let gpu = pollster::block_on(init_gpu(window, mesh)).expect("GPU init failed");
        self.gpu = Some(gpu);
        self.mouse_grace_until =
            std::time::Instant::now() + std::time::Duration::from_millis(1000);
        crate::diag!("[cs-flythrough] resumed: GPU ready, grace until +1000ms");
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                crate::diag!("[cs-flythrough] event: CloseRequested -> exiting");
                event_loop.exit();
            }
            WindowEvent::KeyboardInput { event: ref ke, is_synthetic, .. } => {
                crate::diag!("[cs-flythrough] event: KeyboardInput synthetic={is_synthetic} state={:?}", ke.state);
                let in_grace = std::time::Instant::now() < self.mouse_grace_until;
                if !is_synthetic && !in_grace && ke.state == winit::event::ElementState::Pressed {
                    crate::diag!("[cs-flythrough] KeyboardInput: real keypress -> shutdown");
                    self.shutdown = true;
                }
            }
            WindowEvent::Resized(new_size) => {
                crate::diag!("[cs-flythrough] event: Resized {}x{}", new_size.width, new_size.height);
                if let Some(gpu) = &mut self.gpu {
                    gpu.config.width = new_size.width;
                    gpu.config.height = new_size.height;
                    gpu.surface.configure(&gpu.core.device, &gpu.config);
                    let (dt, dv) =
                        create_depth_texture(&gpu.core.device, new_size.width, new_size.height);
                    gpu.depth_texture = dt;
                    gpu.depth_view = dv;
                }
            }
            WindowEvent::RedrawRequested => {
                if self.shutdown {
                    crate::diag!("[cs-flythrough] RedrawRequested: shutdown=true -> exiting");
                    event_loop.exit();
                    return;
                }
                let gpu = match &mut self.gpu {
                    Some(g) => g,
                    None => return,
                };
                let now = std::time::Instant::now();
                let delta_secs = (now - self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;

                let pose = self.camera.update(delta_secs);
                let aspect = gpu.config.width as f32 / gpu.config.height as f32;
                let fov_y = 2.0 * (1.0_f32 / aspect).atan();
                let proj = Mat4::perspective_rh(fov_y, aspect, 4.0, 4096.0);
                let vp: [[f32; 4]; 4] = (proj * pose.view).to_cols_array_2d();
                gpu.core.queue.write_buffer(&gpu.core.vp_buf, 0, bytemuck::cast_slice(&vp));

                let frame = match gpu.surface.get_current_texture() {
                    Ok(f) => f,
                    Err(_) => return,
                };
                let view_tex = frame.texture.create_view(&Default::default());
                let mut encoder = gpu.core.device.create_command_encoder(&Default::default());
                gpu.core.encode_frame(&mut encoder, &view_tex, &gpu.depth_view);
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
            let grace_remaining = self.mouse_grace_until.saturating_duration_since(std::time::Instant::now());
            if grace_remaining.is_zero() && should_exit_on_mouse(delta) {
                crate::diag!("[cs-flythrough] MouseMotion delta={:?} -> shutdown", delta);
                self.shutdown = true;
            } else if !grace_remaining.is_zero() {
                crate::diag!("[cs-flythrough] MouseMotion delta={:?} grace={}ms (ignored)", delta, grace_remaining.as_millis());
            }
        }
    }
}

/// Initialize surface-agnostic GPU state.
/// `output_format` is baked into the render pipelines at compile time.
/// Windowed path passes surface_format from caps. Headless path passes Rgba8UnormSrgb.
pub(crate) async fn init_gpu_core(
    adapter: &wgpu::Adapter,
    mesh: &MeshData,
    output_format: wgpu::TextureFormat,
) -> Result<GpuCore> {
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .context("creating wgpu device")?;

    let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("vertex"),
        contents: bytemuck::cast_slice(&mesh.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("index"),
        contents: bytemuck::cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });

    let geo_index_count = mesh.sky_index_offset;
    let sky_index_count = mesh.indices.len() as u32 - mesh.sky_index_offset;
    let sky_index_offset = mesh.sky_index_offset;

    crate::diag!("[cs-flythrough] vertices: {}  geo_indices: {}  sky_indices: {}", mesh.vertices.len(), geo_index_count, sky_index_count);
    crate::diag!("[cs-flythrough] diffuse atlas: {}x{}  lightmap atlas: {}x{}", mesh.diffuse_atlas.width(), mesh.diffuse_atlas.height(), mesh.lightmap_atlas.width(), mesh.lightmap_atlas.height());

    let diffuse_tex = upload_rgba_texture(&device, &queue, &mesh.diffuse_atlas, "diffuse");
    let lightmap_tex = upload_rgba_texture(&device, &queue, &mesh.lightmap_atlas, "lightmap");
    let diffuse_view = diffuse_tex.create_view(&Default::default());
    let lightmap_view = lightmap_tex.create_view(&Default::default());

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let vp_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("view_proj"),
        size: 64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let vertex_attrs = [
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 12, shader_location: 1 },
        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 20, shader_location: 2 },
    ];

    let geo_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("geo_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { multisampled: false, view_dimension: wgpu::TextureViewDimension::D2, sample_type: wgpu::TextureSampleType::Float { filterable: true } }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
            wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { multisampled: false, view_dimension: wgpu::TextureViewDimension::D2, sample_type: wgpu::TextureSampleType::Float { filterable: true } }, count: None },
            wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
        ],
    });

    let geo_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("geo_bg"),
        layout: &geo_bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: vp_buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&diffuse_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&lightmap_view) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&sampler) },
        ],
    });

    let sky_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sky_bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });

    let sky_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sky_bg"),
        layout: &sky_bgl,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: vp_buf.as_entire_binding() }],
    });

    let geo_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("geometry"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/geometry.wgsl").into()),
    });
    let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sky"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/sky.wgsl").into()),
    });

    let depth_format = wgpu::TextureFormat::Depth32Float;

    let geo_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("geo_layout"),
        bind_group_layouts: &[&geo_bgl],
        immediate_size: 0,
    });
    let sky_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sky_layout"),
        bind_group_layouts: &[&sky_bgl],
        immediate_size: 0,
    });

    // vertex_attrs referenced directly in each pipeline — do NOT use a closure
    let geo_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("geo"),
        layout: Some(&geo_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &geo_shader,
            entry_point: Some("vs_main"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &vertex_attrs,
            }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &geo_shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sky"),
        layout: Some(&sky_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &sky_shader,
            entry_point: Some("vs_main"),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &vertex_attrs,
            }],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &sky_shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: output_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::Less,
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    Ok(GpuCore {
        device,
        queue,
        vertex_buf,
        index_buf,
        geo_index_count,
        sky_index_count,
        sky_index_offset,
        vp_buf,
        geo_pipeline,
        sky_pipeline,
        geo_bind_group,
        sky_bind_group,
    })
}

async fn init_gpu(window: Arc<Window>, mesh: MeshData) -> Result<GpuState> {
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
        .context("no compatible GPU adapter — ensure DirectX 12 or Vulkan drivers are installed")?;

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

    crate::diag!("[cs-flythrough] surface: {:?}  size: {}x{}", surface_format, config.width, config.height);

    // init_gpu_core creates the wgpu device internally — do NOT call request_device here.
    let core = init_gpu_core(&adapter, &mesh, surface_format).await?;
    // Configure the surface exactly once, using the device created by init_gpu_core.
    surface.configure(&core.device, &config);

    let (depth_texture, depth_view) = create_depth_texture(&core.device, config.width, config.height);

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

fn upload_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    img: &image::RgbaImage,
    label: &str,
) -> wgpu::Texture {
    let (width, height) = img.dimensions();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        img.as_raw(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width.max(1)),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
    );
    texture
}
