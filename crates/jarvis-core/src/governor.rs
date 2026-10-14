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
    /// Whether the last reading had a write fail, so that a
    /// streak of failures is logged once.
    record_failing: bool,
    /// Events that could not be written, oldest first (see `apply`).
    pending: Vec<Event>,
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
            record_failing: false,
            pending: Vec::new(),
        }
    }

    /// Read, decide, record and publish every `poll` until `stop`.
    ///
    /// At most one read is ever in flight. A read that has not finished
    /// after `poll` counts as a failed reading (so the jobs are held), and
    /// the next tick goes on waiting for that same read instead of starting
    /// another: a hung operating-system call costs one blocked thread, not
    /// one per tick. If the read finishes late, its answer is used.
    pub async fn run(mut self, stop: CancellationToken) {
        let mut guard = FailClosed {
            hub: Arc::clone(&self.hub),
            origin: self.origin,
            armed: true,
        };
        let source = Arc::clone(&self.source);
        let mut in_flight: Option<ReadingFuture<'_>> = None;
        loop {
            let read = in_flight.get_or_insert_with(|| source.read());
            let outcome = tokio::select! {
                biased;
                () = stop.cancelled() => {
                    guard.armed = false;
                    self.report_unrecorded();
                    return;
                }
                outcome = tokio::time::timeout(self.poll, read) => outcome,
            };
            match outcome {
                Ok(reading) => {
                    in_flight = None;
                    self.apply(reading).await;
                    tokio::select! {
                        biased;
                        () = stop.cancelled() => {
                            guard.armed = false;
                            self.report_unrecorded();
                            return;
                        }
                        () = tokio::time::sleep(self.poll) => {}
                    }
                }
                Err(_) => {
                    let late =
                        ActivityError("the read of the process list is still running".into());
                    self.apply(Err(late)).await;
                }
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
        let releasing = events.iter().any(|e| matches!(e, Event::Released { .. }));
        let alerted = events.iter().any(|e| matches!(e, Event::Overdue { .. }));

        // Holding is the safe direction, so the gate closes first and the
        // record follows. A slow or failing database must not keep jobs
        // starting while a game runs.
        let next = if releasing {
            Some(next)
        } else {
            self.publish(next);
            None
        };

        // Events are recorded in the order they happened. One that cannot be
        // written waits in `pending` and is tried again, before anything
        // newer, at every later reading.
        let mut streak = false;
        let mut in_order = self.flush_pending(&mut streak).await;
        let mut release_recorded = false;
        for event in events {
            if matches!(event, Event::Released { .. }) {
                // Never queued: the state machine emits it again at the next
                // reading for as long as the gate has not opened.
                if in_order && self.try_record(&event, &mut streak).await {
                    release_recorded = true;
                }
                continue;
            }
            if in_order && self.try_record(&event, &mut streak).await {
                continue;
            }
            in_order = false;
            self.queue(event);
        }
        self.record_failing = streak;

        // Opening the gate is the other direction: it waits for the record
        // of the release and of everything before it. Otherwise the gate
        // stays shut and the next reading tries again.
        if let Some(next) = next {
            if release_recorded {
                self.publish(next);
            } else {
                self.publish(staying(self.state.clone(), alerted, now));
            }
        }
    }

    /// Retry the events that earlier readings could not record, oldest
    /// first. Returns whether none is left.
    async fn flush_pending(&mut self, streak: &mut bool) -> bool {
        while let Some(event) = self.pending.first().cloned() {
            if !self.try_record(&event, streak).await {
                return false;
            }
            self.pending.remove(0);
        }
        true
    }

    /// Write one event. A failure is logged once per streak of failures.
    async fn try_record(&self, event: &Event, streak: &mut bool) -> bool {
        match self.record(event).await {
            Ok(()) => true,
            Err(error) => {
                if !self.record_failing {
                    tracing::error!(
                        %error,
                        ?event,
                        "could not record a governor event; it is kept and tried again, \
                         and the gate stays shut until a release can be recorded"
                    );
                }
                *streak = true;
                false
            }
        }
    }

    /// Keep an event that could not be written. It is never dropped: a later
    /// release would otherwise open the gate with an event missing. While
    /// anything waits the gate cannot open, so the state machine produces at
    /// most one hold and one alert in that time; more than [`MAX_PENDING`]
    /// waiting means a mistake, which is logged.
    fn queue(&mut self, event: Event) {
        self.pending.push(event);
        if self.pending.len() > MAX_PENDING {
            tracing::error!(
                waiting = self.pending.len(),
                "more governor events wait to be recorded than there should ever be"
            );
        }
    }

    /// Events that never reached the audit log are held only in memory, so
    /// stopping loses them. Say so rather than drop them silently.
    fn report_unrecorded(&self) {
        if !self.pending.is_empty() {
            tracing::error!(
                lost = self.pending.len(),
                events = ?self.pending,
                "the governor stopped with events that were never recorded; \
                 they are lost because they were only held in memory"
            );
        }
    }

    /// Make `next` the state sessions see.
    fn publish(&mut self, next: State) {
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

/// Closes the gate if the governor task ends for any reason but a stop
/// request: a panic in a source, or the task being dropped. Nothing would
/// update the gate afterwards, so it must not be left open.
struct FailClosed {
    hub: Arc<Hub>,
    origin: Instant,
    armed: bool,
}

impl Drop for FailClosed {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        tracing::error!("the governor stopped unexpectedly; queued jobs are held");
        let now = millis(self.origin.elapsed());
        let state = State::Held {
            since: now,
            hold: Hold::DetectionFailed,
            overdue: false,
        };
        self.hub.set_governor(Status::of(&state, self.origin));
    }
}

/// The state to keep when a release could not be recorded: the game is gone
/// but the jobs stay held, so it is releasing and the next reading tries
/// again. (Without a resume delay the step went straight from held to open.)
/// An alert raised in the same step is not raised again.
fn staying(state: State, alerted: bool, now: u64) -> State {
    match state {
        State::Held { since, overdue, .. } => State::Releasing {
            since,
            until: now,
            overdue: overdue || alerted,
        },
        State::Releasing {
            since,
            until,
            overdue,
        } => State::Releasing {
            since,
            until,
            overdue: overdue || alerted,
        },
        State::Open => State::Open,
    }
}

/// More events than this waiting to be written means a mistake. They are
/// still kept; only an error is logged.
const MAX_PENDING: usize = 8;

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
