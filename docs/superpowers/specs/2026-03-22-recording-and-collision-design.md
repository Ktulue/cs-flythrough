# Waypoint Recording Mode & BSP Collision Detection

**Date:** 2026-03-22
**Status:** Draft
**Branch:** `feat/de-dust2-route`

## Problem

Hand-authoring waypoint routes requires blind coordinate probing via headless captures. Each position must be tested individually, and even probe-verified routes still clip through walls because the Catmull-Rom spline arcs between waypoints without knowing about geometry. This process takes hours per map and doesn't scale.

## Solution

Two features, built in order:

1. **BSP collision detection** — point-in-solid tests using the BSP node/plane tree. Prevents the camera from entering solid geometry by sliding along wall surfaces.
2. **Waypoint recording mode** — fly through the map with WASD controls, press a key to drop waypoints, export to TOML. A human walking the path produces naturally valid routes in minutes. Uses collision to prevent flying through walls.

## Feature 1: BSP Collision Detection

### Approach: Wall Slide

When the camera moves from position A to position B:

1. Test if B is inside solid geometry (BSP point-in-solid traversal)
2. If B is open: use B as-is
3. If B is solid: trace back along the A→B vector to find the wall surface, then slide the remaining movement along the wall's plane normal

This produces smooth wall-sliding behavior — the camera glides along walls rather than stopping or clipping through.

### BSP Data Access

Currently `bsp::load()` parses the BSP via `qbsp`, extracts `MeshData`, and discards the `qbsp::BspData`. The collision module needs access to the BSP node/plane/leaf tree.

**Approach:** Return `BspData` alongside `MeshData` from `bsp::load()`. Add a `CollisionData` struct that holds the subset of `BspData` needed (nodes, planes, leaves). This is constructed once after BSP load and passed to the camera and recording systems.

### BSP Point-in-Solid Test

GoldSrc BSPs use a binary tree of splitting planes. Each internal node has a plane and two children. Leaves are marked as solid or empty (via content flags).

```
fn point_in_solid(collision: &CollisionData, point: Vec3) -> bool
```

1. Start at root node (node 0)
2. Compute signed distance from point to the node's plane
3. If positive: recurse into front child
4. If negative: recurse into back child
5. If leaf: return true if contents flags include SOLID

**Solid detection:** `BspLeafContentFlags` is a bitflag type. Check `contents.0.contains(BspLeafContentFlags::SOLID)`. Note that sky leaves and clip brushes also map to SOLID — this is intentional. Clip brushes should block the camera. Sky leaves acting as collision barriers is acceptable since the camera shouldn't fly into the skybox anyway.

**glam version note:** The project uses glam 0.32 but `qbsp` uses glam 0.30. All `Vec3` values from `qbsp` planes/nodes must be converted to project-local `glam::Vec3` before use, matching the existing conversion pattern in `bsp/parse.rs`.

### Resolve Position

```
fn resolve_position(collision: &CollisionData, from: Vec3, to: Vec3) -> Vec3
```

1. If `point_in_solid(to)` is false: return `to` (no collision)
2. Binary search along the `from→to` segment to find the contact point (max 16 iterations, epsilon 0.5 units)
3. During the binary search, track the splitting plane at each front→back transition — this is the contact surface normal
4. Project the remaining movement vector onto the contact plane
5. Verify the slid position is not solid; if it is, return `from` as fallback

**Edge case — camera starts inside solid:** If `from` is already solid (bad `--camera-pos`, or startup), push the camera toward the nearest open space by testing positions along each axis at increasing distances. Log a warning.

**Edge case — thin walls:** The epsilon of 0.5 units prevents infinite subdivision. If the binary search can't find a clean boundary (both sides solid), return `from`.

### New Code

- `src/bsp/collision.rs` — `CollisionData` struct, `point_in_solid()`, and `resolve_position()` functions
- Uses `qbsp::BspData` (nodes, planes, leaves) extracted during BSP load

### Integration

- `src/bsp/parse.rs` — `bsp::load()` returns `(MeshData, CollisionData)` instead of just `MeshData`
- `src/camera.rs` — call `resolve_position()` in `Camera::update()` before computing the view matrix
- `src/recording.rs` — call `resolve_position()` in free-camera movement each frame
- The collision module has no dependencies on camera or rendering — it only needs BSP geometry data

## Feature 2: Waypoint Recording Mode

