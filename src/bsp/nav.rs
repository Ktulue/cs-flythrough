use std::path::Path;
use anyhow::{anyhow, Context, Result};
use glam::Vec3;

const NAV_MAGIC: u32 = 0xFEED_FACE;

/// Cursor over a byte slice for reading little-endian binary data.
struct NavCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> NavCursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_u8(&mut self) -> Result<u8> {
        if self.remaining() < 1 {
            return Err(anyhow!("unexpected EOF reading u8 at offset {}", self.pos));
        }
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    fn read_u16_le(&mut self) -> Result<u16> {
        if self.remaining() < 2 {
            return Err(anyhow!("unexpected EOF reading u16 at offset {}", self.pos));
        }
        let bytes: [u8; 2] = self.data[self.pos..self.pos + 2].try_into().unwrap();
        self.pos += 2;
        Ok(u16::from_le_bytes(bytes))
    }

    fn read_u32_le(&mut self) -> Result<u32> {
        if self.remaining() < 4 {
            return Err(anyhow!("unexpected EOF reading u32 at offset {}", self.pos));
        }
        let bytes: [u8; 4] = self.data[self.pos..self.pos + 4].try_into().unwrap();
        self.pos += 4;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_f32_le(&mut self) -> Result<f32> {
        if self.remaining() < 4 {
            return Err(anyhow!("unexpected EOF reading f32 at offset {}", self.pos));
        }
        let bytes: [u8; 4] = self.data[self.pos..self.pos + 4].try_into().unwrap();
        self.pos += 4;
        Ok(f32::from_le_bytes(bytes))
    }

    fn skip(&mut self, n: usize) -> Result<()> {
        if self.remaining() < n {
            return Err(anyhow!(
                "unexpected EOF skipping {} bytes at offset {}",
                n,
                self.pos
            ));
        }
        self.pos += n;
        Ok(())
    }
}

/// Internal representation of a parsed NAV area with its connection graph.
struct NavArea {
    id: u32,
    center: Vec3,
    connections: Vec<u32>,  // neighbor area IDs
}

/// Return area centers in DFS traversal order, following corridor connections.
fn dfs_order(areas: &[(Vec3, Vec<usize>)]) -> Vec<Vec3> {
    let mut visited = vec![false; areas.len()];
    let mut order = Vec::with_capacity(areas.len());

    for start in 0..areas.len() {
        if visited[start] { continue; }
        let mut stack = vec![start];
        while let Some(idx) = stack.pop() {
            if visited[idx] { continue; }
            visited[idx] = true;
            order.push(areas[idx].0);
            // Push neighbors in reverse so first connection is visited first
            for &neighbor_idx in areas[idx].1.iter().rev() {
                if !visited[neighbor_idx] {
                    stack.push(neighbor_idx);
                }
            }
        }
    }
    order
}

/// Parse a CS 1.6 NAV file and return area centers as waypoints.
///
/// Waypoints are returned in DFS traversal order following the connection graph,
/// so the camera follows actual corridors rather than jumping across the map.
///
/// Area center is the midpoint of all four corners:
/// - `center_x = (nw_x + se_x) / 2`
/// - `center_y = (nw_y + se_y) / 2`
/// - `center_z = (nw_z + se_z + ne_z + sw_z) / 4`
///
/// `min_extent` filters out NAV areas whose narrower XY dimension is below the
/// threshold.  Tight areas (doorways, spawn boxes, corridor ends) have their
/// centers close to wall surfaces, which places the camera immediately against
/// a wall.  Pass `0.0` to include all areas.
///
/// Areas whose average Z floor is below `min_z` are also excluded.  Water
/// volumes and sub-floor navigation areas have very negative Z values and
/// produce camera positions below the visible geometry.  Typical GoldSrc
/// playable floors are above Z = -200; pass `f32::NEG_INFINITY` to disable.
pub fn load_waypoints(nav_path: &Path, min_extent: f32, min_z: f32) -> Result<Vec<Vec3>> {
    let data = std::fs::read(nav_path)
        .with_context(|| format!("failed to read NAV file: {}", nav_path.display()))?;

    let mut c = NavCursor::new(&data);

    // Header
    let magic = c.read_u32_le().context("reading NAV magic")?;
    if magic != NAV_MAGIC {
        return Err(anyhow!(
            "invalid NAV magic: expected 0x{:08X}, got 0x{:08X}",
            NAV_MAGIC,
            magic
        ));
    }

    let version = c.read_u32_le().context("reading NAV version")?;

    // CS 1.6 uses version 6; we tolerate any version >= 2 that we understand.
    if version < 2 {
        return Err(anyhow!(
            "NAV version {version} is too old (need >= 2, CS 1.6 uses 6)"
        ));
    }

    // NAV v5 (Counter-Strike: Condition Zero / early CS 1.6) adds two sections
    // between the version and the area count that v6 does not have:
    //   1. bsp_size (u32): the BSP file size, used for validation at load time.
    //   2. place directory: u16 place_count, then for each place a u16 name_length
    //      followed by name_length bytes of the place name (null-terminated).
    // v6 (standard CS 1.6) omits these: area_count immediately follows version.
    if version == 5 {
        // Skip bsp_size validation field.
        c.skip(4).context("v5: skipping bsp_size")?;

        // Skip place name directory.
        let place_count = c.read_u16_le().context("v5: reading place_count")?;
        for pi in 0..place_count {
            let name_len = c.read_u16_le()
                .with_context(|| format!("v5: place {pi}: reading name_len"))?;
            c.skip(name_len as usize)
                .with_context(|| format!("v5: place {pi}: skipping {name_len} name bytes"))?;
        }
    }

    let area_count = c.read_u32_le().context("reading area count")?;

    let mut raw_areas: Vec<NavArea> = Vec::with_capacity(area_count as usize);

    for area_idx in 0..area_count {
        let id = c
            .read_u32_le()
            .with_context(|| format!("area {area_idx}: reading id"))?;

        // flags field: u8 in NAV v5, u32 in v6+.
        // (v5 = CS:CZ shipped format; v6 = standard CS 1.6 format)
        if version == 5 {
            c.read_u8().with_context(|| format!("area {area_idx}: reading flags (v5 u8)"))?;
        } else {
            c.read_u32_le().with_context(|| format!("area {area_idx}: reading flags (v6+ u32)"))?;
        }

        // NW corner XYZ
        let nw_x = c
            .read_f32_le()
            .with_context(|| format!("area {area_idx}: nw_x"))?;
        let nw_y = c
            .read_f32_le()
            .with_context(|| format!("area {area_idx}: nw_y"))?;
        let nw_z = c
            .read_f32_le()
            .with_context(|| format!("area {area_idx}: nw_z"))?;

        // SE corner XYZ
        let se_x = c
            .read_f32_le()
            .with_context(|| format!("area {area_idx}: se_x"))?;
        let se_y = c
            .read_f32_le()
            .with_context(|| format!("area {area_idx}: se_y"))?;
        let se_z = c
            .read_f32_le()
            .with_context(|| format!("area {area_idx}: se_z"))?;

        // NE corner Z height
        let ne_z = c
            .read_f32_le()
            .with_context(|| format!("area {area_idx}: ne_z"))?;

        // SW corner Z height
        let sw_z = c
            .read_f32_le()
            .with_context(|| format!("area {area_idx}: sw_z"))?;

        // Compute center
        let center = Vec3::new(
            (nw_x + se_x) / 2.0,
            (nw_y + se_y) / 2.0,
            (nw_z + se_z + ne_z + sw_z) / 4.0,
        );

        // Minimum XY dimension: area center is this many units from the nearest
        // axis-aligned wall.  Doorways and spawn boxes are typically 64-96 units;
        // open corridors and plazas are 128+ units.
        let extent = (se_x - nw_x).abs().min((se_y - nw_y).abs());

        // --- Connections: 4 directions ---
        let mut connections = Vec::new();
        for _dir in 0..4u32 {
            let conn_count = c
                .read_u32_le()
                .with_context(|| format!("area {area_idx}: connection count"))?;
            for _ in 0..conn_count {
                connections.push(
                    c.read_u32_le()
                        .with_context(|| format!("area {area_idx}: reading connection id"))?,
                );
            }
        }

        if extent >= min_extent && center.z >= min_z {
            raw_areas.push(NavArea { id, center, connections });
        }

        // --- Hiding spots (version >= 2) ---
        let hiding_count = c
            .read_u8()
            .with_context(|| format!("area {area_idx}: hiding_count"))?;
        // Each hiding spot: id(u32) + position[3*f32] + flags(u8) = 17 bytes
        c.skip(hiding_count as usize * 17)
            .with_context(|| format!("area {area_idx}: skipping {hiding_count} hiding spots"))?;

        // --- Approach spots (version < 15; CS 1.6 v6 always has these) ---
        let approach_count = c
            .read_u8()
            .with_context(|| format!("area {area_idx}: approach_count"))?;
        // Each approach spot: from_id(u32) + prev_id(u32) + how(u8) + next_id(u32) + how2(u8) = 14 bytes
        c.skip(approach_count as usize * 14)
            .with_context(|| {
                format!("area {area_idx}: skipping {approach_count} approach spots")
            })?;

        // --- Encounter paths (version >= 2) ---
        let path_count = c
            .read_u32_le()
            .with_context(|| format!("area {area_idx}: path_count"))?;
        for path_idx in 0..path_count {
            let _from_id = c
                .read_u32_le()
                .with_context(|| format!("area {area_idx} path {path_idx}: from_id"))?;
            let _from_dir = c
                .read_u8()
                .with_context(|| format!("area {area_idx} path {path_idx}: from_dir"))?;
            let _to_id = c
                .read_u32_le()
                .with_context(|| format!("area {area_idx} path {path_idx}: to_id"))?;
            let _to_dir = c
                .read_u8()
                .with_context(|| format!("area {area_idx} path {path_idx}: to_dir"))?;
            let spot_count = c
                .read_u8()
                .with_context(|| format!("area {area_idx} path {path_idx}: spot_count"))?;
            // Each spot: spot_id(u32) + t(u8) = 5 bytes
            c.skip(spot_count as usize * 5).with_context(|| {
                format!("area {area_idx} path {path_idx}: skipping {spot_count} spots")
            })?;
        }

        // --- Place ID (version >= 5) ---
        let _place_id = c
            .read_u16_le()
            .with_context(|| format!("area {area_idx}: place_id"))?;

    }

    // Build id→index map and resolve connection IDs to array indices.
    let id_to_idx: std::collections::HashMap<u32, usize> = raw_areas.iter()
        .enumerate()
        .map(|(i, a)| (a.id, i))
        .collect();

    let areas_resolved: Vec<(Vec3, Vec<usize>)> = raw_areas.iter().map(|a| {
        let conn_indices: Vec<usize> = a.connections.iter()
            .filter_map(|id| id_to_idx.get(id).copied())
            .collect();
        (a.center, conn_indices)
    }).collect();

    let waypoints = dfs_order(&areas_resolved);

    if waypoints.len() < 4 {
        return Err(anyhow!(
            "NAV file has only {} areas; at least 4 are required for the Catmull-Rom spline",
            waypoints.len()
        ));
    }

    Ok(waypoints)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a minimal NAV v6 binary from a slice of area definitions.
    /// Each area tuple: (nw[3], se[3], ne_z, sw_z)
    fn make_nav_bytes(version: u32, areas: &[([f32; 3], [f32; 3], f32, f32)]) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();

        // Header
        buf.extend_from_slice(&NAV_MAGIC.to_le_bytes());
        buf.extend_from_slice(&version.to_le_bytes());

        // Area count
        let area_count = areas.len() as u32;
        buf.extend_from_slice(&area_count.to_le_bytes());

        for (idx, (nw, se, ne_z, sw_z)) in areas.iter().enumerate() {
            // id
            buf.extend_from_slice(&(idx as u32 + 1).to_le_bytes());
            // flags: u8 for v5, u32 for v6+
            if version == 5 {
                buf.push(0u8);
            } else {
                buf.extend_from_slice(&0u32.to_le_bytes());
            }
            // nw corner
            for &v in nw.iter() {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            // se corner
            for &v in se.iter() {
                buf.extend_from_slice(&v.to_le_bytes());
            }
            // ne_z
            buf.extend_from_slice(&ne_z.to_le_bytes());
            // sw_z
            buf.extend_from_slice(&sw_z.to_le_bytes());

            // 4 directions, each with 0 connections
            for _ in 0..4u32 {
                buf.extend_from_slice(&0u32.to_le_bytes());
            }

            // hiding_count = 0 (u8)
            buf.push(0u8);

            // approach_count = 0 (u8)  [present because version < 15]
            buf.push(0u8);

            // path_count = 0 (u32)  [present because version >= 2]
            buf.extend_from_slice(&0u32.to_le_bytes());

            // place_id = 0 (u16)  [present because version >= 5]
            buf.extend_from_slice(&0u16.to_le_bytes());
        }

        buf
    }

    fn write_temp_nav(bytes: &[u8]) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().expect("temp file");
        f.write_all(bytes).expect("write nav bytes");
        f
    }

