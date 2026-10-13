use jarvis_protocol::{
    ApprovalId, ApprovalStatus, AuditEventKind, ErrorCode, Fingerprint, Goal, JobStatus,
    PolicyDecision, Summary, TaskStatus, WireError,
};
use serde_json::json;

use super::*;

fn new_task(session_id: SessionId, request_id: &str) -> NewTask {
    NewTask {
        task_id: TaskId::new(),
        session_id,
        job_id: None,
        request_id: RequestId::try_from(request_id.to_owned()).unwrap(),
        tool: ToolName::try_from("system.info".to_owned()).unwrap(),
        args: json!({}),
        step: None,
    }
}

const MAX_ATTEMPTS: u32 = 3;

fn open(dir: &tempfile::TempDir) -> Store {
    Store::open(&dir.path().join("jarvis.db")).unwrap()
}

#[test]
fn migrations_are_recorded_and_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    assert_eq!(store.schema_version().unwrap(), latest_version());
    drop(store);
    let store = open(&dir);
    assert_eq!(store.schema_version().unwrap(), latest_version());
    let rows: i64 = store
        .conn()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, i64::try_from(MIGRATIONS.len()).unwrap());
}

#[test]
fn completed_task_and_audit_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let session = SessionId::new();
    let task = new_task(session, "r1");
    {
        let store = open(&dir);
        store.create_task(&task).unwrap();
        store
            .transition(
                task.task_id,
                &[TaskStatus::Received],
                &TaskChange {
                    decision: Some(PolicyDecision::Allow),
                    capabilities: Some(vec![Capability::SystemInfo]),
                    ..TaskChange::to(TaskStatus::Executing)
                },
                &[AuditEventKind::ExecutionStarted {
                    tool: "system.info".into(),
                }],
            )
            .unwrap();
        store
            .transition(
                task.task_id,
                &[TaskStatus::Executing],
                &TaskChange {
                    result: Some(r#"{"ok":true}"#.into()),
                    ..TaskChange::to(TaskStatus::Completed)
                },
                &[],
            )
            .unwrap();
    }

    let store = open(&dir);
    let record = store.task(task.task_id).unwrap().unwrap();
    assert_eq!(record.status, TaskStatus::Completed);
    assert_eq!(record.decision, Some(PolicyDecision::Allow));
    assert_eq!(record.capabilities, Some(vec![Capability::SystemInfo]));
    assert_eq!(record.result.as_deref(), Some(r#"{"ok":true}"#));

    let kinds: Vec<_> = store
        .audit_events(Some(task.task_id), 100)
        .unwrap()
        .into_iter()
        .map(|event| event.event.name())
        .collect();
    assert_eq!(kinds, ["request_received", "execution_started"]);
}

#[test]
fn duplicate_request_id_in_a_session_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let session = SessionId::new();
    store.create_task(&new_task(session, "r1")).unwrap();
    assert!(matches!(
        store.create_task(&new_task(session, "r1")),
        Err(StoreError::DuplicateRequest(_))
    ));
    // The same request ID in another session is a different request.
    store
        .create_task(&new_task(SessionId::new(), "r1"))
        .unwrap();
}

#[test]
fn transitions_from_unexpected_states_change_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let task = new_task(SessionId::new(), "r1");
    store.create_task(&task).unwrap();
    let error = store
        .transition(
            task.task_id,
            &[TaskStatus::Executing],
            &TaskChange::to(TaskStatus::Completed),
            &[AuditEventKind::ExecutionStarted { tool: "x".into() }],
        )
        .unwrap_err();
    assert!(matches!(error, StoreError::IllegalTransition { .. }));
    assert_eq!(
        store.task(task.task_id).unwrap().unwrap().status,
        TaskStatus::Received
    );
    // The audit event in the failed transaction was rolled back too.
    assert_eq!(store.audit_events(Some(task.task_id), 10).unwrap().len(), 1);
}

#[test]
fn terminal_tasks_cannot_be_changed_even_by_direct_sql() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let task = new_task(SessionId::new(), "r1");
    store.create_task(&task).unwrap();
    let failed = TaskChange {
        error: Some(WireError::new(ErrorCode::ToolFailed, "boom")),
        ..TaskChange::to(TaskStatus::Failed)
    };
    store
        .transition(task.task_id, &[TaskStatus::Received], &failed, &[])
        .unwrap();

    assert!(matches!(
        store.transition(
            task.task_id,
            &[TaskStatus::Failed],
            &TaskChange::to(TaskStatus::Completed),
            &[]
        ),
        Err(StoreError::IllegalTransition { .. })
    ));
    let conn = store.conn().unwrap();
    let error = conn
        .execute("UPDATE tasks SET status = 'completed'", [])
        .unwrap_err();
    assert!(error.to_string().contains("terminal tasks are immutable"));
    let error = conn.execute("DELETE FROM tasks", []).unwrap_err();
    assert!(error.to_string().contains("never deleted"));
    // REPLACE deletes the old row first; recursive triggers make that visible.
    let error = conn
        .execute(
            "INSERT OR REPLACE INTO tasks
             (task_id, session_id, request_id, tool, args, status, created_at, updated_at)
             VALUES (?1, ?2, 'r1', 'shell.exec', '{}', 'received', 'now', 'now')",
            params![task.task_id.to_string(), task.session_id.to_string()],
        )
        .unwrap_err();
    assert!(error.to_string().contains("never deleted"), "{error}");
}

#[test]
fn audit_log_is_append_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    store
        .append(
            None,
            None,
            &AuditEventKind::CoreStopped {
                reason: "test".into(),
            },
        )
        .unwrap();
    let conn = store.conn().unwrap();
    for sql in [
        "UPDATE audit_events SET kind = 'forged'",
        "DELETE FROM audit_events",
        "INSERT OR REPLACE INTO audit_events (seq, at, kind, detail)
         VALUES (1, 'now', 'forged', '{}')",
        "REPLACE INTO audit_events (seq, at, kind, detail) VALUES (1, 'now', 'forged', '{}')",
    ] {
        let error = conn.execute(sql, []).unwrap_err();
        assert!(error.to_string().contains("append-only"), "{sql}: {error}");
    }
}

