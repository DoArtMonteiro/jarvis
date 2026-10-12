//! The start-up isolation check: it passes under the real confinement and
//! fails, closed, when a boundary is missing. Needs a Python 3.11+
//! interpreter, named by `JARVIS_TEST_PYTHON` or found as `python3`
//! (`python` on Windows).

use std::path::{Path, PathBuf};
use std::time::Duration;

use jarvis_core::config::WorkerConfig;
use jarvis_core::isolation::{self, IsolationError};

fn python() -> String {
    std::env::var("JARVIS_TEST_PYTHON")
        .unwrap_or_else(|_| if cfg!(windows) { "python" } else { "python3" }.to_owned())
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate lives two levels below the repository root")
        .to_path_buf()
}

fn worker() -> WorkerConfig {
    WorkerConfig {
        program: python(),
        args: vec!["-m".to_owned(), "jarvis_worker".to_owned()],
        cwd: Some(repo().join("services/intelligence-python/src")),
        handshake_timeout: Duration::from_secs(10),
        memory_limit_bytes: 512 * 1024 * 1024,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_real_confinement_passes_every_check() {
    let scratch = tempfile::tempdir().unwrap();
    let launch = isolation::launch(&worker(), &[]).unwrap();
    let verification = isolation::verify(&launch, scratch.path())
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let names: Vec<_> = verification.checks.iter().map(|c| c.name).collect();
    let mut expected = vec![
        "no loopback TCP",
        "no loopback UDP",
        "no access to the Core's files",
    ];
    if isolation::home_dir().is_some() {
        expected.push("no access to the user's home");
    }
    expected.push("no child processes");
    assert_eq!(names, expected);
    assert!(verification.checks.iter().all(|c| c.passed));
    // The canary is gone again.
    assert_eq!(std::fs::read_dir(scratch.path()).unwrap().count(), 0);
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_missing_boundary_is_detected() {
    let scratch = tempfile::tempdir().unwrap();
    let mut launch = isolation::launch(&worker(), &[]).unwrap();
    // A grant that exposes the Core's data, and network allowed: the probe
    // can read the canary and reach the Core's listener, and the check
    // must say so.
    launch.confinement = launch
        .confinement
        .grant(jarvis_sandbox::Grant::read(scratch.path()));
    launch.confinement.deny_network = false;
    match isolation::verify(&launch, scratch.path()).await {
        Err(IsolationError::NotEnforced(checks)) => {
            let failed: Vec<_> = checks
                .iter()
                .filter(|c| !c.passed)
                .map(|c| c.name)
                .collect();
            assert_eq!(
                failed,
                [
                    "no loopback TCP",
                    "no loopback UDP",
                    "no access to the Core's files"
                ]
            );
        }
        other => panic!("expected the check to fail, got {other:?}"),
    }
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread")]
async fn a_worker_with_network_access_is_refused() {
    // An AppContainer worker always runs without network; asking for
    // network access is refused rather than silently ignored.
    let scratch = tempfile::tempdir().unwrap();
    let mut launch = isolation::launch(&worker(), &[]).unwrap();
    launch.confinement.deny_network = false;
    assert!(matches!(
        isolation::verify(&launch, scratch.path()).await,
        Err(IsolationError::Spawn(_))
    ));
}

#[test]
fn concurrent_cores_each_start_their_worker_in_isolation() {
    // Separate Core processes share one AppContainer profile on Windows; a
    // launch in one must not fail or undo a launch in another.
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    use std::time::Instant;

    let core = |config: &Path, args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_jarvis-core"));
        command.args(args).arg("--config").arg(config);
        command.stdin(Stdio::null());
        command
    };
    let cores: Vec<_> = (0..8)
        .map(|n| {
            let dir = tempfile::tempdir().unwrap();
            let config = dir.path().join("jarvis.toml");
            let text = format!(
                "database = {database:?}\n\n[worker]\nprogram = {python:?}\n\
                 args = [\"-m\", \"jarvis_worker\"]\ncwd = {cwd:?}\n\n\
                 [fixtures]\nroot = {fixtures:?}\n\n[workspace]\nroot = {workspace:?}\n\n\
                 [rpc]\nsocket = {socket:?}\npipe = \"jarvis-concurrent-{id}-{n}\"\n\
                 allow_shutdown = true\n\n[policy]\n\"system.info\" = \"allow\"\n",
                database = dir.path().join("jarvis.db"),
                python = python(),
                cwd = repo().join("services/intelligence-python/src"),
                fixtures = repo().join("tests/fixtures/files"),
                workspace = dir.path().join("workspace"),
                socket = dir.path().join("rpc").join("jarvis.sock"),
                id = std::process::id(),
            );
            std::fs::write(&config, text).unwrap();
            let log = std::fs::File::create(dir.path().join("core.log")).unwrap();
            let child = core(&config, &["serve"])
                .stdout(Stdio::piped())
                .stderr(log)
                .spawn()
                .unwrap();
            (dir, config, child)
        })
        .collect();
    let mut failures = Vec::new();
    for (dir, config, mut child) in cores {
        let mut ready = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut ready)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        let worker = loop {
            let output = core(&config, &["health", "--json"]).output().unwrap();
            let health: serde_json::Value =
                serde_json::from_slice(&output.stdout).unwrap_or_default();
            let worker = health["worker"].clone();
            if worker["state"] == "idle" || worker["state"] == "failed" || Instant::now() > deadline
            {
                break worker;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        if !ready.starts_with("ready ") || worker["state"] != "idle" || worker["restarts"] != 0 {
            let log = std::fs::read_to_string(dir.path().join("core.log")).unwrap_or_default();
            failures.push(format!("{worker}\n{log}"));
        }
        let _ = core(&config, &["shutdown"]).output();
        let _ = child.wait();
    }
    assert!(failures.is_empty(), "{}", failures.join("\n---\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_probe_that_cannot_run_fails_closed() {
    // A program that is not an interpreter produces no report.
    let scratch = tempfile::tempdir().unwrap();
    let mut launch = isolation::launch(&worker(), &[]).unwrap();
    launch.command.program = isolation::resolve_program(if cfg!(windows) { "cmd" } else { "true" })
        .expect("a basic system program");
    let result = isolation::verify(&launch, scratch.path()).await;
    assert!(
        matches!(
            result,
            Err(IsolationError::Probe(_) | IsolationError::Spawn(_))
        ),
        "{result:?}"
    );
}
