# Headless Rendering Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `--headless` CLI mode that renders CS 1.6 BSP maps to PNG files without a window, enabling autonomous visual bug detection.

**Architecture:** Extract a surface-agnostic `GpuCore` from `renderer.rs` (with an `encode_frame` method that takes any color+depth `TextureView`), add `CameraPose` return type to `camera::Camera::update()`, then build `headless.rs` (render loop) and `capture.rs` (PNG/JSON output) on top. The windowed path is mechanically updated to use `gpu.core.X` field access — zero logic changes.

**Tech Stack:** Rust, wgpu 28 (DX12/Vulkan), glam 0.32, image 0.25 — all already in `Cargo.toml`. No new dependencies.

---

## Chunk 1: Foundation — CameraPose + capture.rs

### Task 1: Update .gitignore

**Files:**
- Modify: `.gitignore`

- [ ] **Step 1: Add captures/ and bug-report.json to .gitignore**

Append to `.gitignore`:
```
captures/
bug-report.json
```

- [ ] **Step 2: Commit**
```bash
git add .gitignore
git commit -m "chore: gitignore captures/ and bug-report.json"
```

---

### Task 2: Add CameraPose to camera.rs

`Camera::update()` currently returns `Mat4`. We change it to return a `CameraPose` struct that also carries `eye`, `yaw`, and `pitch` — values already computed internally, costing nothing to expose.

**Files:**
- Modify: `src/camera.rs`

- [ ] **Step 1: Write the failing tests**

In `src/camera.rs`, update the `tests` module. Replace `test_update_returns_matrix` and add two new tests:

```rust
#[test]
fn test_update_returns_pose_with_view() {
    let mut cam = Camera::new(four_square_pts(), 133.0, 2.0, 2.0).unwrap();
    let pose = cam.update(0.016);
    // view matrix should not be identity (camera is positioned)
    assert_ne!(pose.view, Mat4::IDENTITY);
}

#[test]
fn test_update_pose_eye_is_above_waypoint() {
    // Waypoints in the XY plane at Z=0; eye should be Z > 0 (64 + bob)
    let mut cam = Camera::new(four_square_pts(), 133.0, 0.0, 0.0).unwrap();
    let pose = cam.update(0.0);
    assert!(pose.eye.z > 60.0, "eye z={} expected >60", pose.eye.z);
}

#[test]
fn test_update_pose_yaw_pitch_finite() {
    let mut cam = Camera::new(four_square_pts(), 133.0, 2.0, 2.0).unwrap();
    let pose = cam.update(0.016);
    assert!(pose.yaw.is_finite());
    assert!(pose.pitch.is_finite());
    // pitch must be in [-π/2, π/2]
    assert!(pose.pitch.abs() <= std::f32::consts::FRAC_PI_2 + 1e-5);
}
```

- [ ] **Step 2: Run tests to verify they fail**
```bash
cd "F:/GDriveClone/Claude_Code/cs-flythrough"
cargo test camera::tests 2>&1 | head -40
```
Expected: compile error — `CameraPose` not defined, `pose.view`/`pose.eye`/`pose.yaw` not found.

- [ ] **Step 3: Add CameraPose struct and update Camera::update()**

At the top of `src/camera.rs`, after the `use` statements, add:

```rust
/// Pose returned by Camera::update() each frame.
pub struct CameraPose {
    pub view: Mat4,
    pub eye: Vec3,
    /// Radians. atan2(forward.y, forward.x). JSON output should convert to degrees.
    pub yaw: f32,
    /// Radians. asin(forward.z.clamp(-1.0, 1.0)). JSON output should convert to degrees.
    pub pitch: f32,
}
```

Replace the body of `Camera::update` (currently returns `Mat4`) with:

```rust
pub fn update(&mut self, delta_secs: f32) -> CameraPose {
    let n = self.waypoints.len() as f32;
    self.t = (self.t + self.speed * delta_secs / (n * MAP_UNIT_SCALE)) % 1.0;

    let pos = catmull_rom_position(&self.waypoints, self.t);
    let forward = catmull_rom_tangent(&self.waypoints, self.t).normalize_or_zero();

    if self.first_update {
        self.first_update = false;
        crate::diag!("[cs-flythrough] camera pos: {:?}  forward: {:?}  t={:.4}  waypoints: {}", pos, forward, self.t, self.waypoints.len());
    }
    let elapsed = self.start_time.elapsed().as_secs_f32();
    let bob = self.bob_amplitude * (elapsed * self.bob_frequency * std::f32::consts::TAU).sin();

    let eye = pos + Vec3::new(0.0, 0.0, 64.0 + bob);
    let target = eye + forward;
    let up = Vec3::Z;

    let view = Mat4::look_at_rh(eye, target, up);
    let yaw = forward.y.atan2(forward.x);
    let pitch = forward.z.clamp(-1.0, 1.0).asin();

    CameraPose { view, eye, yaw, pitch }
}
```

- [ ] **Step 4: Fix the compiler error in renderer.rs**