#[test]
fn task_identity_is_immutable() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    store
        .create_task(&new_task(SessionId::new(), "r1"))
        .unwrap();
    let error = store
        .conn()
        .unwrap()
        .execute("UPDATE tasks SET tool = 'shell.exec'", [])
        .unwrap_err();
    assert!(error.to_string().contains("identity is immutable"));
}

#[test]
fn recovery_marks_unfinished_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let session = SessionId::new();
    let received = new_task(session, "r1");
    let executing = new_task(session, "r2");
    let awaiting = new_task(session, "r3");
    let done = new_task(session, "r4");
    {
        let store = open(&dir);
        for task in [&received, &executing, &awaiting, &done] {
            store.create_task(task).unwrap();
        }
        store
            .transition(
                executing.task_id,
                &[TaskStatus::Received],
                &TaskChange::to(TaskStatus::Executing),
                &[],
            )
            .unwrap();
        store
            .transition(
                awaiting.task_id,
                &[TaskStatus::Received],
                &TaskChange::to(TaskStatus::AwaitingConfirmation),
                &[],
            )
            .unwrap();
        store
            .transition(
                done.task_id,
                &[TaskStatus::Received],
                &TaskChange::to(TaskStatus::Denied),
                &[],
            )
            .unwrap();
        // Dropped without closing the tasks, as after a crash.
    }

    let store = open(&dir);
    assert_eq!(
        store.recover(MAX_ATTEMPTS).unwrap(),
        Recovery {
            tasks: 3,
            approvals: 0,
            jobs: 0
        }
    );
    let status = |task: &NewTask| store.task(task.task_id).unwrap().unwrap().status;
    assert_eq!(status(&received), TaskStatus::Interrupted);
    assert_eq!(status(&executing), TaskStatus::Interrupted);
    assert_eq!(status(&awaiting), TaskStatus::Expired);
    assert_eq!(status(&done), TaskStatus::Denied);

    let recovered: Vec<_> = store
        .audit_events(None, 100)
        .unwrap()
        .into_iter()
        .filter(|event| matches!(event.event, AuditEventKind::TaskRecovered { .. }))
        .collect();
    assert_eq!(recovered.len(), 3);
    assert!(
        store.recover(MAX_ATTEMPTS).unwrap().is_empty(),
        "recovery is idempotent"
    );
}

#[test]
fn expiring_a_session_only_touches_its_own_pending_confirmations() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let mine = SessionId::new();
    let other = SessionId::new();
    let a = new_task(mine, "r1");
    let b = new_task(other, "r1");
    for task in [&a, &b] {
        store.create_task(task).unwrap();
        store
            .transition(
                task.task_id,
                &[TaskStatus::Received],
                &TaskChange::to(TaskStatus::AwaitingConfirmation),
                &[],
            )
            .unwrap();
    }
    assert_eq!(store.expire_awaiting(mine, "session closed").unwrap(), 1);
    assert_eq!(
        store.task(a.task_id).unwrap().unwrap().status,
        TaskStatus::Expired
    );
    assert_eq!(
        store.task(b.task_id).unwrap().unwrap().status,
        TaskStatus::AwaitingConfirmation
    );
}

#[test]
fn second_core_cannot_open_same_database() {
    let dir = tempfile::tempdir().unwrap();
    let first = open(&dir);
    assert!(matches!(
        Store::open(&dir.path().join("jarvis.db")),
        Err(StoreError::Locked(_))
    ));
    // Read-only inspection does not need the lock.
    let reader = Store::open_read_only(&dir.path().join("jarvis.db")).unwrap();
    assert_eq!(reader.schema_version().unwrap(), latest_version());
    drop(first);
    open(&dir);
}

#[test]
fn newer_schema_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("jarvis.db");
    {
        let store = Store::open(&path).unwrap();
        store
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (999, 'future', 'now')",
                [],
            )
            .unwrap();
    }
    assert!(matches!(
        Store::open(&path),
        Err(StoreError::SchemaTooNew { found: 999, .. })
    ));
    assert!(matches!(
        Store::open_read_only(&path),
        Err(StoreError::SchemaTooNew { .. })
    ));
}

#[test]
fn writable_open_refuses_a_foreign_sqlite_file() {
    let dir = tempfile::tempdir().unwrap();
    let foreign = dir.path().join("foreign.db");
    Connection::open(&foreign)
        .unwrap()
        .execute_batch("CREATE TABLE t (x INTEGER);")
        .unwrap();
    assert!(matches!(
        Store::open(&foreign),
        Err(StoreError::NotInitialised(_))
    ));
}

#[test]
fn audit_limit_keeps_the_most_recent_events() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    for n in 0..5 {
        let event = AuditEventKind::CoreStopped {
            reason: n.to_string(),
        };
        store.append(None, None, &event).unwrap();
    }
    let reasons: Vec<_> = store
        .audit_events(None, 2)
        .unwrap()
        .into_iter()
        .map(|event| match event.event {
            AuditEventKind::CoreStopped { reason } => reason,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(reasons, ["3", "4"]);
}

#[test]
fn read_only_open_refuses_missing_or_foreign_files() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        Store::open_read_only(&dir.path().join("missing.db")),
        Err(StoreError::NotInitialised(_))
    ));
    let foreign = dir.path().join("foreign.db");
    Connection::open(&foreign)
        .unwrap()
        .execute_batch("CREATE TABLE t (x INTEGER);")
        .unwrap();
    assert!(matches!(
        Store::open_read_only(&foreign),
        Err(StoreError::NotInitialised(_))
    ));
}

#[tokio::test]
async fn async_bridge_runs_on_the_blocking_pool() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let task = new_task(SessionId::new(), "r1");
    let id = task.task_id;
    store.call(move |s| s.create_task(&task)).await.unwrap();
    let record = store.call(move |s| s.task(id)).await.unwrap().unwrap();
    assert_eq!(record.status, TaskStatus::Received);
}

// --- jobs ---------------------------------------------------------------------------------------

fn goal(text: &str) -> Goal {
    Goal::try_from(text.to_owned()).unwrap()
}

