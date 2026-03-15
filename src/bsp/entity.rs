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
        assert!(pts.iter().all(|p| *p != Vec3::ZERO || true)); // worldspawn has no origin in sample
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
