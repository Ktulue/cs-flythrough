# CS 1.6 Map Screensaver — Design Spec
**Date:** 2026-03-14
**Status:** Approved
**Scope:** Skateboard — de_dust2.bsp rendering with first-person Catmull-Rom flythrough

---

## Vision

An ambient screensaver that loads Counter-Strike 1.6 GoldSrc BSP map files and renders a smooth, continuous first-person camera flythrough — no UI, no HUD, just pure nostalgic exploration. The target experience is the Windows 95 maze screensaver: continuous forward movement through a 3D environment, smooth and looping, never stops or snaps.

The screensaver reads map and texture files directly from the user's existing CS 1.6 or CS: Condition Zero installation. No Valve assets are bundled. Users point the app at their own install directory.

---

## Skateboard Scope

- One map: `de_dust2.bsp` (present in both `cstrike/maps` and `czero/maps`)
- Screensaver mode only (`/s` flag) — settings dialog and preview mode are future milestones
- Fullscreen window, exits immediately on any mouse movement or keypress
- Textured geometry with baked lightmaps (authentic GoldSrc look)
- First-person camera on a looping Catmull-Rom spline through BSP entity waypoints
- Walking camera bob for embodied feel

---

## Architecture

Single Rust binary: `cs-flythrough.exe`. Three runtime modes via command-line flag (Windows screensaver convention):

| Mode | Flag | Skateboard Status |
|---|---|---|
| Screensaver | `/s` | Implemented (bare `.exe`, not `.scr`-registered) |
| Settings | `/c` | Future milestone |
| Preview | `/p HWND` | Future milestone |

**Skateboard clarification:** The binary accepts `/s` and runs the fullscreen flythrough. It is NOT yet registered as a `.scr` file with Windows and will not appear in the screensaver control panel. `.scr` registration is milestone 1 in the future roadmap. Flag parsing is wired from the start so adding `/c` and `/p` later requires no structural changes.

### Modules

| Module | Responsibility |
|---|---|
| `config` | Load/save `cs-flythrough.toml` — CS install path, map selection mode, camera speed, bob settings |
| `maplist` | Enumerate available `.bsp` files; read/write `map-compatibility.toml`; filter failed maps |
| `bsp` | Parse BSP via `qbsp`; load WAD textures via `goldsrc-rs`; output `MeshData` (CPU-side) |
| `camera` | Extract entity waypoints, sort spatially, build Catmull-Rom spline, advance each frame |
| `renderer` | Own the `winit` event loop and wgpu device/surface; handle input events inside the event loop callback; upload buffers; run render loop at 60fps |
| `input` | Provide detection logic (mouse delta threshold, keypress filter); called from within `renderer`'s event loop callback; sets shared `Arc<AtomicBool>` shutdown flag |

**Startup sequence:** config → maplist → bsp (outputs `MeshData`) → maplist writes status → renderer init (uploads `MeshData` to GPU) → camera init → render loop → exit on input.

---

## Configuration

### `cs-flythrough.toml`
```toml
cs_install_path = "C:/Program Files (x86)/Steam/steamapps/common/Counter-Strike"
map_selection = "single"          # "single" | "list" | "all"
map = "de_dust2"                  # used when map_selection = "single"
# maps = ["de_dust2", "cs_italy"] # used when map_selection = "list"
camera_speed = 133.0              # units/sec (CS 1.6 walk speed default)
bob_amplitude = 2.0               # vertical oscillation in units
bob_frequency = 2.0               # cycles per second
```

### `map-compatibility.toml`
Auto-maintained at runtime. Records exact parse results for every map attempted.
```toml
[maps]
"de_dust2" = { status = "ok" }
"de_survivor" = { status = "failed", reason = "missing WAD: halflife.wad" }
"cs_militia" = { status = "untested" }
```

**Statuses:**
- `ok` — loaded successfully at least once
- `failed` — parse error; excluded from rotation; exact Rust error chain stored in `reason`
- `untested` — discovered but never attempted (default)

