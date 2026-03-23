# cs-flythrough TODO

Tracks remaining work against the skateboard plan and beyond.
See full plan: `docs/superpowers/plans/2026-03-14-cs-flythrough-skateboard.md`

---

## Skateboard (MVP) — Core Complete

All 11 implementation tasks from the plan are done. Remaining:

- [x] Task 1: Project scaffold (Cargo.toml, main.rs, shaders)
- [x] Task 2: Config module (TOML load/save, defaults)
- [x] Task 3: Maplist module (BSP path resolution, compatibility tracking)
- [x] Task 4: BSP entity lump parser (waypoint extraction)
- [x] Task 5: WAD texture loading + atlas packing
- [x] Task 6: BSP parse → MeshData (vertices, indices, lightmap, diffuse)
- [x] Task 7: Camera system (nearest-neighbor sort, centripetal Catmull-Rom, head bob)
- [x] Task 8: Input module (mouse exit threshold)
- [x] Task 9: Renderer (wgpu geometry + sky passes, VP uniform, depth buffer)
- [x] Task 10: Startup sequence wired in main.rs
- [x] Task 11: Integration test (headless de_dust2 BSP load)
- [ ] **Task 12: Skateboard sign-off** — once PR #5 merges, do a final manual run, verify `map-compatibility.toml` is populated, and tag the release.

---

## In Progress

- [ ] **PR #5 review** (`feat/visual-bug-detection`) — VIS-001/002/003 rendering fixes:
  - VIS-001: NAV area extent + Z filters to reduce camera clipping into walls
  - VIS-002: Dark/unlit brush faces fixed (no_lighting_color + lightmap sentinel)
  - VIS-003: Texture banding fixed (per-fragment UV wrapping instead of per-vertex)

---

## Remaining Features

### Multi-map rotation
Config already defines `MapSelection::Single/List/All` and `maps: Option<Vec<String>>` but `main.rs` always uses single-map mode.

- [ ] `MapSelection::List` — cycle through the explicit `maps = [...]` array in config, advancing to the next map after each full path traversal (or on load failure)
- [ ] `MapSelection::All` — scan `cstrike/maps/*.bsp`, build the full list, filter by `map-compatibility.toml` (skip Failed entries), cycle through in random or alphabetical order
- [ ] Persist the "last played map index" across runs so it doesn't restart from the same map every time the screensaver activates
- [ ] Add `maplist::scan_all_maps(cs_install_path)` helper that returns all `.bsp` file stems found in `cstrike/maps/` and `czero/maps/`

### Settings dialog
- [ ] Implement `/c` mode — a basic Windows dialog (or terminal prompt fallback) to let users set `cs_install_path` and `map_selection` without hand-editing the TOML

### Distribution
- [ ] Rename output binary to `cs-flythrough.scr` in the release build (or document the manual rename step) so Windows recognizes it as a screensaver
- [ ] Add install instructions to README: copy `.scr` + `cs-flythrough.toml` to `C:\Windows\System32`, right-click → Preview

---

## Stretch / Nice-to-Have

- [ ] FOV as a config option (currently hardcoded to `2 * atan(1/aspect)` in headless, `90°` in renderer)
- [ ] Mipmap generation for diffuse atlas (reduces texture shimmer on distant geometry)
- [ ] Ambient occlusion or a subtle fog falloff to give depth cues in open areas
- [ ] Configurable sky color (currently hardcoded desert-sky blue in renderer clear color and sky shader)
- [ ] Map name OSD — brief on-screen map name display when a new map starts (multi-map mode)

---

## Done (Beyond Plan — Already Shipped)

- [x] NAV file support — loads `.nav` files for player-path-following camera routes (PR #4)
- [x] NAV area filtering — min_extent and min_z filters to exclude narrow/underground areas (PR #4)
- [x] Headless rendering mode — `--headless --walkthrough` flag for PNG capture + JSON manifest (PR #3)
- [x] Debug log module — writes `cs-flythrough-debug.log` next to the binary
- [x] Capture module — PNG write + JSON frame manifest for headless walkthroughs
