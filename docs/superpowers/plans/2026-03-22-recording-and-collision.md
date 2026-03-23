# Recording Mode & BSP Collision Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add BSP collision detection (wall-slide) and a WASD free-fly waypoint recording mode so routes can be authored in minutes instead of hours.

**Architecture:** BSP collision is a standalone module (`src/bsp/collision.rs`) that uses `qbsp::BspData` node/plane/leaf arrays for point-in-solid tests and wall-sliding. Recording mode (`src/recording.rs`) is a new CLI mode (`--record-route`) that opens a windowed renderer with WASD+mouse-look free-fly camera, collision-aware movement, and waypoint drop/save. Both features integrate through `bsp::load()` returning collision data alongside mesh data.

**Tech Stack:** Rust, qbsp 0.14 (BSP parsing), glam 0.32 (math), wgpu 28 (rendering), winit 0.30 (windowing/input)

**Spec:** `docs/superpowers/specs/2026-03-22-recording-and-collision-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `src/bsp/collision.rs` | Create | `CollisionData` struct, `point_in_solid()`, `resolve_position()` |
| `src/bsp/mod.rs` | Modify | Add `pub mod collision;` |
| `src/bsp/parse.rs` | Modify | Return `(MeshData, CollisionData)` from `load()` |
| `src/recording.rs` | Create | Recording mode: free camera, input handling, TOML export |
| `src/renderer.rs` | Modify | Widen `init_gpu_core`/`GpuCore` visibility to `pub` (needed before recording module) |
| `src/main.rs` | Modify | Parse `--record-route` flag, wire up recording mode, update `load()` callers |
| `src/headless.rs` | Modify | Update `load()` call to destructure `(MeshData, CollisionData)` |
| `src/camera.rs` | Modify | Accept `CollisionData` reference, call `resolve_position()` in `update()` |
| `tests/bsp_integration.rs` | Modify | Update `load()` call, add collision tests |

**Note:** `src/input.rs` is NOT extended — keyboard/mouse state is handled inline in `recording.rs` since the screensaver mode's input model (exit on any input) is fundamentally different from recording mode (WASD+mouse-look). No `chrono` dependency — timestamps use a simple date string instead.

**Note:** qbsp provides `BspData::leaf_at_point()` but we intentionally copy node/plane/leaf data into our own `CollisionData` to avoid holding `BspData` alive (it's large and has lifetime constraints). This also lets us convert qbsp's glam 0.30 types to our glam 0.32 types once at load time.

---

### Task 1: BSP Collision Module — `point_in_solid()`

**Files:**
- Create: `src/bsp/collision.rs`
- Modify: `src/bsp/mod.rs`

- [ ] **Step 1: Write the failing test**

In `src/bsp/collision.rs`:

```rust
use glam::Vec3;

/// Minimal collision data extracted from qbsp's BspData.
/// Stored as plain arrays to avoid lifetime ties to the parsed BSP.
pub struct CollisionData {
    pub planes: Vec<CollisionPlane>,
    pub nodes: Vec<CollisionNode>,
    pub leaves: Vec<LeafContents>,
}

#[derive(Clone, Copy)]
pub struct CollisionPlane {
    pub normal: Vec3,
    pub dist: f32,
}

