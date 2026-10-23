//! Operations performed by the Session Agent, and the messages between the
//! Core and the agent.
//!
//! The Session Agent is a trusted process of the user that the Core starts
//! and talks to over pipes it creates (ADR 0017). A worker can ask for an
//! agent operation only as a typed tool call, which goes through policy and,
//! because every agent capability is class B, a person's approval. What
//! crosses the agent channel is a single-use grant for one exact operation.
//!
//! Desktop operations are defined here so that a person can be shown exactly
//! what one would do, but none can be performed yet: the Core refuses them
//! before they reach the agent, and the agent's operation set has no variant
//! for them.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::text::check_visible;
use crate::{ApprovalId, Label};

macro_rules! visible_text {
    ($(#[$doc:meta])* $name:ident, $kind:literal, min = $min:expr, max = $max:expr, layout = $layout:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Maximum length in characters.
            pub const MAX_CHARS: usize = $max;

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                check_visible($kind, &value, $min, $max, $layout)?;
                Ok(Self(value))
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

visible_text!(
    /// A short label a health check echoes back. Short enough to be shown to
    /// a person in full.
    AgentLabel,
    "label",
    min = 1,
    max = 64,
    layout = false
);

visible_text!(
    /// Clipboard text: bounded, visible, and no control characters other than
    /// tab and newline.
    ClipboardText,
    "clipboard text",
    min = 0,
    max = 4096,
    layout = true
);

visible_text!(
    /// Text to type: bounded, visible, and no control characters at all.
    /// Typed, a newline or a tab would be Enter or Tab, which can submit a
    /// form or move focus; nor can an escape sequence or a key chord be
    /// expressed.
    TypedText,
    "typed text",
    min = 1,
    max = 4096,
    layout = false
);

/// A window the person would pick through the Session Agent's own picker.
/// There is no picker yet, so no value is valid for any operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WindowTarget(pub u64);

impl fmt::Display for WindowTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "window #{}", self.0)
    }
}

/// Arguments of `session_agent.healthcheck`: run the Session Agent's
/// synthetic operation, which touches no desktop or personal data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthcheckArgs {
    pub label: AgentLabel,
}

/// Arguments of `desktop.capture_window`: one image of one window, never the
/// whole screen. Not performable yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureWindowArgs {
    pub target: WindowTarget,
}

/// Arguments of `desktop.read_clipboard_text`. Not performable yet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadClipboardTextArgs {}

/// Arguments of `desktop.write_clipboard_text`. Not performable yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteClipboardTextArgs {
    pub text: ClipboardText,
}

/// Arguments of `desktop.type_text`: type text into one picked window. No
/// key chords, no other window. Not performable yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeTextArgs {
    pub target: WindowTarget,
    pub text: TypedText,
}

/// What the worker learns when an agent operation completed. The agent's own
/// report goes to the Core only and is kept in the audit log: results of
/// agent operations may one day be personal data, so none is ever sent to
/// the worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentReceipt {
    /// The tool that completed, for example `session_agent.healthcheck`.
    pub agent_operation: String,
}

/// Version of the Core-to-agent channel. Both ends ship together, so each
/// accepts only its own version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentVersion(pub u32);

impl AgentVersion {
    pub const CURRENT: Self = Self(1);
}

/// The longest a grant may live, in milliseconds.
pub const MAX_GRANT_TTL_MS: u64 = 60_000;

/// The largest frame either side of the agent channel accepts, in bytes.
pub const MAX_AGENT_FRAME_BYTES: usize = 16 * 1024;

/// An operation the Session Agent can perform. Only the synthetic health
/// check exists: there is deliberately no variant for any desktop
/// operation, so none can be sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentOperation {
    Healthcheck { label: AgentLabel },
}

/// The one message the Core sends: perform this operation once, under the
/// grant named by the approval it came from, within `ttl_ms` of receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoreToAgent {
    Perform {
        v: AgentVersion,
        grant_id: ApprovalId,
        operation: AgentOperation,
        ttl_ms: u64,
    },
}

/// What the agent reports for a health check. Nothing about the desktop,
/// the user or the machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentHealth {
    /// The label from the request, unchanged.
    pub label: AgentLabel,
    /// Operations this agent process has performed, this one included.
    pub performed: u64,
}

/// Why the agent did not perform an operation. Each one means it did
/// nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRefusal {
    /// This agent has already seen the grant.
    ReplayedGrant,
    /// The grant's remaining lifetime was zero or out of range.
    Expired,
}

impl AgentRefusal {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReplayedGrant => "replayed_grant",
            Self::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentOutcome {
    Done { report: AgentHealth },
    Refused { reason: AgentRefusal },
}

/// Messages the agent sends: `ready` once at start, then one `result` per
/// `perform`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentToCore {
    Ready {
        v: AgentVersion,
        agent_version: Label,
    },
    Result {
        v: AgentVersion,
        grant_id: ApprovalId,
        outcome: AgentOutcome,
    },
}

/// Why an agent-channel frame was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentDecodeError {
    #[error("frame is larger than {MAX_AGENT_FRAME_BYTES} bytes")]
    TooLarge,
    #[error("malformed frame: {0}")]
    Malformed(String),
    #[error("unsupported agent channel version {0}")]
    UnsupportedVersion(u32),
}