    /// Four areas with known coordinates; verify centers.
    #[test]
    fn test_parse_minimal_nav() {
        let areas = [
            ([0.0f32, 0.0, 10.0], [100.0f32, 100.0, 10.0], 10.0f32, 10.0f32),
            ([200.0, 0.0, 20.0], [300.0, 100.0, 20.0], 20.0, 20.0),
            ([0.0, 200.0, 30.0], [100.0, 300.0, 30.0], 30.0, 30.0),
            ([200.0, 200.0, 40.0], [300.0, 300.0, 40.0], 40.0, 40.0),
        ];

        let bytes = make_nav_bytes(6, &areas);
        let tmp = write_temp_nav(&bytes);
        let waypoints = load_waypoints(tmp.path(), 0.0, f32::NEG_INFINITY).expect("should parse");

        assert_eq!(waypoints.len(), 4);

        // Area 0: center = ((0+100)/2, (0+100)/2, (10+10+10+10)/4) = (50, 50, 10)
        assert!((waypoints[0].x - 50.0).abs() < 1e-5, "area0 x");
        assert!((waypoints[0].y - 50.0).abs() < 1e-5, "area0 y");
        assert!((waypoints[0].z - 10.0).abs() < 1e-5, "area0 z");

        // Area 1: center = (250, 50, 20)
        assert!((waypoints[1].x - 250.0).abs() < 1e-5, "area1 x");
        assert!((waypoints[1].y - 50.0).abs() < 1e-5, "area1 y");
        assert!((waypoints[1].z - 20.0).abs() < 1e-5, "area1 z");

        // Area 3: center = (250, 250, 40)
        assert!((waypoints[3].x - 250.0).abs() < 1e-5, "area3 x");
        assert!((waypoints[3].y - 250.0).abs() < 1e-5, "area3 y");
        assert!((waypoints[3].z - 40.0).abs() < 1e-5, "area3 z");
    }