Both files live in the same directory as the binary. `map-compatibility.toml` is always written in all map selection modes including `single` — the skateboard always records the de_dust2 load result.

---

## Data Flow

```
cs-flythrough.toml
      │
      ▼
  [config] ──► cs_install_path, map_selection_mode, camera params
      │
      ▼
  [maplist] ──► reads map-compatibility.toml
             ──► resolves → de_dust2.bsp (absolute path)
             ──► writes status immediately after bsp load completes:
                   ok → written after MeshData is fully constructed
                   failed → written immediately on any bsp/WAD error
      │
      ▼
   [bsp] ──► qbsp: parses BSP30 → triangle mesh + lightmap atlas
          ──► goldsrc-rs: loads WAD files → diffuse textures
          ──► outputs: MeshData { vertices, indices, diffuse_atlas, lightmap_atlas,
                                  geometry_index_count, sky_index_offset, entity_origins }
      │
      ├──► [renderer] ── uploads MeshData to wgpu buffers; owns render loop
      │
      └──► [camera] ── sorts waypoints spatially (nearest-neighbor)
                    ── builds closed Catmull-Rom spline
                    ── each frame: advance t, apply eye height + bob → view_matrix
                          │
                          ▼
                    [renderer] ── binds view_matrix uniform; draws frame

[input logic] ── lives inside renderer's winit event loop callback
              ── on WindowEvent::KeyboardInput: sets shutdown=true
              ── on DeviceEvent::MouseMotion: sets shutdown=true only if delta magnitude > MOUSE_EXIT_THRESHOLD
                   MOUSE_EXIT_THRESHOLD = 10.0 — compile-time constant in `input.rs`
                   (filters sub-pixel hardware jitter; known trade-off: deliberate micro-movements below threshold won't exit)
              ── renderer checks flag at start of each RedrawRequested; exits event loop when true
```

---

## BSP/WAD Loading Pipeline

### Step 1 — BSP Parse (`qbsp`)
- Load `de_dust2.bsp`
- `qbsp` is responsible for all BSP30 file reading and mesh construction. `goldsrc-rs` does NOT read the BSP file.
- Outputs: triangle mesh (vertices with position, diffuse UV, lightmap UV), lightmap patches, entity lump (raw string), texture name list
- `qbsp` bakes lightmap patches into a single RGBA atlas — ready to upload
- **Crate status:** `qbsp` is confirmed to exist on crates.io and is documented to support Quake 1, 2, and GoldSrc BSP30 with mesh generation and lightmap atlas output. Verify API at implementation start. If GoldSrc support proves broken at runtime, the fallback is `goldsrc-rs` BSP parsing (it can read BSP30 face/vertex/edge lumps) combined with a manual lightmap bake — this requires a spec revision but is a known path. The Valve Developer Community BSP30 spec is the authoritative reference.

### Step 2 — Entity Lump Parse
Hand-written parser over the plain-text entity lump. Extracts `origin` from:
- `info_player_start` / `info_player_deathmatch` — spawn points
- `func_bombsite` — bomb sites A and B
- `hostage_entity` — hostage positions (CZ maps)

These become the camera spline waypoints. **Minimum required: 4 waypoints** (Catmull-Rom requires at least 4 control points). If fewer than 4 are extracted, log a fatal error with the count found and exit cleanly — do not attempt to render.

**Eye height note:** `info_player_start` and `info_player_deathmatch` origins are at floor level — the +64 unit eye height offset in the camera module is correct for these. `func_bombsite` origins are the center-of-volume of the trigger brush, which may be above floor level. The +64 offset will be applied uniformly to all origins; bombsite camera height may need empirical tuning after first run.

