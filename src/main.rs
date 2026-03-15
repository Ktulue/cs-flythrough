mod config;
mod maplist;
mod bsp;
mod camera;
mod renderer;
mod input;

use anyhow::Result;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("/s");

    match mode {
        "/s" => run_screensaver(),
        "/c" => {
            eprintln!("Settings dialog not yet implemented.");
            Ok(())
        }
        "/p" => {
            eprintln!("Preview mode not yet implemented.");
            Ok(())
        }
        _ => {
            eprintln!("Unknown mode: {mode}. Use /s to run screensaver.");
            Ok(())
        }
    }
}

fn run_screensaver() -> Result<()> {
    todo!("implement startup sequence")
}
