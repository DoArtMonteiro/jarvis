//! Where the configuration file and the Core binary live.

use std::env;
use std::path::PathBuf;

const CORE_EXE: &str = "jarvis-core.exe";

/// `DONNA_CONFIG`, else `%LOCALAPPDATA%\Donna\donna.toml`.
pub fn config_path() -> PathBuf {
    if let Some(path) = env::var_os("DONNA_CONFIG").filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    let base = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    base.join("Donna").join("donna.toml")
}

/// Directory for the app's own files (the Core's log).
pub fn data_dir() -> PathBuf {
    config_path()
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
}

/// `DONNA_CORE_EXE`, else `jarvis-core.exe` next to this executable, else the
/// repository's release build when it exists.
pub fn core_exe() -> Option<PathBuf> {
    if let Some(path) = env::var_os("DONNA_CORE_EXE").filter(|value| !value.is_empty()) {
        // A relative path would depend on the working directory.
        let path = PathBuf::from(path);
        return (path.is_absolute() && path.is_file()).then_some(path);
    }
    let beside = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(CORE_EXE)));
    [beside, repo_build()]
        .into_iter()
        .flatten()
        .find(|path| path.is_file())
}

/// The repository's release build, for development only: compiled out of
/// release builds so that they carry no path of the machine that built them.
#[cfg(debug_assertions)]
fn repo_build() -> Option<PathBuf> {
    Some(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../target/release")
            .join(CORE_EXE),
    )
}

#[cfg(not(debug_assertions))]
fn repo_build() -> Option<PathBuf> {
    None
}
