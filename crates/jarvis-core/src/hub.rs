//! In-memory coordination between the RPC server, the worker session and
//! the supervisor. Nothing here is a source of truth: decisions and state
//! live in the store, and these are only signals to look again.

use std::sync::{Arc, Mutex, MutexGuard};

use jarvis_protocol::{WorkerState, WorkerView};
use jarvis_sandbox::Contained;
use tokio::sync::{Notify, watch};

use crate::approval::ApprovalBroker;
use crate::governor::Status as GovernorStatus;

#[derive(Debug)]
pub struct Hub {
    /// Wakes a session waiting on an approval.
    pub approvals: ApprovalBroker,
    /// Wakes the session when a job is queued.
    pub jobs: Notify,
    /// Bumped whenever a job or approval changes, for long-polling clients.
    changes: watch::Sender<u64>,
    /// Whether queued jobs may start (ADR 0016). Open for good unless the
    /// governor is enabled.
    governor: watch::Sender<GovernorStatus>,
    worker: Mutex<WorkerView>,
    contained: Mutex<Option<Arc<Contained>>>,
    /// Who asked the Core to stop, when a client did.
    stop_reason: Mutex<Option<String>>,
}

impl Default for Hub {
    fn default() -> Self {
        Self {
            approvals: ApprovalBroker::default(),
            jobs: Notify::new(),
            changes: watch::Sender::new(0),
            governor: watch::Sender::new(GovernorStatus::OFF),
            worker: Mutex::new(WorkerView {
                state: WorkerState::Starting,
                pid: None,
                restarts: 0,
                containment: String::new(),
            }),
            contained: Mutex::new(None),
            stop_reason: Mutex::new(None),
        }
    }
}

impl Hub {
    pub fn changed(&self) {
        self.changes
            .send_modify(|version| *version = version.wrapping_add(1));
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// A receiver for the governor's status. A session reads it before it
    /// claims a job and waits on it while the gate is closed.
    pub fn subscribe_governor(&self) -> watch::Receiver<GovernorStatus> {
        self.governor.subscribe()
    }

    pub fn governor_status(&self) -> GovernorStatus {
        self.governor.borrow().clone()
    }

    pub fn set_governor(&self, status: GovernorStatus) {
        self.governor.send_replace(status);
    }

    pub fn worker(&self) -> WorkerView {
        lock(&self.worker).clone()
    }

    pub fn update_worker(&self, update: impl FnOnce(&mut WorkerView)) {
        update(&mut lock(&self.worker));
    }

    pub fn set_worker_state(&self, state: WorkerState) {
        self.update_worker(|worker| worker.state = state);
    }

    pub fn set_contained(&self, contained: Option<Arc<Contained>>) {
        *lock(&self.contained) = contained;
    }

    pub fn set_stop_reason(&self, reason: String) {
        *lock(&self.stop_reason) = Some(reason);
    }

    pub fn stop_reason(&self) -> Option<String> {
        lock(&self.stop_reason).clone()
    }

    /// Whether `pid` belongs to the running worker or a process it started.
    pub fn is_worker_process(&self, pid: u32) -> bool {
        lock(&self.contained)
            .as_ref()
            .is_some_and(|contained| contained.contains(pid))
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}
