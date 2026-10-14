//! The governor's decisions as a pure function of the previous state, one
//! observation and the time. No clock, no task and no I/O live here, so
//! every transition can be tested directly.
//!
//! | From | Observation | To | Event |
//! | --- | --- | --- | --- |
//! | `Open` | game | `Held` (since now) | `Hold` |
//! | `Open` | failed | `Held` (since now) | `Hold` |
//! | `Open` | clear | `Open` | |
//! | `Held` | game or failed | `Held` | |
//! | `Held` | clear | `Releasing` (until now + delay) | |
//! | `Releasing` | game or failed | `Held`, same `since` | |
//! | `Releasing` | clear, delay not over | `Releasing` | |
//! | `Releasing` | clear, delay over | `Open` | `Released` |
//! | start-up `Held` | clear | `Open` | |
//! | start-up `Held` | game or failed | `Held`, same `since` | `Hold` |
//!
//! Time never ends a hold. Once a hold has lasted the alert time it is
//! marked overdue and one event says so; the jobs keep waiting until the
//! game is gone. The start-up hold raises no alert: it is not a decision, and
//! the alert for the hold that follows it comes at the first reading at or
//! after the alert time, never before that hold is recorded.

/// Why queued jobs are held.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hold {
    /// No detection has finished since the Core started.
    Starting,
    /// A configured process is running; this is the configured name.
    Game(String),
    /// The process list could not be read.
    DetectionFailed,
}

/// Where the governor is. Times are milliseconds on one monotonic clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    Open,
    Held {
        since: u64,
        hold: Hold,
        overdue: bool,
    },
    /// The game is gone; jobs wait until `until`. `since` is when the hold
    /// began, not when the game left.
    Releasing {
        since: u64,
        until: u64,
        overdue: bool,
    },
}

impl State {
    /// The state of a Core that has just started with the governor enabled.
    /// Nothing is known yet, so jobs wait for the first detection.
    pub fn starting(now: u64) -> Self {
        Self::Held {
            since: now,
            hold: Hold::Starting,
            overdue: false,
        }
    }

    pub fn is_open(&self) -> bool {
        matches!(self, Self::Open)
    }
}

/// What one reading of the process list showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    /// A configured process is running; this is the configured name.
    Game(String),
    /// The list was read and none of them runs.
    Clear,
    /// The list could not be read.
    Failed,
}

/// Something that must be recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Jobs stopped starting, for `Game` or `DetectionFailed`. Never for
    /// `Starting`: that is not a decision.
    Hold(Hold),
    /// Jobs may start again.
    Released { held_ms: u64 },
    /// The hold has lasted the alert time. Once per hold.
    Overdue { held_ms: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    pub resume_delay_ms: u64,
    pub alert_after_ms: u64,
}

pub fn step(
    mut state: State,
    observation: Observation,
    now: u64,
    timing: Timing,
) -> (State, Vec<Event>) {
    let mut events = Vec::new();

    // The alert comes first, so that when the same reading also releases the
    // gate the log still shows the hold was overdue before it ended. The
    // start-up hold is not a decision and raises none: if the first reading
    // turns it into a real hold, the alert follows at the next reading, and
    // it is never recorded before the hold it is about.
    if !matches!(
        state,
        State::Held {
            hold: Hold::Starting,
            ..
        }
    ) && let State::Held { since, overdue, .. } | State::Releasing { since, overdue, .. } =
        &mut state
    {
        let held_ms = now.saturating_sub(*since);
        if !*overdue && held_ms >= timing.alert_after_ms {
            *overdue = true;
            events.push(Event::Overdue { held_ms });
        }
    }

    let next = match (state, observation) {
        (State::Open, Observation::Clear) => State::Open,
        (State::Open, observation) => {
            let hold = hold_for(observation);
            events.push(Event::Hold(hold.clone()));
            State::Held {
                since: now,
                hold,
                overdue: false,
            }
        }

        // The first reading since start-up. Nothing was running when the
        // Core came up, so there is nothing to wait out.
        (
            State::Held {
                hold: Hold::Starting,
                ..
            },
            Observation::Clear,
        ) => State::Open,
        // A game or a failure on the first reading is a hold that began when
        // the Core started.
        (
            State::Held {
                since,
                hold: Hold::Starting,
                overdue,
            },
            observation,
        ) => {
            let hold = hold_for(observation);
            events.push(Event::Hold(hold.clone()));
            State::Held {
                since,
                hold,
                overdue,
            }
        }

        (State::Held { since, overdue, .. }, Observation::Clear) => release_or_wait(
            since,
            now,
            now.saturating_add(timing.resume_delay_ms),
            overdue,
            &mut events,
        ),
        (State::Held { since, overdue, .. }, observation)
        | (
            State::Releasing { since, overdue, .. },
            observation @ (Observation::Game(_) | Observation::Failed),
        ) => State::Held {
            since,
            hold: hold_for(observation),
            overdue,
        },
        (
            State::Releasing {
                since,
                until,
                overdue,
            },
            Observation::Clear,
        ) => release_or_wait(since, now, until, overdue, &mut events),
    };
    (next, events)
}

