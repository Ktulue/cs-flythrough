use anyhow::Result;
use std::path::Path;

mod config;
mod maplist;
mod bsp;
mod camera;
mod renderer;
mod input;
pub mod log;
mod capture;
mod headless;

fn main() {
    // Initialize log file next to the exe so double-click runs produce readable output.
    let log_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("cs-flythrough-debug.log")))
        .unwrap_or_else(|| std::path::PathBuf::from("cs-flythrough-debug.log"));
    log::init(&log_path);

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

    // Try NAV file for natural player-path waypoints; fall back to entity origins.
    let nav_path = bsp_path.with_extension("nav");
    let waypoints = if nav_path.exists() {
        match bsp::nav::load_waypoints(&nav_path) {
            Ok(pts) => {
                eprintln!("cs-flythrough: using {} NAV waypoints from {}", pts.len(), nav_path.display());
                pts  // already DFS-ordered
            }
            Err(e) => {
                eprintln!("cs-flythrough: NAV load failed ({e:#}), falling back to entity origins");
                camera::nearest_neighbor_sort(mesh.entity_origins.clone())
            }
        }
    } else {
        eprintln!("cs-flythrough: no NAV file at {}, using entity origins", nav_path.display());
        camera::nearest_neighbor_sort(mesh.entity_origins.clone())
    };

    let cam = camera::Camera::new(
        waypoints,
        cfg.camera_speed,
        cfg.bob_amplitude,
        cfg.bob_frequency,
    )?;

    renderer::run(mesh, cam)
}
