# de_dust2 — Camera Route

Full interior loop. CT spawn → B site → tunnels → T side → long A → pit → back down long → A site → catwalk → mid → CT spawn.

## Coordinate Reference

BSP world space: X/Y horizontal, Z vertical (up). Player eye height = 64 units above floor.

### Entity Origin Anchors (from BSP entity lump)

These are extracted directly from the BSP — use as reference to sanity-check other coordinates.

| Entity                    | Approx Position       | Notes                        |
|---------------------------|-----------------------|------------------------------|
| info_player_start (CT)    | TBD — probe pending   | CT spawn                     |
| info_player_deathmatch (T)| TBD — probe pending   | T spawn                      |

### Map Bounds (observed from NAV captures)

| Axis | Min      | Max      |
|------|----------|----------|
| X    | ~-2200   | ~+1200   |
| Y    | ~-500    | ~+2700   |
| Z    | ~-200    | ~+400    |

---

## Waypoint Table

Status: **DRAFT** — coordinates need probing and verification with headless captures.

| # | Callout               | X       | Y       | Z    | Notes                                      |
|---|-----------------------|---------|---------|------|--------------------------------------------|
| 1 | CT Spawn              | TBD     | TBD     | TBD  | Face LEFT (toward A site)                  |
| 2 | CT Spawn turn         | TBD     | TBD     | TBD  | 180° smooth arc toward B                   |
| 3 | B Doors               | TBD     | TBD     | TBD  | Through the B doors                        |
| 4 | B Site floor          | TBD     | TBD     | TBD  | Bombsite floor area                        |
| 5 | Back of B (rise)      | TBD     | TBD     | TBD  | Arc UP — clear the back wall               |
| 6 | Back of B (elevated)  | TBD     | TBD     | TBD  | Sweep along back wall / window area        |
| 7 | Upper B / Tunnels     | TBD     | TBD     | TBD  | Into upper tunnels                         |
| 8 | Outside Tunnels       | TBD     | TBD     | TBD  | Exit to T-side exterior                    |
| 9 | T Ramp                | TBD     | TBD     | TBD  | Along the ramp                             |
|10 | T Spawn               | TBD     | TBD     | TBD  | Cross bottom of map                        |
|11 | Outside Long          | TBD     | TBD     | TBD  | Curve right toward long A approach         |
|12 | Long Double Doors     | TBD     | TBD     | TBD  | Through the double doors                   |
|13 | Long A                | TBD     | TBD     | TBD  | Up the length of long                      |
|14 | Pit                   | TBD     | TBD     | TBD  | Enter pit area                             |
|15 | Pit turnaround        | TBD     | TBD     | TBD  | 180° smooth turn — begin return            |
|16 | Long A (return)       | TBD     | TBD     | TBD  | Back down long toward A site               |
|17 | Cars                  | TBD     | TBD     | TBD  | Past the car near A site                   |
|18 | Back of A / Goose     | TBD     | TBD     | TBD  | Sweep behind A along back wall             |
|19 | A Site                | TBD     | TBD     | TBD  | Through the bombsite                       |
|20 | Bricks                | TBD     | TBD     | TBD  | Toward brick wall near CT spawn            |
|21 | Stairs                | TBD     | TBD     | TBD  | Down steps from A platform                 |
|22 | Short / Catwalk       | TBD     | TBD     | TBD  | Along catwalk toward mid                   |
|23 | Top of Mid            | TBD     | TBD     | TBD  | Catwalk meets top of mid                   |
|24 | Mid                   | TBD     | TBD     | TBD  | Down through mid                           |
|25 | Mid Double Doors      | TBD     | TBD     | TBD  | Past mid doors                             |
|26 | CT Spawn (loop)       | TBD     | TBD     | TBD  | Matches waypoint 1 — seamless loop         |

---

## Config Snippet

Paste into `cs-flythrough.toml` when coordinates are ready:

```toml
[[routes]]
map = "de_dust2"
waypoints = [
    # 1  CT Spawn
    [0.0, 0.0, 64.0],
    # 2  CT Spawn turn
    [0.0, 0.0, 64.0],
    # ... fill in as probed
]
```

---

## Probing Log

Record each `--camera-pos` test here so we don't lose good coordinates.

| Date       | Position          | Angle       | Frame / Notes           |
|------------|-------------------|-------------|-------------------------|
| (pending)  |                   |             |                         |
