//! Durable jobs end to end: the real `jarvis-core` binary, real worker
//! processes and a database that outlives them. A job whose Core or worker
//! stopped is resumed without repeating a recorded step, and a step that was
//! only waiting for a person asks again. They need a Python 3.11+ interpreter,
//! named by `JARVIS_TEST_PYTHON` or found as `python3` (`python` on Windows).

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use jarvis_core::store::Store;
use jarvis_protocol::{ApprovalStatus, AuditEvent, AuditEventKind, TaskId};
use serde_json::Value;

const CORE: &str = env!("CARGO_BIN_EXE_jarvis-core");
const WAIT: Duration = Duration::from_secs(30);

/// Repository root, without `..` components: Windows child processes get it
/// as their working directory.
fn repo() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .expect("crate lives two levels below the repository root")
        .to_path_buf()
}

fn python() -> String {
    std::env::var("JARVIS_TEST_PYTHON")
        .unwrap_or_else(|_| if cfg!(windows) { "python" } else { "python3" }.to_owned())
}

const POLICY: &str = r#""system.info" = "allow"
"filesystem.read.fixture" = "allow"
"workspace.write" = "require_confirmation""#;

struct Setup {
    dir: tempfile::TempDir,
    config: PathBuf,
}

impl Setup {
    /// Write a config for `worker` (`"real"` or a rogue mode).
    fn new(worker: &str, policy: &str, extra: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("rpc").join("jarvis.sock");
        let pipe = format!("jarvis-test-{}", TaskId::new());
        let endpoint = if cfg!(windows) {
            format!(r"\\.\pipe\{pipe}")
        } else {
            socket.display().to_string()
        };
        let (cwd, args) = if worker == "real" {
            (
                repo().join("services/intelligence-python/src"),
                r#"["-m", "jarvis_worker"]"#.to_owned(),
            )
        } else {
            (
                repo().join("crates/jarvis-core/tests/workers"),
                format!(r#"["rogue_worker.py", {worker:?}, {endpoint:?}]"#),
            )
        };
        let text = format!(
            r#"
database = {database:?}

[worker]
program = {python:?}
args = {args}
cwd = {cwd:?}

[fixtures]
root = {fixtures:?}

[workspace]
root = {workspace:?}

[limits]
shutdown_grace_ms = 3000

[supervisor]
restart_budget = 3
backoff_initial_ms = 50
backoff_max_ms = 200

[rpc]
socket = {socket:?}
pipe = {pipe:?}
allow_shutdown = true

[policy]
{policy}

{extra}
"#,
            database = dir.path().join("jarvis.db"),
            python = python(),
            fixtures = repo().join("tests/fixtures/files"),
            workspace = dir.path().join("workspace"),
        );
        let config = dir.path().join("jarvis.toml");
        std::fs::write(&config, text).unwrap();
        Self { dir, config }
    }

    fn cli(&self, args: &[&str]) -> Command {
        let mut command = Command::new(CORE);
        command
            .args(args)
            .arg("--config")
            .arg(&self.config)
            .stdin(Stdio::null());
        command
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cli(args).output().unwrap()
    }

    /// Run a client command that prints JSON lines. Exit code 2 (a negative
    /// answer, such as a failed job) still prints the answer.
    fn json(&self, args: &[&str]) -> Vec<Value> {
        let output = self.cli(args).arg("--json").output().unwrap();
        assert!(
            matches!(output.status.code(), Some(0 | 2)),
            "{args:?}: {}",
            stderr(&output)
        );
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn database(&self) -> PathBuf {
        self.dir.path().join("jarvis.db")
    }

    fn workspace_file(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.path().join("workspace").join(name)).ok()
    }

    fn store(&self) -> Store {
        Store::open_read_only(&self.database()).unwrap()
    }

    fn events(&self) -> Vec<AuditEvent> {
        self.store().audit_events(None, 10_000).unwrap()
    }

    fn count(&self, kind: &str) -> usize {
        self.events()
            .into_iter()
            .filter(|event| event.event.name() == kind)
            .count()
    }

    /// Start `jarvis-core serve` and wait for its ready line.
    fn serve(&self) -> Daemon {
        let log =
            std::fs::File::create(self.dir.path().join(format!("core-{}.log", TaskId::new())))
                .unwrap();
        let mut child = self
            .cli(&["serve"])
            .env("JARVIS_TEST_SECRET", "must-not-reach-the-worker")
            .stdout(Stdio::piped())
            .stderr(log)
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (lines, ready) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if lines.send(line).is_err() {
                    return;
                }
            }
        });
        match ready.recv_timeout(WAIT) {
            Ok(line) => assert!(line.starts_with("ready "), "unexpected first line {line:?}"),
            Err(_) => {
                let _ = child.kill();
                panic!("the Core did not become ready: {}", self.log());
            }
        }
        Daemon { child }
    }

    fn log(&self) -> String {
        let mut text = String::new();
        for entry in std::fs::read_dir(self.dir.path()).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|ext| ext == "log") {
                text.push_str(&std::fs::read_to_string(path).unwrap_or_default());
            }
        }
        text
    }

    /// Wait until `ready` holds for the audit log.
    fn wait_for(&self, what: &str, ready: impl Fn(&[AuditEvent]) -> bool) {
        let deadline = Instant::now() + WAIT;
        loop {
            if Store::open_read_only(&self.database())
                .ok()
                .and_then(|store| store.audit_events(None, 10_000).ok())
                .is_some_and(|events| ready(&events))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}: {}",
                self.log()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn wait_for_kind(&self, kind: &str, times: usize) {
        self.wait_for(kind, |events| {
            events.iter().filter(|e| e.event.name() == kind).count() >= times
        });
    }

    fn submit(&self, goal: &str) -> String {
        let job = self.json(&["submit", goal]).remove(0);
        assert_eq!(job["status"], "queued");
        job["job_id"].as_str().unwrap().to_owned()
    }

    fn pending_approval(&self) -> Value {
        let approvals = self.json(&["approvals", "list", "--wait", "30s"]);
        assert_eq!(approvals.len(), 1, "{approvals:?}");
        approvals.into_iter().next().unwrap()
    }

    fn shutdown(&self, mut daemon: Daemon) -> ExitStatus {
        let output = self.run(&["shutdown"]);
        assert!(output.status.success(), "{}", stderr(&output));
        daemon.wait()
    }
}