#[test]
fn jobs_run_in_submission_order_and_finish_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let session = SessionId::new();
    let first = store.submit_job(&goal("first"), "client 1").unwrap();
    let second = store.submit_job(&goal("second"), "client 2").unwrap();
    assert_eq!(first.status, JobStatus::Queued);
    assert_eq!(store.queued_jobs().unwrap(), 2);

    let claimed = store.claim_next_job(session).unwrap().unwrap();
    assert_eq!(claimed.job_id, first.job_id);
    assert_eq!(claimed.status, JobStatus::Running);
    let summary = Summary::try_from("done".to_owned()).unwrap();
    store
        .finish_job(
            first.job_id,
            JobStatus::Completed,
            Some(&summary),
            Some(session),
        )
        .unwrap();
    assert!(
        store
            .finish_job(first.job_id, JobStatus::Failed, None, Some(session))
            .is_err(),
        "a finished job cannot finish again"
    );
    assert!(
        store
            .finish_job(second.job_id, JobStatus::Completed, None, None)
            .is_err(),
        "a queued job cannot finish"
    );
    assert_eq!(
        store.claim_next_job(session).unwrap().unwrap().job_id,
        second.job_id
    );
    assert!(store.claim_next_job(session).unwrap().is_none());

    let done = store.job(first.job_id).unwrap().unwrap();
    assert_eq!(done.status, JobStatus::Completed);
    assert_eq!(done.summary, Some(summary));
    assert_eq!(done.submitted_by, "client 1");
    let jobs: Vec<_> = store
        .jobs(10)
        .unwrap()
        .iter()
        .map(|job| job.job_id)
        .collect();
    assert_eq!(jobs, [second.job_id, first.job_id], "most recent first");
}

#[test]
fn job_rows_are_protected_by_triggers() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let job = store.submit_job(&goal("g"), "client").unwrap();
    let conn = store.conn().unwrap();
    let error = conn
        .execute("UPDATE jobs SET goal = 'something else'", [])
        .unwrap_err();
    assert!(
        error.to_string().contains("identity is immutable"),
        "{error}"
    );
    conn.execute("UPDATE jobs SET status = 'cancelled'", [])
        .unwrap();
    let error = conn
        .execute("UPDATE jobs SET status = 'queued'", [])
        .unwrap_err();
    assert!(error.to_string().contains("terminal jobs"), "{error}");
    let error = conn.execute("DELETE FROM jobs", []).unwrap_err();
    assert!(error.to_string().contains("never deleted"), "{error}");
    let error = conn
        .execute(
            "INSERT OR REPLACE INTO jobs (job_id, goal, submitted_by, status, created_at, updated_at)
             VALUES (?1, 'g', 'x', 'queued', 'now', 'now')",
            params![job.job_id.to_string()],
        )
        .unwrap_err();
    assert!(error.to_string().contains("never deleted"), "{error}");
}

// --- approvals ----------------------------------------------------------------------------------

const HOUR_MS: i64 = 3_600_000;

struct Pending {
    task: NewTask,
    approval: ApprovalRecord,
}

/// A task waiting for a person, with its pending approval.
fn pending(store: &Store, session_id: SessionId, request_id: &str, expires_in_ms: i64) -> Pending {
    let task = NewTask {
        tool: ToolName::try_from("workspace.write_file".to_owned()).unwrap(),
        args: json!({"path": "notes.txt", "content": "hello"}),
        ..new_task(session_id, request_id)
    };
    store.create_task(&task).unwrap();
    let new = NewApproval {
        approval_id: ApprovalId::new(),
        task_id: task.task_id,
        session_id,
        request_id: task.request_id.clone(),
        tool: task.tool.clone(),
        capabilities: vec![Capability::WorkspaceWrite],
        fingerprint: Fingerprint::from_digest(&[request_id.len() as u8; 32]),
        expires_at_ms: now_ms() + expires_in_ms,
    };
    let change = TaskChange {
        decision: Some(PolicyDecision::RequireConfirmation),
        capabilities: Some(vec![Capability::WorkspaceWrite]),
        ..TaskChange::to(TaskStatus::AwaitingConfirmation)
    };
    let evaluated = AuditEventKind::PolicyEvaluated {
        capabilities: vec![Capability::WorkspaceWrite],
        decision: PolicyDecision::RequireConfirmation,
    };
    let approval = store.request_approval(&new, &change, &[evaluated]).unwrap();
    Pending { task, approval }
}

fn started() -> AuditEventKind {
    AuditEventKind::ExecutionStarted {
        tool: "workspace.write_file".into(),
    }
}

fn task_kinds(store: &Store, task_id: TaskId) -> Vec<&'static str> {
    store
        .audit_events(Some(task_id), 100)
        .unwrap()
        .into_iter()
        .map(|event| event.event.name())
        .collect()
}

fn approval_status(store: &Store, approval_id: ApprovalId) -> ApprovalStatus {
    store.approval(approval_id).unwrap().unwrap().status
}

#[test]
fn granting_records_the_decision_and_consuming_starts_execution_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let Pending { task, approval } = pending(&store, SessionId::new(), "r1", HOUR_MS);
    assert_eq!(approval.status, ApprovalStatus::Pending);
    assert_eq!(store.pending_approval_count().unwrap(), 1);
    assert_eq!(approval.args, r#"{"content":"hello","path":"notes.txt"}"#);

    let granted = store
        .grant_approval(
            approval.approval_id,
            &approval.fingerprint,
            "client",
            now_ms(),
        )
        .unwrap();
    assert_eq!(granted.status, ApprovalStatus::Granted);
    assert_eq!(granted.decided_by.as_deref(), Some("client"));
    assert_eq!(
        store.task(task.task_id).unwrap().unwrap().status,
        TaskStatus::AwaitingConfirmation,
        "granting alone does not start anything"
    );

    store
        .consume_approval(
            approval.approval_id,
            &approval.fingerprint,
            now_ms(),
            &[started()],
        )
        .unwrap();
    assert_eq!(
        approval_status(&store, approval.approval_id),
        ApprovalStatus::Consumed
    );
    assert_eq!(
        store.task(task.task_id).unwrap().unwrap().status,
        TaskStatus::Executing
    );
    assert_eq!(
        task_kinds(&store, task.task_id),
        [
            "request_received",
            "policy_evaluated",
            "approval_requested",
            "approval_granted",
            "approval_consumed",
            "execution_started",
        ]
    );
}