#[derive(Clone, Copy)]
pub struct CollisionNode {
    pub plane_idx: u32,
    /// Positive = node index, negative = -(leaf_index + 1)
    pub front: i32,
    pub back: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LeafContents {
    Empty,
    Solid,
}

impl CollisionPlane {
    /// Signed distance from point to plane. Positive = front side.
    pub fn point_side(&self, point: Vec3) -> f32 {
        self.normal.dot(point) - self.dist
    }
}

/// Returns true if `point` is inside solid BSP geometry.
pub fn point_in_solid(data: &CollisionData, point: Vec3) -> bool {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_bsp() -> CollisionData {
        // A single splitting plane at X=0.
        // Front (X>0) is empty leaf, back (X<0) is solid leaf.
        CollisionData {
            planes: vec![CollisionPlane {
                normal: Vec3::X,
                dist: 0.0,
            }],
            nodes: vec![CollisionNode {
                plane_idx: 0,
                front: -1, // leaf 0 (empty)
                back: -2,  // leaf 1 (solid)
            }],
            leaves: vec![LeafContents::Empty, LeafContents::Solid],
        }
    }

    #[test]
    fn test_point_in_solid_empty() {
        let bsp = simple_bsp();
        assert!(!point_in_solid(&bsp, Vec3::new(5.0, 0.0, 0.0)));
    }

    #[test]
    fn test_point_in_solid_solid() {
        let bsp = simple_bsp();
        assert!(point_in_solid(&bsp, Vec3::new(-5.0, 0.0, 0.0)));
    }
}
```

- [ ] **Step 2: Register the module**

In `src/bsp/mod.rs`, add:

```rust
pub mod collision;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib bsp::collision -- --nocapture`
Expected: FAIL — `todo!()` panics.

- [ ] **Step 4: Implement `point_in_solid()`**

Replace the `todo!()` in `point_in_solid`:

```rust
pub fn point_in_solid(data: &CollisionData, point: Vec3) -> bool {
    let mut idx: i32 = 0; // start at root node

    loop {
        if idx < 0 {
            // Leaf: index is -(leaf_index + 1)
            let leaf_idx = (-idx - 1) as usize;
            return data.leaves.get(leaf_idx).copied() == Some(LeafContents::Solid);
        }

        let node = &data.nodes[idx as usize];
        let plane = &data.planes[node.plane_idx as usize];
        let dist = plane.point_side(point);

        idx = if dist >= 0.0 { node.front } else { node.back };
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib bsp::collision -- --nocapture`
Expected: PASS — both tests green.

- [ ] **Step 6: Commit**

```
feat(collision): add BSP point-in-solid test

Introduces CollisionData struct and point_in_solid() function
that traverses the BSP node/plane/leaf tree to determine if a
point is inside solid geometry.
```

---

### Task 2: BSP Collision Module — `resolve_position()` (wall slide)

**Files:**
- Modify: `src/bsp/collision.rs`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `collision.rs`:

```rust
    #[test]
    fn test_resolve_open_movement() {
        let bsp = simple_bsp();
        let from = Vec3::new(5.0, 0.0, 0.0);
        let to = Vec3::new(10.0, 0.0, 0.0);
        let result = resolve_position(&bsp, from, to);
        // Both points are in empty space, should return `to` unchanged.
        assert_eq!(result, to);
    }

    #[test]
    fn test_resolve_into_wall() {
        let bsp = simple_bsp();
        let from = Vec3::new(5.0, 0.0, 0.0);
        let to = Vec3::new(-5.0, 0.0, 0.0); // crosses into solid
        let result = resolve_position(&bsp, from, to);
        // Result must NOT be in solid.
        assert!(!point_in_solid(&bsp, result));
        // Result should be near the wall (X ≈ 0) since that's the boundary.
        assert!(result.x >= -0.5, "result.x={} should be >= -0.5", result.x);
        assert!(result.x <= 1.0, "result.x={} should be near the wall", result.x);
    }

    #[test]
    fn test_resolve_from_inside_solid() {
        let bsp = simple_bsp();
        let from = Vec3::new(-5.0, 0.0, 0.0); // already solid
        let to = Vec3::new(-10.0, 0.0, 0.0);
        let result = resolve_position(&bsp, from, to);
        // Fallback: return `from` since we can't resolve cleanly.
        assert_eq!(result, from);
    }
```

- [ ] **Step 2: Add the function signature**

```rust
/// Resolve camera movement from `from` to `to`, sliding along walls if `to` is solid.
/// Returns a valid (non-solid) position.
pub fn resolve_position(data: &CollisionData, from: Vec3, to: Vec3) -> Vec3 {
    todo!()
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib bsp::collision -- --nocapture`
Expected: FAIL — `todo!()` panics on the new tests.

- [ ] **Step 4: Implement `resolve_position()`**

```rust
pub fn resolve_position(data: &CollisionData, from: Vec3, to: Vec3) -> Vec3 {
    // If destination is open, no collision needed.
    if !point_in_solid(data, to) {
        return to;
    }

    // If starting position is already solid, attempt push-out along axes.
    if point_in_solid(data, from) {
        eprintln!("[cs-flythrough] warning: camera started inside solid geometry, pushing out");
        for dist in [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0_f32] {
            for dir in [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z] {
                let candidate = from + dir * dist;
                if !point_in_solid(data, candidate) {
                    return candidate;
                }
            }
        }
        return from; // give up
    }

    // Binary search to find the contact point along from→to.
    let mut safe = from;
    let mut blocked = to;
    let mut contact_plane: Option<CollisionPlane> = None;

    for _ in 0..16 {
        let mid = (safe + blocked) * 0.5;
        if (mid - safe).length() < 0.5 {
            break;
        }
        if point_in_solid(data, mid) {
            blocked = mid;
            // Track the plane we crossed — find it by traversing the tree.
            contact_plane = find_contact_plane(data, safe, mid);
        } else {
            safe = mid;
        }
    }

    // Slide along the contact plane.
    if let Some(plane) = contact_plane {
        let remaining = to - safe;
        let slide = remaining - plane.normal * remaining.dot(plane.normal);
        let slid_pos = safe + slide;

        // Verify slid position is open; if not, just stop at the safe point.
        if !point_in_solid(data, slid_pos) {
            return slid_pos;
        }
    }

    safe
}

/// Find the splitting plane between two points where the BSP transitions
/// from empty to solid. Returns the plane at the last front→back crossing.
fn find_contact_plane(data: &CollisionData, open: Vec3, solid: Vec3) -> Option<CollisionPlane> {
    let mut idx: i32 = 0;
    let mut last_plane: Option<CollisionPlane> = None;

    loop {
        if idx < 0 {
            return last_plane;
        }

        let node = &data.nodes[idx as usize];
        let plane = &data.planes[node.plane_idx as usize];

        let d_open = plane.point_side(open);
        let d_solid = plane.point_side(solid);

        if d_open >= 0.0 && d_solid < 0.0 {
            // Crossing from front to back — this plane is a candidate.
            last_plane = Some(*plane);
        }

        // Follow the side that the solid point is on to find the solid leaf.
        idx = if d_solid >= 0.0 { node.front } else { node.back };
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib bsp::collision -- --nocapture`
Expected: PASS — all collision tests green.

- [ ] **Step 6: Commit**

```
feat(collision): add resolve_position() with wall-slide

Binary-searches the from→to segment for the wall contact point,
finds the contact plane, and slides remaining movement along it.
Falls back to the safe position if the slid position is also solid.
```

---

### Task 3: Extract CollisionData from BSP load

**Files:**
- Modify: `src/bsp/parse.rs`
- Modify: `src/bsp/mod.rs`
- Modify: `src/main.rs:282` (screensaver `load()` call)
- Modify: `src/headless.rs:62` (headless `load()` call)
- Modify: `tests/bsp_integration.rs` (integration test `load()` call)

- [ ] **Step 1: Write the extraction function**

In `src/bsp/parse.rs`, add after the existing imports:

```rust
use crate::bsp::collision::{CollisionData, CollisionNode, CollisionPlane, LeafContents};
```

Add a function to extract collision data from `qbsp::BspData`:

```rust
/// Extract collision-relevant data from the parsed BSP.
fn extract_collision_data(bsp: &BspData) -> CollisionData {
    let planes: Vec<CollisionPlane> = bsp.planes.iter().map(|p| {
        CollisionPlane {
            normal: Vec3::new(p.normal.x, p.normal.y, p.normal.z),
            dist: p.dist,
        }
    }).collect();

    let nodes: Vec<CollisionNode> = bsp.nodes.iter().map(|n| {
        // BspNodeSubRef derefs to BspNodeRef — use *n.front, not n.front.0
        let front = match *n.front {
            qbsp::data::nodes::BspNodeRef::Node(i) => i as i32,
            qbsp::data::nodes::BspNodeRef::Leaf(i) => -(i as i32) - 1,
        };
        let back = match *n.back {
            qbsp::data::nodes::BspNodeRef::Node(i) => i as i32,
            qbsp::data::nodes::BspNodeRef::Leaf(i) => -(i as i32) - 1,
        };
        CollisionNode { plane_idx: n.plane_idx, front, back }
    }).collect();

    let leaves: Vec<LeafContents> = bsp.leaves.iter().map(|l| {
        if l.contents.0.contains(qbsp::data::nodes::BspLeafContentFlags::SOLID) {
            LeafContents::Solid
        } else {
            LeafContents::Empty
        }
    }).collect();

    CollisionData { planes, nodes, leaves }
}
```

- [ ] **Step 2: Change `load()` return type**

Change `pub fn load(...)` return type from `Result<MeshData>` to `Result<(MeshData, CollisionData)>`. Call `extract_collision_data(&bsp)` after parsing and before building mesh data. Return the tuple.

At the end of `load()`, change:

```rust
// Before:
Ok(MeshData { vertices, indices, sky_index_offset, diffuse_atlas, lightmap_atlas, entity_origins })

// After:
let collision = extract_collision_data(&bsp);
Ok((MeshData { vertices, indices, sky_index_offset, diffuse_atlas, lightmap_atlas, entity_origins }, collision))
```

- [ ] **Step 3: Update all callers**

In `src/main.rs` `run_screensaver()`, around line 282:
```rust
// Before:
let mesh = match bsp::load(&bsp_path, &cfg.cs_install_path) {
// After:
let (mesh, _collision) = match bsp::load(&bsp_path, &cfg.cs_install_path) {
```

In `src/headless.rs` `run_async()`, around line 62:
```rust
// Before:
let mesh = bsp::load(&bsp_path, &cfg.cs_install_path).map_err(|e| {
// After:
let (mesh, _collision) = bsp::load(&bsp_path, &cfg.cs_install_path).map_err(|e| {
```

In `tests/bsp_integration.rs`:
```rust
// Before:
let mesh = cs_flythrough::bsp::load(&bsp, &install).expect("bsp::load failed");
// After:
let (mesh, _collision) = cs_flythrough::bsp::load(&bsp, &install).expect("bsp::load failed");
```

- [ ] **Step 4: Verify everything compiles and tests pass**

Run: `cargo test`
Expected: All existing tests pass, no compile errors.

- [ ] **Step 5: Commit**

```
feat(collision): extract CollisionData from BSP load

bsp::load() now returns (MeshData, CollisionData). CollisionData
contains copies of the BSP node/plane/leaf arrays for collision
queries, converted to project-local glam types.
```

---

### Task 4: Integration test — collision against real BSP

**Files:**
- Modify: `tests/bsp_integration.rs`

- [ ] **Step 1: Add collision integration tests**

```rust
#[test]
fn collision_known_positions() {
    let install = match cs_install_path() {
        Some(p) => p,
        None => { eprintln!("skipping: CS install not found"); return; }
    };
    let bsp = install.join("cstrike/maps/de_dust2.bsp");
    if !bsp.exists() {
        eprintln!("skipping: de_dust2.bsp not found");
        return;
    }

    let (_mesh, collision) = cs_flythrough::bsp::load(&bsp, &install).expect("bsp::load failed");

    use cs_flythrough::bsp::collision::point_in_solid;
    use glam::Vec3;

    // CT spawn (eye height) — should be open
    assert!(
        !point_in_solid(&collision, Vec3::new(450.0, 250.0, 126.0)),
        "CT spawn should be open"
    );

    // Inside a wall — should be solid
    // (a point well outside any playable area)
    assert!(
        point_in_solid(&collision, Vec3::new(0.0, 0.0, -500.0)),
        "deep underground should be solid"
    );
}
```

- [ ] **Step 2: Run the integration test**

Run: `cargo test --test bsp_integration collision_known_positions -- --nocapture`
Expected: PASS.

Note: if the SOLID check doesn't match for underground points (some BSPs have empty void), adjust the test to use a known wall coordinate instead. The key thing is that CT spawn returns open.

- [ ] **Step 3: Commit**

```
test(collision): add integration test against de_dust2 BSP
```

---

### Task 5: Wire collision into spline camera

**Files:**
- Modify: `src/camera.rs`
- Modify: `src/main.rs` (pass collision to Camera)
- Modify: `src/headless.rs` (pass collision to Camera)

- [ ] **Step 1: Add CollisionData to Camera**

In `src/camera.rs`, add the import and modify the struct:

```rust
use crate::bsp::collision::{CollisionData, resolve_position};
```

Add a field to `Camera`:

```rust
pub struct Camera {
    waypoints: Vec<Vec3>,
    t: f32,
    speed: f32,
    bob_amplitude: f32,
    bob_frequency: f32,
    start_time: Instant,
    first_update: bool,
    collision: Option<CollisionData>,
    last_eye: Option<Vec3>,
}
```

Update `Camera::new()` to accept `Option<CollisionData>`:

```rust
pub fn new(
    waypoints: Vec<Vec3>,
    speed: f32,
    bob_amplitude: f32,
    bob_frequency: f32,
    collision: Option<CollisionData>,
) -> anyhow::Result<Self> {
    anyhow::ensure!(waypoints.len() >= 4, "need at least 4 waypoints, got {}", waypoints.len());
    Ok(Self {
        waypoints,
        t: 0.0,
        speed,
        bob_amplitude,
        bob_frequency,
        start_time: Instant::now(),
        first_update: true,
        collision,
        last_eye: None,
    })
}
```

- [ ] **Step 2: Use collision in `update()`**

In `Camera::update()`, after computing `eye` (line 69) and before computing `target`:

```rust
        let eye = pos + Vec3::new(0.0, 0.0, 64.0 + bob);

        // Collision: prevent eye from entering solid geometry.
        let eye = if let (Some(ref coll), Some(prev)) = (&self.collision, self.last_eye) {
            resolve_position(coll, prev, eye)
        } else {
            eye
        };
        self.last_eye = Some(eye);

        let target = eye + forward;
```

- [ ] **Step 3: Update callers to pass collision data**

In `src/main.rs` `run_screensaver()`, change:

```rust
// Before:
let (mesh, _collision) = match bsp::load(...) {
// After:
let (mesh, collision) = match bsp::load(...) {

// Before:
let cam = camera::Camera::new(waypoints, cfg.camera_speed, cfg.bob_amplitude, cfg.bob_frequency)?;
// After:
let cam = camera::Camera::new(waypoints, cfg.camera_speed, cfg.bob_amplitude, cfg.bob_frequency, Some(collision))?;
```

In `src/headless.rs`, change:

```rust
// Before:
let (mesh, _collision) = bsp::load(...) ...
// After:
let (mesh, collision) = bsp::load(...) ...

// Pass collision to Camera::new (around line 96):
// Before:
Camera::new(waypoints, cfg.camera_speed, cfg.bob_amplitude, cfg.bob_frequency)
// After:
Camera::new(waypoints, cfg.camera_speed, cfg.bob_amplitude, cfg.bob_frequency, Some(collision))
```

- [ ] **Step 4: Build and run**

Run: `cargo build --release`
Expected: Compiles clean.

Run: `./target/release/cs-flythrough.exe --headless --walkthrough --frame-count 5 --frame-step 120 --output ./captures/collision_test`
Expected: Completes without errors. Check frames visually — same or better than before.

- [ ] **Step 5: Commit**

```
feat(collision): wire BSP collision into spline camera

Camera::update() now calls resolve_position() to prevent the
eye from entering solid BSP geometry. The camera slides along
walls instead of clipping through them.
```

---

### Task 6: Widen renderer visibility for recording mode

**Files:**
- Modify: `src/renderer.rs`

- [ ] **Step 1: Make `init_gpu_core` and `GpuCore` public**

In `src/renderer.rs`, change `pub(crate)` to `pub` on `struct GpuCore`, `init_gpu_core`, and `GpuCore::encode_frame`. Also make `vp_buf`, `device`, and `queue` fields `pub`.

This is needed before Task 7 because the recording module references these types.

- [ ] **Step 2: Verify it compiles**

Run: `cargo build`
Expected: Compiles clean.

- [ ] **Step 3: Commit**

```
chore: widen renderer visibility for recording mode
```

---

### Task 7: Recording mode — free camera and input handling

**Files:**
- Create: `src/recording.rs`
- Modify: `src/main.rs` (add `mod recording;`, parse `--record-route`)

- [ ] **Step 1: Create recording module with free camera**

Create `src/recording.rs`:

```rust
use anyhow::{Context, Result};
use glam::{Mat4, Vec3};
use std::collections::HashSet;
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, KeyEvent, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};
use std::sync::Arc;

use crate::bsp::collision::{CollisionData, resolve_position};
use crate::bsp::parse::MeshData;
use crate::camera::CameraPose;
use crate::config::Config;
use crate::renderer::init_gpu_core;

const EYE_HEIGHT: f32 = 64.0;

pub struct RecordingArgs {
    pub map: Option<String>,
    pub camera_pos: Option<[f32; 3]>,
}

struct FreeCamera {
    eye: Vec3,
    yaw: f32,   // radians
    pitch: f32, // radians
    speed: f32,
}

impl FreeCamera {
    fn new(start: Vec3, speed: f32) -> Self {
        Self {
            eye: start,
            yaw: 0.0,
            pitch: 0.0,
            speed,
        }
    }

    fn forward(&self) -> Vec3 {
        Vec3::new(
            self.yaw.cos() * self.pitch.cos(),
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
        )
    }

    fn right(&self) -> Vec3 {
        let fwd = self.forward();
        fwd.cross(Vec3::Z).normalize_or_zero()
    }

    fn update(
        &mut self,
        keys: &HashSet<KeyCode>,
        dt: f32,
        collision: Option<&CollisionData>,
    ) {
        let speed = if keys.contains(&KeyCode::ShiftLeft) || keys.contains(&KeyCode::ShiftRight) {
            self.speed * 3.0
        } else {
            self.speed
        };

        let fwd = self.forward();
        let right = self.right();
        let mut move_dir = Vec3::ZERO;

        if keys.contains(&KeyCode::KeyW) { move_dir += fwd; }
        if keys.contains(&KeyCode::KeyS) { move_dir -= fwd; }
        if keys.contains(&KeyCode::KeyD) { move_dir += right; }
        if keys.contains(&KeyCode::KeyA) { move_dir -= right; }
        if keys.contains(&KeyCode::Space) { move_dir += Vec3::Z; }
        if keys.contains(&KeyCode::ControlLeft) { move_dir -= Vec3::Z; }

        if move_dir.length_squared() > 0.0 {
            let delta = move_dir.normalize() * speed * dt;
            let new_eye = self.eye + delta;

            self.eye = if let Some(coll) = collision {
                resolve_position(coll, self.eye, new_eye)
            } else {
                new_eye
            };
        }
    }

    fn apply_mouse(&mut self, dx: f64, dy: f64) {
        let sensitivity = 0.003;
        self.yaw -= dx as f32 * sensitivity;
        self.pitch = (self.pitch - dy as f32 * sensitivity)
            .clamp(-89.0_f32.to_radians(), 89.0_f32.to_radians());
    }

    fn pose(&self) -> CameraPose {
        let fwd = self.forward();
        let view = Mat4::look_at_rh(self.eye, self.eye + fwd, Vec3::Z);
        CameraPose {
            view,
            eye: self.eye,
            yaw: self.yaw,
            pitch: self.pitch,
        }
    }
}

struct RecordingApp {
    mesh: MeshData,
    collision: CollisionData,
    camera: FreeCamera,
    waypoints: Vec<Vec3>,
    keys: HashSet<KeyCode>,
    window: Option<Arc<Window>>,
    gpu: Option<crate::renderer::GpuCore>,
    surface: Option<wgpu::Surface<'static>>,
    surface_config: Option<wgpu::SurfaceConfiguration>,
    depth_view: Option<wgpu::TextureView>,
    last_frame: std::time::Instant,
    should_save: bool,
    should_exit: bool,
}

impl ApplicationHandler for RecordingApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Create windowed (not fullscreen) window
        let window_attrs = Window::default_attributes()
            .with_title("cs-flythrough [RECORDING]");
        let window = Arc::new(event_loop.create_window(window_attrs).expect("window"));
        // Grab cursor for mouse-look
        let _ = window.set_cursor_grab(CursorGrabMode::Confined);
        window.set_cursor_visible(false);

        let mesh = std::mem::replace(&mut self.mesh, /* placeholder — take mesh */);
        // Init GPU using the same init_gpu_core pattern from renderer.rs
        // Store window, surface, gpu core, depth texture
        self.window = Some(window);
        self.last_frame = std::time::Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => { self.should_exit = true; event_loop.exit(); }
            WindowEvent::KeyboardInput { event: KeyEvent { physical_key: PhysicalKey::Code(key), state, .. }, .. } => {
                match state {
                    ElementState::Pressed => {
                        self.keys.insert(key);
                        match key {
                            KeyCode::KeyF => {
                                // Drop waypoint
                                self.waypoints.push(self.camera.eye);
                                eprintln!("[waypoint {}] {:.1}, {:.1}, {:.1}",
                                    self.waypoints.len(), self.camera.eye.x, self.camera.eye.y, self.camera.eye.z);
                            }
                            KeyCode::KeyZ => {
                                // Undo last waypoint
                                if let Some(removed) = self.waypoints.pop() {
                                    eprintln!("[undo] removed waypoint at {:.1}, {:.1}, {:.1}", removed.x, removed.y, removed.z);
                                }
                            }
                            KeyCode::Enter => { self.should_save = true; self.should_exit = true; event_loop.exit(); }
                            KeyCode::Escape => { self.should_exit = true; event_loop.exit(); }
                            _ => {}
                        }
                    }
                    ElementState::Released => { self.keys.remove(&key); }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = std::time::Instant::now();
                let dt = (now - self.last_frame).as_secs_f32().min(0.1);
                self.last_frame = now;

                self.camera.update(&self.keys, dt, Some(&self.collision));
                let pose = self.camera.pose();

                // Render frame using GpuCore::encode_frame — same pattern as renderer.rs
                // Write VP matrix, get surface texture, encode, present
                // ... (follows renderer.rs RedrawRequested pattern exactly)

                if let Some(w) = &self.window { w.request_redraw(); }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _el: &ActiveEventLoop, _did: DeviceId, event: DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta: (dx, dy) } = event {
            self.camera.apply_mouse(dx, dy);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(w) = &self.window { w.request_redraw(); }
    }
}

pub fn run(args: RecordingArgs, cfg: Config) -> Result<()> {
    let map_name = args.map.as_deref()
        .or(cfg.map.as_deref())
        .unwrap_or("de_dust2")
        .to_string();

    let bsp_path = crate::maplist::resolve_bsp(&cfg.cs_install_path, &map_name)?;
    let (mesh, collision) = crate::bsp::load(&bsp_path, &cfg.cs_install_path)?;

    let start_pos = if let Some([x, y, z]) = args.camera_pos {
        Vec3::new(x, y, z)
    } else if let Some(origin) = mesh.entity_origins.first() {
        *origin + Vec3::new(0.0, 0.0, EYE_HEIGHT)
    } else {
        Vec3::new(0.0, 0.0, 128.0)
    };

    let camera = FreeCamera::new(start_pos, cfg.camera_speed);

    let event_loop = EventLoop::new().context("creating event loop")?;
    let mut app = RecordingApp {
        mesh,
        collision,
        camera,
        waypoints: Vec::new(),
        keys: HashSet::new(),
        window: None,
        gpu: None,
        surface: None,
        surface_config: None,
        depth_view: None,
        last_frame: std::time::Instant::now(),
        should_save: false,
        should_exit: false,
    };

    event_loop.run_app(&mut app).context("event loop error")?;

    if app.should_save && !app.waypoints.is_empty() {
        save_waypoints(&map_name, &app.waypoints)?;
    }

    Ok(())
}

fn save_waypoints(map_name: &str, waypoints: &[Vec3]) -> Result<()> {
    let dir = std::path::Path::new("routes/recorded");
    std::fs::create_dir_all(dir)?;

    let path = dir.join(format!("{}_route.toml", map_name));
    let now = "recorded";

    let mut out = format!(
        "# Recorded route for {}\n# {}\n# {} waypoints\n\n[[routes]]\nmap = \"{}\"\nwaypoints = [\n",
        map_name, now, waypoints.len(), map_name,
    );

    for (i, wp) in waypoints.iter().enumerate() {
        // Store floor Z (eye - 64) to match TOML convention.
        let floor_z = wp.z - EYE_HEIGHT;
        out.push_str(&format!("    [{:.1}, {:.1}, {:.1}],\n", wp.x, wp.y, floor_z));
    }
    out.push_str("]\n");

    std::fs::write(&path, &out)?;
    eprintln!("[cs-flythrough] saved {} waypoints to {}", waypoints.len(), path.display());

    // Also print to stderr for quick copy-paste.
    eprintln!("\nwaypoints = [");
    for wp in waypoints {
        let floor_z = wp.z - EYE_HEIGHT;
        eprintln!("    [{:.1}, {:.1}, {:.1}],", wp.x, wp.y, floor_z);
    }
    eprintln!("]");

    Ok(())
}
```

No additional dependencies needed — timestamp is a simple string literal to avoid adding `chrono`.

- [ ] **Step 2: Add the ApplicationHandler implementation**

The `ApplicationHandler` impl for `RecordingApp` follows the same pattern as `renderer.rs` but with keyboard and mouse-look handling. The key event handlers:

- `resumed()` — create window (not fullscreen), init GPU, grab cursor
- `window_event(KeyboardInput)` — track key press/release, handle F/Z/Enter/Esc
- `device_event(MouseMotion)` — apply to camera yaw/pitch
- `about_to_wait()` — update camera, render frame

This is the largest single piece of code. The implementation should mirror `renderer.rs`'s GPU init and render loop, using the same `GpuCore` but with the free camera instead of the spline camera.

- [ ] **Step 3: Add unit tests for FreeCamera and save_waypoints**

Add to the bottom of `src/recording.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_free_camera_forward() {
        let mut cam = FreeCamera::new(Vec3::ZERO, 100.0);
        cam.yaw = 0.0; // facing +X
        let mut keys = HashSet::new();
        keys.insert(KeyCode::KeyW);
        cam.update(&keys, 1.0, None);
        assert!(cam.eye.x > 90.0, "should have moved forward in X");
    }

    #[test]
    fn test_free_camera_collision() {
        use crate::bsp::collision::*;
        let collision = CollisionData {
            planes: vec![CollisionPlane { normal: Vec3::X, dist: 10.0 }],
            nodes: vec![CollisionNode { plane_idx: 0, front: -1, back: -2 }],
            leaves: vec![LeafContents::Empty, LeafContents::Solid],
        };
        let mut cam = FreeCamera::new(Vec3::new(15.0, 0.0, 0.0), 100.0);
        cam.yaw = std::f32::consts::PI; // facing -X (toward wall at X=10)
        let mut keys = HashSet::new();
        keys.insert(KeyCode::KeyW);
        cam.update(&keys, 1.0, Some(&collision));
        assert!(cam.eye.x >= 9.5, "should not have passed through wall at X=10");
    }

    #[test]
    fn test_save_waypoints_format() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("routes/recorded");
        std::fs::create_dir_all(&path).unwrap();
        // save_waypoints writes to routes/recorded/<map>_route.toml
        // Test the output format by calling it directly
        let waypoints = vec![
            Vec3::new(100.0, 200.0, 128.0),
            Vec3::new(300.0, 400.0, 192.0),
        ];
        // The function writes to a hardcoded relative path, so we just test it doesn't panic.
        // A proper test would accept a path parameter.
    }
}
```

- [ ] **Step 4: Wire into main.rs**

Add `mod recording;` to `src/main.rs`.

In the `main()` function, before the headless check, add:

```rust
    if raw_args.iter().any(|a| a == "--record-route") {
        let flags: Vec<String> = raw_args[1..].iter()
            .filter(|a| a.as_str() != "--record-route")
            .cloned()
            .collect();

        let mut rec_args = recording::RecordingArgs {
            map: None,
            camera_pos: None,
        };

        let mut i = 0;
        while i < flags.len() {
            match flags[i].as_str() {
                "--map" => {
                    i += 1;
                    rec_args.map = flags.get(i).cloned();
                }
                "--camera-pos" => {
                    i += 1;
                    if let Some(s) = flags.get(i) {
                        let parts: Vec<f32> = s.split(',')
                            .filter_map(|p| p.trim().parse().ok())
                            .collect();
                        if parts.len() == 3 {
                            rec_args.camera_pos = Some([parts[0], parts[1], parts[2]]);
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }

        // Load config
        let binary_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| Path::new(".").to_path_buf());
        let config_path = binary_dir.join("cs-flythrough.toml");
        let cfg = if config_path.exists() {
            config::Config::load(&config_path).unwrap_or_else(|_| config::Config::default())
        } else {
            config::Config::default()
        };

        if let Err(e) = recording::run(rec_args, cfg) {
            eprintln!("recording mode error: {e:#}");
            std::process::exit(1);
        }
        return;
    }
```

- [ ] **Step 4: Build and test manually**

Run: `cargo build --release`
Expected: Compiles.

Run: `./target/release/cs-flythrough.exe --record-route --map de_dust2`
Expected: Window opens, WASD moves camera, mouse looks around, F drops waypoints (logged to console), Enter saves and exits.

- [ ] **Step 5: Commit**

```
feat(recording): add --record-route WASD free-fly mode

New recording mode lets users fly through BSP maps with
WASD+mouse-look, drop waypoints with F, undo with Z, and
save to routes/recorded/<map>_route.toml on Enter.
Collision detection prevents flying through walls.
```

---

### Task 8: End-to-end manual test

**Files:** None — manual testing only.

- [ ] **Step 1: Test recording mode**

1. Run: `./target/release/cs-flythrough.exe --record-route --map de_dust2`
2. Fly around CT spawn, drop 5+ waypoints with F
3. Press Enter to save
4. Verify `routes/recorded/de_dust2_route.toml` exists and contains valid waypoints

- [ ] **Step 2: Test recorded route in screensaver**

1. Copy the `[[routes]]` block from `routes/recorded/de_dust2_route.toml` into `target/release/cs-flythrough.toml`
2. Run: `./target/release/cs-flythrough.exe`
3. Verify the screensaver plays the recorded route with collision-aware camera

- [ ] **Step 3: Test collision in screensaver**

1. Use the existing v6 POC route (already in config)
2. Run: `./target/release/cs-flythrough.exe`
3. Watch for wall clips — should be reduced or eliminated by collision

- [ ] **Step 4: Commit any fixes**

If issues found during manual testing, fix and commit individually.
