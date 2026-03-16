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
    /// Hand-designed waypoint routes per map. When a route exists for the current
    /// map it takes priority over the NAV file and entity-origin fallback.
    #[serde(default)]
    pub routes: Vec<CustomRoute>,
}

/// A hand-designed camera path for a specific map.
/// Each waypoint is [x, y, z] in BSP world coordinates.
/// The camera follows a closed Catmull-Rom spline through the points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomRoute {
    pub map: String,
    pub waypoints: Vec<[f32; 3]>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MapSelection {
    Single,
    List,
    All,
}

impl Config {
    /// Return the custom route for `map_name` if one exists.
    pub fn find_route(&self, map_name: &str) -> Option<&CustomRoute> {
        self.routes.iter().find(|r| r.map == map_name)
    }

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

    #[test]
    fn test_custom_route_round_trips() {
        let f = write_temp(r#"
cs_install_path = "C:/games/cstrike"
map_selection = "single"
map = "de_dust2"
camera_speed = 133.0
bob_amplitude = 2.0
bob_frequency = 2.0

[[routes]]
map = "de_dust2"
waypoints = [
    [-512.0, 128.0, 64.0],
    [100.0, 200.0, 64.0],
    [300.0, -50.0, 64.0],
    [-100.0, -200.0, 64.0],
]
"#);
        let cfg = Config::load(f.path()).unwrap();
        let route = cfg.find_route("de_dust2").expect("route not found");
        assert_eq!(route.waypoints.len(), 4);
        assert_eq!(route.waypoints[0], [-512.0, 128.0, 64.0]);
    }

    #[test]
    fn test_no_route_returns_none() {
        let f = write_temp(r#"
cs_install_path = "C:/games/cstrike"
map_selection = "single"
map = "de_dust2"
camera_speed = 133.0
bob_amplitude = 2.0
bob_frequency = 2.0
"#);
        let cfg = Config::load(f.path()).unwrap();
        assert!(cfg.find_route("de_dust2").is_none());
    }
}