#[test]
fn an_approval_is_used_at_most_once_even_across_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let id;
    let fingerprint;
    let task_id;
    {
        let store = open(&dir);
        let Pending { task, approval } = pending(&store, SessionId::new(), "r1", HOUR_MS);
        (id, fingerprint, task_id) = (approval.approval_id, approval.fingerprint, task.task_id);
        store
            .grant_approval(id, &fingerprint, "client", now_ms())
            .unwrap();
        store
            .consume_approval(id, &fingerprint, now_ms(), &[started()])
            .unwrap();
        assert!(matches!(
            store.consume_approval(id, &fingerprint, now_ms(), &[started()]),
            Err(ApprovalError::NotGranted(ApprovalStatus::Consumed))
        ));
        assert!(matches!(
            store.grant_approval(id, &fingerprint, "client", now_ms()),
            Err(ApprovalError::NotPending(ApprovalStatus::Consumed))
        ));
        // Dropped mid-execution, as after a crash.
    }
    let store = open(&dir);
    let recovered = store.recover(MAX_ATTEMPTS).unwrap();
    assert_eq!(recovered.approvals, 0, "a consumed approval is not open");
    assert_eq!(recovered.tasks, 1);
    assert_eq!(approval_status(&store, id), ApprovalStatus::Consumed);
    assert_eq!(
        store.task(task_id).unwrap().unwrap().status,
        TaskStatus::Interrupted
    );
    assert!(matches!(
        store.consume_approval(id, &fingerprint, now_ms(), &[started()]),
        Err(ApprovalError::NotGranted(ApprovalStatus::Consumed))
    ));
    let consumed = task_kinds(&store, task_id)
        .into_iter()
        .filter(|kind| *kind == "approval_consumed")
        .count();
    assert_eq!(consumed, 1);
}

#[test]
fn a_wrong_fingerprint_grants_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let Pending { approval, task } = pending(&store, SessionId::new(), "r1", HOUR_MS);
    let other = Fingerprint::from_digest(&[0xee; 32]);
    assert!(matches!(
        store.grant_approval(approval.approval_id, &other, "client", now_ms()),
        Err(ApprovalError::FingerprintMismatch)
    ));
    assert_eq!(
        approval_status(&store, approval.approval_id),
        ApprovalStatus::Pending
    );
    assert!(!task_kinds(&store, task.task_id).contains(&"approval_granted"));
}

#[test]
fn consuming_for_a_different_call_is_refused_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let Pending { approval, task } = pending(&store, SessionId::new(), "r1", HOUR_MS);
    store
        .grant_approval(
            approval.approval_id,
            &approval.fingerprint,
            "client",
            now_ms(),
        )
        .unwrap();
    let other = Fingerprint::from_digest(&[0xee; 32]);
    assert!(matches!(
        store.consume_approval(approval.approval_id, &other, now_ms(), &[started()]),
        Err(ApprovalError::TaskChanged)
    ));
    assert_eq!(
        approval_status(&store, approval.approval_id),
        ApprovalStatus::Granted
    );
    assert_eq!(
        store.task(task.task_id).unwrap().unwrap().status,
        TaskStatus::AwaitingConfirmation
    );
}

#[test]
fn a_pending_approval_cannot_be_consumed() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let Pending { approval, .. } = pending(&store, SessionId::new(), "r1", HOUR_MS);
    assert!(matches!(
        store.consume_approval(approval.approval_id, &approval.fingerprint, now_ms(), &[]),
        Err(ApprovalError::NotGranted(ApprovalStatus::Pending))
    ));
}

#[test]
fn denial_closes_the_task_and_wins_over_any_later_decision() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let Pending { approval, task } = pending(&store, SessionId::new(), "r1", HOUR_MS);
    let reason = Summary::try_from("not today".to_owned()).unwrap();
    let denied = store
        .deny_approval(approval.approval_id, &reason, "client", now_ms())
        .unwrap();
    assert_eq!(denied.status, ApprovalStatus::Denied);
    assert_eq!(denied.reason.as_deref(), Some("not today"));
    let record = store.task(task.task_id).unwrap().unwrap();
    assert_eq!(record.status, TaskStatus::Denied);
    assert!(record.error.unwrap().message.contains("not today"));
    assert!(matches!(
        store.grant_approval(
            approval.approval_id,
            &approval.fingerprint,
            "client",
            now_ms()
        ),
        Err(ApprovalError::NotPending(ApprovalStatus::Denied))
    ));
    assert!(matches!(
        store.deny_approval(approval.approval_id, &reason, "client", now_ms()),
        Err(ApprovalError::NotPending(ApprovalStatus::Denied))
    ));
}

#[test]
fn an_expired_approval_can_be_neither_granted_nor_consumed() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let session = SessionId::new();

    let late = pending(&store, session, "r1", 1_000);
    assert!(matches!(
        store.grant_approval(
            late.approval.approval_id,
            &late.approval.fingerprint,
            "client",
            now_ms() + 2_000
        ),
        Err(ApprovalError::Expired)
    ));
    assert_eq!(
        approval_status(&store, late.approval.approval_id),
        ApprovalStatus::Expired
    );
    assert_eq!(
        store.task(late.task.task_id).unwrap().unwrap().status,
        TaskStatus::Expired
    );

    let slow = pending(&store, session, "r22", 1_000);
    store
        .grant_approval(
            slow.approval.approval_id,
            &slow.approval.fingerprint,
            "client",
            now_ms(),
        )
        .unwrap();
    assert!(matches!(
        store.consume_approval(
            slow.approval.approval_id,
            &slow.approval.fingerprint,
            now_ms() + 2_000,
            &[started()]
        ),
        Err(ApprovalError::Expired)
    ));
    assert_eq!(
        approval_status(&store, slow.approval.approval_id),
        ApprovalStatus::Expired
    );
    assert_eq!(
        store.task(slow.task.task_id).unwrap().unwrap().status,
        TaskStatus::Expired
    );
}