fn decode<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, AgentDecodeError> {
    if bytes.len() > MAX_AGENT_FRAME_BYTES {
        return Err(AgentDecodeError::TooLarge);
    }
    serde_json::from_slice(bytes).map_err(|error| AgentDecodeError::Malformed(error.to_string()))
}

fn check_version(v: AgentVersion) -> Result<(), AgentDecodeError> {
    if v == AgentVersion::CURRENT {
        Ok(())
    } else {
        Err(AgentDecodeError::UnsupportedVersion(v.0))
    }
}

/// Decode a frame the agent received. Strict: unknown fields, unknown
/// operations and other versions are errors.
pub fn decode_core_to_agent(bytes: &[u8]) -> Result<CoreToAgent, AgentDecodeError> {
    let message: CoreToAgent = decode(bytes)?;
    match &message {
        CoreToAgent::Perform { v, .. } => check_version(*v)?,
    }
    Ok(message)
}

/// Decode a frame the Core received from the agent. Strict, as above.
pub fn decode_agent_to_core(bytes: &[u8]) -> Result<AgentToCore, AgentDecodeError> {
    let message: AgentToCore = decode(bytes)?;
    match &message {
        AgentToCore::Ready { v, .. } | AgentToCore::Result { v, .. } => check_version(*v)?,
    }
    Ok(message)
}

/// Encode one frame for the agent channel, without the trailing newline.
pub fn encode_agent_frame<T: Serialize>(message: &T) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(message)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn approved_text_is_visible_and_bounded() {
        assert!(AgentLabel::try_from("ping 1".to_owned()).is_ok());
        let long = "x".repeat(65);
        for bad in ["", "a\nb", "pay\u{202E}evil", "a\u{200B}b", long.as_str()] {
            assert!(AgentLabel::try_from(bad.to_owned()).is_err(), "{bad:?}");
        }
        for bad in [
            "\u{1b}[A",
            "a\u{8}",
            "\r",
            "send\n",
            "a\tb",
            "\u{FEFF}x",
            "",
        ] {
            assert!(TypedText::try_from(bad.to_owned()).is_err(), "{bad:?}");
        }
        assert!(ClipboardText::try_from("line\n\tindented".to_owned()).is_ok());
        assert!(ClipboardText::try_from("\r".to_owned()).is_err());
        assert!(TypedText::try_from("x".repeat(4097)).is_err());
    }

    #[test]
    fn perform_round_trips_and_is_strict() {
        let grant_id = ApprovalId::new();
        let perform = CoreToAgent::Perform {
            v: AgentVersion::CURRENT,
            grant_id,
            operation: AgentOperation::Healthcheck {
                label: AgentLabel::try_from("ping".to_owned()).unwrap(),
            },
            ttl_ms: 5_000,
        };
        let bytes = encode_agent_frame(&perform).unwrap();
        assert_eq!(decode_core_to_agent(&bytes).unwrap(), perform);

        let base = json!({"type": "perform", "v": 1, "grant_id": grant_id,
            "operation": {"op": "healthcheck", "label": "ping"}, "ttl_ms": 5000});
        let mut extra = base.clone();
        extra["caller"] = json!("core");
        let mut desktop = base.clone();
        desktop["operation"] = json!({"op": "capture_window", "target": 1});
        let mut smuggled = base.clone();
        smuggled["operation"]["command"] = json!("cmd.exe");
        let mut version = base.clone();
        version["v"] = json!(2);
        for (frame, case) in [
            (extra, "unknown field"),
            (desktop, "desktop operation"),
            (smuggled, "field inside the operation"),
        ] {
            let bytes = serde_json::to_vec(&frame).unwrap();
            assert!(
                matches!(
                    decode_core_to_agent(&bytes),
                    Err(AgentDecodeError::Malformed(_))
                ),
                "{case}"
            );
        }
        let bytes = serde_json::to_vec(&version).unwrap();
        assert_eq!(
            decode_core_to_agent(&bytes),
            Err(AgentDecodeError::UnsupportedVersion(2))
        );
        let huge = vec![b' '; MAX_AGENT_FRAME_BYTES + 1];
        assert_eq!(decode_core_to_agent(&huge), Err(AgentDecodeError::TooLarge));
    }

    #[test]
    fn agent_replies_are_strict() {
        let grant_id = ApprovalId::new();
        let done = AgentToCore::Result {
            v: AgentVersion::CURRENT,
            grant_id,
            outcome: AgentOutcome::Done {
                report: AgentHealth {
                    label: AgentLabel::try_from("ping".to_owned()).unwrap(),
                    performed: 1,
                },
            },
        };
        let bytes = encode_agent_frame(&done).unwrap();
        assert_eq!(decode_agent_to_core(&bytes).unwrap(), done);
        let refused = json!({"type": "result", "v": 1, "grant_id": grant_id,
            "outcome": {"status": "refused", "reason": "replayed_grant"}});
        assert!(decode_agent_to_core(&serde_json::to_vec(&refused).unwrap()).is_ok());
        let screenshot = json!({"type": "result", "v": 1, "grant_id": grant_id,
            "outcome": {"status": "done", "report": {"label": "x", "performed": 1, "image": "AAAA"}}});
        assert!(decode_agent_to_core(&serde_json::to_vec(&screenshot).unwrap()).is_err());
    }
}
