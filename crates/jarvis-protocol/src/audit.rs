use serde::{Deserialize, Serialize};

use crate::message::Label;
use crate::{
    AgentReport, ApprovalId, Capability, Fingerprint, GovernorReason, JobId, JobStatus,
    ModelProfile, PolicyDecision, ProtocolVersion, RequestId, SessionId, TaskId, TaskStatus,
    ToolName, WireError,
};

/// One entry of the append-only audit log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEvent {
    /// Monotonic sequence number assigned by the store.
    pub seq: i64,
    /// RFC 3339 timestamp in UTC.
    pub at: String,
    pub session_id: Option<SessionId>,
    pub task_id: Option<TaskId>,
    pub event: AuditEventKind,
}

/// What happened. Stored as JSON with a `kind` tag, so the audit log can be
/// read back into these types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuditEventKind {
    CoreStarted {
        core_version: String,
        protocol_version: ProtocolVersion,
    },
    CoreStopped {
        reason: String,
    },
    /// A task left open by a previous run was closed during startup.
    TaskRecovered {
        from: TaskStatus,
        to: TaskStatus,
    },
    WorkerSpawned {
        pid: Option<u32>,
    },
    WorkerExited {
        code: Option<i32>,
        success: bool,
    },
    WorkerKilled {
        reason: String,
    },
    SessionOpened {
        worker: Label,
        worker_version: Label,
    },
    SessionClosed {
        reason: String,
        requests: u32,
    },
    /// A frame that did not create a task.
    FrameRejected {
        request_id: Option<RequestId>,
        error: WireError,
    },
    RequestReceived {
        request_id: RequestId,
        tool: ToolName,
    },
    /// The request named an unknown tool or had invalid arguments.
    RequestRejected {
        error: WireError,
    },
    PolicyEvaluated {
        capabilities: Vec<Capability>,
        decision: PolicyDecision,
    },
    /// Committed before the tool runs. If this event cannot be written, the
    /// tool does not run.
    ExecutionStarted {
        tool: String,
    },
    ExecutionFinished {
        status: TaskStatus,
        duration_ms: u64,
        error: Option<WireError>,
    },
    TaskExpired {
        reason: String,
    },
    /// The worker exited unexpectedly and will be started again.
    WorkerRestartScheduled {
        attempt: u32,
        delay_ms: u64,
    },
    /// The worker failed too often; the Core stops restarting it.
    WorkerRestartAbandoned {
        restarts: u32,
        window_ms: u64,
    },
    /// A local client asked for a job. The goal itself is kept in the jobs
    /// table, not repeated here.
    JobSubmitted {
        job_id: JobId,
        client: String,
    },
    JobStarted {
        job_id: JobId,
    },
    JobFinished {
        job_id: JobId,
        status: JobStatus,
    },
    /// A running job went back to the queue because its worker or the Core
    /// stopped. `attempts` is how many times it has been claimed so far.
    JobRequeued {
        job_id: JobId,
        attempts: u32,
        reason: String,
    },
    /// A job was ended instead of resumed because `task_id` started
    /// executing and its outcome was not recorded.
    JobQuarantined {
        job_id: JobId,
        task_id: TaskId,
    },
    /// A request was answered with the stored result of the completed task
    /// `task_id` at position `seq`. No task was created and no tool ran.
    StepReplayed {
        job_id: JobId,
        seq: u32,
        task_id: TaskId,
    },
    ApprovalRequested {
        approval_id: ApprovalId,
        tool: ToolName,
        capabilities: Vec<Capability>,
        fingerprint: Fingerprint,
        expires_at: String,
    },
    /// A person approved the request through the local RPC interface.
    ApprovalGranted {
        approval_id: ApprovalId,
        client: String,
    },
    ApprovalDenied {
        approval_id: ApprovalId,
        client: String,
    },
    ApprovalExpired {
        approval_id: ApprovalId,
        reason: String,
    },
    /// Committed together with `execution_started`: the approval has been
    /// used and can never be used again.
    ApprovalConsumed {
        approval_id: ApprovalId,
    },
    /// The local model server started and answers (ADR 0021). Metadata
    /// only: conversations are never recorded in the audit log.
    InferenceLoaded {
        profile: ModelProfile,
        model_sha256: String,
        context_size: u32,
        gpu_layers: u32,
        load_ms: u64,
    },
    /// The local model server stopped: on request, for a game, at shutdown,
    /// or because it failed.
    InferenceStopped {
        reason: String,
    },
    /// The gaming governor stopped queued jobs from starting. `process` is
    /// the configured name that matched when `reason` is `game`; nothing
    /// else about the process list is recorded. Jobs already running are
    /// not affected.
    GovernorHold {
        reason: GovernorReason,
        process: Option<String>,
    },
    /// Queued jobs may start again. `held_ms` counts from the hold, resume
    /// delay included. Recorded before the jobs are released.
    GovernorReleased {
        held_ms: u64,
    },
    /// A hold has lasted longer than the configured alert time. Recorded
    /// once per hold. Jobs keep waiting: the governor never releases
    /// because of elapsed time.
    GovernorHoldOverdue {
        held_ms: u64,
    },
    /// A connection to the RPC interface was refused, for example because
    /// it came from the worker process.
    RpcClientRejected {
        client: String,
        reason: String,
    },
    /// The Core started its Session Agent. `image_sha256` is the SHA-256
    /// of the executable the Core started, when it could read it: a record
    /// for later checks, not a defence against a same-user process.
    SessionAgentStarted {
        pid: Option<u32>,
        image_sha256: Option<Fingerprint>,
    },
    /// The Core stopped its Session Agent; every live grant was revoked.
    SessionAgentStopped {
        reason: String,
    },
    /// The approval `grant_id` is being used for a Session Agent operation.
    /// Committed with the approval's consumption, before a grant is issued
    /// or anything is sent. It is followed by `session_agent_completed`,
    /// `session_agent_refused` (nothing was sent, or the agent refused) or
    /// `session_agent_outcome_unknown`.
    SessionAgentDispatched {
        grant_id: ApprovalId,
        tool: String,
        ttl_ms: u64,
    },
    /// The Session Agent performed the operation. The report is for the
    /// Core and the person; the worker only learns that it completed.
    SessionAgentCompleted {
        grant_id: ApprovalId,
        report: AgentReport,
    },
    /// The operation was not performed: the Core or the agent refused it.
    SessionAgentRefused {
        grant_id: Option<ApprovalId>,
        reason: String,
    },
    /// The operation was sent and no valid answer came back. Whether it was
    /// performed is unknown; it is never sent again.
    SessionAgentOutcomeUnknown {
        grant_id: ApprovalId,
        reason: String,
    },
}

