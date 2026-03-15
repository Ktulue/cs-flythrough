# CS 1.6 Map Screensaver — Skateboard Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a Rust binary that loads de_dust2.bsp from a CS 1.6 install, renders a textured + lightmapped first-person flythrough using a Catmull-Rom spline through BSP entity waypoints, and exits on any mouse/keyboard input.

**Architecture:** Six modules with clean interfaces — `config` reads TOML, `maplist` resolves and tracks map compatibility, `bsp` produces CPU-side `MeshData` (no wgpu dependency), `camera` builds a spline from entity waypoints and outputs a `Mat4` per frame, `renderer` owns the winit event loop and wgpu device, `input` provides shutdown detection logic called from within the renderer's event callback.

**Tech Stack:** Rust stable 1.77+, wgpu 22.x (dx12 + vulkan), winit 0.30, qbsp (BSP30 parse + lightmap atlas), goldsrc-rs (WAD loading), guillotiere 0.6 (atlas bin-packing), glam 0.27, anyhow 1, toml 0.8 + serde 1.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/main.rs` | Arg parsing (`/s`, `/c`, `/p`), startup sequence, top-level error reporting |
| `src/config.rs` | Load/save `cs-flythrough.toml`; generate default on first run |
| `src/maplist.rs` | Resolve BSP path; read/write `map-compatibility.toml` |
| `src/bsp/mod.rs` | Public `load(path) -> Result<MeshData>` API |
| `src/bsp/parse.rs` | qbsp integration; build `Vec<Vertex>` + `Vec<u32>`; partition geometry/sky faces |
| `src/bsp/entity.rs` | Entity lump parser; extract `origin` from relevant classnames |
| `src/bsp/wad.rs` | WAD loading via goldsrc-rs; guillotiere atlas packing |
| `src/camera.rs` | Nearest-neighbor sort; Catmull-Rom spline; per-frame `Mat4` output |
| `src/renderer.rs` | wgpu device/surface; buffer upload; event loop; geometry + sky draw calls |
| `src/input.rs` | `MOUSE_EXIT_THRESHOLD`; shutdown detection called from renderer event callback |
| `src/shaders/geometry.wgsl` | Diffuse × lightmap fragment shader |
| `src/shaders/sky.wgsl` | Flat hardcoded sky color shader |
| `tests/bsp_integration.rs` | Headless de_dust2.bsp load; assert `MeshData` non-empty, waypoints ≥ 4 |

---

## Chunk 1: Project Scaffold, Config, Maplist

### Task 1: Cargo.toml and project scaffold

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`
- Create: `src/shaders/geometry.wgsl`
- Create: `src/shaders/sky.wgsl`

- [ ] **Step 1: Verify crate versions**

Before writing `Cargo.toml`, check crates.io for the latest compatible versions:
- Confirm `qbsp` exists and supports GoldSrc BSP30. If it does not exist or lacks BSP30 support, **stop and surface to human** — the spec requires a revision before proceeding.
- Confirm `wgpu` and `winit` versions are compatible (wgpu 22.x targets winit 0.30 — verify in wgpu changelog).
- Get the latest commit hash for `goldsrc-rs` from https://github.com/r4v3n6101/goldsrc-rs to pin the dependency.
- **Record all confirmed versions.** You MUST fill them into `Cargo.toml` in Step 2 — the placeholders `REPLACE_WITH_VERSION` and `REPLACE_WITH_COMMIT_HASH` must not remain in the committed file.

- [ ] **Step 2: Write `Cargo.toml`**

```toml
[package]
name = "cs-flythrough"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "cs-flythrough"
path = "src/main.rs"

[dependencies]
wgpu = { version = "22", features = ["dx12", "vulkan"] }
winit = "0.30"
glam = "0.27"
anyhow = "1"
toml = "0.8"
serde = { version = "1", features = ["derive"] }
image = "0.25"        # for RgbaImage in MeshData
guillotiere = "0.6"
goldsrc-rs = { git = "https://github.com/r4v3n6101/goldsrc-rs", rev = "REPLACE_WITH_COMMIT_HASH" }
qbsp = "REPLACE_WITH_VERSION"  # REQUIRED: fill in from Step 1 before running cargo check

[dev-dependencies]
# none yet

[profile.release]
opt-level = 3
```

- [ ] **Step 3: Write placeholder `src/main.rs`**

```rust
mod config;
mod maplist;
mod bsp;
mod camera;
mod renderer;
mod input;

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("/s");

    match mode {
        "/s" => run_screensaver(),
        "/c" => {
            eprintln!("Settings dialog not yet implemented.");
            Ok(())
        }
        _ => {
            eprintln!("Unknown mode: {mode}. Use /s to run screensaver.");
            Ok(())
        }
    }
}

fn run_screensaver() -> Result<()> {
    todo!("implement startup sequence")
}
```

- [ ] **Step 4: Write placeholder shaders**

`src/shaders/geometry.wgsl`:
```wgsl
// Geometry pass: diffuse * lightmap
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) diffuse_uv: vec2<f32>,
    @location(2) lightmap_uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) diffuse_uv: vec2<f32>,
    @location(1) lightmap_uv: vec2<f32>,
};

@group(0) @binding(0) var<uniform> view_proj: mat4x4<f32>;
@group(0) @binding(1) var diffuse_tex: texture_2d<f32>;
@group(0) @binding(2) var diffuse_sampler: sampler;
@group(0) @binding(3) var lightmap_tex: texture_2d<f32>;
@group(0) @binding(4) var lightmap_sampler: sampler;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = view_proj * vec4<f32>(in.position, 1.0);
    out.diffuse_uv = in.diffuse_uv;
    out.lightmap_uv = in.lightmap_uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let diffuse = textureSample(diffuse_tex, diffuse_sampler, in.diffuse_uv);
    let lightmap = textureSample(lightmap_tex, lightmap_sampler, in.lightmap_uv);
    return diffuse * lightmap;
}
```

`src/shaders/sky.wgsl`:
```wgsl
// Sky pass: flat hardcoded CS 1.6 sky blue
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) diffuse_uv: vec2<f32>,
    @location(2) lightmap_uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
};

@group(0) @binding(0) var<uniform> view_proj: mat4x4<f32>;

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = view_proj * vec4<f32>(in.position, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(0.42, 0.55, 0.68, 1.0);
}
```