In `src/renderer.rs`, find (around line 138):
```rust
let view = self.camera.update(delta_secs);
```
Replace with:
```rust
let pose = self.camera.update(delta_secs);
```

Find (around line 146):
```rust
let vp: [[f32; 4]; 4] = (proj * view).to_cols_array_2d();
```
Replace with:
```rust
let vp: [[f32; 4]; 4] = (proj * pose.view).to_cols_array_2d();
```

- [ ] **Step 5: Run tests**
```bash
cargo test camera::tests 2>&1
```
Expected: all camera tests PASS.

- [ ] **Step 6: Build check**
```bash
cargo build 2>&1
```
Expected: compiles cleanly.

- [ ] **Step 7: Commit**
```bash
git add src/camera.rs src/renderer.rs
git commit -m "feat: add CameraPose return type to Camera::update"
```

---

### Task 3: Create capture.rs

`capture.rs` owns three responsibilities: stripping wgpu row-alignment padding, writing PNG files, and serializing JSON (frame lines + walkthrough manifest). No GPU required — fully unit-testable.

**Files:**
- Create: `src/capture.rs`
- Modify: `src/main.rs` (add `mod capture;`)

- [ ] **Step 1: Write the failing tests**

Create `src/capture.rs` with only the test module (implementations stubbed):

```rust
use std::path::Path;
use anyhow::Result;
use glam::Vec3;

pub struct FrameEntry {
    pub frame: u32,
    pub pos: [f32; 3],
    pub angle: [f32; 2], // [yaw_deg, pitch_deg]
    pub file: String,
}

/// Strip wgpu 256-byte row-alignment padding from raw texture readback data.
/// Returns tightly-packed RGBA bytes (width * 4 * height).
pub fn strip_padding(padded: &[u8], width: u32, height: u32, padded_bpr: u32) -> Vec<u8> {
    todo!()
}

/// Write a PNG from tightly-packed RGBA pixel bytes (already stripped of padding).
pub fn write_png(path: &Path, pixels: &[u8], width: u32, height: u32) -> Result<()> {
    todo!()
}

/// Print a single-frame JSON line to stdout.
pub fn print_frame_json(frame: u32, eye: Vec3, yaw_deg: f32, pitch_deg: f32, file: &Path) {
    todo!()
}

/// Print a fatal error JSON line to stdout.
pub fn print_error_json(msg: &str) {
    todo!()
}

/// Write walkthrough manifest JSON to path.
pub fn write_manifest(
    path: &Path,
    map: &str,
    width: u32,
    height: u32,
    frame_step: u32,
    frames: &[FrameEntry],
) -> Result<()> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_padding_removes_alignment_bytes() {
        // 2×2 image: unpadded_bpr = 8, padded to 256 (wgpu minimum alignment)
        let width = 2u32;
        let height = 2u32;
        let padded_bpr = 256usize;
        let mut input = vec![0u8; padded_bpr * height as usize];
        input[..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);       // row 0 pixels
        input[256..264].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]); // row 1 pixels
        let result = strip_padding(&input, width, height, padded_bpr as u32);
        assert_eq!(result, vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    }

    #[test]
    fn test_strip_padding_no_padding_needed() {
        // 64×1 image: unpadded_bpr = 256, which happens to equal padded_bpr
        let width = 64u32;
        let height = 1u32;
        let padded_bpr = 256u32;
        let input: Vec<u8> = (0..256u8).collect();
        let result = strip_padding(&input, width, height, padded_bpr);
        assert_eq!(result, input);
    }

    #[test]
    fn test_write_png_roundtrip() {
        // 2×2 solid red RGBA image — write to temp file, read back, verify byte fidelity.
        // Note: write_png treats bytes as opaque RGBA data and saves them as-is.
        // The sRGB encoding is applied by the GPU (Rgba8UnormSrgb render target) before
        // strip_padding is called — by the time bytes reach write_png they are already
        // sRGB-encoded. This test verifies byte fidelity, not sRGB correctness
        // (the latter requires a real GPU and is validated by manual inspection of captures).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.png");
        let pixels: Vec<u8> = vec![255, 0, 0, 255].repeat(4); // 4 red pixels
        write_png(&path, &pixels, 2, 2).unwrap();
        let img = image::open(&path).unwrap().to_rgba8();
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn test_write_manifest_structure() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("walkthrough.json");
        let frames = vec![
            FrameEntry { frame: 0, pos: [1.0, 2.0, 3.0], angle: [90.0, 0.0], file: "frame_0000.png".into() },
            FrameEntry { frame: 1, pos: [4.0, 5.0, 6.0], angle: [91.0, 1.0], file: "frame_0001.png".into() },
        ];
        write_manifest(&path, "de_dust2", 1920, 1080, 60, &frames).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        // Structural JSON validity: starts with { ends with }
        assert!(content.trim_start().starts_with('{'), "manifest must start with {{");
        assert!(content.trim_end().ends_with('}'), "manifest must end with }}");
        // Verify bracket balance: count { and }
        let opens = content.chars().filter(|&c| c == '{').count();
        let closes = content.chars().filter(|&c| c == '}').count();
        assert_eq!(opens, closes, "unbalanced braces in manifest JSON");
        // Field presence
        assert!(content.contains("\"map\": \"de_dust2\""));
        assert!(content.contains("\"resolution\": \"1920x1080\""));
        assert!(content.contains("\"frame_step\": 60"));
        assert!(content.contains("\"frame\": 0"));
        assert!(content.contains("\"frame\": 1"));
        assert!(content.contains("frame_0000.png"));
        assert!(content.contains("frame_0001.png"));
        // Last frame must NOT have a trailing comma (invalid JSON)
        // The second frame entry should appear without a trailing comma before the closing ]
        let last_frame_pos = content.rfind("frame_0001.png").unwrap();
        let after_last = &content[last_frame_pos..];
        let closing_brace = after_last.find('}').unwrap();
        let between = &after_last[closing_brace + 1..];
        assert!(!between.trim_start().starts_with(','), "trailing comma after last frame");
    }

    #[test]
    fn test_write_manifest_file_field_escaped() {
        // File paths with backslashes (Windows) must be escaped in JSON
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let frames = vec![
            FrameEntry { frame: 0, pos: [0.0, 0.0, 0.0], angle: [0.0, 0.0], file: r"sub\frame_0000.png".into() },
        ];
        write_manifest(&path, "de_dust2", 1920, 1080, 60, &frames).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        // Raw backslash in file field must be escaped as \\
        assert!(content.contains(r"sub\\frame_0000.png"), "backslash in file path must be escaped");
    }
}
```

