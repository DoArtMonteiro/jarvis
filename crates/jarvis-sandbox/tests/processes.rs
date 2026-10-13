//! Process detection against the real OS: no fake process list, no game.
//!
//! The exit test copies a small system binary to a unique name, runs the
//! copy, and checks that the production function sees it while it runs and
//! stops seeing it once it has exited.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use jarvis_sandbox::processes::find_running;

/// A name nothing else on the machine uses. At most 15 bytes without the
/// extension, so that a Linux `comm` holds all of it.
fn unique_stem() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let mixed = nanos ^ (u128::from(std::process::id()) << 40);
    format!("jv{:012x}", mixed & 0xffff_ffff_ffff)
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_owned()
    }
}

/// Copy a small system binary to `dir/<unique>` and start it for 30 s.
fn start_copy(dir: &Path, stem: &str) -> (Child, PathBuf) {
    let target = dir.join(exe_name(stem));
    let (source, args): (&str, &[&str]) = if cfg!(windows) {
        (r"C:\Windows\System32\PING.EXE", &["-n", "30", "127.0.0.1"])
    } else {
        ("/bin/sleep", &["30"])
    };
    std::fs::copy(source, &target).unwrap();
    let child = Command::new(&target)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    (child, target)
}

fn wait_until(mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn the_test_binarys_own_name_is_found() {
    let exe = std::env::current_exe().unwrap();
    let name = exe.file_name().unwrap().to_str().unwrap().to_owned();
    // Another name first, to show that the configured name that matched is
    // the one returned.
    let found = find_running(&["not-a-running-process.exe", &name]).unwrap();
    assert_eq!(found.as_deref(), Some(name.as_str()));
}

#[test]
fn matching_ignores_ascii_case_and_returns_the_configured_spelling() {
    let exe = std::env::current_exe().unwrap();
    let name = exe
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_ascii_uppercase();
    let found = find_running(&[&name]).unwrap();
    assert_eq!(found.as_deref(), Some(name.as_str()));
}

#[test]
fn a_name_nobody_runs_is_not_found() {
    let name = exe_name(&unique_stem());
    assert_eq!(find_running(&[&name]).unwrap(), None);
    assert_eq!(find_running(&[]).unwrap(), None);
}

#[test]
fn a_process_is_found_while_it_runs_and_not_after_it_exits() {
    let stem = unique_stem();
    let name = exe_name(&stem);
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(find_running(&[&name]).unwrap(), None, "not started yet");

    let (mut child, _target) = start_copy(dir.path(), &stem);
    let seen = wait_until(|| find_running(&[&name]).unwrap().is_some());
    let found = find_running(&[&name]).unwrap();

    child.kill().unwrap();
    child.wait().unwrap();
    let gone = wait_until(|| find_running(&[&name]).unwrap().is_none());

    assert!(seen, "the running process was never detected");
    assert_eq!(found.as_deref(), Some(name.as_str()));
    assert!(gone, "the process was still reported after it exited");
}

#[cfg(target_os = "linux")]
#[test]
fn a_linux_name_longer_than_comm_matches_its_truncation() {
    // `comm` keeps 15 bytes, so the configured name is cut the same way.
    // The price is that two names with the same first 15 bytes are equal.
    let stem = unique_stem();
    let dir = tempfile::tempdir().unwrap();
    let (mut child, _target) = start_copy(dir.path(), &stem);
    let long = format!("{stem}-and-a-long-tail");
    let seen = wait_until(|| find_running(&[&long]).unwrap().is_some());
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(seen);
}