/// `Open` once `now` has reached `until`, otherwise still releasing.
fn release_or_wait(
    since: u64,
    now: u64,
    until: u64,
    overdue: bool,
    events: &mut Vec<Event>,
) -> State {
    if now >= until {
        events.push(Event::Released {
            held_ms: now.saturating_sub(since),
        });
        State::Open
    } else {
        State::Releasing {
            since,
            until,
            overdue,
        }
    }
}

/// The reason a non-clear reading gives for holding.
fn hold_for(observation: Observation) -> Hold {
    match observation {
        Observation::Game(name) => Hold::Game(name),
        Observation::Failed | Observation::Clear => Hold::DetectionFailed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DELAY: u64 = 30_000;
    const ALERT: u64 = 3_600_000;
    const TIMING: Timing = Timing {
        resume_delay_ms: DELAY,
        alert_after_ms: ALERT,
    };

    fn game() -> Observation {
        Observation::Game("cs2.exe".into())
    }

    fn held(since: u64, hold: Hold) -> State {
        State::Held {
            since,
            hold,
            overdue: false,
        }
    }

    fn run(state: State, steps: &[(u64, Observation)]) -> (State, Vec<Event>) {
        let mut state = state;
        let mut events = Vec::new();
        for (now, observation) in steps {
            let (next, mut more) = step(state, observation.clone(), *now, TIMING);
            state = next;
            events.append(&mut more);
        }
        (state, events)
    }

    #[test]
    fn a_game_closes_an_open_gate_and_says_which() {
        let (state, events) = step(State::Open, game(), 5_000, TIMING);
        assert_eq!(state, held(5_000, Hold::Game("cs2.exe".into())));
        assert_eq!(events, [Event::Hold(Hold::Game("cs2.exe".into()))]);
    }

    #[test]
    fn a_failed_detection_closes_an_open_gate() {
        let (state, events) = step(State::Open, Observation::Failed, 5_000, TIMING);
        assert_eq!(state, held(5_000, Hold::DetectionFailed));
        assert_eq!(events, [Event::Hold(Hold::DetectionFailed)]);
    }

    #[test]
    fn a_clear_list_leaves_an_open_gate_alone() {
        let (state, events) = step(State::Open, Observation::Clear, 5_000, TIMING);
        assert_eq!(state, State::Open);
        assert!(events.is_empty());
    }

    #[test]
    fn a_clear_list_starts_the_resume_delay_from_the_original_hold() {
        let (state, events) = step(
            held(1_000, Hold::Game("cs2.exe".into())),
            Observation::Clear,
            5_000,
            TIMING,
        );
        assert_eq!(
            state,
            State::Releasing {
                since: 1_000,
                until: 5_000 + DELAY,
                overdue: false
            }
        );
        assert!(events.is_empty(), "nothing to record until the gate opens");
    }

    #[test]
    fn the_gate_opens_exactly_when_the_delay_is_over_and_counts_the_whole_hold() {
        let releasing = State::Releasing {
            since: 1_000,
            until: 35_000,
            overdue: false,
        };
        let (state, events) = step(releasing.clone(), Observation::Clear, 34_999, TIMING);
        assert_eq!(state, releasing);
        assert!(events.is_empty());
        let (state, events) = step(releasing, Observation::Clear, 35_000, TIMING);
        assert_eq!(state, State::Open);
        assert_eq!(events, [Event::Released { held_ms: 34_000 }]);
    }

    #[test]
    fn a_zero_delay_opens_the_gate_on_the_first_clear_reading() {
        let timing = Timing {
            resume_delay_ms: 0,
            ..TIMING
        };
        let (state, events) = step(
            held(1_000, Hold::Game("cs2.exe".into())),
            Observation::Clear,
            4_000,
            timing,
        );
        assert_eq!(state, State::Open);
        assert_eq!(events, [Event::Released { held_ms: 3_000 }]);
    }

    #[test]
    fn a_game_during_the_delay_holds_again_without_a_new_event_or_a_new_start() {
        let releasing = State::Releasing {
            since: 1_000,
            until: 35_000,
            overdue: false,
        };
        let (state, events) = step(releasing.clone(), game(), 20_000, TIMING);
        assert_eq!(state, held(1_000, Hold::Game("cs2.exe".into())));
        assert!(events.is_empty());
        let (state, events) = step(releasing, Observation::Failed, 20_000, TIMING);
        assert_eq!(state, held(1_000, Hold::DetectionFailed));
        assert!(events.is_empty());
    }

    #[test]
    fn the_delay_starts_over_after_an_interruption() {
        let (state, events) = run(
            held(0, Hold::Game("cs2.exe".into())),
            &[
                (10_000, Observation::Clear),
                (20_000, game()),
                (30_000, Observation::Clear),
                (30_000 + DELAY - 1, Observation::Clear),
            ],
        );
        assert!(matches!(state, State::Releasing { until, .. } if until == 30_000 + DELAY));
        assert!(events.is_empty());
    }

    #[test]
    fn changing_reason_while_held_is_not_a_new_hold() {
        let (state, events) = run(
            held(0, Hold::Game("cs2.exe".into())),
            &[
                (1_000, Observation::Failed),
                (2_000, Observation::Game("other.exe".into())),
            ],
        );
        assert_eq!(state, held(0, Hold::Game("other.exe".into())));
        assert!(events.is_empty());
    }

    #[test]
    fn an_error_streak_is_recorded_once() {
        let (state, events) = run(
            State::Open,
            &[
                (1_000, Observation::Failed),
                (2_000, Observation::Failed),
                (3_000, Observation::Failed),
            ],
        );
        assert_eq!(state, held(1_000, Hold::DetectionFailed));
        assert_eq!(events, [Event::Hold(Hold::DetectionFailed)]);
    }

    #[test]
    fn a_new_streak_after_a_release_is_recorded_again() {
        let (_, events) = run(
            State::Open,
            &[
                (1_000, Observation::Failed),
                (2_000, Observation::Clear),
                (2_000 + DELAY, Observation::Clear),
                (3_000 + DELAY, Observation::Failed),
            ],
        );
        assert_eq!(
            events,
            [
                Event::Hold(Hold::DetectionFailed),
                Event::Released {
                    held_ms: 1_000 + DELAY
                },
                Event::Hold(Hold::DetectionFailed),
            ]
        );
    }

    #[test]
    fn a_clear_first_reading_after_the_alert_time_opens_the_gate_with_no_event() {
        let (state, events) = step(State::starting(0), Observation::Clear, ALERT, TIMING);
        assert_eq!(state, State::Open);
        assert!(events.is_empty(), "no alert for a hold that was never one");
    }

    #[test]
    fn a_hold_found_on_a_late_first_reading_is_recorded_before_its_alert() {
        for (observation, hold) in [
            (Observation::Failed, Hold::DetectionFailed),
            (game(), Hold::Game("cs2.exe".into())),
        ] {
            let (state, events) = step(State::starting(0), observation.clone(), ALERT, TIMING);
            assert_eq!(events, [Event::Hold(hold.clone())]);
            assert_eq!(state, held(0, hold));

            let (state, events) = step(state, observation, ALERT + 1_000, TIMING);
            assert_eq!(
                events,
                [Event::Overdue {
                    held_ms: ALERT + 1_000
                }]
            );
            assert!(matches!(state, State::Held { overdue: true, .. }));
        }
    }

    #[test]
    fn the_alert_fires_once_at_the_alert_time_and_the_hold_goes_on() {
        let mut state = held(1_000, Hold::Game("cs2.exe".into()));
        let mut seen = Vec::new();
        for now in [
            ALERT,
            1_000 + ALERT - 1,
            1_000 + ALERT,
            1_000 + ALERT + 1,
            10 * ALERT,
        ] {
            let (next, events) = step(state, game(), now, TIMING);
            state = next;
            seen.push(events);
        }
        assert_eq!(seen[0], []);
        assert_eq!(seen[1], []);
        assert_eq!(seen[2], [Event::Overdue { held_ms: ALERT }]);
        assert_eq!(seen[3], []);
        assert_eq!(seen[4], []);
        assert_eq!(
            state,
            State::Held {
                since: 1_000,
                hold: Hold::Game("cs2.exe".into()),
                overdue: true
            },
            "time alone never opens the gate"
        );
    }

    #[test]
    fn a_hold_stays_overdue_through_the_delay_and_a_new_hold_alerts_again() {
        let (state, events) = run(
            held(0, Hold::Game("cs2.exe".into())),
            &[
                (ALERT, game()),
                (ALERT + 1_000, Observation::Clear),
                (ALERT + 2_000, game()),
            ],
        );
        assert_eq!(events, [Event::Overdue { held_ms: ALERT }]);
        assert!(matches!(state, State::Held { overdue: true, .. }));

        let (state, events) = run(
            state,
            &[
                (ALERT + 3_000, Observation::Clear),
                (ALERT + 3_000 + DELAY, Observation::Clear),
            ],
        );
        assert_eq!(state, State::Open);
        assert_eq!(
            events,
            [Event::Released {
                held_ms: ALERT + 3_000 + DELAY
            }]
        );

        let t = 2 * ALERT;
        let (state, events) = run(
            state,
            &[(t, game()), (t + ALERT - 1, game()), (t + ALERT, game())],
        );
        assert_eq!(
            events,
            [
                Event::Hold(Hold::Game("cs2.exe".into())),
                Event::Overdue { held_ms: ALERT }
            ]
        );
        assert!(matches!(state, State::Held { overdue: true, .. }));
    }

    #[test]
    fn the_alert_is_recorded_before_a_release_that_happens_on_the_same_reading() {
        let releasing = State::Releasing {
            since: 0,
            until: ALERT,
            overdue: false,
        };
        let (state, events) = step(releasing, Observation::Clear, ALERT, TIMING);
        assert_eq!(state, State::Open);
        assert_eq!(
            events,
            [
                Event::Overdue { held_ms: ALERT },
                Event::Released { held_ms: ALERT }
            ]
        );
    }

    #[test]
    fn at_start_up_a_clear_list_opens_the_gate_with_nothing_to_record() {
        let (state, events) = step(State::starting(100), Observation::Clear, 2_100, TIMING);
        assert_eq!(state, State::Open, "no resume delay when nothing was held");
        assert!(events.is_empty());
    }

    #[test]
    fn at_start_up_a_game_is_a_hold_that_began_at_start_up() {
        let (state, events) = step(State::starting(100), game(), 2_100, TIMING);
        assert_eq!(state, held(100, Hold::Game("cs2.exe".into())));
        assert_eq!(events, [Event::Hold(Hold::Game("cs2.exe".into()))]);
    }

    #[test]
    fn at_start_up_a_failure_is_a_hold_that_began_at_start_up() {
        let (state, events) = step(State::starting(100), Observation::Failed, 2_100, TIMING);
        assert_eq!(state, held(100, Hold::DetectionFailed));
        assert_eq!(events, [Event::Hold(Hold::DetectionFailed)]);
    }

    #[test]
    fn only_open_means_open() {
        assert!(State::Open.is_open());
        assert!(!State::starting(0).is_open());
        assert!(
            !State::Releasing {
                since: 0,
                until: 1,
                overdue: false
            }
            .is_open()
        );
    }
}