#[test]
fn closing_a_session_expires_its_approvals_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let mine = SessionId::new();
    let other = SessionId::new();
    let a = pending(&store, mine, "r1", HOUR_MS);
    let b = pending(&store, other, "r1", HOUR_MS);
    assert_eq!(store.expire_awaiting(mine, "session closed").unwrap(), 1);
    assert_eq!(
        approval_status(&store, a.approval.approval_id),
        ApprovalStatus::Expired
    );
    assert_eq!(
        approval_status(&store, b.approval.approval_id),
        ApprovalStatus::Pending
    );
    assert!(task_kinds(&store, a.task.task_id).contains(&"approval_expired"));
}

#[test]
fn no_approval_survives_a_restart_and_queued_jobs_stay_queued() {
    let dir = tempfile::tempdir().unwrap();
    let (waiting, granted, queued, running);
    {
        let store = open(&dir);
        let session = SessionId::new();
        waiting = pending(&store, session, "r1", HOUR_MS);
        granted = pending(&store, session, "r22", HOUR_MS);
        store
            .grant_approval(
                granted.approval.approval_id,
                &granted.approval.fingerprint,
                "client",
                now_ms(),
            )
            .unwrap();
        running = store.submit_job(&goal("running"), "client").unwrap();
        store.claim_next_job(session).unwrap();
        queued = store.submit_job(&goal("queued"), "client").unwrap();
    }
    let store = open(&dir);
    assert_eq!(
        store.recover(MAX_ATTEMPTS).unwrap(),
        Recovery {
            tasks: 2,
            approvals: 2,
            jobs: 1
        }
    );
    for p in [&waiting, &granted] {
        assert_eq!(
            approval_status(&store, p.approval.approval_id),
            ApprovalStatus::Expired
        );
        assert_eq!(
            store.task(p.task.task_id).unwrap().unwrap().status,
            TaskStatus::Expired
        );
        assert!(matches!(
            store.consume_approval(
                p.approval.approval_id,
                &p.approval.fingerprint,
                now_ms(),
                &[started()]
            ),
            Err(ApprovalError::NotGranted(ApprovalStatus::Expired))
        ));
    }
    // The running job did nothing that could have acted, so it goes back to
    // the queue. Its approvals are gone either way.
    for job in [&running, &queued] {
        assert_eq!(
            store.job(job.job_id).unwrap().unwrap().status,
            JobStatus::Queued
        );
    }
    assert!(store.recover(MAX_ATTEMPTS).unwrap().is_empty());
}

#[test]
fn a_failed_consumption_leaves_nothing_half_done() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let Pending { approval, task } = pending(&store, SessionId::new(), "r1", HOUR_MS);
    store
        .grant_approval(
            approval.approval_id,
            &approval.fingerprint,
            "client",
            now_ms(),
        )
        .unwrap();
    // The last write of the transaction fails, as on a full disk.
    store
        .conn()
        .unwrap()
        .execute_batch(
            "CREATE TEMP TRIGGER fail BEFORE INSERT ON audit_events
             WHEN NEW.kind = 'execution_started'
             BEGIN SELECT RAISE(ABORT, 'simulated disk failure'); END;",
        )
        .unwrap();
    assert!(matches!(
        store.consume_approval(
            approval.approval_id,
            &approval.fingerprint,
            now_ms(),
            &[started()]
        ),
        Err(ApprovalError::Store(_))
    ));
    assert_eq!(
        approval_status(&store, approval.approval_id),
        ApprovalStatus::Granted
    );
    assert_eq!(
        store.task(task.task_id).unwrap().unwrap().status,
        TaskStatus::AwaitingConfirmation
    );
    assert!(!task_kinds(&store, task.task_id).contains(&"approval_consumed"));
}

#[test]
fn approval_rows_are_protected_by_triggers() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let Pending { approval, .. } = pending(&store, SessionId::new(), "r1", HOUR_MS);
    let conn = store.conn().unwrap();
    let id = approval.approval_id.to_string();
    for (sql, expected) in [
        (
            "UPDATE approvals SET fingerprint = ?2 WHERE approval_id = ?1",
            "identity is immutable",
        ),
        (
            "UPDATE approvals SET expires_at_ms = 9e18 WHERE approval_id = ?1 OR ?2 = ''",
            "identity is immutable",
        ),
        (
            "UPDATE approvals SET status = 'consumed' WHERE approval_id = ?1 OR ?2 = ''",
            "illegal approval transition",
        ),
        (
            "DELETE FROM approvals WHERE approval_id = ?1 OR ?2 = ''",
            "never deleted",
        ),
    ] {
        let error = conn.execute(sql, params![id, "f".repeat(64)]).unwrap_err();
        assert!(error.to_string().contains(expected), "{sql}: {error}");
    }
    conn.execute(
        "UPDATE approvals SET status = 'expired' WHERE approval_id = ?1",
        params![id],
    )
    .unwrap();
    let error = conn
        .execute(
            "UPDATE approvals SET decided_by = 'someone' WHERE approval_id = ?1",
            params![id],
        )
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("decided approvals are immutable"),
        "{error}"
    );
    assert!(
        conn.execute(
            "UPDATE approvals SET status = 'granted' WHERE approval_id = ?1",
            params![id],
        )
        .is_err()
    );
    let error = conn
        .execute(
            "INSERT OR REPLACE INTO approvals (approval_id, task_id, session_id, request_id, tool,
                 capabilities, fingerprint, status, requested_at, expires_at, expires_at_ms,
                 updated_at)
             SELECT approval_id, task_id, session_id, request_id, tool, capabilities, fingerprint,
                 'granted', requested_at, expires_at, expires_at_ms, updated_at
             FROM approvals WHERE approval_id = ?1",
            params![id],
        )
        .unwrap_err();
    assert!(error.to_string().contains("never deleted"), "{error}");
}

// --- durable jobs -------------------------------------------------------------------------------