Also add `mod capture;` to `src/main.rs` (after the existing `pub mod log;` line).

- [ ] **Step 2: Run tests to verify they fail**
```bash
cargo test capture::tests 2>&1 | head -30
```
Expected: compile succeeds (stubs with `todo!()`), tests panic with "not yet implemented".

- [ ] **Step 3: Implement strip_padding**

Replace the `strip_padding` stub:
```rust
pub fn strip_padding(padded: &[u8], width: u32, height: u32, padded_bpr: u32) -> Vec<u8> {
    let unpadded_bpr = (width * 4) as usize;
    let padded_bpr = padded_bpr as usize;
    let mut out = vec![0u8; unpadded_bpr * height as usize];
    for row in 0..height as usize {
        let src_start = row * padded_bpr;
        let dst_start = row * unpadded_bpr;
        out[dst_start..dst_start + unpadded_bpr]
            .copy_from_slice(&padded[src_start..src_start + unpadded_bpr]);
    }
    out
}
```

- [ ] **Step 4: Run strip_padding tests**
```bash
cargo test capture::tests::test_strip_padding 2>&1
```
Expected: both strip_padding tests PASS.

- [ ] **Step 5: Implement write_png**

Replace the `write_png` stub:
```rust
pub fn write_png(path: &Path, pixels: &[u8], width: u32, height: u32) -> Result<()> {
    use anyhow::Context;
    let img = image::RgbaImage::from_raw(width, height, pixels.to_vec())
        .ok_or_else(|| anyhow::anyhow!("pixel buffer size mismatch for {}x{}", width, height))?;
    img.save(path).with_context(|| format!("writing PNG to {}", path.display()))
}
```

- [ ] **Step 6: Run write_png test**
```bash
cargo test capture::tests::test_write_png 2>&1
```
Expected: PASS.

- [ ] **Step 7: Implement print_frame_json, print_error_json, and write_manifest**

Replace the remaining stubs:
```rust
pub fn print_frame_json(frame: u32, eye: Vec3, yaw_deg: f32, pitch_deg: f32, file: &Path) {
    let file_escaped = file.display().to_string().replace('\\', "\\\\").replace('"', "\\\"");
    println!(
        "{{\"frame\": {frame}, \"pos\": [{:.3}, {:.3}, {:.3}], \"angle\": [{:.3}, {:.3}], \"file\": \"{file_escaped}\"}}",
        eye.x, eye.y, eye.z, yaw_deg, pitch_deg,
    );
}

pub fn print_error_json(msg: &str) {
    let escaped = msg.replace('\\', "\\\\").replace('"', "\\\"");
    println!("{{\"error\": \"{escaped}\"}}");
}

pub fn write_manifest(
    path: &Path,
    map: &str,
    width: u32,
    height: u32,
    frame_step: u32,
    frames: &[FrameEntry],
) -> Result<()> {
    use anyhow::Context;
    let map_escaped = map.replace('\\', "\\\\").replace('"', "\\\"");
    let mut s = String::new();
    s.push_str("{\n");
    s.push_str(&format!("  \"map\": \"{map_escaped}\",\n"));
    s.push_str(&format!("  \"resolution\": \"{width}x{height}\",\n"));
    s.push_str(&format!("  \"frame_step\": {frame_step},\n"));
    s.push_str("  \"frames\": [\n");
    for (i, f) in frames.iter().enumerate() {
        let comma = if i + 1 < frames.len() { "," } else { "" };
        let file_escaped = f.file.replace('\\', "\\\\").replace('"', "\\\"");
        s.push_str(&format!(
            "    {{\"frame\": {}, \"pos\": [{:.3}, {:.3}, {:.3}], \"angle\": [{:.3}, {:.3}], \"file\": \"{}\"}}{}\n",
            f.frame, f.pos[0], f.pos[1], f.pos[2], f.angle[0], f.angle[1], file_escaped, comma
        ));
    }
    s.push_str("  ]\n");
    s.push('}');
    std::fs::write(path, s).with_context(|| format!("writing manifest to {}", path.display()))
}
```

