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
| Screensaver | `/s` | Implemented |
| Settings | `/c` | Future milestone |
| Preview | `/p HWND` | Future milestone |

Flag parsing is wired from the start so future modes require no structural changes.

### Modules

| Module | Responsibility |
|---|---|
| `config` | Load/save `cs-flythrough.toml` — CS install path, map selection mode, camera speed, bob settings |
| `maplist` | Enumerate available `.bsp` files; read/write `map-compatibility.toml`; filter failed maps |
| `bsp` | Parse BSP via `qbsp`; load WAD textures via `goldsrc-rs`; output `GpuMesh` |
| `camera` | Extract entity waypoints, sort spatially, build Catmull-Rom spline, advance each frame |
| `renderer` | Own wgpu device/surface; upload buffers; run render loop at 60fps |
| `input` | Poll for mouse delta or keypress; signal shutdown |

**Startup sequence:** config → maplist → bsp → renderer init → camera init → render loop → exit on input.

---

## Configuration

### `cs-flythrough.toml`
```toml
cs_install_path = "C:/Program Files (x86)/Steam/steamapps/common/Counter-Strike"
map_selection = "single"          # "single" | "list" | "all"
map = "de_dust2"                  # used when map_selection = "single"
# maps = ["de_dust2", "cs_italy"] # used when map_selection = "list"
camera_speed = 133.0              # units/sec (CS 1.6 walk speed default)
camera_speed_max = 250.0          # units/sec (CS 1.6 run speed)
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

Both files live in the same directory as the binary.

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
             ──► writes back status after load attempt
      │
      ▼
   [bsp] ──► qbsp: parses BSP30 → triangle mesh + lightmap atlas
          ──► goldsrc-rs: loads WAD files → diffuse textures
          ──► outputs: GpuMesh { vertex_buf, index_buf, diffuse_atlas, lightmap_atlas, waypoints }
      │
      ├──► [renderer] ── uploads GpuMesh to wgpu buffers; owns render loop
      │
      └──► [camera] ── sorts waypoints spatially (nearest-neighbor)
                    ── builds closed Catmull-Rom spline
                    ── each frame: advance t, apply eye height + bob → view_matrix
                          │
                          ▼
                    [renderer] ── binds view_matrix uniform; draws frame

[input] ── polls each frame → signals shutdown on any mouse delta or keypress
```

---

## BSP/WAD Loading Pipeline

### Step 1 — BSP Parse (`qbsp`)
- Load `de_dust2.bsp`
- Outputs: triangle mesh (vertices with position, diffuse UV, lightmap UV), lightmap patches, entity lump (raw string)
- `qbsp` bakes lightmap patches into a single RGBA atlas — ready to upload

### Step 2 — Entity Lump Parse
Hand-written parser over the plain-text entity lump. Extracts `origin` from:
- `info_player_start` / `info_player_deathmatch` — spawn points
- `func_bombsite` — bomb sites A and B
- `hostage_entity` — hostage positions (CZ maps)

These become the camera spline waypoints.

### Step 3 — WAD Texture Load (`goldsrc-rs`)
- BSP texture header lists required WAD files (e.g. `de_dust.wad`)
- `goldsrc-rs` opens each WAD from the CS install path
- Extracts referenced textures as RGBA bitmaps
- Packed into a single diffuse atlas via bin-packing

### Step 4 — `GpuMesh` Output
```rust
struct GpuMesh {
    vertex_buf: wgpu::Buffer,      // position, diffuse UV, lightmap UV
    index_buf: wgpu::Buffer,
    diffuse_atlas: wgpu::Texture,
    lightmap_atlas: wgpu::Texture,
    waypoints: Vec<Vec3>,          // entity origins, ordered for spline
}
```

---

## Camera System

### Waypoint Ordering
Entity origins extracted in BSP file order. Nearest-neighbor sort pass produces a spatially coherent path that traverses the map without teleporting.

### Spline
Closed Catmull-Rom spline through sorted waypoints. Last point curves back to first — seamless loop, no visible seam.

### Frame Advance
- `t` advances by `speed * delta_time` each frame
- Camera position and forward direction sampled from spline at `t`
- Camera always faces direction of travel
- **Eye height:** +64 units above entity origin (GoldSrc player eye height)
- **Bob:** sinusoidal vertical oscillation `sin(t * bob_frequency * 2π) * bob_amplitude`

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
- BSP sky faces (texture flag) drawn separately
- Skateboard: solid color or simple gradient
- Future: sky dome texture from WAD

### Uniforms
```
view_projection: Mat4    // camera view * perspective projection
atlas_diffuse: texture
atlas_lightmap: texture
```

No per-object transforms — BSP geometry is in world space.

### Performance Targets
- 60fps with vsync; GPU never spins uncapped
- All geometry uploaded once at load — zero per-frame allocations
- CPU at idle: < 5%
- Exit within one frame of any input

### FOV
90° horizontal — CS 1.6 default.

---

## Error Handling

### Fatal (startup)
Missing config, CS install path not found, `de_dust2.bsp` not found → log to `stderr`, exit with non-zero code. No crash dialog.

### Recoverable (map load)
Missing WAD, malformed entity lump, corrupt BSP lump → write exact Rust error chain to `map-compatibility.toml` as `failed` + `reason`. For the skateboard (single map), also exits cleanly.

No `unwrap()` in production paths. All errors propagate via `anyhow`.

---

## Testing

| Type | Scope |
|---|---|
| Unit | Config parsing, entity lump parser, waypoint nearest-neighbor sort, Catmull-Rom math |
| Integration | Load `de_dust2.bsp` headlessly (no window); assert `GpuMesh` non-empty, waypoint count > 0 |
| Manual | Run the binary, watch the flythrough, tune camera feel iteratively |

No GPU tests in CI. The headless BSP integration test catches the majority of regressions without requiring a display.

---

## Tech Stack

| Concern | Crate / Tool |
|---|---|
| Language | Rust (stable) |
| Rendering | `wgpu` |
| BSP parsing + lightmap atlas | `qbsp` |
| WAD + BSP file I/O | `goldsrc-rs` |
| Config | `toml` + `serde` |
| Error handling | `anyhow` |
| Math | `glam` |
| Window + input | `winit` |

---

## Future Milestones (out of skateboard scope)

1. `.scr` registration — Windows screensaver shell (`/s`, `/c`, `/p` modes)
2. Settings dialog — map picker UI, persists selection to config
3. Multi-map support — list mode, all-maps rotation, map-compatibility filtering
4. Sky dome textures
5. Ambient audio
6. Low-power idle mode (reduce framerate when system is active)

---

## Legal

No Valve assets bundled. Users must have CS 1.6 or CS: Condition Zero installed. The app reads from the user's own installation. Map and WAD files remain owned by Valve.
