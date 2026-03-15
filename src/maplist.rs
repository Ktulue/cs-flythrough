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

/// Resolve a NAV file for the given map name.
///
/// NAV files are not always co-located with BSP files. Steam ships a single `cstrike`
/// directory without NAV files, while the same map's NAV lives in `czero` which may be
/// a sibling Steam install (e.g. "Half-Life 80" next to "Half-Life"). This function
/// tries the configured path first, then scans sibling directories in the Steam
/// common folder so users don't need to configure both paths.
pub fn resolve_nav(cs_install_path: &Path, map_name: &str) -> Option<PathBuf> {
    let nav_name = format!("{map_name}.nav");

    // 1. Direct candidates under cs_install_path.
    for sub in &["cstrike", "czero"] {
        let p = cs_install_path.join(sub).join("maps").join(&nav_name);
        if p.exists() {
            return Some(p);
        }
    }

    // 2. Scan sibling directories of cs_install_path (handles "Half-Life 80" next to
    //    "Half-Life", which is how Steam ships CS 1.6 + Condition Zero on some depots).
    if let Some(parent) = cs_install_path.parent() {
        if let Ok(entries) = std::fs::read_dir(parent) {
            for entry in entries.flatten() {
                let sibling = entry.path();
                if sibling == cs_install_path { continue; }
                for sub in &["cstrike", "czero"] {
                    let p = sibling.join(sub).join("maps").join(&nav_name);
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

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
