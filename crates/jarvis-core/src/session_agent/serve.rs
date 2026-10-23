//! The Session Agent's side of the channel, run by the `jarvis-session-agent`
//! executable.
//!
//! The agent reads only its standard input, which the Core created and holds
//! the other end of, and writes only to its standard output, which only the
//! Core reads. There is no named endpoint, so nothing else can reach it. It
//! decides nothing: it performs an operation the Core already authorised,
//! once, and refuses a grant it has already seen or one whose lifetime has
//! run out. It never retries, and any malformed frame ends it.

use std::collections::{HashSet, VecDeque};
use std::io::{self, BufRead, Read, Write};
use std::time::Instant;

use jarvis_protocol::{
    AgentDecodeError, AgentHealth, AgentOperation, AgentOutcome, AgentRefusal, AgentToCore,
    AgentVersion, ApprovalId, CoreToAgent, Label, MAX_AGENT_FRAME_BYTES, MAX_GRANT_TTL_MS,
    decode_core_to_agent, encode_agent_frame,
};

/// Grants one agent process remembers, to refuse a replay. The Core's own
/// grant record is the primary defence: it uses a grant up before sending it
/// and revokes every grant when the agent stops.
pub const MAX_REMEMBERED_GRANTS: usize = 1024;

/// The agent's state for one run.
#[derive(Debug, Default)]
pub struct AgentState {
    seen: HashSet<ApprovalId>,
    order: VecDeque<ApprovalId>,
    performed: u64,
}

impl AgentState {
    /// Handle one frame from the Core. An error means the frame was not a
    /// valid `perform`; the agent then stops.
    pub fn handle(&mut self, frame: &[u8]) -> Result<AgentToCore, AgentDecodeError> {
        let received = Instant::now();
        let CoreToAgent::Perform {
            grant_id,
            operation,
            ttl_ms,
            ..
        } = decode_core_to_agent(frame)?;
        let outcome = if !self.remember(grant_id) {
            AgentOutcome::Refused {
                reason: AgentRefusal::ReplayedGrant,
            }
        } else if ttl_ms == 0 || ttl_ms > MAX_GRANT_TTL_MS {
            AgentOutcome::Refused {
                reason: AgentRefusal::Expired,
            }
        } else {
            self.perform(operation, received, ttl_ms)
        };
        Ok(AgentToCore::Result {
            v: AgentVersion::CURRENT,
            grant_id,
            outcome,
        })
    }

    /// Record `grant_id`; false if it was already seen.
    fn remember(&mut self, grant_id: ApprovalId) -> bool {
        if !self.seen.insert(grant_id) {
            return false;
        }
        self.order.push_back(grant_id);
        if self.order.len() > MAX_REMEMBERED_GRANTS
            && let Some(oldest) = self.order.pop_front()
        {
            self.seen.remove(&oldest);
        }
        true
    }

    fn perform(
        &mut self,
        operation: AgentOperation,
        received: Instant,
        ttl_ms: u64,
    ) -> AgentOutcome {
        // The lifetime is counted on this process's own monotonic clock.
        if received.elapsed().as_millis() >= u128::from(ttl_ms) {
            return AgentOutcome::Refused {
                reason: AgentRefusal::Expired,
            };
        }
        match operation {
            AgentOperation::Healthcheck { label } => {
                self.performed += 1;
                AgentOutcome::Done {
                    report: AgentHealth {
                        label,
                        performed: self.performed,
                    },
                }
            }
        }
    }
}

/// Why the agent stopped.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Frame(#[from] AgentDecodeError),
    #[error("cannot encode a reply: {0}")]
    Encode(#[from] serde_json::Error),
}

fn write_frame(output: &mut impl Write, message: &AgentToCore) -> Result<(), ServeError> {
    let mut bytes = encode_agent_frame(message)?;
    bytes.push(b'\n');
    output.write_all(&bytes)?;
    output.flush()?;
    Ok(())
}

/// Read one line of at most [`MAX_AGENT_FRAME_BYTES`]. `None` at end of
/// input.
fn read_frame(input: &mut impl BufRead) -> Result<Option<Vec<u8>>, ServeError> {
    let mut line = Vec::new();
    let limit = u64::try_from(MAX_AGENT_FRAME_BYTES + 2).unwrap_or(u64::MAX);
    let read = Read::take(&mut *input, limit).read_until(b'\n', &mut line)?;
    if read == 0 {
        return Ok(None);
    }
    if line.last() == Some(&b'\n') {
        line.pop();
        if line.last() == Some(&b'\r') {
            line.pop();
        }
    } else if line.len() > MAX_AGENT_FRAME_BYTES {
        return Err(AgentDecodeError::TooLarge.into());
    }
    Ok(Some(line))
}

