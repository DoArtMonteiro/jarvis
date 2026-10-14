//! Finding out whether a process with a given executable name is running.
//!
//! This is a read of the system's process list and nothing more. It never
//! opens a process, so it holds no handle to a game and does not look at its
//! memory, threads or modules. It reports which of the names the caller
//! asked about is running, never the list itself.
//!
//! | | Windows | Linux |
//! | --- | --- | --- |
//! | Source | A Toolhelp snapshot of the process table | `/proc/<pid>/comm` for every numeric entry |
//! | Name compared | The executable's file name | The kernel's `comm`, cut to 15 bytes |
//! | Other Unix | Not supported: [`ProcessListError::Unsupported`] | |
//!
//! Names are compared ASCII case-insensitively. On Linux the configured
//! name is cut to 15 bytes before the comparison, because that is all the
//! kernel keeps. Two different programs whose names share their first 15
//! bytes cannot be told apart there, so a configured name longer than that
//! can match a different program.
//!
//! Anything that stops the list from being read completely is an error. The
//! caller must not read an error as "nothing is running".

use std::fmt;
use std::io;

#[cfg(unix)]
use crate::unix as platform;
#[cfg(windows)]
use crate::windows as platform;

/// Why the process list could not be read.
#[derive(Debug)]
pub enum ProcessListError {
    /// This platform has no implementation.
    Unsupported,
    /// The OS refused or failed to produce the list.
    Os(io::Error),
}

impl fmt::Display for ProcessListError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("process detection is not supported on this platform"),
            Self::Os(error) => write!(f, "cannot read the process list: {error}"),
        }
    }
}

impl std::error::Error for ProcessListError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unsupported => None,
            Self::Os(error) => Some(error),
        }
    }
}

/// The first of `names` (in the order given) that some running process is
/// called, spelled as the caller wrote it. `None` means the whole list was
/// read and none of them runs.
pub fn find_running(names: &[&str]) -> Result<Option<String>, ProcessListError> {
    if names.is_empty() {
        return Ok(None);
    }
    let matcher = Matcher::new(names, platform::PROCESS_NAME_LIMIT);
    let mut best: Option<usize> = None;
    platform::visit_process_names(&mut |running| {
        if let Some(index) = matcher.position(running) {
            best = Some(best.map_or(index, |b| b.min(index)));
        }
    })?;
    Ok(best.map(|index| names[index].to_owned()))
}

/// Compares running names against the configured ones.
struct Matcher {
    /// The configured names, lower-cased and cut to the platform's limit.
    wanted: Vec<Vec<u8>>,
}

impl Matcher {
    fn new(names: &[&str], limit: Option<usize>) -> Self {
        let wanted = names
            .iter()
            .map(|name| {
                let bytes = name.as_bytes();
                let cut = limit.map_or(bytes.len(), |limit| limit.min(bytes.len()));
                bytes[..cut].to_ascii_lowercase()
            })
            .collect();
        Self { wanted }
    }

    /// Index of the first configured name equal to `running`.
    fn position(&self, running: &str) -> Option<usize> {
        let running = running.as_bytes().to_ascii_lowercase();
        self.wanted.iter().position(|wanted| *wanted == running)
    }
}

/// A `/proc` entry that is all ASCII digits.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn is_pid(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit())
}

/// The name in a Linux `comm` file: everything before the trailing newline.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn comm_name(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|&byte| byte == b'\n')
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_ignoring_ascii_case_only() {
        let matcher = Matcher::new(&["CS2.exe", "game.exe"], None);
        assert_eq!(matcher.position("cs2.EXE"), Some(0));
        assert_eq!(matcher.position("Game.exe"), Some(1));
        assert_eq!(matcher.position("cs2.exe.bak"), None);
        assert_eq!(matcher.position("xcs2.exe"), None);
        assert_eq!(matcher.position(""), None);
    }

    #[test]
    fn non_ascii_letters_are_not_folded() {
        let matcher = Matcher::new(&["spiel.exe"], None);
        assert_eq!(matcher.position("SPIEL.exe"), Some(0));
        let matcher = Matcher::new(&["\u{e4}.exe"], None);
        assert_eq!(matcher.position("\u{c4}.exe"), None);
    }

    #[test]
    fn a_limit_cuts_the_configured_name_and_nothing_else() {
        let matcher = Matcher::new(&["abcdefghijklmnopqrst"], Some(15));
        assert_eq!(matcher.position("abcdefghijklmno"), Some(0));
        assert_eq!(matcher.position("abcdefghijklmnX"), None);
        assert_eq!(matcher.position("abcdefghijklmnopqrst"), None);
        let short = Matcher::new(&["abc"], Some(15));
        assert_eq!(short.position("abc"), Some(0));
    }

    #[test]
    fn the_first_configured_name_wins() {
        let matcher = Matcher::new(&["b.exe", "a.exe", "b.exe"], None);
        assert_eq!(matcher.position("b.exe"), Some(0));
    }

    #[test]
    fn no_names_means_nothing_to_find() {
        assert!(matches!(find_running(&[]), Ok(None)));
    }

    #[test]
    fn only_numeric_entries_are_processes() {
        assert!(is_pid("1"));
        assert!(is_pid("4242"));
        for other in [
            "",
            "self",
            "thread-self",
            "12a",
            "-1",
            "1.0",
            "sys",
            "cpuinfo",
        ] {
            assert!(!is_pid(other), "{other:?}");
        }
    }

    #[test]
    fn comm_loses_its_newline_and_nothing_else() {
        assert_eq!(comm_name(b"cs2\n"), "cs2");
        assert_eq!(comm_name(b"a b\n"), "a b");
        assert_eq!(comm_name(b"no-newline"), "no-newline");
        assert_eq!(comm_name(b"\n"), "");
        assert_eq!(comm_name(b"\xffx\n"), "\u{fffd}x");
    }
}
