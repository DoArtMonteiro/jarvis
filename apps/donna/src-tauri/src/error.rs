//! The one error type the frontend sees. It is serialised with a `kind` tag
//! so the UI can pick a state (Core unreachable, version mismatch, ...)
//! without parsing messages.

use jarvis_core::rpc::client::ClientError;
use jarvis_protocol::{RpcError, RpcResponse};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DonnaError {
    /// The Core does not answer on its endpoint.
    Unreachable {
        message: String,
    },
    /// The Core speaks another RPC version than this app.
    VersionMismatch {
        found: u64,
        expected: u32,
    },
    Timeout {
        message: String,
    },
    /// The Core answered with an error.
    Rpc {
        code: String,
        message: String,
    },
    /// Input rejected in the app, before anything was sent.
    InvalidInput {
        /// `text_empty`, `text_too_long`, `text_invalid`, `request_too_large` or `invalid`.
        code: String,
        message: String,
    },
    /// The configuration file could not be read.
    Config {
        message: String,
        path: String,
    },
    /// The Core could not be started by the app.
    Launch {
        message: String,
    },
    /// A reply that does not fit the request, or another internal fault.
    Unexpected {
        message: String,
    },
}

impl DonnaError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::coded("invalid", message)
    }

    pub fn coded(code: &str, message: impl Into<String>) -> Self {
        Self::InvalidInput {
            code: code.to_owned(),
            message: message.into(),
        }
    }

    pub fn unexpected(response: &RpcResponse) -> Self {
        // The variant name only: a reply is never echoed into the UI.
        let name = match response {
            RpcResponse::Health(_) => "health",
            RpcResponse::Job(_) => "job",
            RpcResponse::Jobs { .. } => "jobs",
            RpcResponse::Approval(_) => "approval",
            RpcResponse::Approvals { .. } => "approvals",
            RpcResponse::ShuttingDown {} => "shutting_down",
            RpcResponse::ChatTurn(_) => "chat_turn",
            RpcResponse::ChatDelta(_) => "chat_delta",
            RpcResponse::Conversations { .. } => "conversations",
            RpcResponse::Conversation(_) => "conversation",
            RpcResponse::Deleted { .. } => "deleted",
            RpcResponse::Inference(_) => "inference",
            RpcResponse::Error(_) => "error",
        };
        Self::Unexpected {
            message: format!("unexpected reply from the Core: {name}"),
        }
    }

    /// True when the Core could not be reached at all (as opposed to
    /// answering with an error).
    pub fn is_unreachable(&self) -> bool {
        matches!(self, Self::Unreachable { .. })
    }
}

impl From<ClientError> for DonnaError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::VersionMismatch { found, expected } => {
                Self::VersionMismatch { found, expected }
            }
            ClientError::Timeout => Self::Timeout {
                message: error.to_string(),
            },
            ClientError::Connect { .. } | ClientError::Io(_) | ClientError::Closed => {
                Self::Unreachable {
                    message: error.to_string(),
                }
            }
            ClientError::Invalid(_) => Self::Unexpected {
                message: error.to_string(),
            },
        }
    }
}

impl From<RpcError> for DonnaError {
    fn from(error: RpcError) -> Self {
        Self::Rpc {
            code: error.code.to_string(),
            message: error.message,
        }
    }
}

impl std::fmt::Display for DonnaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable { message }
            | Self::Timeout { message }
            | Self::InvalidInput { message, .. }
            | Self::Config { message, .. }
            | Self::Launch { message }
            | Self::Unexpected { message } => f.write_str(message),
            Self::VersionMismatch { found, expected } => {
                write!(f, "Core speaks RPC {found}, this app speaks {expected}")
            }
            Self::Rpc { code, message } => write!(f, "{code}: {message}"),
        }
    }
}
