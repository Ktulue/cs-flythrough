# Headless Rendering & Visual Bug Detection Pipeline

**Date:** 2026-03-15
**Branch:** feat/headless-rendering
**Scope:** Phases 1 & 2 only — Phase 3 (visual bug detection) is a workflow, not code.

---

## Goal

Add a `--headless` CLI mode that renders CS 1.6 BSP maps to PNG files without opening a
window. This enables an autonomous visual bug detection workflow: run a headless walkthrough
capture, inspect the PNGs, fix rendering issues in code, re-capture, verify.

---

## Architecture

### GpuCore Extraction

The existing `renderer.rs` welds GPU setup to the winit surface. We split it into two layers:

**`GpuCore`** — surface-agnostic GPU state: device, queue, vertex/index buffers, VP uniform
buffer, render pipelines, and bind groups. Everything that doesn't care whether output goes
to a screen or a PNG.

**`GpuState`** (windowed) — wraps `GpuCore` and adds: window, surface, surface config, and
depth texture. The event loop and `frame.present()` logic remain here, unchanged.

**`headless.rs`** — new module. Creates an instance without a surface, requests an adapter
with `compatible_surface: None`, builds a `GpuCore` targeting `Rgba8UnormSrgb` format, then
drives its own render loop.

**`capture.rs`** — new module. Owns PNG writing (including wgpu row-alignment padding
removal) and JSON line / manifest serialization.

The depth texture is intentionally excluded from `GpuCore` — it is size-dependent, and
windowed vs. headless use different sizes.

**`init_gpu_core` signature:**
```rust
async fn init_gpu_core(
    adapter: &wgpu::Adapter,
    mesh: &MeshData,
    output_format: wgpu::TextureFormat,
) -> Result<GpuCore>
```
- Windowed: passes `surface_format` from surface capabilities (typically `Bgra8UnormSrgb`)
- Headless: passes `Rgba8UnormSrgb`

The `output_format` is threaded into `ColorTargetState.format` when creating both
`geo_pipeline` and `sky_pipeline`. Pipelines are compiled against their render target format
at creation time — a pipeline compiled for one format cannot be used with a render pass
targeting a different format (wgpu validation error). This is why `GpuCore` is initialized
per-target rather than shared across modes.

**Gamma / sRGB behavior:** the existing texture atlases are uploaded as `Rgba8UnormSrgb`.
The fragment shaders output linear-space values; the GPU applies gamma encoding on write when
the render target is sRGB. Using `Rgba8UnormSrgb` as the headless render target preserves
this behavior — raw readback bytes are gamma-encoded, and the PNG will look visually
identical to the windowed output. Using `Rgba8Unorm` would skip the gamma step and produce
washed-out PNGs.

---

## CLI Interface

`--headless` is checked before the existing `/s`/`/c` dispatch. No new dependencies —
argument parsing remains hand-rolled.

```
cs-flythrough /s                    # windowed screensaver (unchanged)
cs-flythrough /c                    # settings stub (unchanged)
cs-flythrough --headless [options]  # headless capture (new)
```

### Flags

| Flag | Type | Default | Notes |
|---|---|---|---|
| `--headless` | bool | — | required to enter headless mode |
| `--walkthrough` | bool | false | drive spline path; see Mode Rules |
| `--output <dir>` | path | `./captures/` | created if absent |
| `--camera-pos <x,y,z>` | 3 floats | — | fixed pose; see Mode Rules |
| `--camera-angle <yaw,pitch>` | 2 floats (degrees) | — | fixed pose; converted to radians internally; see Mode Rules |
| `--frame-count <n>` | u32 | 1 | must be ≥ 1; 0 is an error |
| `--frame-step <n>` | u32 | 60 | simulation ticks between captures |
| `--resolution <WxH>` | u32×u32 | 1920×1080 | output PNG dimensions |
| `--map <name>` | string | from config | overrides `cs-flythrough.toml` |

**Config file resolution in headless mode:** same as windowed — look for `cs-flythrough.toml`
in the directory containing the binary (`std::env::current_exe()` parent). The `--map` flag
overrides the `map` key from that config; all other config values (cs_install_path,
camera_speed, bob) still apply.

### Mode Rules

Priority (highest to lowest):

1. **Fixed-pose mode** — triggered when `--camera-pos` is provided (with or without
   `--camera-angle`). Computes view matrix directly; Camera and spline are not constructed.
   If `--walkthrough` is also present, emit a warning to stderr:
   `"warning: --walkthrough ignored when --camera-pos is set"` and proceed in fixed-pose
   mode. `--frame-count` defaults to 1 in fixed-pose mode; additional frames render the
   same pose.

2. **Walkthrough mode** — triggered by `--walkthrough` alone. Camera follows the
   Catmull-Rom spline. Outputs `walkthrough.json` manifest after all frames complete.

3. **Single-frame at t=0** — default when neither `--camera-pos` nor `--walkthrough` is
   set. Camera advances 0 ticks before capture.

### frame-count validation

If `--frame-count 0` is passed, emit:
```json
{"error": "frame-count must be at least 1"}
```
Exit 1.

### Stdout (per frame)
```json
{"frame": 0, "pos": [x, y, z], "angle": [yaw, pitch], "file": "captures/frame_0000.png"}
```

### Error output (any fatal failure)
```json
{"error": "<description>"}
```
Exit 1. Applies to: map load failure, waypoint count < 4, frame-count = 0.

---

## Phase 1: Headless Render Flow

### CameraPose

`camera.update()` currently returns only `Mat4`. To support JSON output without inverting
the view matrix, change the return type to a `CameraPose` struct:

