//! The gaming governor (ADR 0016): the loop that reads the process list and
//! decides, and the gate that sessions read before they claim a job.
//!
//! The loop tests use a scripted source and Tokio's paused clock, so the
//! timing assertions are exact. The gate tests drive a real session over
//! in-memory pipes, as `resume.rs` does, against a hub whose governor status
//! the test sets directly or through a real `Governor`.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jarvis_core::config::{GovernorConfig, Limits};
use jarvis_core::gateway::Gateway;
use jarvis_core::governor::{
    ActivityError, ActivitySource, Governor, Reading, ReadingFuture, Status,
};
use jarvis_core::hub::Hub;
use jarvis_core::policy::Policy;
use jarvis_core::session::{SessionContext, SessionEnd, SessionError, SessionReport, run_session};
use jarvis_core::store::Store;
use jarvis_protocol::{
    AuditEventKind, Capability, CoreMessage, Goal, GovernorPhase, GovernorReason, JobId, JobStatus,
    PolicyDecision, decode_core_message,
};
use jarvis_tools::{FixtureRoot, Toolbox, WorkspaceRoot};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, Lines};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

// --- a scripted source --------------------------------------------------------------------------

/// Answers with the next scripted reading, then repeats the last one. It
/// never goes through `spawn_blocking`, so paused time stays exact.
#[derive(Debug)]
struct Scripted {
    queue: Mutex<VecDeque<Reading>>,
    current: Mutex<Reading>,
    reads: AtomicUsize,
    at: Mutex<Vec<Instant>>,
    /// When set, a read first waits for a permit: the first detection is
    /// still pending until the test adds one.
    gate: Option<Arc<Semaphore>>,
}

impl Scripted {
    fn script(readings: Vec<Reading>) -> Arc<Self> {
        let last = readings.last().cloned().unwrap_or(Ok(None));
        Arc::new(Self {
            queue: Mutex::new(readings.into()),
            current: Mutex::new(last),
            reads: AtomicUsize::new(0),
            at: Mutex::new(Vec::new()),
            gate: None,
        })
    }

    fn blocked(then: Reading) -> (Arc<Self>, Arc<Semaphore>) {
        let gate = Arc::new(Semaphore::new(0));
        let source = Arc::new(Self {
            queue: Mutex::new(VecDeque::from([then.clone()])),
            current: Mutex::new(then),
            reads: AtomicUsize::new(0),
            at: Mutex::new(Vec::new()),
            gate: Some(Arc::clone(&gate)),
        });
        (source, gate)
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

impl ActivitySource for Scripted {
    fn read(&self) -> ReadingFuture<'_> {
        Box::pin(async move {
            if let Some(gate) = &self.gate {
                gate.acquire().await.unwrap().forget();
            }
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.at.lock().unwrap().push(Instant::now());
            let mut current = self.current.lock().unwrap();
            if let Some(next) = self.queue.lock().unwrap().pop_front() {
                *current = next;
            }
            current.clone()
        })
    }
}

fn game() -> Reading {
    Ok(Some("cs2.exe".to_owned()))
}

fn clear() -> Reading {
    Ok(None)
}

fn broken() -> Reading {
    Err(ActivityError("the process list is unavailable".into()))
}

// --- the loop -----------------------------------------------------------------------------------

fn settings(poll_ms: u64, resume_ms: u64, alert_ms: u64) -> GovernorConfig {
    GovernorConfig {
        enabled: true,
        processes: vec!["cs2.exe".to_owned()],
        poll: Duration::from_millis(poll_ms),
        resume_delay: Duration::from_millis(resume_ms),
        alert_after: Duration::from_millis(alert_ms),
    }
}

struct Rig {
    dir: tempfile::TempDir,
    store: Store,
    hub: Arc<Hub>,
    status: watch::Receiver<Status>,
    stop: CancellationToken,
    task: JoinHandle<()>,
    started: Instant,
}

impl Rig {
    fn start(source: Arc<Scripted>, config: &GovernorConfig) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("jarvis.db")).unwrap();
        let hub = Arc::new(Hub::default());
        let governor = Governor::new(config, store.clone(), Arc::clone(&hub), source);
        let status = hub.subscribe_governor();
        let stop = CancellationToken::new();
        let task = tokio::spawn(governor.run(stop.clone()));
        Self {
            dir,
            store,
            hub,
            status,
            stop,
            task,
            started: Instant::now(),
        }
    }

    async fn reaches(&mut self, phase: GovernorPhase) -> Status {
        let wait = self.status.wait_for(|status| status.phase == phase);
        let reached = tokio::time::timeout(Duration::from_secs(36_000), wait).await;
        reached
            .unwrap_or_else(|_| panic!("the governor never reached {phase:?}"))
            .unwrap()
            .clone()
    }

    async fn holds_for(&mut self, reason: GovernorReason) -> Status {
        let wait = self.status.wait_for(|status| status.reason == Some(reason));
        let reached = tokio::time::timeout(Duration::from_secs(36_000), wait).await;
        reached
            .unwrap_or_else(|_| panic!("the governor never held for {reason:?}"))
            .unwrap()
            .clone()
    }

    /// Time since the governor started, in whole milliseconds.
    fn elapsed_ms(&self) -> u128 {
        self.started.elapsed().as_millis()
    }

    fn audit(&self) -> Vec<AuditEventKind> {
        self.store
            .audit_events(None, 1000)
            .unwrap()
            .into_iter()
            .map(|event| event.event)
            .filter(|event| event.name().starts_with("governor_"))
            .collect()
    }

    /// Run SQL on a second connection to the Core's database.
    fn sql(&self, text: &str) {
        let conn = rusqlite::Connection::open(self.dir.path().join("jarvis.db")).unwrap();
        conn.busy_timeout(Duration::from_secs(5)).unwrap();
        conn.execute_batch(text).unwrap();
    }

    async fn stop(self) {
        self.stop.cancel();
        self.task.await.unwrap();
    }
}