struct Daemon {
    child: Child,
}

impl Daemon {
    fn wait(&mut self) -> ExitStatus {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                panic!("the Core did not exit");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Kill the Core without letting it clean up, as a crash or power loss
    /// would.
    fn crash(&mut self) {
        self.child.kill().unwrap();
        self.child.wait().unwrap();
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The Core is killed while step 2, a write, waits for a person. After the
/// restart the job comes back, step 1 is answered from its stored result, and
/// step 2 asks again with a new approval.
#[test]
fn a_job_resumes_after_a_core_crash_without_repeating_a_completed_step() {
    let setup = Setup::new("real", POLICY, "");
    let mut daemon = setup.serve();
    let job = setup.submit("system info, then write b.txt: hello");
    let first = setup.pending_approval();
    let first_id = first["approval_id"].as_str().unwrap().to_owned();
    assert_eq!(setup.count("execution_started"), 1, "system.info ran");
    let completed = setup
        .store()
        .tasks(10)
        .unwrap()
        .into_iter()
        .find(|task| task.tool == "system.info")
        .unwrap();
    daemon.crash();

    let daemon = setup.serve();
    let second = setup.pending_approval();
    let second_id = second["approval_id"].as_str().unwrap().to_owned();
    assert_ne!(second_id, first_id, "a new approval, not the old one");
    assert_ne!(second["fingerprint"], first["fingerprint"]);
    assert_eq!(second["arguments"]["path"], "b.txt");
    assert_eq!(setup.count("job_requeued"), 1);
    assert_eq!(setup.count("step_replayed"), 1);
    assert_eq!(
        setup.count("execution_started"),
        1,
        "system.info did not run again"
    );
    let shown = setup.json(&["approvals", "show", &first_id]).remove(0);
    assert_eq!(shown["status"], "expired");
    let output = setup.run(&["approvals", "approve", &first_id, "--yes"]);
    assert_eq!(output.status.code(), Some(1), "the old approval is dead");
    assert_eq!(setup.workspace_file("b.txt"), None);

    let fingerprint = second["fingerprint"].as_str().unwrap();
    let output = setup.run(&[
        "approvals",
        "approve",
        &second_id,
        "--yes",
        "--fingerprint",
        fingerprint,
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let done = setup.json(&["job", &job, "--wait"]).remove(0);
    assert_eq!(done["status"], "completed", "{done}");
    assert_eq!(setup.workspace_file("b.txt").as_deref(), Some("hello"));
    assert!(setup.shutdown(daemon).success());

    assert_eq!(setup.count("execution_started"), 2, "one per tool, no more");
    assert_eq!(setup.count("approval_consumed"), 1);
    let replayed: Vec<_> = setup
        .events()
        .into_iter()
        .filter_map(|event| match event.event {
            AuditEventKind::StepReplayed { seq, task_id, .. } => Some((seq, task_id)),
            _ => None,
        })
        .collect();
    assert_eq!(replayed, [(1, completed.task_id)]);
    let old = setup
        .store()
        .approval(first_id.parse().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(old.status, ApprovalStatus::Expired);
}

/// A worker that dies right after step 1 is answered, on every attempt. Each
/// attempt replays step 1, so the tool ran once, and the job ends when its
/// attempts are spent.
#[test]
fn a_job_whose_worker_keeps_dying_replays_its_first_step_until_the_attempts_are_spent() {
    let setup = Setup::new("die-after-first-response", POLICY, "");
    let daemon = setup.serve();
    let job = setup.submit("anything");
    setup.wait_for_kind("job_finished", 1);
    let job = setup.json(&["job", &job]).remove(0);
    assert_eq!(job["status"], "interrupted");
    assert!(job["summary"].as_str().unwrap().contains("attempt limit"));
    assert_eq!(setup.count("job_started"), 3);
    assert_eq!(setup.count("job_requeued"), 2);
    assert_eq!(setup.count("execution_started"), 1, "the tool ran once");
    assert_eq!(setup.count("step_replayed"), 2, "attempts 2 and 3 replayed");
    assert_eq!(setup.store().tasks(10).unwrap().len(), 1);
    drop(daemon);
}
