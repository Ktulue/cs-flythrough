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

fn parse_headless_args(args: &[String]) -> Result<headless::HeadlessArgs, String> {
    let mut result = headless::HeadlessArgs {
        walkthrough: false,
        output_dir: std::path::PathBuf::from("./captures/"),
        camera_pos: None,
        camera_angle_deg: None,
        frame_count: 1,
        frame_step: 60,
        width: 1920,
        height: 1080,
        map: None,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--walkthrough" => result.walkthrough = true,
            "--output" => {
                i += 1;
                let v = args.get(i).ok_or("--output requires a directory path")?;
                result.output_dir = v.into();
            }
            "--camera-pos" => {
                i += 1;
                let s = args.get(i).ok_or("--camera-pos requires x,y,z")?;
                let parts: Vec<f32> = s.split(',')
                    .map(|p| p.trim().parse::<f32>().map_err(|_| format!("invalid float in --camera-pos: '{p}'")))
                    .collect::<Result<_, _>>()?;
                if parts.len() != 3 {
                    return Err(format!("--camera-pos requires exactly 3 comma-separated values, got {}", parts.len()));
                }
                result.camera_pos = Some([parts[0], parts[1], parts[2]]);
            }
            "--camera-angle" => {
                i += 1;
                let s = args.get(i).ok_or("--camera-angle requires yaw,pitch (degrees)")?;
                let parts: Vec<f32> = s.split(',')
                    .map(|p| p.trim().parse::<f32>().map_err(|_| format!("invalid float in --camera-angle: '{p}'")))
                    .collect::<Result<_, _>>()?;
                if parts.len() != 2 {
                    return Err(format!("--camera-angle requires exactly 2 comma-separated values, got {}", parts.len()));
                }
                result.camera_angle_deg = Some([parts[0], parts[1]]);
            }
            "--frame-count" => {
                i += 1;
                result.frame_count = args.get(i).ok_or("--frame-count requires a value")?
                    .parse().map_err(|_| "--frame-count must be a non-negative integer".to_string())?;
            }
            "--frame-step" => {
                i += 1;
                result.frame_step = args.get(i).ok_or("--frame-step requires a value")?
                    .parse().map_err(|_| "--frame-step must be a positive integer".to_string())?;
            }
            "--resolution" => {
                i += 1;
                let s = args.get(i).ok_or("--resolution requires WxH (e.g. 1920x1080)")?;
                let parts: Vec<&str> = s.splitn(2, 'x').collect();
                if parts.len() != 2 {
                    return Err("--resolution must be WxH format (e.g. 1920x1080)".to_string());
                }
                result.width = parts[0].parse().map_err(|_| "invalid width in --resolution".to_string())?;
                result.height = parts[1].parse().map_err(|_| "invalid height in --resolution".to_string())?;
            }
            "--map" => {
                i += 1;
                result.map = Some(args.get(i).ok_or("--map requires a map name")?.clone());
            }
            other => return Err(format!("unknown flag: '{other}'")),
        }
        i += 1;
    }
    Ok(result)
}