### Launch

```
cs-flythrough.exe --record-route [--map de_dust2] [--camera-pos x,y,z]
```

- Opens a windowed rendering of the map (same as normal screensaver mode)
- Camera starts at the first entity origin (CT spawn) or `--camera-pos` if specified
- Existing `--map` flag selects which map to load

### Controls

| Key | Action |
|-----|--------|
| W/A/S/D | Horizontal movement (forward/left/back/right relative to look direction) |
| Space | Move up |
| Ctrl | Move down |
| Mouse | Look direction (yaw/pitch) |
| Shift (hold) | 3x speed multiplier |
| F | Drop waypoint at current position |
| Z | Undo (remove last waypoint) |
| Enter | Save waypoints and exit |
| Esc | Cancel and exit without saving |

### Movement

- Base speed: `camera_speed` from config (default 133 u/s)
- Shift multiplier: 3x (399 u/s)
- No head bob in recording mode — steady camera for precise placement
- No gravity or ground clamping — free flight in all directions
- Collision detection active — camera slides along walls, can't fly into solid geometry

### Mouse Capture

Recording mode requires mouse-look. On window creation:

- Set `CursorGrabMode::Confined` (or `Locked` if supported) to prevent the cursor from leaving the window
- Hide the cursor
- Disable the existing `should_exit_on_mouse` screensaver behavior — mouse movement controls the camera, not exit

### Output

On `Enter`, writes waypoints to `routes/recorded/<map>_route.toml`:

```toml
# Recorded route for de_dust2
# 2026-03-22 14:30:00
# 12 waypoints

[[routes]]
map = "de_dust2"
waypoints = [
    [450.0, 250.0, 62.0],
    [100.0, 130.0, 62.0],
    # ...
]
```

Also prints the full waypoint array to stderr on exit for quick copy-paste.

**The user must copy the `[[routes]]` block into `cs-flythrough.toml` to use it.** The recorded file is a staging area, not auto-loaded by the screensaver. This is intentional — it lets the user review and tweak before committing the route.

**Z values in output:** The free camera position IS the eye position (no +64 offset in recording mode). On save, each waypoint's Z is stored as `eye_z - 64` to match the TOML convention where the spline camera adds +64 for eye height.

**Minimum waypoint count:** On `Enter`, if fewer than 4 waypoints have been recorded, print a warning ("need at least 4 waypoints for a spline route, got N") and still save the file. The route won't be usable for spline playback until it has 4+ points, but saving preserves the user's work.

### New Code

- `src/recording.rs` — recording mode entry point, free camera, waypoint list management, TOML export
- `src/input.rs` — extend with keyboard state tracking and mouse-look (currently only tracks mouse-exit threshold)

### Integration

- `src/main.rs` — parse `--record-route` flag, launch `recording::run()` instead of screensaver or headless mode

## Build Order

1. **BSP collision module** (`src/bsp/collision.rs`) — small, testable unit with no UI dependencies
2. **Recording mode** (`src/recording.rs`, input extensions) — uses collision, adds WASD/mouse input and TOML export
3. **Spline camera integration** — wire collision into existing `Camera::update()`

The collision module is built first because recording mode depends on it. Each step is independently testable.

## Testing

### Unit Tests (collision)

- `test_point_in_solid_open`: known CT spawn position returns false (open space)
- `test_point_in_solid_wall`: known inside-wall position returns true
- `test_resolve_position_open`: movement through open space returns destination unchanged
- `test_resolve_position_wall`: movement into a wall returns a slid position that is not inside solid geometry
- `test_resolve_position_start_in_solid`: camera starting inside solid returns a pushed-out position

### Integration Tests

- Load de_dust2 BSP, run collision queries against real geometry
- Verify CT spawn, T spawn, B site positions are all open
- Verify known wall positions are solid

### Manual Testing

- Run `--record-route` on de_dust2, fly around, verify can't enter walls
- Record a route, verify output TOML is valid and loads in normal screensaver mode
- Run screensaver with collision enabled, verify no wall clips on existing v6 route

## Out of Scope

- No HUD, minimap, or visual waypoint markers
- No route editing (reorder, delete individual waypoints, insert between)
- No sound or visual feedback for waypoint drops (console log only)
- No ground clamping or gravity in recording mode
- No collision response beyond wall-slide (no bouncing, no friction)
- No auto-loading of recorded routes (user copies to main config)