    /// Only 3 areas — parser must return Err because camera needs >= 4.
    #[test]
    fn test_too_few_areas_returns_err() {
        let areas = [
            ([0.0f32, 0.0, 0.0], [10.0f32, 10.0, 0.0], 0.0f32, 0.0f32),
            ([20.0, 0.0, 0.0], [30.0, 10.0, 0.0], 0.0, 0.0),
            ([0.0, 20.0, 0.0], [10.0, 30.0, 0.0], 0.0, 0.0),
        ];

        let bytes = make_nav_bytes(6, &areas);
        let tmp = write_temp_nav(&bytes);
        let result = load_waypoints(tmp.path(), 0.0, f32::NEG_INFINITY);

        assert!(result.is_err(), "expected Err for 3 areas");
        let msg = format!("{:#}", result.unwrap_err());
        assert!(msg.contains("3"), "error should mention area count: {msg}");
    }

    /// Bad magic number must return Err.
    #[test]
    fn test_bad_magic_returns_err() {
        let mut bytes = make_nav_bytes(6, &[
            ([0.0, 0.0, 0.0], [1.0, 1.0, 0.0], 0.0, 0.0),
        ]);
        // Overwrite the magic bytes
        bytes[0] = 0xDE;
        bytes[1] = 0xAD;
        bytes[2] = 0xBE;
        bytes[3] = 0xEF;

        let tmp = write_temp_nav(&bytes);
        let result = load_waypoints(tmp.path(), 0.0, f32::NEG_INFINITY);
        assert!(result.is_err(), "expected Err for bad magic");
        let msg = format!("{:#}", result.unwrap_err());
        assert!(msg.contains("magic"), "error should mention magic: {msg}");
    }