### Step 3 — WAD Texture Load (`goldsrc-rs`)
- `goldsrc-rs` is responsible exclusively for WAD file reading. It does NOT touch the BSP file.
- WAD file paths are read from the BSP texture lump's embedded path strings (e.g. `de_dust.wad`). These paths are resolved relative to `cs_install_path/cstrike/` (or `czero/`). If a WAD path is absolute in the BSP, the filename is extracted and resolved against the install path.
- `goldsrc-rs` opens each resolved WAD and extracts the referenced textures as RGBA bitmaps
- All diffuse textures packed into a single atlas via `guillotiere` (Rust 2D bin-packing crate)
- Packed atlas uploaded as a single `wgpu::Texture`

### Step 4 — `MeshData` Output (CPU-side)

`bsp` outputs a CPU-side struct. GPU upload happens in `renderer` after the wgpu device is initialized.

```rust
// Vertex layout — stride: 32 bytes
struct Vertex {
    position:     [f32; 3],   // @location(0) — world-space XYZ
    diffuse_uv:   [f32; 2],   // @location(1) — UV into diffuse atlas
    lightmap_uv:  [f32; 2],   // @location(2) — UV into lightmap atlas
}

struct MeshData {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,             // geometry indices first, sky indices appended
    sky_index_offset: u32,         // index into `indices` where sky faces begin
    diffuse_atlas: RgbaImage,      // packed diffuse textures, CPU memory
    lightmap_atlas: RgbaImage,     // baked lightmap atlas, CPU memory
    entity_origins: Vec<Vec3>,     // unsorted entity origins; camera module applies nearest-neighbor sort
}
```

`renderer` uploads `MeshData` to wgpu buffers after device init, producing the final GPU buffers. `bsp` has no wgpu dependency.

---

## Camera System

### Waypoint Ordering
Entity origins extracted in BSP file order. Nearest-neighbor sort pass produces a spatially coherent path that traverses the map without teleporting.

### Spline
Closed Catmull-Rom spline through sorted waypoints. Last point curves back to first — seamless loop, no visible seam.

### Frame Advance
- `t` advances by `speed * delta_time` each frame (spline parameter, 0.0–1.0 over the full loop)
- Camera position and forward direction sampled from spline at `t`
- Camera always faces direction of travel
- **Eye height:** +64 units above entity origin (GoldSrc player eye height)
- **Bob:** sinusoidal vertical oscillation using elapsed wall time — `sin(elapsed_secs * bob_frequency * 2π) * bob_amplitude`. Uses wall time, NOT `t`, so bob rate stays constant regardless of camera speed.

### Movement Speeds
- Default: 133 units/sec (CS 1.6 walking speed)
- Fast: 250 units/sec (CS 1.6 running speed) — configurable

### Output
`view_matrix: Mat4` per frame. Renderer binds as uniform. Camera has no wgpu dependency.

---

## Rendering Pipeline

### Two passes per frame

**Geometry pass**
- Single indexed draw call over all BSP faces
- Fragment shader: `sample(diffuse_atlas, uv) * sample(lightmap_atlas, lightmap_uv)`
- Produces authentic GoldSrc lighting: baked shadows, no dynamic lights

**Sky pass**
- Primary sky identification: `qbsp` face surface flags — faces carrying `SURF_SKY` are sky faces. If `qbsp` does not expose surface flags for GoldSrc BSP30 (verify at implementation start), fallback: identify sky faces by texture name — GoldSrc sky textures begin with the prefix `sky` (e.g. `sky_dust`, `sky_aztec`).
- `bsp` module partitions all faces into geometry and sky before building buffers. Sky indices are appended after geometry indices in the shared index buffer; `sky_index_offset` in `MeshData` marks the split.
- Sky faces drawn with a flat color fragment shader; no atlas sampling; separate draw call using the same vertex/index buffer with index offset = `sky_index_offset`
- Hardcoded sky color for skateboard: `vec4(0.42, 0.55, 0.68, 1.0)` (CS 1.6 sky blue)
- No sky color uniform for the skateboard — color is a shader constant
- Future: sky dome texture from WAD, sky color configurable

### Uniforms
```
view_projection: Mat4    // camera view * perspective projection
atlas_diffuse: texture
atlas_lightmap: texture
```

No per-object transforms — BSP geometry is in world space.