fn main() {
    let log_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join("cs-flythrough-debug.log")))
        .unwrap_or_else(|| std::path::PathBuf::from("cs-flythrough-debug.log"));
    log::init(&log_path);

    let raw_args: Vec<String> = std::env::args().collect();

    // Headless mode — checked before Windows screensaver convention
    if raw_args.iter().any(|a| a == "--headless") {
        let flags: Vec<String> = raw_args[1..].iter()
            .filter(|a| a.as_str() != "--headless")
            .cloned()
            .collect();

        let headless_args = match parse_headless_args(&flags) {
            Ok(a) => a,
            Err(e) => {
                capture::print_error_json(&e);
                std::process::exit(1);
            }
        };

        let binary_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| Path::new(".").to_path_buf());
        let config_path = binary_dir.join("cs-flythrough.toml");

        let cfg = if config_path.exists() {
            match config::Config::load(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    capture::print_error_json(&format!("failed to load config: {e:#}"));
                    std::process::exit(1);
                }
            }
        } else {
            capture::print_error_json("cs-flythrough.toml not found — run without --headless first to generate it");
            std::process::exit(1);
        };

        if let Err(e) = headless::run(headless_args, cfg) {
            capture::print_error_json(&format!("{e:#}"));
            std::process::exit(1);
        }
        return;
    }

    // Windows screensaver convention
    let mode = raw_args.get(1).map(|s| s.as_str()).unwrap_or("/s");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_headless_args_defaults() {
        let args = parse_headless_args(&[]).unwrap();
        assert_eq!(args.frame_count, 1);
        assert_eq!(args.frame_step, 60);
        assert_eq!(args.width, 1920);
        assert_eq!(args.height, 1080);
        assert!(!args.walkthrough);
        assert!(args.camera_pos.is_none());
        assert!(args.camera_angle_deg.is_none());
        assert!(args.map.is_none());
    }

    #[test]
    fn test_headless_args_resolution() {
        let args = parse_headless_args(&["--resolution".into(), "640x480".into()]).unwrap();
        assert_eq!(args.width, 640);
        assert_eq!(args.height, 480);
    }

    #[test]
    fn test_headless_args_camera_pos() {
        let args = parse_headless_args(&["--camera-pos".into(), "1.0,2.5,-3.0".into()]).unwrap();
        assert_eq!(args.camera_pos, Some([1.0, 2.5, -3.0]));
    }

    #[test]
    fn test_headless_args_camera_angle() {
        let args = parse_headless_args(&["--camera-angle".into(), "90.0,0.0".into()]).unwrap();
        assert_eq!(args.camera_angle_deg, Some([90.0, 0.0]));
    }

    #[test]
    fn test_headless_args_walkthrough_and_frame_count() {
        let args = parse_headless_args(&[
            "--walkthrough".into(),
            "--frame-count".into(), "10".into(),
            "--frame-step".into(), "30".into(),
        ]).unwrap();
        assert!(args.walkthrough);
        assert_eq!(args.frame_count, 10);
        assert_eq!(args.frame_step, 30);
    }

    #[test]
    fn test_headless_args_map_override() {
        let args = parse_headless_args(&["--map".into(), "cs_office".into()]).unwrap();
        assert_eq!(args.map.as_deref(), Some("cs_office"));
    }

    #[test]
    fn test_headless_args_frame_count_zero_passes_parsing() {
        // Validation of frame_count=0 is done in headless::run, not the parser
        let args = parse_headless_args(&["--frame-count".into(), "0".into()]).unwrap();
        assert_eq!(args.frame_count, 0);
    }

    #[test]
    fn test_headless_args_frame_step_zero_passes_parsing() {
        // Validation of frame_step=0 is done in headless::run, not the parser
        let args = parse_headless_args(&["--frame-step".into(), "0".into()]).unwrap();
        assert_eq!(args.frame_step, 0);
    }

    #[test]
    fn test_headless_args_unknown_flag_errors() {
        assert!(parse_headless_args(&["--bogus".into()]).is_err());
    }

    #[test]
    fn test_headless_args_missing_value_errors() {
        assert!(parse_headless_args(&["--output".into()]).is_err());
        assert!(parse_headless_args(&["--frame-count".into()]).is_err());
    }

    #[test]
    fn test_headless_args_camera_pos_wrong_count_errors() {
        assert!(parse_headless_args(&["--camera-pos".into(), "1.0,2.0".into()]).is_err());
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
    let waypoints = if let Some(nav_path) = maplist::resolve_nav(&cfg.cs_install_path, map_name) {
        match bsp::nav::load_waypoints(&nav_path, 250.0, -64.0) {
            Ok(pts) => {
                eprintln!("cs-flythrough: using {} NAV waypoints from {}", pts.len(), nav_path.display());
                // Sort spatially, decimate to remove tight clusters, then smooth.
                let sorted = camera::nearest_neighbor_sort(pts);
                let decimated = camera::decimate_waypoints(sorted, 250.0);
                camera::smooth_waypoints(decimated, 3)
            }
            Err(e) => {
                eprintln!("cs-flythrough: NAV load failed ({e:#}), falling back to entity origins");
                camera::smooth_waypoints(camera::nearest_neighbor_sort(mesh.entity_origins.clone()), 3)
            }
        }
    } else {
        eprintln!("cs-flythrough: no NAV file found for '{map_name}', using entity origins");
        camera::smooth_waypoints(camera::nearest_neighbor_sort(mesh.entity_origins.clone()), 3)
    };

    let cam = camera::Camera::new(
        waypoints,
        cfg.camera_speed,
        cfg.bob_amplitude,
        cfg.bob_frequency,
    )?;

    renderer::run(mesh, cam)
}