/// Where a task of a job stopped.
#[derive(Clone, Copy)]
enum Stop {
    /// Created, never decided.
    Received,
    /// Waiting for a person; never started.
    Awaiting,
    /// `execution_started` was recorded, then the task ended like this
    /// (`None`: it never ended).
    Started(Option<TaskStatus>),
    /// `execution_started` was recorded, then the task ended `failed` with
    /// this error code (`None`: no error was recorded).
    Failed(Option<ErrorCode>),
}

/// Create the task for position `seq` of `attempt` and leave it as `stop`.
fn step(
    store: &Store,
    session: SessionId,
    job: JobId,
    (attempt, seq): (u32, u32),
    stop: Stop,
) -> TaskId {
    let task = NewTask {
        job_id: Some(job),
        step: Some(JobStep { attempt, seq }),
        ..new_task(session, &format!("a{attempt}s{seq}"))
    };
    store.create_task(&task).unwrap();
    match stop {
        Stop::Received => {}
        Stop::Awaiting => store
            .transition(
                task.task_id,
                &[TaskStatus::Received],
                &TaskChange::to(TaskStatus::AwaitingConfirmation),
                &[],
            )
            .unwrap(),
        Stop::Started(_) | Stop::Failed(_) => {
            let end = match stop {
                Stop::Started(end) => end,
                _ => Some(TaskStatus::Failed),
            };
            let started = AuditEventKind::ExecutionStarted {
                tool: "system.info".into(),
            };
            store
                .transition(
                    task.task_id,
                    &[TaskStatus::Received],
                    &TaskChange::to(TaskStatus::Executing),
                    &[started],
                )
                .unwrap();
            if let Some(end) = end {
                let error = match stop {
                    Stop::Failed(code) => code.map(|code| WireError::new(code, "failed")),
                    _ => None,
                };
                let change = TaskChange {
                    result: (end == TaskStatus::Completed).then(|| "{}".to_owned()),
                    error,
                    ..TaskChange::to(end)
                };
                store
                    .transition(task.task_id, &[TaskStatus::Executing], &change, &[])
                    .unwrap();
            }
        }
    }
    task.task_id
}

fn running_job(store: &Store, session: SessionId) -> JobId {
    let job = store.submit_job(&goal("durable"), "client").unwrap();
    let claimed = store.claim_next_job(session).unwrap().unwrap();
    assert_eq!(claimed.job_id, job.job_id);
    job.job_id
}

fn job_kinds(store: &Store) -> Vec<&'static str> {
    store
        .audit_events(None, 1000)
        .unwrap()
        .into_iter()
        .map(|event| event.event.name())
        .filter(|kind| kind.starts_with("job_"))
        .collect()
}

fn status_of(store: &Store, job: JobId) -> JobStatus {
    store.job(job).unwrap().unwrap().status
}

#[test]
fn claiming_a_job_counts_an_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let session = SessionId::new();
    let job = store.submit_job(&goal("g"), "client").unwrap();
    assert_eq!(job.attempts, 0);
    let claimed = store.claim_next_job(session).unwrap().unwrap();
    assert_eq!(claimed.attempts, 1);
    assert_eq!(
        store
            .requeue_or_close(job.job_id, MAX_ATTEMPTS, Some(session), "test")
            .unwrap(),
        JobStatus::Queued
    );
    assert_eq!(store.job(job.job_id).unwrap().unwrap().attempts, 1);
    assert_eq!(store.claim_next_job(session).unwrap().unwrap().attempts, 2);
}

#[test]
fn a_queued_job_stays_queued_across_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let job = {
        let store = open(&dir);
        store.submit_job(&goal("waiting"), "client").unwrap().job_id
    };
    let store = open(&dir);
    assert!(store.recover(MAX_ATTEMPTS).unwrap().is_empty());
    assert_eq!(status_of(&store, job), JobStatus::Queued);
    assert_eq!(job_kinds(&store), ["job_submitted"]);
    let claimed = store.claim_next_job(SessionId::new()).unwrap().unwrap();
    assert_eq!(claimed.job_id, job);
}

#[test]
fn recovery_requeues_a_running_job_until_its_attempts_are_spent() {
    const LIMIT: u32 = 2;
    let dir = tempfile::tempdir().unwrap();
    let session = SessionId::new();
    let job = {
        let store = open(&dir);
        let job = running_job(&store, session);
        // A step that ended with a known outcome does not stop a resume.
        let done = Stop::Started(Some(TaskStatus::Completed));
        step(&store, session, job, (1, 1), done);
        job
    };

    // Attempt 1 of 2 was running when the Core stopped.
    let store = open(&dir);
    assert_eq!(store.recover(LIMIT).unwrap().jobs, 1);
    assert_eq!(status_of(&store, job), JobStatus::Queued);
    let requeued: Vec<_> = store
        .audit_events(None, 1000)
        .unwrap()
        .into_iter()
        .filter_map(|event| match event.event {
            AuditEventKind::JobRequeued {
                job_id,
                attempts,
                reason,
            } if job_id == job => Some((attempts, reason)),
            _ => None,
        })
        .collect();
    assert_eq!(requeued.len(), 1);
    assert_eq!(requeued[0].0, 1);
    assert!(requeued[0].1.contains("restarted"), "{}", requeued[0].1);
    assert_eq!(store.claim_next_job(session).unwrap().unwrap().attempts, 2);
    drop(store);

    // Attempt 2 of 2 was running when the Core stopped again: no third try.
    let store = open(&dir);
    assert_eq!(store.recover(LIMIT).unwrap().jobs, 1);
    let done = store.job(job).unwrap().unwrap();
    assert_eq!(done.status, JobStatus::Interrupted);
    assert!(done.summary.unwrap().as_str().contains("attempt limit"));
    assert_eq!(job_kinds(&store).last(), Some(&"job_finished"));
    assert!(store.recover(LIMIT).unwrap().is_empty(), "idempotent");
}

