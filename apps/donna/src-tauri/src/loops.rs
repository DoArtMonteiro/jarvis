//! Background loops that turn the Core's long polls into UI events.

use std::sync::Arc;
use std::time::Duration;

use jarvis_core::rpc::client::Client;
use jarvis_protocol::{
    ApprovalView, ChatDelta, HealthReport, InferenceView, MessageId, RpcRequest, RpcResponse,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::DonnaError;
use crate::state::{AppState, Boot, into_result, locked};

const STATUS_INTERVAL: Duration = Duration::from_secs(2);
const RETRY_DELAY: Duration = Duration::from_secs(2);
const APPROVALS_BUSY_DELAY: Duration = Duration::from_millis(1500);
const CHAT_WAIT_MS: u32 = 20_000;
const APPROVALS_WAIT_MS: u32 = 25_000;
const MAX_STREAM_FAILURES: u32 = 5;

#[derive(Debug, Clone, Serialize)]
struct DeltaEvent {
    turn_id: String,
    conversation_id: Option<String>,
    /// Byte offset the delta starts at; 0 means "replace what you have".
    from_cursor: u32,
    delta: ChatDelta,
}

#[derive(Debug, Clone, Serialize)]
struct StreamError {
    turn_id: String,
    conversation_id: Option<String>,
    error: DonnaError,
}

/// Poll one reply until it is final, emitting `chat-delta` events. At most
/// one task follows a given turn.
pub fn follow(
    app: AppHandle,
    state: Arc<AppState>,
    turn_id: MessageId,
    conversation_id: Option<String>,
) {
    let key = turn_id.to_string();
    if !locked(&state.following).insert(key.clone()) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let result = follow_loop(&app, &state, turn_id, &key, conversation_id.clone()).await;
        locked(&state.following).remove(&key);
        if let Err(error) = result {
            let _ = app.emit(
                "chat-error",
                StreamError {
                    turn_id: key,
                    conversation_id,
                    error,
                },
            );
        }
        state.status_wake.notify_one();
    });
}

