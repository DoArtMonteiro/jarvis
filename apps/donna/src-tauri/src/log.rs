//! A small append-only log for the app's own faults (next to `core.log`).

use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::paths;

/// Append one line. Failing to log is never an error.
pub fn line(message: &str) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let _ = std::fs::create_dir_all(paths::data_dir());
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::data_dir().join("donna.log"))
    {
        let _ = writeln!(file, "[{seconds}] {message}");
    }
}