- [ ] **Step 8: Run all capture tests**
```bash
cargo test capture::tests 2>&1
```
Expected: all 5 tests PASS.

- [ ] **Step 9: Build check**
```bash
cargo build 2>&1
```
Expected: compiles cleanly.

- [ ] **Step 10: Commit**
```bash
git add src/capture.rs src/main.rs
git commit -m "feat: add capture module (PNG write, JSON output, manifest)"
```

---

## Chunk 2: GpuCore Extraction

### Task 4: Extract GpuCore from renderer.rs

This is a mechanical refactor — no render logic changes. We pull the surface-agnostic GPU state into `GpuCore`, add an `encode_frame` method, and update `GpuState` to wrap it. The windowed path's event-loop logic is unchanged except for field-access prefixes (`gpu.core.device` instead of `gpu.device`).

**Files:**
- Modify: `src/renderer.rs`

- [ ] **Step 1: Add GpuCore struct**

In `src/renderer.rs`, replace the `GpuState` struct definition with the following two structs. The fields are identical to before — just reorganized:

```rust
/// Surface-agnostic GPU state. Shared between windowed and headless modes.
/// Pipelines are compiled against `output_format` at construction time.
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
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
```

- [ ] **Step 2: Extract init_gpu_core from init_gpu**

Rename the existing `async fn init_gpu` to `async fn init_gpu_core` with this new signature, and carve out the windowed-specific parts into a new `init_gpu`:

Replace the existing `async fn init_gpu(window: Arc<Window>, mesh: MeshData) -> Result<GpuState>` with:

```rust
/// Initialize surface-agnostic GPU state.
/// `output_format` is baked into the render pipelines at compile time.
/// Windowed: pass surface_format from caps. Headless: pass Rgba8UnormSrgb.
pub(crate) async fn init_gpu_core(
    adapter: &wgpu::Adapter,
    mesh: MeshData,
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
            binding: 0, visibility: wgpu::ShaderStages::VERTEX,
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
        label: Some("geo_layout"), bind_group_layouts: &[&geo_bgl], immediate_size: 0,
    });
    let sky_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("sky_layout"), bind_group_layouts: &[&sky_bgl], immediate_size: 0,
    });

    // vertex_attrs is referenced directly in each pipeline descriptor below.
    // Do not use a closure — VertexBufferLayout borrows &vertex_attrs and the
    // closure's lifetime makes the borrow relationship harder to reason about.

    let geo_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("geo"),
        layout: Some(&geo_pipeline_layout),
        vertex: wgpu::VertexState { module: &geo_shader, entry_point: Some("vs_main"), buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<crate::bsp::parse::Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &vertex_attrs }], compilation_options: Default::default() },
        fragment: Some(wgpu::FragmentState { module: &geo_shader, entry_point: Some("fs_main"), targets: &[Some(wgpu::ColorTargetState { format: output_format, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState { format: depth_format, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::Less, stencil: Default::default(), bias: Default::default() }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let sky_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("sky"),
        layout: Some(&sky_pipeline_layout),
        vertex: wgpu::VertexState { module: &sky_shader, entry_point: Some("vs_main"), buffers: &[wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<crate::bsp::parse::Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &vertex_attrs }], compilation_options: Default::default() },
        fragment: Some(wgpu::FragmentState { module: &sky_shader, entry_point: Some("fs_main"), targets: &[Some(wgpu::ColorTargetState { format: output_format, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })], compilation_options: Default::default() }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
        depth_stencil: Some(wgpu::DepthStencilState { format: depth_format, depth_write_enabled: true, depth_compare: wgpu::CompareFunction::Less, stencil: Default::default(), bias: Default::default() }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    Ok(GpuCore {
        device, queue, vertex_buf, index_buf,
        geo_index_count, sky_index_count, sky_index_offset,
        vp_buf, geo_pipeline, sky_pipeline, geo_bind_group, sky_bind_group,
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
    let core = init_gpu_core(&adapter, mesh, surface_format).await?;
    // Configure the surface exactly once, using the device created by init_gpu_core.
    surface.configure(&core.device, &config);

    let (depth_texture, depth_view) = create_depth_texture(&core.device, config.width, config.height);

    Ok(GpuState { window, surface, core, config, depth_texture, depth_view })
}
```

> **Note:** `init_gpu` above calls `init_gpu_core` which creates the device internally. The `surface.configure` call uses `core.device` obtained from `init_gpu_core`. The initial `configure` before `init_gpu_core` is no longer needed — remove it. The clean version: surface is created first (to inform the adapter query), adapter is queried with `compatible_surface`, then `init_gpu_core` creates device+queue+pipelines, and finally `surface.configure(&core.device, &config)` is called once.

