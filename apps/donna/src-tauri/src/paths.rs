//! Where the configuration file and the Core binary live.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const CORE_EXE: &str = "jarvis-core.exe";
const CONFIG_FILE: &str = "donna.toml";

/// `DONNA_CONFIG`, else the `donna.toml` of the profile this app is installed
/// in (`<profile>\app\donna.exe` beside `<profile>\donna.toml`), else
/// `%LOCALAPPDATA%\Donna\donna.toml`.
pub fn config_path() -> PathBuf {
    let exe_dir = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from));
    resolve_config(
        env::var_os("DONNA_CONFIG"),
        exe_dir.as_deref(),
        env::var_os("LOCALAPPDATA").map(PathBuf::from),
    )
}

fn resolve_config(
    explicit: Option<OsString>,
    exe_dir: Option<&Path>,
    local_app_data: Option<PathBuf>,
) -> PathBuf {
    if let Some(path) = explicit.filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    // Without this, a profile's app would read another profile's
    // configuration, and its Core would migrate that profile's database.
    if let Some(profile) = exe_dir
        .and_then(Path::parent)
        .map(|dir| dir.join(CONFIG_FILE))
        .filter(|path| path.is_file())
    {
        return profile;
    }
    local_app_data
        .unwrap_or_else(env::temp_dir)
        .join("Donna")
        .join(CONFIG_FILE)
}

/// The directory searched for `jarvis-core.exe`, for error messages.
pub fn exe_dir_display() -> String {
    env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.display().to_string()))
        .unwrap_or_else(|| "?".to_owned())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch(name: &str) -> PathBuf {
        let dir = env::temp_dir().join(format!("donna-paths-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("app")).unwrap();
        dir
    }

    #[test]
    fn the_installed_profile_wins_over_the_default() {
        let profile = scratch("profile");
        fs::write(profile.join(CONFIG_FILE), "").unwrap();
        let found = resolve_config(None, Some(&profile.join("app")), Some("C:/lad".into()));
        assert_eq!(found, profile.join(CONFIG_FILE));
        fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn without_a_profile_file_the_default_is_used() {
        let profile = scratch("none");
        let found = resolve_config(None, Some(&profile.join("app")), Some("C:/lad".into()));
        assert_eq!(
            found,
            PathBuf::from("C:/lad").join("Donna").join(CONFIG_FILE)
        );
        fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn an_explicit_path_wins_and_an_empty_one_is_ignored() {
        let profile = scratch("explicit");
        fs::write(profile.join(CONFIG_FILE), "").unwrap();
        let app = profile.join("app");
        let found = resolve_config(Some("D:/x.toml".into()), Some(&app), None);
        assert_eq!(found, PathBuf::from("D:/x.toml"));
        let found = resolve_config(Some(OsString::new()), Some(&app), None);
        assert_eq!(found, profile.join(CONFIG_FILE));
        fs::remove_dir_all(profile).unwrap();
    }
}
