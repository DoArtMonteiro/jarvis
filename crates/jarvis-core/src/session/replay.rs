//! Answering a request from the step an earlier attempt of the job completed.

use jarvis_protocol::{
    CoreMessage, ErrorCode, JobStatus, PolicyDecision, Summary, ToolCall, ToolName, ToolOutcome,
    ToolRequest, ToolResponse, ToolResult, WireError,
};
use tokio::io::{AsyncRead, AsyncWrite};

use super::{Session, SessionError, Step};
use crate::policy::required_capabilities;
use crate::store::{JobStep, StoreError, TaskRecord};

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Session<'_, R, W> {
    /// A position an earlier attempt completed is answered from the ledger.
    /// Returns the outcome of doing so, or `None` when `request` has to run
    /// as usual: attempt 1 has no earlier attempt to ask.
    pub(super) async fn replay_if_completed(
        &mut self,
        request: &ToolRequest,
        step: JobStep,
    ) -> Result<Option<Step>, SessionError> {
        if step.attempt <= 1 {
            return Ok(None);
        }
        let (job_id, seq, attempt) = (request.job_id, step.seq, step.attempt);
        let prior = self
            .ctx
            .store
            .call(move |s| s.completed_step(job_id, seq, attempt))
            .await?;
        match prior {
            Some(prior) => self.replay(request, seq, prior).await.map(Some),
            None => Ok(None),
        }
    }

    /// Answer a request with the stored result of the step an earlier attempt
    /// completed at this position. Nothing runs and no task is created. If
    /// the worker no longer asks for the same thing, the job cannot be
    /// resumed safely: it fails and the session ends.
    async fn replay(
        &mut self,
        request: &ToolRequest,
        seq: u32,
        prior: TaskRecord,
    ) -> Result<Step, SessionError> {
        let result = match self.check_replay(request, &prior) {
            Ok(result) => result,
            Err(reason) => return self.replay_mismatch(request, seq, reason).await,
        };
        let (session_id, job_id, task_id) = (self.id, request.job_id, prior.task_id);
        let request_id = request.request_id.clone();
        let recorded = self
            .ctx
            .store
            .call(move |s| s.replay_step(session_id, job_id, seq, task_id, &request_id))
            .await;
        match recorded {
            Ok(()) => {}
            Err(StoreError::DuplicateRequest(request_id)) => {
                return self.duplicate_request(request_id).await;
            }
            Err(error) => return Err(error.into()),
        }
        self.requests += 1;
        self.advance_to(seq);
        self.used.insert(request.request_id.clone());
        tracing::info!(session = %self.id, task = %task_id, seq, "step replayed");
        let response = CoreMessage::ToolResponse(ToolResponse {
            request_id: request.request_id.clone(),
            task_id,
            outcome: ToolOutcome::Completed { result },
        });
        self.send(&response).await
    }

    /// The stored result of `prior` if `request` asks for exactly what
    /// `prior` completed, current policy still allows it and the stored
    /// result is valid. Otherwise why not, in words that do not repeat
    /// anything the worker sent.
    fn check_replay(
        &self,
        request: &ToolRequest,
        prior: &TaskRecord,
    ) -> Result<ToolResult, &'static str> {
        let asked = ToolCall::from_request(&request.tool, &request.args)
            .map_err(|_| "the request is not a valid call")?;
        let stored_tool =
            ToolName::try_from(prior.tool.clone()).map_err(|_| "the stored step is unreadable")?;
        let stored_args =
            serde_json::from_str(&prior.args).map_err(|_| "the stored step is unreadable")?;
        let stored = ToolCall::from_request(&stored_tool, &stored_args)
            .map_err(|_| "the stored step is unreadable")?;
        if asked != stored {
            return Err("the request differs from the step that completed");
        }
        let decision = self
            .ctx
            .policy
            .evaluate(&required_capabilities(&asked))
            .decision;
        if decision == PolicyDecision::Deny {
            return Err("policy no longer allows the step");
        }
        let json = prior
            .result
            .as_deref()
            .filter(|json| json.len() <= self.ctx.limits.max_result_bytes())
            .ok_or("the stored result is missing or too large")?;
        let result: ToolResult =
            serde_json::from_str(json).map_err(|_| "the stored result is invalid")?;
        if !result.answers(&asked) {
            return Err("the stored result does not answer the request");
        }
        Ok(result)
    }

    async fn replay_mismatch(
        &mut self,
        request: &ToolRequest,
        seq: u32,
        reason: &str,
    ) -> Result<Step, SessionError> {
        tracing::warn!(session = %self.id, seq, "cannot replay a completed step: {reason}");
        if let Some(job) = self.job.take() {
            let summary = Summary::lossy(&format!("step {seq} could not be replayed: {reason}"));
            self.finish_job(job.id, JobStatus::Failed, Some(summary))
                .await?;
        }
        let error = WireError::new(
            ErrorCode::UnexpectedMessage,
            format!(
                "step {seq} of this job already completed and this request does not match it: {reason}"
            ),
        );
        self.terminate(Some(request.request_id.clone()), error)
            .await
    }
}