/// Announce readiness, then answer each `perform` until the Core closes the
/// channel. Returns `Ok` at end of input and an error for anything else.
pub fn serve(
    input: &mut impl BufRead,
    output: &mut impl Write,
    agent_version: &str,
) -> Result<(), ServeError> {
    let version = Label::try_from(agent_version.to_owned())
        .map_err(|error| AgentDecodeError::Malformed(error.to_string()))?;
    write_frame(
        output,
        &AgentToCore::Ready {
            v: AgentVersion::CURRENT,
            agent_version: version,
        },
    )?;
    let mut state = AgentState::default();
    while let Some(frame) = read_frame(input)? {
        let reply = state.handle(&frame)?;
        write_frame(output, &reply)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use jarvis_protocol::{AgentLabel, decode_agent_to_core};
    use serde_json::json;

    use super::*;

    fn perform(grant_id: ApprovalId, ttl_ms: u64) -> Vec<u8> {
        encode_agent_frame(&CoreToAgent::Perform {
            v: AgentVersion::CURRENT,
            grant_id,
            operation: AgentOperation::Healthcheck {
                label: AgentLabel::try_from("ping".to_owned()).unwrap(),
            },
            ttl_ms,
        })
        .unwrap()
    }

    fn outcome(reply: AgentToCore) -> AgentOutcome {
        match reply {
            AgentToCore::Result { outcome, .. } => outcome,
            AgentToCore::Ready { .. } => panic!("not a result"),
        }
    }

    #[test]
    fn a_grant_is_performed_once() {
        let mut state = AgentState::default();
        let grant = ApprovalId::new();
        let first = outcome(state.handle(&perform(grant, 5_000)).unwrap());
        assert!(matches!(first, AgentOutcome::Done { ref report } if report.performed == 1));
        assert_eq!(
            outcome(state.handle(&perform(grant, 5_000)).unwrap()),
            AgentOutcome::Refused {
                reason: AgentRefusal::ReplayedGrant
            }
        );
        for ttl in [0, MAX_GRANT_TTL_MS + 1] {
            assert_eq!(
                outcome(state.handle(&perform(ApprovalId::new(), ttl)).unwrap()),
                AgentOutcome::Refused {
                    reason: AgentRefusal::Expired
                }
            );
        }
    }

    #[test]
    fn the_loop_announces_itself_and_stops_on_a_bad_frame() {
        let grant = ApprovalId::new();
        let mut input = perform(grant, 5_000);
        input.push(b'\n');
        let mut output = Vec::new();
        serve(&mut input.as_slice(), &mut output, "0.2.0").unwrap();
        let lines: Vec<_> = output
            .split(|&b| b == b'\n')
            .filter(|l| !l.is_empty())
            .collect();
        assert_eq!(lines.len(), 2);
        assert!(matches!(
            decode_agent_to_core(lines[0]).unwrap(),
            AgentToCore::Ready { .. }
        ));
        assert!(matches!(
            decode_agent_to_core(lines[1]).unwrap(),
            AgentToCore::Result { grant_id, .. } if grant_id == grant
        ));

        let mut bad = serde_json::to_vec(&json!({"type": "perform", "v": 1, "grant_id": grant,
            "operation": {"op": "run", "command": "cmd.exe"}, "ttl_ms": 5000}))
        .unwrap();
        bad.push(b'\n');
        assert!(serve(&mut bad.as_slice(), &mut Vec::new(), "0.2.0").is_err());

        let mut huge = vec![b'x'; MAX_AGENT_FRAME_BYTES + 10];
        huge.push(b'\n');
        assert!(serve(&mut huge.as_slice(), &mut Vec::new(), "0.2.0").is_err());
    }

    #[test]
    fn remembered_grants_are_bounded() {
        let mut state = AgentState::default();
        for _ in 0..(MAX_REMEMBERED_GRANTS + 10) {
            state.handle(&perform(ApprovalId::new(), 5_000)).unwrap();
        }
        assert_eq!(state.seen.len(), MAX_REMEMBERED_GRANTS);
        assert_eq!(state.order.len(), MAX_REMEMBERED_GRANTS);
    }
}