impl AuditEventKind {
    /// Value of the `kind` column, identical to the serialised tag.
    pub fn name(&self) -> &'static str {
        match self {
            Self::CoreStarted { .. } => "core_started",
            Self::CoreStopped { .. } => "core_stopped",
            Self::TaskRecovered { .. } => "task_recovered",
            Self::WorkerSpawned { .. } => "worker_spawned",
            Self::WorkerExited { .. } => "worker_exited",
            Self::WorkerKilled { .. } => "worker_killed",
            Self::SessionOpened { .. } => "session_opened",
            Self::SessionClosed { .. } => "session_closed",
            Self::FrameRejected { .. } => "frame_rejected",
            Self::RequestReceived { .. } => "request_received",
            Self::RequestRejected { .. } => "request_rejected",
            Self::PolicyEvaluated { .. } => "policy_evaluated",
            Self::ExecutionStarted { .. } => "execution_started",
            Self::ExecutionFinished { .. } => "execution_finished",
            Self::TaskExpired { .. } => "task_expired",
            Self::WorkerRestartScheduled { .. } => "worker_restart_scheduled",
            Self::WorkerRestartAbandoned { .. } => "worker_restart_abandoned",
            Self::JobSubmitted { .. } => "job_submitted",
            Self::JobStarted { .. } => "job_started",
            Self::JobFinished { .. } => "job_finished",
            Self::JobRequeued { .. } => "job_requeued",
            Self::JobQuarantined { .. } => "job_quarantined",
            Self::StepReplayed { .. } => "step_replayed",
            Self::ApprovalRequested { .. } => "approval_requested",
            Self::ApprovalGranted { .. } => "approval_granted",
            Self::ApprovalDenied { .. } => "approval_denied",
            Self::ApprovalExpired { .. } => "approval_expired",
            Self::ApprovalConsumed { .. } => "approval_consumed",
            Self::InferenceLoaded { .. } => "inference_loaded",
            Self::InferenceStopped { .. } => "inference_stopped",
            Self::GovernorHold { .. } => "governor_hold",
            Self::GovernorReleased { .. } => "governor_released",
            Self::GovernorHoldOverdue { .. } => "governor_hold_overdue",
            Self::RpcClientRejected { .. } => "rpc_client_rejected",
            Self::SessionAgentStarted { .. } => "session_agent_started",
            Self::SessionAgentStopped { .. } => "session_agent_stopped",
            Self::SessionAgentDispatched { .. } => "session_agent_dispatched",
            Self::SessionAgentCompleted { .. } => "session_agent_completed",
            Self::SessionAgentRefused { .. } => "session_agent_refused",
            Self::SessionAgentOutcomeUnknown { .. } => "session_agent_outcome_unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_matches_serialised_tag() {
        let events = [
            AuditEventKind::CoreStopped {
                reason: "test".into(),
            },
            AuditEventKind::PolicyEvaluated {
                capabilities: vec![Capability::SystemInfo],
                decision: PolicyDecision::Allow,
            },
            AuditEventKind::ExecutionStarted {
                tool: "system.info".into(),
            },
            AuditEventKind::ApprovalConsumed {
                approval_id: ApprovalId::new(),
            },
            AuditEventKind::WorkerRestartScheduled {
                attempt: 1,
                delay_ms: 500,
            },
            AuditEventKind::JobFinished {
                job_id: JobId::new(),
                status: JobStatus::Completed,
            },
            AuditEventKind::JobRequeued {
                job_id: JobId::new(),
                attempts: 1,
                reason: "worker stream failed".into(),
            },
            AuditEventKind::JobQuarantined {
                job_id: JobId::new(),
                task_id: TaskId::new(),
            },
            AuditEventKind::StepReplayed {
                job_id: JobId::new(),
                seq: 2,
                task_id: TaskId::new(),
            },
            AuditEventKind::GovernorHold {
                reason: GovernorReason::Game,
                process: Some("cs2.exe".into()),
            },
            AuditEventKind::GovernorHold {
                reason: GovernorReason::DetectionFailed,
                process: None,
            },
            AuditEventKind::GovernorReleased { held_ms: 61_000 },
            AuditEventKind::GovernorHoldOverdue {
                held_ms: 14_400_000,
            },
            AuditEventKind::SessionAgentDispatched {
                grant_id: ApprovalId::new(),
                tool: "session_agent.healthcheck".into(),
                ttl_ms: 5_000,
            },
            AuditEventKind::SessionAgentCompleted {
                grant_id: ApprovalId::new(),
                report: AgentReport::Healthcheck(crate::AgentHealth {
                    label: crate::AgentLabel::try_from("ping".to_owned()).unwrap(),
                    performed: 1,
                }),
            },
            AuditEventKind::SessionAgentRefused {
                grant_id: None,
                reason: "desktop operations are not available".into(),
            },
            AuditEventKind::SessionAgentOutcomeUnknown {
                grant_id: ApprovalId::new(),
                reason: "the agent exited".into(),
            },
        ];
        for event in events {
            let value = serde_json::to_value(&event).unwrap();
            assert_eq!(value["kind"], event.name());
            let back: AuditEventKind = serde_json::from_value(value).unwrap();
            assert_eq!(back, event);
        }
    }
}