    /// Mixed Z heights: verify the averaging is correct.
    #[test]
    fn test_z_height_averaging() {
        let areas = [
            ([0.0f32, 0.0, 0.0], [10.0f32, 10.0, 4.0], 8.0f32, 12.0f32),
            ([20.0, 0.0, 0.0], [30.0, 10.0, 0.0], 0.0, 0.0),
            ([0.0, 20.0, 0.0], [10.0, 30.0, 0.0], 0.0, 0.0),
            ([20.0, 20.0, 0.0], [30.0, 30.0, 0.0], 0.0, 0.0),
        ];

        let bytes = make_nav_bytes(6, &areas);
        let tmp = write_temp_nav(&bytes);
        let waypoints = load_waypoints(tmp.path(), 0.0, f32::NEG_INFINITY).expect("should parse");

        // Area 0: nw_z=0, se_z=4, ne_z=8, sw_z=12 → average = (0+4+8+12)/4 = 6
        assert!(
            (waypoints[0].z - 6.0).abs() < 1e-5,
            "z average wrong: {}",
            waypoints[0].z
        );
    }

    /// Parser must skip hiding spots (17 bytes each: id+pos+flags) and approach spots
    /// (14 bytes each: from+prev+how+next+how2) without desyncing.
    /// Regression test for the 13-byte and 10-byte size bugs.
    #[test]
    fn test_hiding_and_approach_spots_skipped_correctly() {
        let areas = [
            ([0.0f32, 0.0, 0.0], [100.0f32, 100.0, 0.0], 0.0f32, 0.0f32),
            ([200.0, 0.0, 0.0], [300.0, 100.0, 0.0], 0.0, 0.0),
            ([0.0, 200.0, 0.0], [100.0, 300.0, 0.0], 0.0, 0.0),
            ([200.0, 200.0, 0.0], [300.0, 300.0, 0.0], 0.0, 0.0),
        ];

        // Build v6 NAV bytes manually with non-zero hiding/approach counts in area 0.
        let mut buf: Vec<u8> = Vec::new();
        buf.extend_from_slice(&NAV_MAGIC.to_le_bytes());
        buf.extend_from_slice(&6u32.to_le_bytes()); // version 6
        buf.extend_from_slice(&(areas.len() as u32).to_le_bytes());

        for (idx, (nw, se, ne_z, sw_z)) in areas.iter().enumerate() {
            buf.extend_from_slice(&(idx as u32 + 1).to_le_bytes()); // id
            buf.extend_from_slice(&0u32.to_le_bytes()); // flags (u32 for v6)
            for &v in nw { buf.extend_from_slice(&v.to_le_bytes()); }
            for &v in se { buf.extend_from_slice(&v.to_le_bytes()); }
            buf.extend_from_slice(&ne_z.to_le_bytes());
            buf.extend_from_slice(&sw_z.to_le_bytes());
            for _ in 0..4u32 { buf.extend_from_slice(&0u32.to_le_bytes()); } // 0 connections per dir

            if idx == 0 {
                // 2 hiding spots × 17 bytes each
                buf.push(2u8);
                for spot_id in [101u32, 102u32] {
                    buf.extend_from_slice(&spot_id.to_le_bytes()); // id (4 bytes)
                    for _ in 0..3 { buf.extend_from_slice(&0.0f32.to_le_bytes()); } // pos
                    buf.push(0x01u8); // flags
                }
                // 1 approach spot × 14 bytes
                buf.push(1u8);
                buf.extend_from_slice(&2u32.to_le_bytes()); // from_id
                buf.extend_from_slice(&3u32.to_le_bytes()); // prev_id
                buf.push(0u8); // how
                buf.extend_from_slice(&4u32.to_le_bytes()); // next_id
                buf.push(0u8); // how2
            } else {
                buf.push(0u8); // hiding_count = 0
                buf.push(0u8); // approach_count = 0
            }

            buf.extend_from_slice(&0u32.to_le_bytes()); // path_count = 0
            buf.extend_from_slice(&0u16.to_le_bytes()); // place_id = 0
        }

        let tmp = write_temp_nav(&buf);
        let waypoints = load_waypoints(tmp.path(), 0.0, f32::NEG_INFINITY)
            .expect("should parse when hiding/approach spots are correctly sized");
        assert_eq!(waypoints.len(), 4, "all 4 areas should be parsed");
        // Area 0 center should be (50, 50, 0)
        assert!((waypoints[0].x - 50.0).abs() < 1e-5, "area0 x after skip");
    }
}