#[test]
fn a_task_with_an_unknown_outcome_quarantines_its_job() {
    // Started, and not known to have completed or failed.
    let unknown = [
        None,
        Some(TaskStatus::TimedOut),
        Some(TaskStatus::Cancelled),
        Some(TaskStatus::Interrupted),
    ];
    for end in unknown {
        for at_recovery in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let store = open(&dir);
            let session = SessionId::new();
            let job = running_job(&store, session);
            let done = Stop::Started(Some(TaskStatus::Completed));
            step(&store, session, job, (1, 1), done);
            let task = step(&store, session, job, (1, 2), Stop::Started(end));
            let status = if at_recovery {
                store.recover(MAX_ATTEMPTS).unwrap();
                status_of(&store, job)
            } else {
                store
                    .requeue_or_close(job, MAX_ATTEMPTS, Some(session), "worker stopped")
                    .unwrap()
            };
            let which = format!("{end:?} at_recovery={at_recovery}");
            assert_eq!(status, JobStatus::Interrupted, "{which}");
            let record = store.job(job).unwrap().unwrap();
            assert!(
                record.summary.unwrap().as_str().contains(&task.to_string()),
                "the summary names the task: {which}"
            );
            let quarantined = AuditEventKind::JobQuarantined {
                job_id: job,
                task_id: task,
            };
            assert!(
                store
                    .audit_events(None, 1000)
                    .unwrap()
                    .iter()
                    .any(|event| event.event == quarantined),
                "{which}"
            );
            assert_eq!(job_kinds(&store).last(), Some(&"job_finished"), "{which}");
            assert!(!job_kinds(&store).contains(&"job_requeued"), "{which}");
        }
    }
}

#[test]
fn a_known_outcome_does_not_quarantine_a_job() {
    let stops = [
        (Stop::Received, "received"),
        (Stop::Awaiting, "awaiting confirmation"),
        (
            Stop::Failed(Some(ErrorCode::ToolFailed)),
            "failed by the tool",
        ),
        (Stop::Started(Some(TaskStatus::Completed)), "completed"),
    ];
    for (stop, name) in stops {
        for at_recovery in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let store = open(&dir);
            let session = SessionId::new();
            let job = running_job(&store, session);
            step(&store, session, job, (1, 1), stop);
            let status = if at_recovery {
                store.recover(MAX_ATTEMPTS).unwrap();
                status_of(&store, job)
            } else {
                store
                    .requeue_or_close(job, MAX_ATTEMPTS, Some(session), "worker stopped")
                    .unwrap()
            };
            let which = format!("{name} at_recovery={at_recovery}");
            assert_eq!(status, JobStatus::Queued, "{which}");
            assert!(!job_kinds(&store).contains(&"job_quarantined"), "{which}");
            assert!(job_kinds(&store).contains(&"job_requeued"), "{which}");
        }
    }
}

#[test]
fn only_a_running_job_can_be_requeued_or_closed() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let running = running_job(&store, SessionId::new());
    let queued = store.submit_job(&goal("g"), "client").unwrap().job_id;
    assert!(
        store
            .requeue_or_close(queued, MAX_ATTEMPTS, None, "test")
            .is_err()
    );
    store
        .finish_job(running, JobStatus::Completed, None, None)
        .unwrap();
    assert!(
        store
            .requeue_or_close(running, MAX_ATTEMPTS, None, "test")
            .is_err()
    );
}

#[test]
fn job_steps_are_insert_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let session = SessionId::new();
    let job = running_job(&store, session);
    let task = step(&store, session, job, (1, 1), Stop::Received);
    let conn = store.conn().unwrap();
    let error = conn
        .execute("UPDATE job_steps SET seq = 9", [])
        .unwrap_err();
    assert!(error.to_string().contains("insert-only"), "{error}");
    let error = conn.execute("DELETE FROM job_steps", []).unwrap_err();
    assert!(error.to_string().contains("insert-only"), "{error}");
    let error = conn
        .execute(
            "INSERT OR REPLACE INTO job_steps (task_id, job_id, attempt, seq)
             VALUES (?1, ?2, 1, 1)",
            params![task.to_string(), job.to_string()],
        )
        .unwrap_err();
    assert!(error.to_string().contains("insert-only"), "{error}");
    let error = conn
        .execute("UPDATE jobs SET attempts = 0", [])
        .unwrap_err();
    assert!(error.to_string().contains("never decrease"), "{error}");
}

#[test]
fn a_position_holds_one_task_per_attempt_and_the_task_must_belong_to_the_job() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let session = SessionId::new();
    let job = running_job(&store, session);
    step(&store, session, job, (1, 1), Stop::Received);
    let again = NewTask {
        job_id: Some(job),
        step: Some(JobStep { attempt: 1, seq: 1 }),
        ..new_task(session, "other")
    };
    assert!(store.create_task(&again).is_err());
    let orphan = NewTask {
        step: Some(JobStep { attempt: 1, seq: 2 }),
        ..new_task(session, "orphan")
    };
    assert!(store.create_task(&orphan).is_err(), "a step needs a job");
    assert_eq!(
        store.tasks(10).unwrap().len(),
        1,
        "a refused step leaves no task behind"
    );
}

#[test]
fn a_completed_step_is_found_for_later_attempts_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let session = SessionId::new();
    let job = running_job(&store, session);
    let ok = Stop::Started(Some(TaskStatus::Completed));
    let done = step(&store, session, job, (1, 1), ok);
    let failed = Stop::Failed(Some(ErrorCode::ToolFailed));
    step(&store, session, job, (1, 2), failed);
    step(&store, session, job, (1, 3), Stop::Awaiting);
    let found = |seq, attempt| store.completed_step(job, seq, attempt).unwrap();
    assert_eq!(found(1, 2).unwrap().task_id, done);
    assert_eq!(
        found(1, 3).unwrap().task_id,
        done,
        "still found in attempt 3"
    );
    assert!(found(1, 1).is_none(), "not from the same attempt");
    assert!(found(2, 2).is_none(), "a failed step is not completed");
    assert!(found(3, 2).is_none(), "an unfinished step is not completed");
    assert!(found(4, 2).is_none());
    assert!(store.completed_step(JobId::new(), 1, 2).unwrap().is_none());
}