```rust
pub struct CameraPose {
    pub view: Mat4,
    pub eye: Vec3,
    pub yaw: f32,   // radians, atan2(forward.y, forward.x); JSON output converts to degrees
    pub pitch: f32, // radians, asin(forward.z.clamp(-1.0, 1.0)); JSON output converts to degrees
}
```

`camera.update()` already computes `pos`, `eye`, and `forward` — returning them costs
nothing. `renderer.rs` uses only `pose.view`; the additional fields are zero-cost in the
windowed path.

For fixed-pose mode in headless, construct `CameraPose` directly from the user's `pos` and
`angle` flags — no Camera needed.

### Per-frame loop in `headless.rs`

**Before the loop:** allocate one staging buffer (reused across all frames):
- Size: `padded_bytes_per_row * height` bytes
- Usage: `MAP_READ | COPY_DST`
- `padded_bytes_per_row = (width * 4 + 255) & !255`

**Per frame:**

1. Allocate color texture: `Rgba8UnormSrgb`, usage `RENDER_ATTACHMENT | COPY_SRC`
2. Allocate depth texture: `Depth32Float`, usage `RENDER_ATTACHMENT`
3. Obtain `CameraPose`:
   - Fixed-pose: construct directly from `--camera-pos` / `--camera-angle`
   - Spline: call `camera.update(1.0 / 60.0)` (see tick model in Phase 2)
4. Write VP matrix to `vp_buf`
5. Record and submit render pass targeting the color texture
6. Copy texture to staging buffer: `encoder.copy_texture_to_buffer(...)`
7. Submit encoder: `queue.submit([encoder.finish()])`
8. **GPU readback sequence** (mandatory, in this order):
   ```rust
   let (tx, rx) = std::sync::mpsc::channel();
   staging_slice.map_async(wgpu::MapMode::Read, move |r| { tx.send(r).unwrap(); });
   device.poll(wgpu::Maintain::Wait);   // flush GPU work
   rx.recv().unwrap().unwrap();         // block until mapped
   // now access mapped view
   let data = staging_slice.get_mapped_range();
   ```
9. Strip 256-byte row-alignment padding, write PNG via `image`
10. Drop mapped view, call `staging_buf.unmap()`
11. Print JSON line to stdout

### Row Alignment

wgpu requires `bytes_per_row` to be a multiple of `COPY_BYTES_PER_ROW_ALIGNMENT` (256).
`capture.rs` always strips padding row-by-row before handing raw bytes to the `image` crate:

```rust
let unpadded = width * 4;
let padded = (unpadded + 255) & !255;
// copy `unpadded` bytes per row, skipping `padded - unpadded` tail bytes
```

For `Rgba8UnormSrgb`, `copy_texture_to_buffer` copies the raw sRGB-encoded bytes (same bytes
the GPU wrote). The `image` crate receives RGBA bytes which PNG viewers interpret as sRGB —
correct.

### Output Filenames

`frame_0000.png`, `frame_0001.png`, … — zero-padded to 4 digits.

---

## Phase 2: Walkthrough Mode

`--walkthrough` drives the existing `Camera` along the Catmull-Rom spline and captures
frames at regular intervals.

### Tick Model

Each tick advances the camera by `1/60` second. Frame 0 is captured at t=0 (before any
ticks advance), making it consistent with the default single-frame mode. Subsequent frames
advance `frame_step` ticks between captures.

```
render + capture frame 0      // at t = 0, zero ticks advanced
for frame_idx in 1..frame_count:
    for _ in 0..frame_step:
        camera.update(1.0 / 60.0)   // advance without capturing
    render + capture frame_idx
```

This is deterministic — same flags always produce identical frames. `--frame-count 1` in
walkthrough mode and default single-frame mode capture the same position (t=0).

### Waypoint failure

If `Camera::new` returns `Err` (fewer than 4 waypoints from NAV or entity origins), emit:
```json
{"error": "not enough waypoints for spline camera: need ≥ 4, got N"}
```
Exit 1.

### Manifest

Written to `<output_dir>/walkthrough.json` after all frames complete:

```json
{
  "map": "de_dust2",
  "resolution": "1920x1080",
  "frame_step": 60,
  "frames": [
    {"frame": 0, "pos": [x, y, z], "angle": [yaw, pitch], "file": "frame_0000.png"}
  ]
}
```

`pos` and `angle` come directly from `CameraPose.eye` and `CameraPose.{yaw, pitch}` — no
matrix inversion or decomposition needed.

---

## File Structure

```
cs-flythrough/
├── src/
│   ├── main.rs          # CLI arg parsing + mode dispatch (modified)
│   ├── renderer.rs      # GpuCore extracted; windowed GpuState unchanged (modified)
│   ├── headless.rs      # NEW: headless render loop
│   ├── capture.rs       # NEW: PNG write + JSON output + manifest
│   ├── camera.rs        # CameraPose return type added (modified)
│   ├── config.rs        # unchanged
│   └── bsp/             # unchanged
├── captures/            # gitignored
└── .gitignore           # add captures/, bug-report.json
```

**No new Cargo.toml dependencies** — `image` is already present.

---

## Out of Scope

- Phase 3 (visual bug detection) is a workflow, not code. It consists of:
  running a walkthrough capture, reading the PNGs, logging issues to `bug-report.json`
  by hand, fixing the rendering code, re-capturing, and verifying.
- Software/fallback GPU adapter (WARP, SwiftShader) — headless targets local dev machines
  with real GPU drivers.
- Settings dialog (`/c` mode).
