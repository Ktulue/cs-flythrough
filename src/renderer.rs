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

struct GpuState {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    geo_index_count: u32,
    sky_index_count: u32,
    sky_index_offset: u32,
    vp_buf: wgpu::Buffer,
    geo_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    geo_bind_group: wgpu::BindGroup,
    sky_bind_group: wgpu::BindGroup,
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
}

impl App {
    fn new(mesh: MeshData, camera: Camera) -> Self {
        Self {
            mesh: Some(mesh),
            camera,
            gpu: None,
            last_frame: std::time::Instant::now(),
            shutdown: false,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
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
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { .. } => {
                self.shutdown = true;
            }
            WindowEvent::Resized(new_size) => {
                if let Some(gpu) = &mut self.gpu {
                    gpu.config.width = new_size.width;
                    gpu.config.height = new_size.height;
                    gpu.surface.configure(&gpu.device, &gpu.config);
                    let (dt, dv) =
                        create_depth_texture(&gpu.device, new_size.width, new_size.height);
                    gpu.depth_texture = dt;
                    gpu.depth_view = dv;
                }
            }
            WindowEvent::RedrawRequested => {
                if self.shutdown {
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

                let view = self.camera.update(delta_secs);
                let aspect = gpu.config.width as f32 / gpu.config.height as f32;
                // CS 1.6 uses 90° horizontal FOV. perspective_rh takes vertical FOV,
                // so derive fov_y from fov_x: fov_y = 2 * atan(tan(fov_x/2) / aspect).
                // tan(90°/2) = tan(45°) = 1.0, simplifying to: fov_y = 2 * atan(1/aspect).
                let fov_y = 2.0 * (1.0_f32 / aspect).atan();
                let proj = Mat4::perspective_rh(fov_y, aspect, 4.0, 4096.0);
                let vp: [[f32; 4]; 4] = (proj * view).to_cols_array_2d();
                gpu.queue
                    .write_buffer(&gpu.vp_buf, 0, bytemuck::cast_slice(&vp));

                let frame = match gpu.surface.get_current_texture() {
                    Ok(f) => f,
                    Err(_) => return,
                };
                let view_tex = frame.texture.create_view(&Default::default());
                let mut encoder = gpu.device.create_command_encoder(&Default::default());

                {
                    let mut rpass =
                        encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: None,
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &view_tex,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color {
                                        r: 0.0,
                                        g: 0.0,
                                        b: 0.0,
                                        a: 1.0,
                                    }),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: Some(
                                wgpu::RenderPassDepthStencilAttachment {
                                    view: &gpu.depth_view,
                                    depth_ops: Some(wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(1.0),
                                        store: wgpu::StoreOp::Discard,
                                    }),
                                    stencil_ops: None,
                                },
                            ),
                            ..Default::default()
                        });

                    rpass.set_vertex_buffer(0, gpu.vertex_buf.slice(..));
                    rpass.set_index_buffer(
                        gpu.index_buf.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );

                    // Geometry pass (diffuse × lightmap)
                    rpass.set_pipeline(&gpu.geo_pipeline);
                    rpass.set_bind_group(0, &gpu.geo_bind_group, &[]);
                    rpass.draw_indexed(0..gpu.geo_index_count, 0, 0..1);

                    // Sky pass (flat color) — encoded in the same wgpu render pass as geometry.
                    // BSP sky faces are at the map boundary and don't overlap interior geometry,
                    // so sharing a depth buffer with geometry is correct and depth-fight-free.
                    rpass.set_pipeline(&gpu.sky_pipeline);
                    rpass.set_bind_group(0, &gpu.sky_bind_group, &[]);
                    rpass.draw_indexed(
                        gpu.sky_index_offset..gpu.sky_index_offset + gpu.sky_index_count,
                        0,
                        0..1,
                    );
                }

                gpu.queue.submit(std::iter::once(encoder.finish()));
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
            if should_exit_on_mouse(delta) {
                self.shutdown = true;
            }
        }
    }
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
        .context(
            "no compatible GPU adapter — ensure DirectX 12 or Vulkan drivers are installed",
        )?;

    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor::default())
        .await
        .context("creating wgpu device")?;

    let size = window.inner_size();
    let caps = surface.get_capabilities(&adapter);
    let surface_format = caps.formats[0];
    let config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width: size.width.max(1),
        height: size.height.max(1),
        present_mode: wgpu::PresentMode::AutoVsync,
        alpha_mode: caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);

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

    let diffuse_tex = upload_rgba_texture(&device, &queue, &mesh.diffuse_atlas, "diffuse");
    let lightmap_tex =
        upload_rgba_texture(&device, &queue, &mesh.lightmap_atlas, "lightmap");
    let diffuse_view = diffuse_tex.create_view(&Default::default());
    let lightmap_view = lightmap_tex.create_view(&Default::default());

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    // ViewProj uniform buffer — Mat4 = 16 × f32 = 64 bytes
    let vp_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("view_proj"),
        size: 64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // VertexBufferLayout attributes — defined once, referenced by both pipelines.
    // VertexBufferLayout borrows the attributes slice so we cannot clone it; instead
    // we build a separate layout value for each pipeline using the same attributes array.
    let vertex_attrs = [
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 12,
            shader_location: 1,
        },
        wgpu::VertexAttribute {
            format: wgpu::VertexFormat::Float32x2,
            offset: 20,
            shader_location: 2,
        },
    ];

    // Geometry bind group layout: uniform + diffuse texture + sampler + lightmap texture + sampler
    let geo_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("geo_bgl"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    let geo_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("geo_bg"),
        layout: &geo_bgl,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: vp_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&diffuse_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&lightmap_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    // Sky bind group layout: uniform only
    let sky_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("sky_bgl"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });

    let sky_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("sky_bg"),
        layout: &sky_bgl,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: vp_buf.as_entire_binding(),
        }],
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

    let geo_pipeline_layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("geo_layout"),
            bind_group_layouts: &[&geo_bgl],
            immediate_size: 0,
        });
    let sky_pipeline_layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sky_layout"),
            bind_group_layouts: &[&sky_bgl],
            immediate_size: 0,
        });

    let geo_pipeline =
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                    format: surface_format,
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

    let sky_pipeline =
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                    format: surface_format,
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

    let (depth_texture, depth_view) =
        create_depth_texture(&device, config.width, config.height);

    Ok(GpuState {
        window,
        surface,
        device,
        queue,
        config,
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