- [ ] **Step 3: Update GpuState field accesses in the event loop**

In the `window_event` handler, update all accesses through `gpu` to use `gpu.core.*` for the extracted fields:

- `gpu.device.*` → `gpu.core.device.*`
- `gpu.queue.*` → `gpu.core.queue.*`
- `gpu.vp_buf` → `gpu.core.vp_buf`
- The render pass block: replace with a call to `gpu.core.encode_frame(&mut encoder, &view_tex, &gpu.depth_view)`

The `RedrawRequested` handler becomes:
```rust
WindowEvent::RedrawRequested => {
    if self.shutdown {
        event_loop.exit();
        return;
    }
    let gpu = match &mut self.gpu { Some(g) => g, None => return };
    let now = std::time::Instant::now();
    let delta_secs = (now - self.last_frame).as_secs_f32().min(0.1);
    self.last_frame = now;

    let pose = self.camera.update(delta_secs);
    let aspect = gpu.config.width as f32 / gpu.config.height as f32;
    let fov_y = 2.0 * (1.0_f32 / aspect).atan();
    let proj = Mat4::perspective_rh(fov_y, aspect, 4.0, 4096.0);
    let vp: [[f32; 4]; 4] = (proj * pose.view).to_cols_array_2d();
    gpu.core.queue.write_buffer(&gpu.core.vp_buf, 0, bytemuck::cast_slice(&vp));

    let frame = match gpu.surface.get_current_texture() { Ok(f) => f, Err(_) => return };
    let view_tex = frame.texture.create_view(&Default::default());
    let mut encoder = gpu.core.device.create_command_encoder(&Default::default());
    gpu.core.encode_frame(&mut encoder, &view_tex, &gpu.depth_view);
    gpu.core.queue.submit(std::iter::once(encoder.finish()));
    frame.present();
    gpu.window.request_redraw();
}
```

The `Resized` handler:
```rust
WindowEvent::Resized(new_size) => {
    if let Some(gpu) = &mut self.gpu {
        gpu.config.width = new_size.width;
        gpu.config.height = new_size.height;
        gpu.surface.configure(&gpu.core.device, &gpu.config);
        let (dt, dv) = create_depth_texture(&gpu.core.device, new_size.width, new_size.height);
        gpu.depth_texture = dt;
        gpu.depth_view = dv;
    }
}
```

The `resumed` handler requires **no field-access changes** — it only constructs `GpuState` by calling `init_gpu`, which now returns the new struct layout. The body is otherwise unchanged:
```rust
fn resumed(&mut self, event_loop: &ActiveEventLoop) {
    // ... window creation unchanged ...
    let mesh = self.mesh.take().expect("mesh already consumed");
    let gpu = pollster::block_on(init_gpu(window, mesh)).expect("GPU init failed");
    self.gpu = Some(gpu);
    // ... grace period unchanged ...
}
```

- [ ] **Step 4: Build check (the critical test for this task)**
```bash
cargo build 2>&1
```
Expected: compiles cleanly. Fix any remaining field-access errors.

- [ ] **Step 5: Run all tests**
```bash
cargo test 2>&1
```
Expected: all existing tests still pass.

- [ ] **Step 6: Commit**
```bash
git add src/renderer.rs
git commit -m "refactor: extract GpuCore from renderer.rs with encode_frame method"
```

---

## Chunk 3: headless.rs + CLI

### Task 5: Create headless.rs

This module drives the render loop without a window. It initializes a `GpuCore` (no surface), allocates a reusable staging buffer, and loops: render → readback → strip padding → write PNG → print JSON.

**Files:**
- Create: `src/headless.rs`
- Modify: `src/main.rs` (add `mod headless;`)

- [ ] **Step 1: Create src/headless.rs skeleton**