#[test]
fn replaying_a_step_is_one_audit_event_and_no_task() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let first = SessionId::new();
    let job = running_job(&store, first);
    let ok = Stop::Started(Some(TaskStatus::Completed));
    let done = step(&store, first, job, (1, 1), ok);
    let second = SessionId::new();
    let request = RequestId::try_from("a2s1".to_owned()).unwrap();
    store.replay_step(second, job, 1, done, &request).unwrap();
    assert_eq!(store.tasks(10).unwrap().len(), 1, "no task was created");
    let events = store.audit_events(Some(done), 100).unwrap();
    let replayed = events.last().unwrap();
    assert_eq!(
        replayed.event,
        AuditEventKind::StepReplayed {
            job_id: job,
            seq: 1,
            task_id: done
        }
    );
    assert_eq!(replayed.session_id, Some(second));
    let started = events
        .iter()
        .filter(|event| event.event.name() == "execution_started")
        .count();
    assert_eq!(started, 1);
    // A request_id already used by a task of the session cannot be replayed.
    let used = RequestId::try_from("a1s1".to_owned()).unwrap();
    assert!(matches!(
        store.replay_step(first, job, 1, done, &used),
        Err(StoreError::DuplicateRequest(_))
    ));
}

#[test]
fn a_failure_the_tool_did_not_report_quarantines_its_job() {
    // The tool may have acted before the Core's own check or the tool itself
    // went wrong, so these are not known outcomes.
    let stops = [
        (
            Stop::Failed(Some(ErrorCode::ResultRejected)),
            "result rejected",
        ),
        (Stop::Failed(Some(ErrorCode::Internal)), "internal"),
        (Stop::Failed(None), "no error recorded"),
    ];
    for (stop, name) in stops {
        for at_recovery in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let store = open(&dir);
            let session = SessionId::new();
            let job = running_job(&store, session);
            let task = step(&store, session, job, (1, 1), stop);
            let status = if at_recovery {
                store.recover(MAX_ATTEMPTS).unwrap();
                status_of(&store, job)
            } else {
                store
                    .requeue_or_close(job, MAX_ATTEMPTS, Some(session), "worker stopped")
                    .unwrap()
            };
            let which = format!("{name} at_recovery={at_recovery}");
            assert_eq!(status, JobStatus::Interrupted, "{which}");
            let quarantined = AuditEventKind::JobQuarantined {
                job_id: job,
                task_id: task,
            };
            assert!(
                store
                    .audit_events(None, 1000)
                    .unwrap()
                    .iter()
                    .any(|event| event.event == quarantined),
                "{which}"
            );
        }
    }
}

#[test]
fn a_job_from_before_the_ledger_is_never_requeued() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let job = store.submit_job(&goal("old"), "client").unwrap().job_id;
    // As migration 0003 leaves a job that was running when the Core stopped.
    store
        .conn()
        .unwrap()
        .execute("UPDATE jobs SET status = 'running'", [])
        .unwrap();
    assert_eq!(store.recover(MAX_ATTEMPTS).unwrap().jobs, 1);
    let done = store.job(job).unwrap().unwrap();
    assert_eq!(done.status, JobStatus::Interrupted);
    assert!(done.summary.unwrap().as_str().contains("step ledger"));
    assert_eq!(job_kinds(&store).last(), Some(&"job_finished"));
    assert!(!job_kinds(&store).contains(&"job_requeued"));
}

/// Recovery must look at the tasks before it rewrites them. The rewrite turns
/// an `executing` task into `interrupted`, which would hide what it was doing
/// if the job were settled afterwards, so the audit order is the evidence.
#[test]
fn recovery_settles_a_job_before_it_rewrites_its_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let session = SessionId::new();
    let job = running_job(&store, session);
    let task = step(&store, session, job, (1, 1), Stop::Started(None));
    assert_eq!(
        store.task(task).unwrap().unwrap().status,
        TaskStatus::Executing
    );
    store.recover(MAX_ATTEMPTS).unwrap();
    assert_eq!(status_of(&store, job), JobStatus::Interrupted);
    let seq_of = |wanted: &dyn Fn(&AuditEventKind) -> bool| {
        store
            .audit_events(None, 1000)
            .unwrap()
            .into_iter()
            .find(|event| wanted(&event.event))
            .map(|event| event.seq)
            .unwrap()
    };
    let quarantined = seq_of(&|event| {
        *event
            == AuditEventKind::JobQuarantined {
                job_id: job,
                task_id: task,
            }
    });
    let rewritten = seq_of(&|event| matches!(event, AuditEventKind::TaskRecovered { .. }));
    assert!(quarantined < rewritten, "the job was settled first");
}

#[test]
fn a_job_ended_without_a_retry_still_records_an_unknown_outcome() {
    let ends = [JobStatus::Failed, JobStatus::Interrupted];
    for end in ends {
        let dir = tempfile::tempdir().unwrap();
        let store = open(&dir);
        let session = SessionId::new();
        let job = running_job(&store, session);
        let task = step(
            &store,
            session,
            job,
            (1, 1),
            Stop::Started(Some(TaskStatus::TimedOut)),
        );
        store.finish_job(job, end, None, Some(session)).unwrap();
        assert_eq!(status_of(&store, job), end);
        let kinds = job_kinds(&store);
        assert_eq!(
            &kinds[kinds.len() - 2..],
            ["job_quarantined", "job_finished"],
            "{end:?}"
        );
        let quarantined = AuditEventKind::JobQuarantined {
            job_id: job,
            task_id: task,
        };
        assert!(
            store
                .audit_events(None, 1000)
                .unwrap()
                .iter()
                .any(|event| event.event == quarantined)
        );
    }
    // Known outcomes, and a job that completed, leave no such record.
    let dir = tempfile::tempdir().unwrap();
    let store = open(&dir);
    let session = SessionId::new();
    let job = running_job(&store, session);
    let failed = Stop::Failed(Some(ErrorCode::ToolFailed));
    step(&store, session, job, (1, 1), failed);
    store
        .finish_job(job, JobStatus::Failed, None, Some(session))
        .unwrap();
    assert!(!job_kinds(&store).contains(&"job_quarantined"));
}
