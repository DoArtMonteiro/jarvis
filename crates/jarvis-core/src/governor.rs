//! The gaming governor (ADR 0016).
//!
//! While a configured process runs, the Core does not start queued jobs.
//! That is all it does: it never pauses a job that is running, never opens,
//! suspends or inspects any process, and never changes a priority. The
//! decisions are made by [`machine::step`], a pure function. This module is
//! the thin task around it: read the process list every `poll`, feed the
//! result to `step`, record what must be recorded, and publish the outcome to
//! the [`Hub`], where each session reads it before it claims a job.
//!
//! A governor that is off is never started, and the gate it would have
//! controlled stays open.

pub mod machine;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use jarvis_protocol::{AuditEventKind, GovernorPhase, GovernorReason, GovernorView};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::config::GovernorConfig;
use crate::hub::Hub;
use crate::store::{Store, StoreError};
use machine::{Event, Hold, Observation, State, Timing, step};

/// Why a reading of the process list failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ActivityError(pub String);

/// The result of one reading: the configured name of a running game, or
/// `None` when none runs.
pub type Reading = Result<Option<String>, ActivityError>;

pub type ReadingFuture<'a> = Pin<Box<dyn Future<Output = Reading> + Send + 'a>>;

/// Where the governor learns whether a game runs. The real source reads the
/// OS process list; tests substitute their own.
pub trait ActivitySource: Send + Sync + std::fmt::Debug + 'static {
    fn read(&self) -> ReadingFuture<'_>;
}

/// Reads the OS process list for the configured names.
#[derive(Debug, Clone)]
pub struct ProcessActivity {
    names: Vec<String>,
}

impl ProcessActivity {
    pub fn new(names: Vec<String>) -> Self {
        Self { names }
    }
}

impl ActivitySource for ProcessActivity {
    fn read(&self) -> ReadingFuture<'_> {
        let names = self.names.clone();
        Box::pin(async move {
            // The list is read on a thread of its own: it is a system call
            // that can take a moment on a busy machine.
            let read = tokio::task::spawn_blocking(move || {
                let names: Vec<&str> = names.iter().map(String::as_str).collect();
                jarvis_sandbox::processes::find_running(&names)
            })
            .await;
            match read {
                Ok(Ok(found)) => Ok(found),
                Ok(Err(error)) => Err(ActivityError(error.to_string())),
                Err(_) => Err(ActivityError("the process list reader panicked".into())),
            }
        })
    }
}

/// What the governor publishes for the sessions to read and for `health`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    pub phase: GovernorPhase,
    pub reason: Option<GovernorReason>,
    pub process: Option<String>,
    /// When the current hold began.
    pub since: Option<Instant>,
    pub overdue: bool,
}

impl Status {
    /// The governor is disabled: the gate is open for good.
    pub const OFF: Self = Self {
        phase: GovernorPhase::Off,
        reason: None,
        process: None,
        since: None,
        overdue: false,
    };

    /// Whether a session may claim a job.
    pub fn is_open(&self) -> bool {
        matches!(self.phase, GovernorPhase::Off | GovernorPhase::Open)
    }

    /// The status for `state`, where `origin` is the instant at which the
    /// state machine's clock read zero.
    pub fn of(state: &State, origin: Instant) -> Self {
        let at = |ms: u64| origin + Duration::from_millis(ms);
        match state {
            State::Open => Self {
                phase: GovernorPhase::Open,
                ..Self::OFF
            },
            State::Held {
                since,
                hold,
                overdue,
            } => {
                let (reason, process) = match hold {
                    Hold::Starting => (GovernorReason::Starting, None),
                    Hold::Game(name) => (GovernorReason::Game, Some(name.clone())),
                    Hold::DetectionFailed => (GovernorReason::DetectionFailed, None),
                };
                Self {
                    phase: GovernorPhase::Held,
                    reason: Some(reason),
                    process,
                    since: Some(at(*since)),
                    overdue: *overdue,
                }
            }
            State::Releasing { since, overdue, .. } => Self {
                phase: GovernorPhase::Releasing,
                reason: None,
                process: None,
                since: Some(at(*since)),
                overdue: *overdue,
            },
        }
    }

    /// The wire view of this status as of `now`.
    pub fn view(&self, now: Instant) -> GovernorView {
        GovernorView {
            state: self.phase,
            reason: self.reason,
            process: self.process.clone(),
            held_ms: self.since.map(|since| {
                u64::try_from(now.saturating_duration_since(since).as_millis()).unwrap_or(u64::MAX)
            }),
            overdue: self.overdue,
        }
    }
}

/// The governor task. [`Governor::new`] closes the gate at once, before any
/// session can look at it; [`Governor::run`] then keeps it up to date.
#[derive(Debug)]
pub struct Governor {
    store: Store,
    hub: Arc<Hub>,
    source: Arc<dyn ActivitySource>,
    poll: Duration,
    timing: Timing,
    origin: Instant,
    state: State,
    /// Whether the previous reading failed, so that a streak of failures is
    /// logged once.
    failing: bool,
}

impl Governor {
    pub fn new(
        config: &GovernorConfig,
        store: Store,
        hub: Arc<Hub>,
        source: Arc<dyn ActivitySource>,
    ) -> Self {
        let origin = Instant::now();
        let state = State::starting(0);
        hub.set_governor(Status::of(&state, origin));
        Self {
            store,
            hub,
            source,
            poll: config.poll,
            timing: Timing {
                resume_delay_ms: millis(config.resume_delay),
                alert_after_ms: millis(config.alert_after),
            },
            origin,
            state,
            failing: false,
        }
    }