- [ ] **Step 5: Verify it compiles**

```bash
cargo check
```
Expected: no errors (todo!() is fine). Fix any dependency resolution errors before proceeding.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml src/main.rs src/shaders/
git commit -m "feat: scaffold project with placeholder main and shaders"
```

---

### Task 2: Config module

**Files:**
- Create: `src/config.rs`

- [ ] **Step 1: Write the failing tests**

Add at the bottom of `src/config.rs` (create the file):

```rust
use serde::{Deserialize, Serialize};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub cs_install_path: PathBuf,
    pub map_selection: MapSelection,
    pub map: Option<String>,
    pub maps: Option<Vec<String>>,
    pub camera_speed: f32,
    pub bob_amplitude: f32,
    pub bob_frequency: f32,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MapSelection {
    Single,
    List,
    All,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading config at {}", path.display()))?;
        toml::from_str(&text).context("parsing config TOML")
    }

    pub fn write_default(path: &Path) -> Result<()> {
        let default = r#"# cs-flythrough configuration
cs_install_path = "C:/Program Files (x86)/Steam/steamapps/common/Counter-Strike"
map_selection = "single"
map = "de_dust2"
camera_speed = 133.0
bob_amplitude = 2.0
bob_frequency = 2.0
"#;
        std::fs::write(path, default)
            .with_context(|| format!("writing default config to {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(content: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        f
    }

    #[test]
    fn test_load_single_map_config() {
        let f = write_temp(r#"
cs_install_path = "C:/games/cstrike"
map_selection = "single"
map = "de_dust2"
camera_speed = 133.0
bob_amplitude = 2.0
bob_frequency = 2.0
"#);
        let cfg = Config::load(f.path()).unwrap();
        assert_eq!(cfg.map_selection, MapSelection::Single);
        assert_eq!(cfg.map.as_deref(), Some("de_dust2"));
        assert!((cfg.camera_speed - 133.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_load_missing_file_returns_err() {
        let result = Config::load(Path::new("/nonexistent/path/config.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn test_write_default_creates_parseable_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cs-flythrough.toml");
        Config::write_default(&path).unwrap();
        let cfg = Config::load(&path).unwrap();
        assert_eq!(cfg.map_selection, MapSelection::Single);
    }
}
```

- [ ] **Step 2: Add `tempfile` to dev-dependencies**

In `Cargo.toml` under `[dev-dependencies]`:
```toml
tempfile = "3"
```

- [ ] **Step 3: Run tests**

```bash
cargo test config
```
Expected: all three tests pass (`test_load_single_map_config`, `test_load_missing_file_returns_err`, `test_write_default_creates_parseable_file`). The implementation is inline in the same file, so all tests should pass immediately.

- [ ] **Step 4: Verify module compiles**

`src/main.rs` already has `mod config;` from Task 1. Verify it compiles:

```bash
cargo check
```

- [ ] **Step 5: Commit**

```bash
git add src/config.rs Cargo.toml
git commit -m "feat: config module with TOML load and default generation"
```

---

### Task 3: Maplist module

**Files:**
- Create: `src/maplist.rs`

- [ ] **Step 1: Write `src/maplist.rs` with tests**

```rust
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MapStatus {
    Ok,
    Failed,
    Untested,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapRecord {
    pub status: MapStatus,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Compatibility {
    #[serde(default)]
    pub maps: HashMap<String, MapRecord>,
}

impl Compatibility {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).context("serializing compatibility")?;
        std::fs::write(path, text)
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn set_ok(&mut self, map: &str) {
        self.maps.insert(map.to_string(), MapRecord { status: MapStatus::Ok, reason: None });
    }

    pub fn set_failed(&mut self, map: &str, reason: String) {
        self.maps.insert(map.to_string(), MapRecord { status: MapStatus::Failed, reason: Some(reason) });
    }

    pub fn is_excluded(&self, map: &str) -> bool {
        matches!(
            self.maps.get(map).map(|r| &r.status),
            Some(MapStatus::Failed)
        )
    }
}

/// Resolve the absolute path to a BSP file given the CS install path and map name.
/// Tries `cstrike/maps/<name>.bsp` then `czero/maps/<name>.bsp`.
pub fn resolve_bsp(cs_install_path: &Path, map_name: &str) -> Result<PathBuf> {
    let candidates = [
        cs_install_path.join("cstrike").join("maps").join(format!("{map_name}.bsp")),
        cs_install_path.join("czero").join("maps").join(format!("{map_name}.bsp")),
    ];
    candidates
        .into_iter()
        .find(|p| p.exists())
        .with_context(|| format!("BSP not found for map '{map_name}' in {}", cs_install_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_set_ok_and_is_excluded() {
        let mut compat = Compatibility::default();
        compat.set_ok("de_dust2");
        assert!(!compat.is_excluded("de_dust2"));
    }

    #[test]
    fn test_set_failed_and_is_excluded() {
        let mut compat = Compatibility::default();
        compat.set_failed("de_survivor", "missing WAD: halflife.wad".to_string());
        assert!(compat.is_excluded("de_survivor"));
    }

    #[test]
    fn test_untested_map_is_not_excluded() {
        let compat = Compatibility::default();
        assert!(!compat.is_excluded("cs_militia"));
    }

    #[test]
    fn test_save_and_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("map-compatibility.toml");
        let mut compat = Compatibility::default();
        compat.set_ok("de_dust2");
        compat.set_failed("de_survivor", "test reason".to_string());
        compat.save(&path).unwrap();

        let loaded = Compatibility::load(&path);
        assert!(!loaded.is_excluded("de_dust2"));
        assert!(loaded.is_excluded("de_survivor"));
    }

    #[test]
    fn test_load_missing_file_returns_default() {
        let compat = Compatibility::load(Path::new("/nonexistent.toml"));
        assert!(compat.maps.is_empty());
    }

    #[test]
    fn test_resolve_bsp_returns_err_when_install_missing() {
        let result = resolve_bsp(Path::new("/nonexistent/cs/install"), "de_dust2");
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("de_dust2"), "error should mention map name");
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test maplist
```
Expected: all pass.

- [ ] **Step 3: Commit**

```bash
git add src/maplist.rs
git commit -m "feat: maplist module with BSP path resolution and compatibility tracking"
```

---

## Chunk 2: BSP Loading Pipeline

### Task 4: Entity lump parser

**Files:**
- Create: `src/bsp/entity.rs`
- Create: `src/bsp/mod.rs` (stub)

- [ ] **Step 1: Create `src/bsp/mod.rs` stub**

```rust
pub mod entity;
pub mod wad;
pub mod parse;

pub use parse::MeshData;
```

And add `mod bsp;` is already in `main.rs`.

- [ ] **Step 2: Write `src/bsp/entity.rs` with tests**

```rust
use glam::Vec3;

/// Classnames whose `origin` field becomes a camera waypoint.
const WAYPOINT_CLASSNAMES: &[&str] = &[
    "info_player_start",
    "info_player_deathmatch",
    "func_bombsite",
    "hostage_entity",
];

/// Parse the GoldSrc entity lump (plain text key-value blocks) and return
/// the `origin` vectors for all entities whose classname is in WAYPOINT_CLASSNAMES.
/// Returns an error if fewer than 4 origins are found.
pub fn extract_waypoints(entity_lump: &str) -> anyhow::Result<Vec<Vec3>> {
    let mut waypoints = Vec::new();

    for block in entity_lump.split('{') {
        let block = block.trim();
        if block.is_empty() { continue; }

        let classname = parse_value(block, "classname").unwrap_or("");
        if !WAYPOINT_CLASSNAMES.contains(&classname) { continue; }

        if let Some(origin_str) = parse_value(block, "origin") {
            if let Some(v) = parse_origin(origin_str) {
                waypoints.push(v);
            }
        }
    }

    anyhow::ensure!(
        waypoints.len() >= 4,
        "entity lump yielded {} waypoints, minimum 4 required for Catmull-Rom spline",
        waypoints.len()
    );

    Ok(waypoints)
}

fn parse_value<'a>(block: &'a str, key: &str) -> Option<&'a str> {
    for line in block.lines() {
        let line = line.trim();
        // Format: "key" "value"
        if let Some(rest) = line.strip_prefix('"') {
            if let Some((k, after_key)) = rest.split_once('"') {
                if k == key {
                    let value = after_key.trim().trim_matches('"');
                    return Some(value);
                }
            }
        }
    }
    None
}

fn parse_origin(s: &str) -> Option<Vec3> {
    let mut parts = s.split_whitespace();
    let x: f32 = parts.next()?.parse().ok()?;
    let y: f32 = parts.next()?.parse().ok()?;
    let z: f32 = parts.next()?.parse().ok()?;
    Some(Vec3::new(x, y, z))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_LUMP: &str = r#"
{
"classname" "worldspawn"
"sky" "desert"
}
{
"classname" "info_player_start"
"origin" "100 200 0"
}
{
"classname" "info_player_start"
"origin" "150 250 0"
}
{
"classname" "info_player_deathmatch"
"origin" "-100 -200 0"
}
{
"classname" "func_bombsite"
"origin" "400 400 0"
}
{
"classname" "light"
"origin" "0 0 128"
}
"#;

    #[test]
    fn test_extract_four_waypoints() {
        let pts = extract_waypoints(SAMPLE_LUMP).unwrap();
        assert_eq!(pts.len(), 4);
    }

    #[test]
    fn test_worldspawn_excluded() {
        let pts = extract_waypoints(SAMPLE_LUMP).unwrap();
        // worldspawn has no origin in our sample, and is not a waypoint classname
        assert!(pts.iter().all(|p| p != &Vec3::ZERO));
    }

    #[test]
    fn test_origin_parsed_correctly() {
        let pts = extract_waypoints(SAMPLE_LUMP).unwrap();
        assert!(pts.contains(&Vec3::new(100.0, 200.0, 0.0)));
        assert!(pts.contains(&Vec3::new(400.0, 400.0, 0.0)));
    }

    #[test]
    fn test_fewer_than_four_returns_error() {
        let lump = r#"
{
"classname" "info_player_start"
"origin" "0 0 0"
}
"#;
        assert!(extract_waypoints(lump).is_err());
    }

    #[test]
    fn test_empty_lump_returns_error() {
        assert!(extract_waypoints("").is_err());
    }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test entity
```
Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add src/bsp/mod.rs src/bsp/entity.rs
git commit -m "feat: BSP entity lump parser with waypoint extraction"
```

---

### Task 5: WAD texture loading

**Files:**
- Create: `src/bsp/wad.rs`

- [ ] **Step 1: Read `goldsrc-rs` WAD API**

Before writing this module, read the goldsrc-rs source or docs to understand how to:
1. Open a WAD file
2. List entries by name
3. Extract texture data as raw bytes (width, height, RGBA or indexed)

Adjust the implementation below to match the actual API. The pattern below uses a plausible API — adapt as needed.

- [ ] **Step 2: Write `src/bsp/wad.rs`**

```rust
use anyhow::{Context, Result};
use guillotiere::{AtlasAllocator, Size};
use image::{ImageBuffer, Rgba, RgbaImage};
use std::collections::HashMap;
use std::path::Path;

/// Maximum atlas dimension. GoldSrc textures are small (max 512×512 per texture).
const ATLAS_SIZE: i32 = 4096;

pub struct TextureAtlas {
    pub image: RgbaImage,
    /// Map from texture name to normalized UV rect: (u_min, v_min, u_max, v_max)
    pub uvs: HashMap<String, [f32; 4]>,
}

/// Load all textures listed in `texture_names` from the WAD files at `wad_paths`.
/// Returns a packed atlas and per-texture UV rects.
pub fn load_textures(
    texture_names: &[String],
    wad_paths: &[impl AsRef<Path>],
) -> Result<TextureAtlas> {
    // Step 1: load raw RGBA data for each texture from WADs
    let mut raw: HashMap<String, (u32, u32, Vec<u8>)> = HashMap::new();

    for wad_path in wad_paths {
        let wad_path = wad_path.as_ref();
        // goldsrc-rs WAD loading — adapt to actual crate API
        let wad = goldsrc_format::wad::Wad::open(wad_path)
            .with_context(|| format!("opening WAD: {}", wad_path.display()))?;

        for name in texture_names {
            if raw.contains_key(name) { continue; }
            if let Ok(entry) = wad.texture(name) {
                // Convert indexed + palette to RGBA
                let rgba = indexed_to_rgba(&entry.data, &entry.palette, entry.width, entry.height);
                raw.insert(name.clone(), (entry.width, entry.height, rgba));
            }
        }
    }

    // Step 2: pack into atlas
    let mut allocator = AtlasAllocator::new(Size::new(ATLAS_SIZE, ATLAS_SIZE));
    let mut atlas_img: RgbaImage = ImageBuffer::new(ATLAS_SIZE as u32, ATLAS_SIZE as u32);
    let mut uvs = HashMap::new();

    for (name, (w, h, pixels)) in &raw {
        let alloc = allocator
            .allocate(Size::new(*w as i32, *h as i32))
            .with_context(|| format!("atlas full, could not fit texture '{name}'"))?;

        let rect = alloc.rectangle;
        for row in 0..*h {
            for col in 0..*w {
                let src_idx = ((row * w + col) * 4) as usize;
                let px = Rgba([pixels[src_idx], pixels[src_idx+1], pixels[src_idx+2], pixels[src_idx+3]]);
                atlas_img.put_pixel(rect.min.x as u32 + col, rect.min.y as u32 + row, px);
            }
        }

        let u_min = rect.min.x as f32 / ATLAS_SIZE as f32;
        let v_min = rect.min.y as f32 / ATLAS_SIZE as f32;
        let u_max = rect.max.x as f32 / ATLAS_SIZE as f32;
        let v_max = rect.max.y as f32 / ATLAS_SIZE as f32;
        uvs.insert(name.clone(), [u_min, v_min, u_max, v_max]);
    }

    Ok(TextureAtlas { image: atlas_img, uvs })
}

fn indexed_to_rgba(data: &[u8], palette: &[[u8; 3]], width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((width * height * 4) as usize);
    for &idx in data.iter().take((width * height) as usize) {
        let color = palette.get(idx as usize).copied().unwrap_or([255, 0, 255]);
        out.extend_from_slice(&[color[0], color[1], color[2], 255]);
    }
    out
}
```

**Note:** The `goldsrc_format::wad::Wad` API above is illustrative — adapt field names and method calls to match the actual goldsrc-rs crate once you've read its source. The palette conversion logic and indexed texture handling are standard GoldSrc WAD format behavior.

- [ ] **Step 3: Run `cargo check`**

```bash
cargo check
```
Fix any API mismatches against the actual goldsrc-rs crate. No unit tests for this module — WAD loading requires real WAD files; it will be covered by the integration test in Task 9.

- [ ] **Step 4: Commit**

```bash
git add src/bsp/wad.rs
git commit -m "feat: WAD texture loading with guillotiere atlas packing"
```

---

### Task 6: BSP parse and MeshData assembly

**Files:**
- Create: `src/bsp/parse.rs`

- [ ] **Step 1: Read `qbsp` GoldSrc BSP30 API**

Before writing, read the qbsp crate source or docs to understand:
1. How to load a BSP file (function signature, return type)
2. How to access the triangle mesh (vertices, indices, UVs, lightmap UVs)
3. How to access the lightmap atlas bytes
4. How to access per-face surface flags (for `SURF_SKY` detection)
5. How to access the entity lump string
6. How to access the texture name list

If qbsp does not expose surface flags for GoldSrc BSP30, use texture name prefix `"sky"` as the fallback for sky face detection. Document which method was used in a code comment.

- [ ] **Step 2: Write `src/bsp/parse.rs`**

```rust
use anyhow::{Context, Result};
use glam::Vec3;
use image::RgbaImage;
use std::path::Path;

use crate::bsp::entity::extract_waypoints;
use crate::bsp::wad::{load_textures, TextureAtlas};

/// Vertex layout — 32 bytes, 3 attributes.
/// @location(0): position (XYZ world-space)
/// @location(1): diffuse UV (into diffuse atlas)
/// @location(2): lightmap UV (into lightmap atlas)
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub diffuse_uv: [f32; 2],
    pub lightmap_uv: [f32; 2],
}

/// CPU-side mesh data. No wgpu dependency. Uploaded by renderer after device init.
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    /// Geometry indices first (0..sky_index_offset), sky indices after.
    pub indices: Vec<u32>,
    /// Index into `indices` where sky faces begin.
    pub sky_index_offset: u32,
    pub diffuse_atlas: RgbaImage,
    pub lightmap_atlas: RgbaImage,
    /// Unsorted entity origins. Camera module applies nearest-neighbor sort.
    pub entity_origins: Vec<Vec3>,
}

pub fn load(bsp_path: &Path, cs_install_path: &Path) -> Result<MeshData> {
    // --- Step 1: Parse BSP via qbsp ---
    // Adapt to actual qbsp API. Below is illustrative.
    let bsp = qbsp::load(bsp_path)
        .with_context(|| format!("qbsp failed to load {}", bsp_path.display()))?;

    // --- Step 2: Extract entity waypoints ---
    let entity_origins = extract_waypoints(bsp.entity_lump())
        .context("entity lump parse")?;

    // --- Step 3: Determine WAD paths from BSP texture lump ---
    // BSP texture lump contains WAD filenames (e.g. "de_dust.wad").
    // Resolve them relative to the game subdirectory.
    let game_dirs = ["cstrike", "czero"];
    let wad_names = bsp.wad_list(); // adapt to actual qbsp API
    let wad_paths: Vec<_> = wad_names.iter().flat_map(|wad_name| {
        let filename = Path::new(wad_name).file_name()?;
        let path = game_dirs.iter().find_map(|dir| {
            let p = cs_install_path.join(dir).join(filename);
            p.exists().then_some(p)
        })?;
        Some(path)
    }).collect();

    // --- Step 4: Load WAD textures into diffuse atlas ---
    let texture_names: Vec<String> = bsp.texture_names().map(String::from).collect();
    let diffuse = load_textures(&texture_names, &wad_paths)
        .context("WAD texture loading")?;

    // --- Step 5: Build vertex and index buffers ---
    // qbsp provides per-face data; adapt field access to actual API.
    let mut geo_verts: Vec<Vertex> = Vec::new();
    let mut geo_idx: Vec<u32> = Vec::new();
    let mut sky_verts: Vec<Vertex> = Vec::new();
    let mut sky_idx: Vec<u32> = Vec::new();

    for face in bsp.faces() {
        let is_sky = is_sky_face(&face); // see helper below

        let tex_name = face.texture_name();
        let uv_rect = diffuse.uvs.get(tex_name).copied().unwrap_or([0.0, 0.0, 1.0, 1.0]);

        let face_verts: Vec<Vertex> = face.triangles().map(|tri_vert| {
            // Remap diffuse UV from face-local to atlas UV
            let du = uv_rect[0] + tri_vert.diffuse_uv[0] * (uv_rect[2] - uv_rect[0]);
            let dv = uv_rect[1] + tri_vert.diffuse_uv[1] * (uv_rect[3] - uv_rect[1]);
            Vertex {
                position: tri_vert.position.into(),
                diffuse_uv: [du, dv],
                lightmap_uv: tri_vert.lightmap_uv.into(),
            }
        }).collect();

        if is_sky {
            let base = sky_verts.len() as u32;
            sky_idx.extend((0..face_verts.len() as u32).map(|i| base + i));
            sky_verts.extend(face_verts);
        } else {
            let base = geo_verts.len() as u32;
            geo_idx.extend((0..face_verts.len() as u32).map(|i| base + i));
            geo_verts.extend(face_verts);
        }
    }

    // Merge into single buffer: geometry first, sky appended
    let sky_index_offset = geo_idx.len() as u32;
    let mut vertices = geo_verts;
    let sky_base = vertices.len() as u32;
    vertices.extend(sky_verts);
    let mut indices = geo_idx;
    indices.extend(sky_idx.iter().map(|i| i + sky_base));

    // --- Step 6: Lightmap atlas ---
    let lightmap_atlas: RgbaImage = bsp.lightmap_atlas_rgba()
        .context("building lightmap atlas")?;

    Ok(MeshData {
        vertices,
        indices,
        sky_index_offset,
        diffuse_atlas: diffuse.image,
        lightmap_atlas,
        entity_origins,
    })
}

/// Identify sky faces by surface flag or texture name prefix.
/// Primary: SURF_SKY flag from qbsp face flags (adapt to actual API).
/// Fallback: texture name begins with "sky".
fn is_sky_face(face: &impl FaceLike) -> bool {
    // Try surface flag first
    if let Some(flags) = face.surface_flags() {
        return flags & SURF_SKY != 0;
    }
    // Fallback: texture name prefix
    face.texture_name().starts_with("sky")
}

// Adapt or remove this constant based on the actual qbsp surface flag values
const SURF_SKY: u32 = 0x4; // GoldSrc SURF_SKY flag value — verify against BSP30 spec

// Trait for duck-typing the face API — replace with actual qbsp face type
trait FaceLike {
    fn texture_name(&self) -> &str;
    fn surface_flags(&self) -> Option<u32>;
    fn triangles(&self) -> impl Iterator<Item = TriVert>;
}

struct TriVert {
    pub position: Vec3,
    pub diffuse_uv: [f32; 2],
    pub lightmap_uv: [f32; 2],
}
```

**Note:** Add `bytemuck = "1"` to `[dependencies]` in `Cargo.toml` for the `Pod`/`Zeroable` derives. The `FaceLike` trait and `TriVert` struct above are stand-ins — replace them with the actual qbsp types once you've read its API. The structural logic (geo/sky partition, sky_index_offset) is correct and should not change.

- [ ] **Step 3: Add bytemuck to Cargo.toml**

```toml
bytemuck = { version = "1", features = ["derive"] }
```

- [ ] **Step 4: Run `cargo check`**

```bash
cargo check
```
Fix API mismatches. The goal is clean compilation, not full correctness — integration test in Task 9 verifies correctness.

- [ ] **Step 5: Commit**

```bash
git add src/bsp/parse.rs Cargo.toml
git commit -m "feat: BSP parse module producing CPU-side MeshData"
```

---

## Chunk 3: Camera, Renderer, Input, Integration

### Task 7: Camera system

**Files:**
- Create: `src/camera.rs`

- [ ] **Step 1: Write `src/camera.rs` with tests**

```rust
use glam::{Mat4, Vec3};
use std::time::Instant;

pub struct Camera {
    waypoints: Vec<Vec3>,  // sorted by nearest-neighbor
    t: f32,                // spline parameter, 0.0..1.0 over full loop
    speed: f32,            // units/sec
    bob_amplitude: f32,
    bob_frequency: f32,
    start_time: Instant,
}

impl Camera {
    /// Create a camera from unsorted entity origins.
    /// Returns Err if fewer than 4 origins are provided.
    pub fn new(
        origins: Vec<Vec3>,
        speed: f32,
        bob_amplitude: f32,
        bob_frequency: f32,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(origins.len() >= 4, "need at least 4 waypoints, got {}", origins.len());
        let waypoints = nearest_neighbor_sort(origins);
        Ok(Self {
            waypoints,
            t: 0.0,
            speed,
            bob_amplitude,
            bob_frequency,
            start_time: Instant::now(),
        })
    }

    /// Advance the spline parameter and return the view matrix.
    pub fn update(&mut self, delta_secs: f32) -> Mat4 {
        let n = self.waypoints.len() as f32;
        self.t = (self.t + self.speed * delta_secs / (n * 256.0)) % 1.0;

        let pos = catmull_rom_position(&self.waypoints, self.t);
        let forward = catmull_rom_tangent(&self.waypoints, self.t).normalize_or_zero();
        let elapsed = self.start_time.elapsed().as_secs_f32();
        let bob = self.bob_amplitude * (elapsed * self.bob_frequency * std::f32::consts::TAU).sin();

        // Eye height: +64 units above waypoint Z, plus bob
        let eye = pos + Vec3::new(0.0, 0.0, 64.0 + bob);
        let target = eye + forward;
        let up = Vec3::Z;

        Mat4::look_at_rh(eye, target, up)
    }
}

/// Sort waypoints using nearest-neighbor starting from index 0.
fn nearest_neighbor_sort(mut pts: Vec<Vec3>) -> Vec<Vec3> {
    if pts.is_empty() { return pts; }
    let mut sorted = Vec::with_capacity(pts.len());
    sorted.push(pts.remove(0));
    while !pts.is_empty() {
        let last = *sorted.last().unwrap();
        let nearest = pts.iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                last.distance_squared(**a)
                    .partial_cmp(&last.distance_squared(**b))
                    .unwrap()
            })
            .map(|(i, _)| i)
            .unwrap();
        sorted.push(pts.remove(nearest));
    }
    sorted
}

/// Closed Catmull-Rom spline position at parameter t ∈ [0,1].
fn catmull_rom_position(pts: &[Vec3], t: f32) -> Vec3 {
    let n = pts.len();
    let scaled = t * n as f32;
    let i = scaled.floor() as usize;
    let local_t = scaled - i.floor() as f32;

    let p0 = pts[(i + n - 1) % n];
    let p1 = pts[i % n];
    let p2 = pts[(i + 1) % n];
    let p3 = pts[(i + 2) % n];

    catmull_rom(p0, p1, p2, p3, local_t)
}

/// Closed Catmull-Rom tangent at parameter t ∈ [0,1] (for forward direction).
fn catmull_rom_tangent(pts: &[Vec3], t: f32) -> Vec3 {
    let epsilon = 0.001_f32;
    let t1 = (t + epsilon) % 1.0;
    let t0 = (t - epsilon + 1.0) % 1.0;
    (catmull_rom_position(pts, t1) - catmull_rom_position(pts, t0)) / (2.0 * epsilon)
}

fn catmull_rom(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, t: f32) -> Vec3 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * (
        (2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn four_square_pts() -> Vec<Vec3> {
        vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1000.0, 0.0, 0.0),
            Vec3::new(1000.0, 1000.0, 0.0),
            Vec3::new(0.0, 1000.0, 0.0),
        ]
    }

    #[test]
    fn test_new_requires_four_points() {
        assert!(Camera::new(vec![], 133.0, 2.0, 2.0).is_err());
        assert!(Camera::new(vec![Vec3::ZERO; 3], 133.0, 2.0, 2.0).is_err());
        assert!(Camera::new(four_square_pts(), 133.0, 2.0, 2.0).is_ok());
    }

    #[test]
    fn test_nearest_neighbor_sort_visits_all() {
        let pts = four_square_pts();
        let sorted = nearest_neighbor_sort(pts.clone());
        assert_eq!(sorted.len(), pts.len());
        // All original points should appear
        for p in &pts {
            assert!(sorted.contains(p));
        }
    }

    #[test]
    fn test_catmull_rom_at_t0_equals_p1() {
        let pts = four_square_pts();
        let pos = catmull_rom_position(&pts, 0.0);
        assert!((pos - pts[0]).length() < 0.01);
    }

    #[test]
    fn test_catmull_rom_loops_smoothly() {
        let pts = four_square_pts();
        let pos_start = catmull_rom_position(&pts, 0.0);
        let pos_near_end = catmull_rom_position(&pts, 0.999);
        // Near-end should be close to start for a closed spline
        assert!((pos_start - pos_near_end).length() < 50.0);
    }

    #[test]
    fn test_update_returns_matrix() {
        let mut cam = Camera::new(four_square_pts(), 133.0, 2.0, 2.0).unwrap();
        let mat = cam.update(0.016);
        // Matrix should not be identity (camera is positioned)
        assert_ne!(mat, Mat4::IDENTITY);
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test camera
```
Expected: all pass.

- [ ] **Step 3: Commit**

```bash
git add src/camera.rs
git commit -m "feat: camera module with nearest-neighbor sort and Catmull-Rom spline"
```

---

### Task 8: Input module

**Files:**
- Create: `src/input.rs`

- [ ] **Step 1: Write `src/input.rs`**

```rust
/// Mouse delta magnitude must exceed this threshold to trigger screensaver exit.
/// Filters sub-pixel hardware jitter. Known trade-off: deliberate micro-movements
/// below threshold won't exit. This is a compile-time constant.
pub const MOUSE_EXIT_THRESHOLD: f64 = 10.0;

/// Returns true if the given mouse delta should trigger screensaver exit.
pub fn should_exit_on_mouse(delta: (f64, f64)) -> bool {
    let (dx, dy) = delta;
    (dx * dx + dy * dy).sqrt() > MOUSE_EXIT_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_large_delta_exits() {
        assert!(should_exit_on_mouse((20.0, 0.0)));
        assert!(should_exit_on_mouse((0.0, 15.0)));
        assert!(should_exit_on_mouse((10.1, 0.0)));
    }

    #[test]
    fn test_small_delta_does_not_exit() {
        assert!(!should_exit_on_mouse((0.0, 0.0)));
        assert!(!should_exit_on_mouse((1.0, 1.0)));
        assert!(!should_exit_on_mouse((7.0, 7.0)));
    }

    #[test]
    fn test_threshold_boundary() {
        // exactly at threshold: should NOT exit (strictly greater than)
        assert!(!should_exit_on_mouse((MOUSE_EXIT_THRESHOLD, 0.0)));
    }
}
```

- [ ] **Step 2: Run tests**

```bash
cargo test input
```
Expected: all pass.

- [ ] **Step 3: Commit**

```bash
git add src/input.rs
git commit -m "feat: input module with mouse exit threshold"
```

---

### Task 9: Renderer

**Files:**
- Create: `src/renderer.rs`

- [ ] **Step 1: Write `src/renderer.rs`**

This is the largest module. It owns the winit event loop and wgpu device. No unit tests — verified manually and by the integration test.

```rust
use anyhow::{Context, Result};
use glam::Mat4;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use wgpu::util::DeviceExt;
use winit::{
    event::{DeviceEvent, Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    window::{Fullscreen, WindowBuilder},
};

use crate::bsp::parse::{MeshData, Vertex};
use crate::camera::Camera;
use crate::input::should_exit_on_mouse;

pub fn run(mesh: MeshData, mut camera: Camera) -> Result<()> {
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = Arc::clone(&shutdown);

    let event_loop = EventLoop::new().context("creating event loop")?;
    let monitor = event_loop.primary_monitor()
        .or_else(|| event_loop.available_monitors().next())
        .context("no monitor found")?;

    let window = WindowBuilder::new()
        .with_title("cs-flythrough")
        .with_fullscreen(Some(Fullscreen::Borderless(Some(monitor))))
        .build(&event_loop)
        .context("creating window")?;

    // Hide cursor
    window.set_cursor_visible(false);

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12 | wgpu::Backends::VULKAN,
        ..Default::default()
    });

    let surface = instance.create_surface(&window).context("creating surface")?;

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        compatible_surface: Some(&surface),
        force_fallback_adapter: false,
    })).context("no compatible GPU adapter found — ensure DirectX 12 or Vulkan drivers are installed")?;

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor::default(),
        None,
    )).context("creating wgpu device")?;

    let size = window.inner_size();
    let surface_format = surface.get_capabilities(&adapter).formats[0];
    let mut config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width: size.width,
        height: size.height,
        present_mode: wgpu::PresentMode::AutoVsync,
        alpha_mode: wgpu::CompositeAlphaMode::Auto,
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
    };
    surface.configure(&device, &config);

    // Upload vertex + index buffers
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

    // Upload diffuse atlas
    let diffuse_tex = upload_rgba_texture(&device, &queue, &mesh.diffuse_atlas, "diffuse");
    let lightmap_tex = upload_rgba_texture(&device, &queue, &mesh.lightmap_atlas, "lightmap");

    // Build pipelines
    let geo_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("geometry"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/geometry.wgsl").into()),
    });
    let sky_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("sky"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/sky.wgsl").into()),
    });

    let vertex_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex>() as u64,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 12, shader_location: 1 },
            wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 20, shader_location: 2 },
        ],
    };

    // ViewProj uniform buffer
    let vp_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("view_proj"),
        size: 64, // Mat4 = 16 x f32 = 64 bytes
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    // Bind group layouts and pipelines omitted for brevity — implement following
    // standard wgpu patterns for texture + uniform bindings.
    // See: https://sotrh.github.io/learn-wgpu/beginner/tutorial5-textures/
    // Geometry pipeline: binds vp_buf, diffuse_tex, lightmap_tex
    // Sky pipeline: binds vp_buf only

    let mut last_frame = std::time::Instant::now();
    let sky_index_first = sky_index_offset;

    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Poll);

        if shutdown_clone.load(Ordering::Relaxed) {
            elwt.exit();
            return;
        }

        match event {
            Event::DeviceEvent { event: DeviceEvent::MouseMotion { delta }, .. } => {
                if should_exit_on_mouse(delta) {
                    shutdown_clone.store(true, Ordering::Relaxed);
                }
            }
            Event::WindowEvent { event: WindowEvent::KeyboardInput { .. }, .. } => {
                shutdown_clone.store(true, Ordering::Relaxed);
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                elwt.exit();
            }
            Event::WindowEvent { event: WindowEvent::Resized(new_size), .. } => {
                config.width = new_size.width;
                config.height = new_size.height;
                surface.configure(&device, &config);
            }
            Event::AboutToWait => {
                let now = std::time::Instant::now();
                let delta_secs = (now - last_frame).as_secs_f32();
                last_frame = now;

                let view = camera.update(delta_secs);
                let aspect = config.width as f32 / config.height as f32;
                let proj = Mat4::perspective_rh(
                    90_f32.to_radians(),
                    aspect,
                    4.0,    // near clip
                    4096.0, // far clip
                );
                let vp: [[f32; 4]; 4] = (proj * view).to_cols_array_2d();
                queue.write_buffer(&vp_buf, 0, bytemuck::cast_slice(&vp));

                let frame = match surface.get_current_texture() {
                    Ok(f) => f,
                    Err(_) => return,
                };
                let view_tex = frame.texture.create_view(&Default::default());
                let mut encoder = device.create_command_encoder(&Default::default());

                {
                    let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: None,
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view_tex,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        ..Default::default()
                    });

                    rpass.set_vertex_buffer(0, vertex_buf.slice(..));
                    rpass.set_index_buffer(index_buf.slice(..), wgpu::IndexFormat::Uint32);

                    // Geometry pass — TODO: set geo pipeline + bind group
                    // rpass.set_pipeline(&geo_pipeline);
                    // rpass.set_bind_group(0, &geo_bind_group, &[]);
                    rpass.draw_indexed(0..geo_index_count, 0, 0..1);

                    // Sky pass — TODO: set sky pipeline + bind group
                    // rpass.set_pipeline(&sky_pipeline);
                    // rpass.set_bind_group(0, &sky_bind_group, &[]);
                    rpass.draw_indexed(sky_index_first..sky_index_first + sky_index_count, 0, 0..1);
                }

                queue.submit(std::iter::once(encoder.finish()));
                frame.present();
            }
            _ => {}
        }
    }).context("event loop error")?;

    Ok(())
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
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
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
        wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4 * width), rows_per_image: None },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    texture
}
```

**Note:** The bind group layout, pipeline creation, and bind group setup for geometry and sky passes are marked with `// TODO:` comments. These follow standard wgpu patterns — implement following the wgpu texture tutorial (https://sotrh.github.io/learn-wgpu/beginner/tutorial5-textures/). Add `pollster = "0.3"` to `Cargo.toml` dependencies for `block_on`.

- [ ] **Step 2: Add `pollster` to Cargo.toml**

```toml
pollster = "0.3"
```

- [ ] **Step 3: Run `cargo check`**

```bash
cargo check
```
Fix compilation errors. The `// TODO:` pipeline sections will cause runtime panics if called without implementation — complete them before running.

- [ ] **Step 4: Complete pipeline creation**

Implement the geometry and sky `RenderPipeline` creation and `BindGroup` setup following the wgpu patterns. Place this code in `run()` after the buffer uploads. The geometry pipeline needs three bindings (view_proj uniform, diffuse texture + sampler, lightmap texture + sampler). The sky pipeline needs one binding (view_proj uniform).

- [ ] **Step 5: Commit**

```bash
git add src/renderer.rs Cargo.toml
git commit -m "feat: wgpu renderer with geometry and sky render passes"
```

---

### Task 10: Wire startup sequence in main.rs

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Replace `run_screensaver` placeholder**

```rust
use anyhow::Result;
use std::path::Path;

mod config;
mod maplist;
mod bsp;
mod camera;
mod renderer;
mod input;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("/s");

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

fn run_screensaver() -> Result<()> {
    let binary_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| Path::new(".").to_path_buf());

    let config_path = binary_dir.join("cs-flythrough.toml");
    let compat_path = binary_dir.join("map-compatibility.toml");

    // Generate default config and exit if missing
    if !config_path.exists() {
        config::Config::write_default(&config_path)?;
        eprintln!(
            "Created default config at {}. Edit cs_install_path and run again.",
            config_path.display()
        );
        return Ok(());
    }

    let cfg = config::Config::load(&config_path)?;

    if !cfg.cs_install_path.exists() {
        eprintln!(
            "CS install path not found: {}. Edit cs-flythrough.toml.",
            cfg.cs_install_path.display()
        );
        std::process::exit(1);
    }

    let map_name = cfg.map.as_deref().unwrap_or("de_dust2");
    let mut compat = maplist::Compatibility::load(&compat_path);

    if compat.is_excluded(map_name) {
        eprintln!("Map '{map_name}' is marked failed in map-compatibility.toml. Remove the entry to retry.");
        std::process::exit(1);
    }

    let bsp_path = maplist::resolve_bsp(&cfg.cs_install_path, map_name)?;

    let mesh = match bsp::load(&bsp_path, &cfg.cs_install_path) {
        Ok(m) => {
            compat.set_ok(map_name);
            compat.save(&compat_path)?;
            m
        }
        Err(e) => {
            compat.set_failed(map_name, format!("{e:#}"));
            compat.save(&compat_path)?;
            return Err(e);
        }
    };

    let cam = camera::Camera::new(
        mesh.entity_origins.clone(),
        cfg.camera_speed,
        cfg.bob_amplitude,
        cfg.bob_frequency,
    )?;

    renderer::run(mesh, cam)
}
```

- [ ] **Step 2: Run `cargo check`**

```bash
cargo check
```

- [ ] **Step 3: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire startup sequence in main — config, maplist, bsp, camera, renderer"
```

---

### Task 11: Integration test (headless BSP load)

**Files:**
- Create: `tests/bsp_integration.rs`

- [ ] **Step 1: Write the integration test**

This test requires a real `de_dust2.bsp` and WAD files. It skips gracefully if the CS install is not present (for CI environments).

```rust
use std::path::PathBuf;

/// Path to your CS 1.6 or CZ install. Set via environment variable CS_INSTALL_PATH
/// or edit the fallback path below.
fn cs_install_path() -> Option<PathBuf> {
    std::env::var("CS_INSTALL_PATH")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            let fallback = PathBuf::from("C:/Program Files (x86)/Steam/steamapps/common/Counter-Strike");
            fallback.exists().then_some(fallback)
        })
}

#[test]
fn test_de_dust2_loads() {
    let Some(install) = cs_install_path() else {
        eprintln!("Skipping integration test: CS install not found. Set CS_INSTALL_PATH env var.");
        return;
    };

    let bsp_path = cs_flythrough::maplist::resolve_bsp(&install, "de_dust2")
        .expect("de_dust2.bsp not found in CS install");

    let mesh = cs_flythrough::bsp::load(&bsp_path, &install)
        .expect("BSP load failed");

    assert!(!mesh.vertices.is_empty(), "no vertices");
    assert!(!mesh.indices.is_empty(), "no indices");
    assert!(mesh.entity_origins.len() >= 4, "fewer than 4 entity origins");
    assert!(mesh.sky_index_offset <= mesh.indices.len() as u32);
    println!(
        "de_dust2: {} vertices, {} indices, {} waypoints, sky_offset={}",
        mesh.vertices.len(),
        mesh.indices.len(),
        mesh.entity_origins.len(),
        mesh.sky_index_offset,
    );
}
```

- [ ] **Step 2: Expose library API for integration test**

Integration tests use the crate as a library. Add `lib.rs` or update `Cargo.toml` to expose modules:

In `Cargo.toml`:
```toml
[lib]
name = "cs_flythrough"
path = "src/lib.rs"
```

Create `src/lib.rs`:
```rust
pub mod bsp;
pub mod camera;
pub mod config;
pub mod input;
pub mod maplist;
// renderer is NOT exported — it owns an event loop and would panic in headless test environments
```

- [ ] **Step 3: Run integration test**

```bash
CS_INSTALL_PATH="C:/Program Files (x86)/Steam/steamapps/common/Counter-Strike" cargo test --test bsp_integration -- --nocapture
```
Expected: test passes, prints vertex/index/waypoint counts.

- [ ] **Step 4: Commit**

```bash
git add tests/bsp_integration.rs src/lib.rs Cargo.toml
git commit -m "test: headless BSP integration test for de_dust2"
```

---

### Task 12: First run and manual tuning

- [ ] **Step 1: Build release binary**

```bash
cargo build --release
```
Expected: compiles with no warnings. Fix any remaining issues.

- [ ] **Step 2: Create config and run**

Copy `cs-flythrough.exe` to a test directory. Run once to generate default config:
```bash
./cs-flythrough.exe /s
```
Expected: creates `cs-flythrough.toml`, prints "Edit cs_install_path and run again."

Edit `cs-flythrough.toml` to point to your CS install path.

- [ ] **Step 3: Run screensaver**

```bash
./cs-flythrough.exe /s
```
Expected: fullscreen window opens showing de_dust2 with textured + lightmapped geometry, camera moving smoothly through the map. Mouse movement or any key closes it.

- [ ] **Step 4: Tune camera feel**

Adjust in `cs-flythrough.toml` until the movement feels like the maze screensaver:
- `camera_speed` — increase if too slow, decrease if too fast
- `bob_amplitude` — reduce if bobbing is too dramatic
- `bob_frequency` — adjust cycles per second

- [ ] **Step 5: Verify map-compatibility.toml**

After a successful run, check that `map-compatibility.toml` contains:
```toml
[maps]
"de_dust2" = { status = "ok" }
```

- [ ] **Step 6: Final commit**

```bash
git add -A
git commit -m "feat: cs-flythrough skateboard complete — de_dust2 flythrough working"
```
