use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

static LOG: OnceLock<Mutex<File>> = OnceLock::new();

/// Initialize the log file. Call once at startup before any `diag!` calls.
pub fn init(path: &Path) {
    if let Ok(f) = File::create(path) {
        let _ = LOG.set(Mutex::new(f));
    }
}

/// Write a line to the log file (and also to stderr for terminal runs).
pub fn write_line(msg: &str) {
    eprintln!("{msg}");
    if let Some(m) = LOG.get() {
        if let Ok(mut f) = m.lock() {
            let _ = writeln!(f, "{msg}");
        }
    }
}

/// Log a formatted message to the log file and stderr.
#[macro_export]
macro_rules! diag {
    ($($arg:tt)*) => {
        $crate::log::write_line(&format!($($arg)*))
    };
}