### Shaders
- Language: WGSL
- Embedded in the binary via `include_str!` at compile time
- Source files: `src/shaders/geometry.wgsl`, `src/shaders/sky.wgsl`
- `geometry.wgsl`: samples diffuse and lightmap atlases, multiplies results
- `sky.wgsl`: outputs hardcoded flat sky color constant

### Performance Targets
- 60fps with vsync; `wgpu::PresentMode::AutoVsync` (prefers Fifo/vsync, gracefully falls back on hardware without vsync support)
- All geometry uploaded once at load — zero per-frame allocations
- CPU at idle: < 5%
- Exit within one frame of any input

### Projection Parameters
- FOV: 90° horizontal — CS 1.6 default
- Aspect ratio: window width / height, recalculated on window resize
- Near clip: 4.0 units (avoids z-fighting at close walls)
- Far clip: 4096.0 units (covers full GoldSrc map extent)

---

## Error Handling

### Fatal (startup)
The following exit cleanly with a non-zero code and a `stderr` message. No crash dialog.
- Config file absent → generate a default `cs-flythrough.toml` with placeholder values, print a message telling the user to edit the path, then exit
- CS install path not found → exit with path and instructions
- `de_dust2.bsp` not found at the resolved path → exit with path and instructions
- wgpu adapter or surface creation fails (no Dx12 or Vulkan support) → exit with message: "No compatible GPU backend found. Ensure DirectX 12 or Vulkan drivers are installed."

### Fatal (map load)
The following write to `map-compatibility.toml` as `failed` with the exact Rust error chain in `reason`, then exit cleanly:
- Missing WAD file
- Malformed or unreadable BSP lump
- Entity lump parsed successfully but fewer than 4 waypoints extracted (Catmull-Rom minimum)

No `unwrap()` in production paths. All errors propagate via `anyhow`.

---

## Testing

| Type | Scope |
|---|---|
| Unit | Config parsing, entity lump parser, waypoint nearest-neighbor sort, Catmull-Rom math |
| Integration | Load `de_dust2.bsp` headlessly (no window); assert `MeshData` non-empty, waypoint count >= 4 |
| Manual | Run the binary, watch the flythrough, tune camera feel iteratively |

No GPU tests in CI. The headless BSP integration test catches the majority of regressions without requiring a display.

---

## Tech Stack

| Concern | Crate / Tool | Minimum Version | Notes |
|---|---|---|---|
| Language | Rust stable | 1.77+ | — |
| Rendering | `wgpu` | 22.x | Cargo features: `wgpu = { features = ["dx12", "vulkan"] }`. Runtime backend: auto-select, preferring Dx12 on Windows then Vulkan as fallback. Pin `wgpu` and `winit` to a confirmed-compatible pair at implementation start — check wgpu changelog for `winit` 0.30 compatibility. |
| BSP parsing + lightmap atlas | `qbsp` | latest | **Verify GoldSrc BSP30 support before starting** |
| WAD file I/O | `goldsrc-rs` | latest | WAD only — does not read BSP |
| Diffuse texture atlas packing | `guillotiere` | 0.6 | 2D bin-packing for diffuse atlas |
| Config | `toml` + `serde` | toml 0.8, serde 1 | — |
| Error handling | `anyhow` | 1 | — |
| Math | `glam` | 0.27 | — |
| Window + input | `winit` | 0.30 | — |

---

## Future Milestones (out of skateboard scope)

1. `.scr` registration — rename binary to `.scr`, register with Windows shell so it appears in the screensaver control panel; wire `/c` settings dialog and `/p` preview mode
2. Settings dialog — map picker UI, persists selection to config
3. Multi-map support — list mode, all-maps rotation, map-compatibility filtering
4. Sky dome textures
5. Ambient audio
6. Low-power idle mode (reduce framerate when system is active)

---

## Legal

No Valve assets bundled. Users must have CS 1.6 or CS: Condition Zero installed. The app reads from the user's own installation. Map and WAD files remain owned by Valve.