fn hold_game() -> AuditEventKind {
    AuditEventKind::GovernorHold {
        reason: GovernorReason::Game,
        process: Some("cs2.exe".to_owned()),
    }
}

fn hold_failed() -> AuditEventKind {
    AuditEventKind::GovernorHold {
        reason: GovernorReason::DetectionFailed,
        process: None,
    }
}

#[tokio::test(start_paused = true)]
async fn until_the_first_reading_the_gate_is_closed_and_nothing_is_recorded() {
    let (source, _gate) = Scripted::blocked(clear());
    let config = settings(1_000, 3_000, 86_400_000);
    let rig = Rig::start(source, &config);
    let status = rig.hub.governor_status();
    assert_eq!(status.phase, GovernorPhase::Held);
    assert_eq!(status.reason, Some(GovernorReason::Starting));
    assert!(!status.is_open());
    assert!(rig.audit().is_empty());
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_clear_first_reading_opens_the_gate_with_no_delay_and_no_event() {
    let config = settings(1_000, 30_000, 86_400_000);
    let mut rig = Rig::start(Scripted::script(vec![clear()]), &config);
    rig.reaches(GovernorPhase::Open).await;
    assert_eq!(rig.elapsed_ms(), 0, "no resume delay at start-up");
    assert!(
        rig.audit().is_empty(),
        "nothing was held, so nothing to record"
    );
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn the_gate_opens_after_exactly_the_resume_delay_and_the_release_is_recorded_first() {
    let config = settings(1_000, 3_000, 86_400_000);
    let source = Scripted::script(vec![game(), game(), clear()]);
    let mut rig = Rig::start(Arc::clone(&source), &config);

    let held = rig.holds_for(GovernorReason::Game).await;
    assert_eq!(held.process.as_deref(), Some("cs2.exe"));
    assert_eq!(rig.audit(), [hold_game()]);

    // The game is gone at the third reading (2 s). The delay runs from there.
    let releasing = rig.reaches(GovernorPhase::Releasing).await;
    assert!(!releasing.is_open());
    assert_eq!(rig.elapsed_ms(), 2_000);

    rig.reaches(GovernorPhase::Open).await;
    assert_eq!(rig.elapsed_ms(), 5_000, "2 s plus the 3 s delay");
    // The gate is open, so the event that says so must already be written.
    assert_eq!(
        rig.audit(),
        [
            hold_game(),
            AuditEventKind::GovernorReleased { held_ms: 5_000 }
        ]
    );
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_game_that_comes_back_during_the_delay_restarts_it() {
    let config = settings(1_000, 3_000, 86_400_000);
    // game, clear, clear, game, clear, clear, clear, clear ...
    let source = Scripted::script(vec![game(), clear(), clear(), game(), clear()]);
    let mut rig = Rig::start(source, &config);
    rig.reaches(GovernorPhase::Releasing).await;
    rig.reaches(GovernorPhase::Held).await;
    assert_eq!(rig.elapsed_ms(), 3_000);
    rig.reaches(GovernorPhase::Releasing).await;
    assert_eq!(rig.elapsed_ms(), 4_000);
    rig.reaches(GovernorPhase::Open).await;
    assert_eq!(rig.elapsed_ms(), 7_000);
    assert_eq!(
        rig.audit(),
        [
            hold_game(),
            AuditEventKind::GovernorReleased { held_ms: 7_000 }
        ],
        "one hold, one release: the interruption is not a new hold"
    );
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_long_hold_raises_one_alert_and_is_never_released_by_time() {
    let config = settings(1_000, 3_000, 5_000);
    let source = Scripted::script(vec![game()]);
    let mut rig = Rig::start(Arc::clone(&source), &config);

    let wait = rig.status.wait_for(|status| status.overdue);
    let status = tokio::time::timeout(Duration::from_secs(36_000), wait)
        .await
        .expect("the alert never came")
        .unwrap()
        .clone();
    assert_eq!(status.phase, GovernorPhase::Held);
    assert_eq!(rig.elapsed_ms(), 5_000);
    assert_eq!(
        rig.audit(),
        [
            hold_game(),
            AuditEventKind::GovernorHoldOverdue { held_ms: 5_000 }
        ]
    );

    // A long time later: still held, still one alert.
    tokio::time::sleep(Duration::from_secs(3_600)).await;
    assert_eq!(rig.hub.governor_status().phase, GovernorPhase::Held);
    assert!(!rig.hub.governor_status().is_open());
    assert_eq!(rig.audit().len(), 2);
    let view = rig.hub.governor_status().view(Instant::now());
    assert!(view.overdue);
    assert!(view.held_ms.unwrap() >= 3_600_000);
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_failed_detection_holds_and_a_streak_is_recorded_once() {
    let config = settings(1_000, 3_000, 86_400_000);
    let source = Scripted::script(vec![broken(), broken(), broken(), broken(), clear()]);
    let mut rig = Rig::start(Arc::clone(&source), &config);

    let held = rig.holds_for(GovernorReason::DetectionFailed).await;
    assert_eq!(held.process, None);
    rig.reaches(GovernorPhase::Releasing).await;
    assert_eq!(source.reads(), 5, "the streak lasted four readings");
    assert_eq!(
        rig.audit(),
        [hold_failed()],
        "one event for the whole streak"
    );
    rig.reaches(GovernorPhase::Open).await;
    assert_eq!(
        rig.audit(),
        [
            hold_failed(),
            AuditEventKind::GovernorReleased { held_ms: 7_000 }
        ]
    );
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn if_the_release_cannot_be_recorded_the_gate_stays_closed_until_it_can() {
    let config = settings(1_000, 2_000, 86_400_000);
    let mut rig = Rig::start(Scripted::script(vec![game(), clear()]), &config);
    rig.sql(
        "CREATE TRIGGER no_release BEFORE INSERT ON audit_events
         WHEN NEW.kind = 'governor_released'
         BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END;",
    );
    rig.reaches(GovernorPhase::Releasing).await;

    // Many readings later the delay is long over, but the gate is shut.
    tokio::time::sleep(Duration::from_secs(30)).await;
    let status = rig.hub.governor_status();
    assert_eq!(status.phase, GovernorPhase::Releasing);
    assert!(!status.is_open());
    assert_eq!(rig.audit(), [hold_game()]);

    rig.sql("DROP TRIGGER no_release");
    rig.reaches(GovernorPhase::Open).await;
    let audit = rig.audit();
    assert_eq!(audit.len(), 2);
    assert!(matches!(
        audit[1],
        AuditEventKind::GovernorReleased { held_ms } if held_ms >= 30_000
    ));
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn a_hold_applies_even_if_it_cannot_be_recorded() {
    let config = settings(1_000, 2_000, 86_400_000);
    let mut rig = Rig::start(Scripted::script(vec![clear(), game()]), &config);
    rig.sql(
        "CREATE TRIGGER no_hold BEFORE INSERT ON audit_events
         WHEN NEW.kind = 'governor_hold'
         BEGIN SELECT RAISE(ABORT, 'audit unavailable'); END;",
    );
    rig.reaches(GovernorPhase::Open).await;
    let held = rig.reaches(GovernorPhase::Held).await;
    assert_eq!(held.reason, Some(GovernorReason::Game));
    assert!(!rig.hub.governor_status().is_open(), "jobs are held anyway");
    assert!(rig.audit().is_empty());
    rig.stop().await;
}

#[tokio::test(start_paused = true)]
async fn the_task_ends_when_the_core_shuts_down() {
    let config = settings(1_000, 2_000, 86_400_000);
    let mut rig = Rig::start(Scripted::script(vec![game()]), &config);
    rig.reaches(GovernorPhase::Held).await;
    tokio::time::timeout(Duration::from_secs(60), rig.stop())
        .await
        .expect("the governor did not stop");
}

// --- the gate -----------------------------------------------------------------------------------

struct Env {
    dir: tempfile::TempDir,
    store: Store,
    hub: Arc<Hub>,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("fixtures")).unwrap();
        std::fs::create_dir_all(dir.path().join("workspace")).unwrap();
        let store = Store::open(&dir.path().join("jarvis.db")).unwrap();
        Self {
            dir,
            store,
            hub: Arc::new(Hub::default()),
        }
    }

    fn submit(&self, goal: &str) -> JobId {
        let goal = Goal::try_from(goal.to_owned()).unwrap();
        self.store.submit_job(&goal, "test client").unwrap().job_id
    }

    fn job(&self, job: JobId) -> (JobStatus, u32) {
        let record = self.store.job(job).unwrap().unwrap();
        (record.status, record.attempts)
    }

    /// Hold jobs for a game, as the governor would.
    fn hold(&self) {
        let origin = Instant::now();
        let state = jarvis_core::governor::machine::State::Held {
            since: 0,
            hold: jarvis_core::governor::machine::Hold::Game("cs2.exe".into()),
            overdue: false,
        };
        self.hub.set_governor(Status::of(&state, origin));
    }

    fn open(&self) {
        let state = jarvis_core::governor::machine::State::Open;
        self.hub.set_governor(Status::of(&state, Instant::now()));
    }

    /// A session on this database and hub, with the handshake done.
    async fn session(&self) -> Harness {
        self.session_with_timeout(Duration::from_secs(120)).await
    }

    async fn session_with_timeout(&self, job_timeout: Duration) -> Harness {
        let limits = Limits {
            tool_timeout: Duration::from_secs(10),
            ..Limits::default()
        };
        let toolbox = Toolbox::new(
            "test",
            FixtureRoot::open(&self.dir.path().join("fixtures"), 1024).unwrap(),
            WorkspaceRoot::open(&self.dir.path().join("workspace"), 1024).unwrap(),
        );
        let policy: BTreeMap<_, _> = [(Capability::SystemInfo, PolicyDecision::Allow)]
            .into_iter()
            .collect();
        let ctx = SessionContext {
            store: self.store.clone(),
            policy: Policy::new(policy),
            gateway: Gateway::new(
                Arc::new(toolbox),
                limits.tool_timeout,
                limits.max_result_bytes(),
            ),
            limits,
            handshake_timeout: Duration::from_secs(10),
            approval_ttl: Duration::from_secs(60),
            job_timeout,
            core_version: "test",
            hub: Arc::clone(&self.hub),
        };
        let (to_core, core_reader) = tokio::io::duplex(1 << 20);
        let (core_writer, from_core) = tokio::io::duplex(1 << 20);
        let shutdown = CancellationToken::new();
        let token = shutdown.clone();
        let session =
            tokio::spawn(async move { run_session(&ctx, core_reader, core_writer, &token).await });
        let mut harness = Harness {
            shutdown,
            to_core: Some(to_core),
            from_core: BufReader::new(from_core).lines(),
            session,
        };
        harness
            .send(json!({"protocol": 2, "type": "hello", "worker": "test", "worker_version": "1"}))
            .await;
        assert!(matches!(harness.recv().await, CoreMessage::Welcome(_)));
        harness
    }
}

struct Harness {
    shutdown: CancellationToken,
    to_core: Option<DuplexStream>,
    from_core: Lines<BufReader<DuplexStream>>,
    session: JoinHandle<Result<SessionReport, SessionError>>,
}

impl Harness {
    async fn send(&mut self, value: Value) {
        let pipe = self.to_core.as_mut().unwrap();
        pipe.write_all(&serde_json::to_vec(&value).unwrap())
            .await
            .unwrap();
        pipe.write_all(b"\n").await.unwrap();
    }

    async fn recv(&mut self) -> CoreMessage {
        let line = self
            .from_core
            .next_line()
            .await
            .unwrap()
            .expect("the Core closed the stream");
        decode_core_message(line.as_bytes()).expect("the Core sent an invalid frame")
    }

    /// Whether the Core sends anything within `wait`.
    async fn stays_quiet_for(&mut self, wait: Duration) -> bool {
        tokio::time::timeout(wait, self.from_core.next_line())
            .await
            .is_err()
    }

    async fn takes(&mut self, job: JobId) {
        let message = tokio::time::timeout(Duration::from_secs(10), self.recv())
            .await
            .expect("the job was never handed out");
        match message {
            CoreMessage::Job(assigned) => assert_eq!(assigned.job_id, job),
            other => panic!("expected a job, got {other:?}"),
        }
    }

    async fn completes(&mut self, job: JobId) {
        self.send(json!({"protocol": 2, "type": "job_result", "job_id": job,
                         "outcome": "completed", "summary": "done"}))
            .await;
    }

    async fn worker_exits(mut self) -> SessionReport {
        self.to_core.take();
        self.ended().await
    }

    async fn ended(self) -> SessionReport {
        tokio::time::timeout(Duration::from_secs(10), self.session)
            .await
            .expect("the session did not end")
            .unwrap()
            .unwrap()
    }
}

const QUIET: Duration = Duration::from_millis(300);

#[tokio::test]
async fn while_held_a_queued_job_is_not_claimed() {
    let env = Env::new();
    env.hold();
    let job = env.submit("wait for the game");
    let mut h = env.session().await;

    assert!(h.stays_quiet_for(QUIET).await, "no job reached the worker");
    assert_eq!(
        env.job(job),
        (JobStatus::Queued, 0),
        "no attempt was counted"
    );
    let started = env
        .store
        .audit_events(None, 100)
        .unwrap()
        .into_iter()
        .any(|event| event.event.name() == "job_started");
    assert!(!started);

    // A wake-up for a new job does not get past the gate either.
    env.hub.jobs.notify_one();
    assert!(h.stays_quiet_for(QUIET).await);
    assert_eq!(env.job(job), (JobStatus::Queued, 0));

    h.worker_exits().await;
}

#[tokio::test]
async fn releasing_the_gate_wakes_an_idle_session_by_itself() {
    let env = Env::new();
    env.hold();
    // Queued through the store: nothing tells the session about it.
    let job = env.submit("wait for the game");
    let mut h = env.session().await;
    assert!(h.stays_quiet_for(QUIET).await);

    env.open();
    h.takes(job).await;
    assert_eq!(env.job(job), (JobStatus::Running, 1));
    h.completes(job).await;
    let report = h.worker_exits().await;
    assert_eq!(report.end, SessionEnd::WorkerClosed);
    assert_eq!(env.job(job).0, JobStatus::Completed);
}

#[tokio::test(start_paused = true)]
async fn a_hold_longer_than_the_job_timeout_does_not_use_up_the_job() {
    let env = Env::new();
    env.hold();
    let job = env.submit("wait for the game");
    let mut h = env.session_with_timeout(Duration::from_secs(120)).await;

    // Five job timeouts pass while the game runs.
    tokio::time::sleep(Duration::from_secs(600)).await;
    assert_eq!(env.job(job), (JobStatus::Queued, 0));

    env.open();
    h.takes(job).await;
    // The clock for the job starts now, not when it was queued or held.
    tokio::time::sleep(Duration::from_secs(100)).await;
    h.completes(job).await;
    let report = h.worker_exits().await;
    assert_eq!(report.end, SessionEnd::WorkerClosed, "not JobTimedOut");
    assert_eq!(env.job(job), (JobStatus::Completed, 1));
}

#[tokio::test]
async fn a_requeued_job_is_held_too() {
    let env = Env::new();
    env.open();
    let job = env.submit("two attempts");
    let mut first = env.session().await;
    first.takes(job).await;
    first.worker_exits().await;
    assert_eq!(
        env.job(job),
        (JobStatus::Queued, 1),
        "requeued after the worker left"
    );

    env.hold();
    let mut second = env.session().await;
    assert!(second.stays_quiet_for(QUIET).await);
    assert_eq!(env.job(job), (JobStatus::Queued, 1));

    env.open();
    second.takes(job).await;
    assert_eq!(env.job(job), (JobStatus::Running, 2));
    second.completes(job).await;
    second.worker_exits().await;
}

#[tokio::test]
async fn a_hold_that_starts_while_a_job_runs_leaves_it_running() {
    let env = Env::new();
    env.open();
    let job = env.submit("already under way");
    let next = env.submit("queued behind it");
    let mut h = env.session().await;
    h.takes(job).await;

    env.hold();
    h.send(
        json!({"protocol": 2, "type": "tool_request", "request_id": "r1",
                  "job_id": job, "tool": "system.info", "args": {}}),
    )
    .await;
    match h.recv().await {
        CoreMessage::ToolResponse(response) => {
            assert_eq!(response.request_id.as_str(), "r1");
        }
        other => panic!("expected a tool_response, got {other:?}"),
    }
    h.completes(job).await;
    assert!(h.stays_quiet_for(QUIET).await, "the next job waits");
    assert_eq!(env.job(job).0, JobStatus::Completed);
    assert_eq!(env.job(next), (JobStatus::Queued, 0));
    h.worker_exits().await;
}

#[tokio::test]
async fn a_disabled_governor_never_holds_anything() {
    let env = Env::new();
    assert_eq!(env.hub.governor_status(), Status::OFF);
    let job = env.submit("no governor");
    let mut h = env.session().await;
    h.takes(job).await;
    h.completes(job).await;
    h.worker_exits().await;
}

#[tokio::test]
async fn no_job_starts_before_the_first_detection() {
    let env = Env::new();
    let job = env.submit("queued before the Core restarted");
    let (source, first_reading) = Scripted::blocked(clear());
    let config = settings(10, 0, 86_400_000);
    let governor = Governor::new(
        &config,
        env.store.clone(),
        Arc::clone(&env.hub),
        source.clone(),
    );
    let stop = CancellationToken::new();
    let task = tokio::spawn(governor.run(stop.clone()));
    let mut h = env.session().await;

    assert!(
        h.stays_quiet_for(QUIET).await,
        "the first detection is pending"
    );
    assert_eq!(source.reads(), 0);
    assert_eq!(env.job(job), (JobStatus::Queued, 0));

    first_reading.add_permits(1);
    h.takes(job).await;
    assert_eq!(env.job(job), (JobStatus::Running, 1));
    h.completes(job).await;
    stop.cancel();
    task.await.unwrap();
    h.worker_exits().await;
}

#[tokio::test]
async fn a_game_that_is_running_at_start_up_holds_a_job_queued_before_the_restart() {
    let env = Env::new();
    let job = env.submit("queued before the Core restarted");
    let source = Scripted::script(vec![game()]);
    let config = settings(10, 20, 86_400_000);
    let governor = Governor::new(&config, env.store.clone(), Arc::clone(&env.hub), source);
    let stop = CancellationToken::new();
    let task = tokio::spawn(governor.run(stop.clone()));
    let mut h = env.session().await;
    assert!(h.stays_quiet_for(QUIET).await);
    let status = env.hub.governor_status();
    assert_eq!(status.phase, GovernorPhase::Held);
    assert_eq!(status.reason, Some(GovernorReason::Game));
    assert_eq!(env.job(job), (JobStatus::Queued, 0));
    stop.cancel();
    task.await.unwrap();
    h.worker_exits().await;
}

#[tokio::test]
async fn a_held_job_survives_a_restart_unchanged() {
    let env = Env::new();
    env.hold();
    let job = env.submit("wait for the game");
    let h = env.session().await;
    h.shutdown.cancel();
    let report = h.ended().await;
    assert_eq!(report.end, SessionEnd::Shutdown);

    // What the Core does when it starts again.
    env.store.call(|s| s.recover(3)).await.unwrap();
    assert_eq!(env.job(job), (JobStatus::Queued, 0));
}
