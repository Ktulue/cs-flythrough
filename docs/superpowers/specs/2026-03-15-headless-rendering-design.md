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
with `compatible_surface: None`, builds a `GpuCore` targeting `Rgba8Unorm` format, then
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
- Windowed: passes `surface_format` from surface capabilities
- Headless: hardcodes `Rgba8Unorm` (PNG-native, no sRGB conversion needed at read-back)

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
| `--walkthrough` | bool | false | drive spline path; mutually exclusive with fixed pose |
| `--output <dir>` | path | `./captures/` | created if absent |
| `--camera-pos <x,y,z>` | 3 floats | — | fixed pose; skips spline |
| `--camera-angle <yaw,pitch>` | 2 floats | — | fixed pose; skips spline |
| `--frame-count <n>` | u32 | 1 | number of frames to capture |
| `--frame-step <n>` | u32 | 60 | simulation ticks between captures |
| `--resolution <WxH>` | u32×u32 | 1920×1080 | output PNG dimensions |
| `--map <name>` | string | from config | overrides `cs-flythrough.toml` |

### Mode Rules

- `--camera-pos` + `--camera-angle` → **fixed-pose mode**: view matrix computed directly;
  `--walkthrough` is ignored; `--frame-count` defaults to 1.
- `--walkthrough` alone → **spline mode**: camera follows Catmull-Rom path, capturing every
  `--frame-step` ticks; writes `walkthrough.json` manifest.
- Neither → **single frame at spline t=0**.

### Stdout (per frame)
```json
{"frame": 0, "pos": [x, y, z], "angle": [yaw, pitch], "file": "captures/frame_0000.png"}
```

### Error (map load failure)
```json
{"error": "failed to load map 'de_dust2': <reason>"}
```
Exit code 1.

---

## Phase 1: Headless Render Flow

Per-frame loop in `headless.rs`:

1. Allocate color texture: `Rgba8Unorm`, usage `RENDER_ATTACHMENT | COPY_SRC`
2. Allocate depth texture: `Depth32Float`, usage `RENDER_ATTACHMENT`
3. Compute view matrix:
   - **Fixed-pose**: `Mat4::look_at_rh(eye, target, Vec3::Z)` derived from yaw/pitch directly
   - **Spline**: `camera.update(frame_step as f32 / 60.0)` advanced N ticks before capture
4. Write VP matrix to `vp_buf`
5. Record and submit render pass targeting the color texture
6. `copy_texture_to_buffer` into a staging buffer
7. Map staging buffer → strip 256-byte row-alignment padding → write PNG
8. Print JSON line to stdout

### Row Alignment

wgpu requires `bytes_per_row` to be a multiple of `COPY_BYTES_PER_ROW_ALIGNMENT` (256).
`capture.rs` always strips padding row-by-row before handing raw bytes to the `image` crate:

```rust
let unpadded = width * 4;
let padded = (unpadded + 255) & !255;
// copy `unpadded` bytes per row, skipping `padded - unpadded` padding bytes
```

### Output Filenames

`frame_0000.png`, `frame_0001.png`, … — zero-padded to 4 digits.

---

## Phase 2: Walkthrough Mode

`--walkthrough` drives the existing `Camera` along the Catmull-Rom spline and captures
frames at regular intervals.

### Tick Model

Each tick advances the camera by `1/60` second. Between captures, the camera advances
`frame_step` ticks silently (no render). This is deterministic — same flags always produce
identical frames.

```
for frame_idx in 0..frame_count:
    for _ in 0..frame_step:
        camera.update(1.0 / 60.0)   // advance without capturing
    render + capture frame_idx
```

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

`pos` and `angle` are extracted from the view matrix used to render each frame — ground-truth
accurate, not an approximation.

---

## File Structure

```
cs-flythrough/
├── src/
│   ├── main.rs          # CLI arg parsing + mode dispatch (modified)
│   ├── renderer.rs      # GpuCore extracted; windowed GpuState unchanged (modified)
│   ├── headless.rs      # NEW: headless render loop
│   ├── capture.rs       # NEW: PNG write + JSON output + manifest
│   ├── camera.rs        # unchanged
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