    /// Read, decide, record and publish every `poll` until `stop`.
    pub async fn run(mut self, stop: CancellationToken) {
        loop {
            let reading = tokio::select! {
                biased;
                () = stop.cancelled() => return,
                reading = self.source.read() => reading,
            };
            self.apply(reading).await;
            tokio::select! {
                biased;
                () = stop.cancelled() => return,
                () = tokio::time::sleep(self.poll) => {}
            }
        }
    }

    async fn apply(&mut self, reading: Reading) {
        let observation = match reading {
            Ok(Some(name)) => Observation::Game(name),
            Ok(None) => Observation::Clear,
            Err(error) => {
                // A failing source is read again every poll; say so once.
                if !self.failing {
                    tracing::warn!(%error, "cannot read the process list; queued jobs are held");
                }
                Observation::Failed
            }
        };
        self.failing = observation == Observation::Failed;

        let now = millis(self.origin.elapsed());
        let (next, events) = step(self.state.clone(), observation, now, self.timing);

        // Every event is recorded before the state it leads to takes effect.
        // A hold or an alert that cannot be recorded still applies: holding
        // is the safe direction. A release that cannot be recorded does not:
        // the gate stays shut and the next reading tries again.
        let mut released = true;
        for event in &events {
            if let Err(error) = self.record(event).await {
                tracing::error!(%error, ?event, "could not record a governor event");
                released &= !matches!(event, Event::Released { .. });
            }
        }
        let next = if released {
            next
        } else {
            staying(self.state.clone(), &events)
        };
        if next != self.state {
            tracing::info!(state = ?next, "governor");
            self.state = next;
            self.hub.set_governor(Status::of(&self.state, self.origin));
        }
    }

    async fn record(&self, event: &Event) -> Result<(), StoreError> {
        let kind = audit_event(event);
        self.store
            .call(move |s| s.append(None, None, &kind).map(|_| ()))
            .await
    }
}

/// The state to keep when a release could not be recorded: unchanged, except
/// that an alert recorded in the same step is not raised again.
fn staying(mut state: State, events: &[Event]) -> State {
    if events.iter().any(|e| matches!(e, Event::Overdue { .. }))
        && let State::Held { overdue, .. } | State::Releasing { overdue, .. } = &mut state
    {
        *overdue = true;
    }
    state
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn audit_event(event: &Event) -> AuditEventKind {
    match event {
        Event::Hold(Hold::Game(name)) => AuditEventKind::GovernorHold {
            reason: GovernorReason::Game,
            process: Some(name.clone()),
        },
        Event::Hold(Hold::DetectionFailed) => AuditEventKind::GovernorHold {
            reason: GovernorReason::DetectionFailed,
            process: None,
        },
        Event::Hold(Hold::Starting) => AuditEventKind::GovernorHold {
            reason: GovernorReason::Starting,
            process: None,
        },
        Event::Released { held_ms } => AuditEventKind::GovernorReleased { held_ms: *held_ms },
        Event::Overdue { held_ms } => AuditEventKind::GovernorHoldOverdue { held_ms: *held_ms },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_names_the_hold_and_how_long_it_has_lasted() {
        let origin = Instant::now();
        let state = State::Held {
            since: 1_000,
            hold: Hold::Game("cs2.exe".into()),
            overdue: true,
        };
        let status = Status::of(&state, origin);
        assert!(!status.is_open());
        let view = status.view(origin + Duration::from_millis(61_000));
        assert_eq!(view.state, GovernorPhase::Held);
        assert_eq!(view.reason, Some(GovernorReason::Game));
        assert_eq!(view.process.as_deref(), Some("cs2.exe"));
        assert_eq!(view.held_ms, Some(60_000));
        assert!(view.overdue);
    }

    #[test]
    fn only_off_and_open_let_jobs_start() {
        let origin = Instant::now();
        assert!(Status::OFF.is_open());
        assert!(Status::of(&State::Open, origin).is_open());
        assert!(!Status::of(&State::starting(0), origin).is_open());
        let releasing = State::Releasing {
            since: 0,
            until: 5,
            overdue: false,
        };
        let status = Status::of(&releasing, origin);
        assert!(!status.is_open());
        assert_eq!(status.view(origin).state, GovernorPhase::Releasing);
        assert_eq!(status.view(origin).reason, None);
    }

    #[test]
    fn an_open_or_off_governor_reports_no_hold() {
        let origin = Instant::now();
        let view = Status::of(&State::Open, origin).view(origin);
        assert_eq!(view.state, GovernorPhase::Open);
        assert_eq!(
            (view.reason, view.process, view.held_ms),
            (None, None, None)
        );
        assert_eq!(Status::OFF.view(origin), GovernorView::OFF);
    }

    #[test]
    fn audit_events_carry_only_the_configured_name() {
        let kind = audit_event(&Event::Hold(Hold::Game("cs2.exe".into())));
        assert_eq!(
            kind,
            AuditEventKind::GovernorHold {
                reason: GovernorReason::Game,
                process: Some("cs2.exe".into())
            }
        );
        assert_eq!(
            audit_event(&Event::Released { held_ms: 7 }),
            AuditEventKind::GovernorReleased { held_ms: 7 }
        );
    }
}