async fn follow_loop(
    app: &AppHandle,
    state: &AppState,
    turn_id: MessageId,
    key: &str,
    conversation_id: Option<String>,
) -> Result<(), DonnaError> {
    let mut cursor = 0u32;
    let mut failures = 0u32;
    let mut client: Option<Client> = None;
    while !state.is_closing() {
        let mut connection = match client.take() {
            Some(connection) => connection,
            None => match state.connect().await {
                Ok(connection) => connection,
                Err(error) => {
                    failures += 1;
                    if failures > MAX_STREAM_FAILURES {
                        return Err(error);
                    }
                    tokio::time::sleep(Duration::from_secs(u64::from(failures))).await;
                    continue;
                }
            },
        };
        let request = RpcRequest::ChatPoll {
            turn_id,
            cursor,
            wait_ms: CHAT_WAIT_MS,
        };
        let response = match connection.call(&request).await {
            Ok(response) => into_result(response)?,
            Err(error) => {
                failures += 1;
                if failures > MAX_STREAM_FAILURES {
                    return Err(error.into());
                }
                tokio::time::sleep(Duration::from_secs(u64::from(failures))).await;
                continue;
            }
        };
        let RpcResponse::ChatDelta(delta) = response else {
            return Err(DonnaError::unexpected(&response));
        };
        failures = 0;
        let is_final = delta.status.is_final();
        let from_cursor = cursor;
        cursor = delta.next_cursor;
        if is_final || !delta.text.is_empty() {
            let _ = app.emit(
                "chat-delta",
                DeltaEvent {
                    turn_id: key.to_owned(),
                    conversation_id: conversation_id.clone(),
                    from_cursor,
                    delta,
                },
            );
        }
        if is_final {
            return Ok(());
        }
        client = Some(connection);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
struct ApprovalsEvent {
    approvals: Vec<ApprovalView>,
}

/// Long-poll pending approvals and emit `approvals` when the set changes.
pub async fn approvals_loop(app: AppHandle, state: Arc<AppState>) {
    let mut client: Option<Client> = None;
    let mut last: Option<Vec<(String, String)>> = None;
    let mut busy = false;
    while !state.is_closing() {
        let mut connection = match client.take() {
            Some(connection) => connection,
            None => match state.connect().await {
                Ok(connection) => connection,
                Err(_) => {
                    tokio::time::sleep(RETRY_DELAY).await;
                    continue;
                }
            },
        };
        let request = RpcRequest::ListApprovals {
            wait_ms: if busy { 0 } else { APPROVALS_WAIT_MS },
        };
        let approvals = match connection.call(&request).await.map(into_result) {
            Ok(Ok(RpcResponse::Approvals { approvals })) => approvals,
            _ => {
                tokio::time::sleep(RETRY_DELAY).await;
                continue;
            }
        };
        let signature: Vec<(String, String)> = approvals
            .iter()
            .map(|view| {
                (
                    view.approval_id.to_string(),
                    view.fingerprint.as_str().to_owned(),
                )
            })
            .collect();
        busy = !approvals.is_empty();
        if last.as_ref() != Some(&signature) {
            last = Some(signature);
            let _ = app.emit("approvals", ApprovalsEvent { approvals });
        }
        client = Some(connection);
        if busy {
            // The Core answers at once while anything is pending.
            tokio::select! {
                () = tokio::time::sleep(APPROVALS_BUSY_DELAY) => {}
                () = state.approvals_wake.notified() => {}
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
enum CoreStatus {
    Ok,
    Starting,
    Unreachable,
    VersionMismatch,
    ConfigError,
    LaunchFailed,
}

#[derive(Debug, Clone, Serialize)]
struct StatusEvent {
    core: CoreStatus,
    error: Option<DonnaError>,
    health: Option<HealthReport>,
    inference: Option<InferenceView>,
    inference_error: Option<String>,
}

impl StatusEvent {
    fn down(core: CoreStatus, error: Option<DonnaError>) -> Self {
        Self {
            core,
            error,
            health: None,
            inference: None,
            inference_error: None,
        }
    }
}

async fn gather(state: &AppState, client: &mut Option<Client>) -> StatusEvent {
    let mut connection = match client.take() {
        Some(connection) => connection,
        None => match state.connect().await {
            Ok(connection) => connection,
            Err(error) => return failure(state, &error),
        },
    };
    let health = match connection.call(&RpcRequest::Health {}).await {
        Ok(response) => match into_result(response) {
            Ok(RpcResponse::Health(report)) => report,
            Ok(other) => {
                return StatusEvent::down(
                    CoreStatus::Unreachable,
                    Some(DonnaError::unexpected(&other)),
                );
            }
            Err(error) => return StatusEvent::down(CoreStatus::Unreachable, Some(error)),
        },
        Err(error) => return failure(state, &error.into()),
    };
    let (inference, inference_error) = match connection.call(&RpcRequest::InferenceStatus {}).await
    {
        Ok(response) => match into_result(response) {
            Ok(RpcResponse::Inference(view)) => (Some(view), None),
            Ok(other) => (None, Some(DonnaError::unexpected(&other).to_string())),
            Err(error) => (None, Some(error.to_string())),
        },
        Err(error) => return failure(state, &error.into()),
    };
    *client = Some(connection);
    {
        let mut boot = locked(&state.boot);
        if matches!(*boot, Boot::Failed(_)) {
            *boot = Boot::Idle;
        }
    }
    StatusEvent {
        core: CoreStatus::Ok,
        error: None,
        health: Some(health),
        inference,
        inference_error,
    }
}

fn failure(state: &AppState, error: &DonnaError) -> StatusEvent {
    match error {
        DonnaError::Config { .. } => {
            StatusEvent::down(CoreStatus::ConfigError, Some(error.clone()))
        }
        DonnaError::VersionMismatch { .. } => {
            StatusEvent::down(CoreStatus::VersionMismatch, Some(error.clone()))
        }
        _ => match locked(&state.boot).clone() {
            Boot::Starting => StatusEvent::down(CoreStatus::Starting, None),
            Boot::Failed(message) => StatusEvent::down(
                CoreStatus::LaunchFailed,
                Some(DonnaError::Launch { message }),
            ),
            Boot::Idle => StatusEvent::down(CoreStatus::Unreachable, Some(error.clone())),
        },
    }
}

/// Poll Health and InferenceStatus and emit `status` every couple of
/// seconds, or sooner when something asks for a refresh.
pub async fn status_loop(app: AppHandle, state: Arc<AppState>) {
    let mut client: Option<Client> = None;
    while !state.is_closing() {
        let event = gather(&state, &mut client).await;
        let _ = app.emit("status", event);
        tokio::select! {
            () = tokio::time::sleep(STATUS_INTERVAL) => {}
            () = state.status_wake.notified() => {}
        }
    }
}