```rust
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use std::path::{Path, PathBuf};
use wgpu; // explicit import — wgpu:: paths are used throughout this module

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
    // Validate frame_count
    if args.frame_count == 0 {
        capture::print_error_json("frame-count must be at least 1");
        std::process::exit(1);
    }

    // Warn about conflicting flags
    if args.camera_pos.is_some() && args.walkthrough {
        eprintln!("warning: --walkthrough ignored when --camera-pos is set");
    }

    // Create output directory
    std::fs::create_dir_all(&args.output_dir)
        .with_context(|| format!("creating output dir {}", args.output_dir.display()))?;

    // Resolve map
    let map_name = args.map.as_deref()
        .or(cfg.map.as_deref())
        .unwrap_or("de_dust2")
        .to_string();

    // Load BSP
    let bsp_path = maplist::resolve_bsp(&cfg.cs_install_path, &map_name)
        .map_err(|e| {
            capture::print_error_json(&format!("failed to load map '{map_name}': {e:#}"));
            e
        })?;

    let mesh = bsp::load(&bsp_path, &cfg.cs_install_path)
        .map_err(|e| {
            capture::print_error_json(&format!("failed to load map '{map_name}': {e:#}"));
            e
        })?;

    // Extract entity origins before mesh is consumed by init_gpu_core
    let entity_origins = mesh.entity_origins.clone();

    // Build Camera for spline modes (not needed for fixed-pose)
    let camera = if args.camera_pos.is_none() {
        let nav_path = bsp_path.with_extension("nav");
        let waypoints = if nav_path.exists() {
            match bsp::nav::load_waypoints(&nav_path) {
                Ok(pts) => pts,
                Err(_) => cam_mod::nearest_neighbor_sort(entity_origins),
            }
        } else {
            cam_mod::nearest_neighbor_sort(entity_origins)
        };
        let n = waypoints.len();
        Some(Camera::new(waypoints, cfg.camera_speed, cfg.bob_amplitude, cfg.bob_frequency)
            .map_err(|_| {
                let msg = format!("not enough waypoints for spline camera: need ≥ 4, got {n}");
                capture::print_error_json(&msg);
                anyhow::anyhow!("{msg}")
            })?)
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

    let core = init_gpu_core(&adapter, mesh, wgpu::TextureFormat::Rgba8UnormSrgb).await?;

    // Staging buffer: allocated once, reused across all frames
    let unpadded_bpr = args.width * 4;
    let padded_bpr = (unpadded_bpr + 255) & !255;
    let staging_buf = core.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("staging"),
        size: (padded_bpr * args.height) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // Depth texture: allocated once (resolution is fixed)
    let depth_tex = core.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("headless_depth"),
        size: wgpu::Extent3d { width: args.width, height: args.height, depth_or_array_layers: 1 },
        mip_level_count: 1, sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let depth_view = depth_tex.create_view(&Default::default());

    // Render frames
    let mut manifest_frames: Vec<FrameEntry> = Vec::new();
    let mut camera = camera;

    for frame_idx in 0..args.frame_count {
        // Advance spline (frame 0 is captured at t=0, before any tick advance)
        if frame_idx > 0 {
            if let Some(ref mut cam) = camera {
                for _ in 0..args.frame_step {
                    cam.update(1.0 / 60.0);
                }
            }
        }

        // Obtain pose
        let pose = if let Some(pos) = args.camera_pos {
            fixed_pose(pos, args.camera_angle_deg)
        } else {
            camera.as_mut().unwrap().update(0.0)
        };

        // Write VP matrix
        let aspect = args.width as f32 / args.height as f32;
        let fov_y = 2.0 * (1.0_f32 / aspect).atan();
        let proj = Mat4::perspective_rh(fov_y, aspect, 4.0, 4096.0);
        let vp: [[f32; 4]; 4] = (proj * pose.view).to_cols_array_2d();
        core.queue.write_buffer(&core.vp_buf, 0, bytemuck::cast_slice(&vp));

        // Allocate color texture (fresh each frame — it's discarded after readback)
        let color_tex = core.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("headless_color"),
            size: wgpu::Extent3d { width: args.width, height: args.height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let color_view = color_tex.create_view(&Default::default());

        // Render
        let mut encoder = core.device.create_command_encoder(&Default::default());
        core.encode_frame(&mut encoder, &color_view, &depth_view);

        // Copy to staging buffer
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
            wgpu::Extent3d { width: args.width, height: args.height, depth_or_array_layers: 1 },
        );
        core.queue.submit(std::iter::once(encoder.finish()));

        // GPU readback (must poll after submit, before accessing mapped data)
        let staging_slice = staging_buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        staging_slice.map_async(wgpu::MapMode::Read, move |r| { tx.send(r).unwrap(); });
        core.device.poll(wgpu::Maintain::Wait);
        rx.recv().unwrap().unwrap();

        // Strip padding and write PNG
        let filename = format!("frame_{frame_idx:04}.png");
        let out_path = args.output_dir.join(&filename);
        {
            let raw = staging_slice.get_mapped_range();
            let pixels = capture::strip_padding(&raw, args.width, args.height, padded_bpr);
            capture::write_png(&out_path, &pixels, args.width, args.height)?;
        }
        staging_buf.unmap();

        // JSON line to stdout
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

    // Write manifest for walkthrough mode
    if args.camera_pos.is_none() && args.walkthrough {
        let manifest_path = args.output_dir.join("walkthrough.json");
        capture::write_manifest(&manifest_path, &map_name, args.width, args.height, args.frame_step, &manifest_frames)?;
    }

    Ok(())
}

/// Build a CameraPose directly from fixed-pose CLI args.
/// pos: [x, y, z] world units. angle_deg: [yaw, pitch] in degrees (optional; defaults to looking +X).
fn fixed_pose(pos: [f32; 3], angle_deg: Option<[f32; 2]>) -> CameraPose {
    let [x, y, z] = pos;
    // Use the user-supplied position directly as the eye position.
    // The +64 eye-height offset is a spline-mode behavior (waypoint → eye); in fixed-pose
    // mode the user controls the exact position via --camera-pos.
    let eye = Vec3::new(x, y, z);

    let (yaw, pitch) = if let Some([yaw_d, pitch_d]) = angle_deg {
        (yaw_d.to_radians(), pitch_d.to_radians())
    } else {
        (0.0_f32, 0.0_f32)
    };

    let forward = Vec3::new(
        yaw.cos() * pitch.cos(),
        yaw.sin() * pitch.cos(),
        pitch.sin(),
    );

    let view = Mat4::look_at_rh(eye, eye + forward, Vec3::Z);
    CameraPose { view, eye, yaw, pitch }
}
```

Add `mod headless;` to `src/main.rs`.

- [ ] **Step 2: Build check**
```bash
cargo build 2>&1
```
Expected: compiles cleanly. Fix any import or field-name errors.

- [ ] **Step 3: Commit**
```bash
git add src/headless.rs src/main.rs
git commit -m "feat: add headless render module"
```

---

### Task 6: CLI argument parsing + main.rs dispatch

Add `parse_headless_args` to `main.rs` and wire up the dispatch.

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Write failing tests for arg parsing**

Add to `src/main.rs` (at the bottom, inside a `#[cfg(test)]` module):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_headless_args_defaults() {
        let args = parse_headless_args(&[]).unwrap();
        assert_eq!(args.frame_count, 1);
        assert_eq!(args.frame_step, 60);
        assert_eq!(args.width, 1920);
        assert_eq!(args.height, 1080);
        assert!(!args.walkthrough);
        assert!(args.camera_pos.is_none());
        assert!(args.camera_angle_deg.is_none());
        assert!(args.map.is_none());
    }

    #[test]
    fn test_headless_args_resolution() {
        let args = parse_headless_args(&["--resolution".into(), "640x480".into()]).unwrap();
        assert_eq!(args.width, 640);
        assert_eq!(args.height, 480);
    }

    #[test]
    fn test_headless_args_camera_pos() {
        let args = parse_headless_args(&["--camera-pos".into(), "1.0,2.5,-3.0".into()]).unwrap();
        assert_eq!(args.camera_pos, Some([1.0, 2.5, -3.0]));
    }

    #[test]
    fn test_headless_args_camera_angle() {
        let args = parse_headless_args(&["--camera-angle".into(), "90.0,0.0".into()]).unwrap();
        assert_eq!(args.camera_angle_deg, Some([90.0, 0.0]));
    }

    #[test]
    fn test_headless_args_walkthrough_and_frame_count() {
        let args = parse_headless_args(&[
            "--walkthrough".into(),
            "--frame-count".into(), "10".into(),
            "--frame-step".into(), "30".into(),
        ]).unwrap();
        assert!(args.walkthrough);
        assert_eq!(args.frame_count, 10);
        assert_eq!(args.frame_step, 30);
    }

    #[test]
    fn test_headless_args_map_override() {
        let args = parse_headless_args(&["--map".into(), "cs_office".into()]).unwrap();
        assert_eq!(args.map.as_deref(), Some("cs_office"));
    }

    #[test]
    fn test_headless_args_frame_count_zero_passes_parsing() {
        // Validation of frame_count=0 is done in headless::run, not the parser
        let args = parse_headless_args(&["--frame-count".into(), "0".into()]).unwrap();
        assert_eq!(args.frame_count, 0);
    }

    #[test]
    fn test_headless_args_unknown_flag_errors() {
        assert!(parse_headless_args(&["--bogus".into()]).is_err());
    }

    #[test]
    fn test_headless_args_missing_value_errors() {
        assert!(parse_headless_args(&["--output".into()]).is_err());
        assert!(parse_headless_args(&["--frame-count".into()]).is_err());
    }

    #[test]
    fn test_headless_args_camera_pos_wrong_count_errors() {
        assert!(parse_headless_args(&["--camera-pos".into(), "1.0,2.0".into()]).is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**
```bash
cargo test tests:: 2>&1 | head -20
```
Expected: compile error — `parse_headless_args` not defined.

- [ ] **Step 3: Implement parse_headless_args in main.rs**

Add this function to `src/main.rs` (before `fn main()`):

```rust
fn parse_headless_args(args: &[String]) -> Result<headless::HeadlessArgs, String> {
    let mut result = headless::HeadlessArgs {
        walkthrough: false,
        output_dir: std::path::PathBuf::from("./captures/"),
        camera_pos: None,
        camera_angle_deg: None,
        frame_count: 1,
        frame_step: 60,
        width: 1920,
        height: 1080,
        map: None,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--walkthrough" => result.walkthrough = true,
            "--output" => {
                i += 1;
                let v = args.get(i).ok_or("--output requires a directory path")?;
                result.output_dir = v.into();
            }
            "--camera-pos" => {
                i += 1;
                let s = args.get(i).ok_or("--camera-pos requires x,y,z")?;
                let parts: Vec<f32> = s.split(',')
                    .map(|p| p.trim().parse::<f32>().map_err(|_| format!("invalid float in --camera-pos: '{p}'")))
                    .collect::<Result<_, _>>()?;
                if parts.len() != 3 {
                    return Err(format!("--camera-pos requires exactly 3 comma-separated values, got {}", parts.len()));
                }
                result.camera_pos = Some([parts[0], parts[1], parts[2]]);
            }
            "--camera-angle" => {
                i += 1;
                let s = args.get(i).ok_or("--camera-angle requires yaw,pitch (degrees)")?;
                let parts: Vec<f32> = s.split(',')
                    .map(|p| p.trim().parse::<f32>().map_err(|_| format!("invalid float in --camera-angle: '{p}'")))
                    .collect::<Result<_, _>>()?;
                if parts.len() != 2 {
                    return Err(format!("--camera-angle requires exactly 2 comma-separated values, got {}", parts.len()));
                }
                result.camera_angle_deg = Some([parts[0], parts[1]]);
            }
            "--frame-count" => {
                i += 1;
                result.frame_count = args.get(i).ok_or("--frame-count requires a value")?
                    .parse().map_err(|_| "--frame-count must be a non-negative integer".to_string())?;
            }
            "--frame-step" => {
                i += 1;
                result.frame_step = args.get(i).ok_or("--frame-step requires a value")?
                    .parse().map_err(|_| "--frame-step must be a positive integer".to_string())?;
            }
            "--resolution" => {
                i += 1;
                let s = args.get(i).ok_or("--resolution requires WxH (e.g. 1920x1080)")?;
                let parts: Vec<&str> = s.splitn(2, 'x').collect();
                if parts.len() != 2 {
                    return Err("--resolution must be WxH format (e.g. 1920x1080)".to_string());
                }
                result.width = parts[0].parse().map_err(|_| "invalid width in --resolution".to_string())?;
                result.height = parts[1].parse().map_err(|_| "invalid height in --resolution".to_string())?;
            }
            "--map" => {
                i += 1;
                result.map = Some(args.get(i).ok_or("--map requires a map name")?.clone());
            }
            other => return Err(format!("unknown flag: '{other}'")),
        }
        i += 1;
    }
    Ok(result)
}
```

- [ ] **Step 4: Run the parser tests**
```bash
cargo test tests:: 2>&1
```
Expected: all parser tests PASS.

- [ ] **Step 5: Wire up dispatch in main()**

Replace `fn main()` in `src/main.rs`:

```rust
fn main() {
    let log_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("cs-flythrough-debug.log")))
        .unwrap_or_else(|| std::path::PathBuf::from("cs-flythrough-debug.log"));
    log::init(&log_path);

    let raw_args: Vec<String> = std::env::args().collect();

    // Headless mode — checked before Windows screensaver convention
    if raw_args.iter().any(|a| a == "--headless") {
        let flags: Vec<String> = raw_args[1..].iter()
            .filter(|a| a.as_str() != "--headless")
            .cloned()
            .collect();

        let headless_args = match parse_headless_args(&flags) {
            Ok(a) => a,
            Err(e) => {
                capture::print_error_json(&e);
                std::process::exit(1);
            }
        };

        let binary_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| Path::new(".").to_path_buf());
        let config_path = binary_dir.join("cs-flythrough.toml");

        let cfg = if config_path.exists() {
            match config::Config::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    capture::print_error_json(&format!("failed to load config: {e:#}"));
                    std::process::exit(1);
                }
            }
        } else {
            capture::print_error_json("cs-flythrough.toml not found — run without --headless first to generate it");
            std::process::exit(1);
        };

        if let Err(e) = headless::run(headless_args, cfg) {
            capture::print_error_json(&format!("{e:#}"));
            std::process::exit(1);
        }
        return;
    }

    // Windows screensaver convention
    let mode = raw_args.get(1).map(|s| s.as_str()).unwrap_or("/s");
    let result = match mode {
        "/s" => run_screensaver(),
        "/c" => {
            eprintln!("Settings dialog not yet implemented.");
            Ok(())
        }
        _ => {
            eprintln!("Unknown mode: {mode}. Use /s to run screensaver.");
            Ok(())
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}
```

- [ ] **Step 6: Final build + test**
```bash
cargo build 2>&1
cargo test 2>&1
```
Expected: compiles cleanly, all tests pass.

- [ ] **Step 7: Manual smoke test (requires real GPU + CS install)**

If a CS install is available:
```bash
cargo run -- --headless --frame-count 1 --output ./captures/
```
Expected stdout: `{"frame": 0, "pos": [...], "angle": [...], "file": "captures/frame_0000.png"}`
Expected file: `captures/frame_0000.png` exists and is non-empty.

For walkthrough:
```bash
cargo run -- --headless --walkthrough --frame-count 5 --frame-step 60 --output ./captures/
```
Expected: 5 PNGs + `captures/walkthrough.json`.

- [ ] **Step 8: Commit**
```bash
git add src/main.rs
git commit -m "feat: add --headless CLI mode with walkthrough capture"
```

---

## Done

All tasks complete. The feature is ready for PR:

```bash
gh pr create --title "feat: headless rendering & walkthrough capture" \
  --body "Adds --headless CLI mode for automated visual bug detection. See docs/superpowers/specs/2026-03-15-headless-rendering-design.md."
```
