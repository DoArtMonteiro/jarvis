//! The job a session is working on: handing it out, ending it, and settling
//! it when the session ends early (ADR 0015).

use jarvis_protocol::{
    CoreMessage, ErrorCode, JobAssignment, JobId, JobOutcome, JobResult, JobStatus, Summary,
    WireError, WorkerState,
};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::time::Instant;

use super::{RunningJob, Session, SessionEnd, SessionError, Step};

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Session<'_, R, W> {
    pub(super) async fn dispatch_next_job(&mut self) -> Result<Step, SessionError> {
        let session_id = self.id;
        let Some(job) = self
            .ctx
            .store
            .call(move |s| s.claim_next_job(session_id))
            .await?
        else {
            return Ok(None);
        };
        tracing::info!(session = %self.id, job = %job.job_id, "job started");
        self.job = Some(RunningJob {
            id: job.job_id,
            deadline: Instant::now() + self.ctx.job_timeout,
            attempt: job.attempts,
            seq: 0,
        });
        self.ctx.hub.set_worker_state(WorkerState::Busy);
        self.ctx.hub.changed();
        let message = CoreMessage::Job(JobAssignment {
            job_id: job.job_id,
            goal: job.goal,
        });
        self.send(&message).await
    }

    pub(super) async fn job_timed_out(&mut self) -> Result<SessionEnd, SessionError> {
        let Some(job) = self.job.take() else {
            return Ok(SessionEnd::WorkerGone);
        };
        let summary = Some(Summary::lossy(&format!(
            "the job exceeded the {} s limit",
            self.ctx.job_timeout.as_secs()
        )));
        self.finish_job(job.id, JobStatus::Failed, summary).await?;
        tracing::warn!(session = %self.id, job = %job.id, "job timed out; ending the session");
        Ok(SessionEnd::JobTimedOut(job.id))
    }

    pub(super) async fn finish_job(
        &mut self,
        job_id: JobId,
        status: JobStatus,
        summary: Option<Summary>,
    ) -> Result<(), SessionError> {
        let session_id = self.id;
        self.ctx
            .store
            .call(move |s| s.finish_job(job_id, status, summary.as_ref(), Some(session_id)))
            .await?;
        self.ctx.hub.set_worker_state(WorkerState::Idle);
        self.ctx.hub.changed();
        Ok(())
    }

    pub(super) async fn requeue_or_close(
        &mut self,
        job_id: JobId,
        end: &SessionEnd,
    ) -> Result<(), SessionError> {
        let (session_id, max_attempts, reason) =
            (self.id, self.ctx.limits.max_job_attempts, end.describe());
        let status = self
            .ctx
            .store
            .call(move |s| s.requeue_or_close(job_id, max_attempts, Some(session_id), &reason))
            .await?;
        tracing::info!(session = %self.id, job = %job_id, %status, "job settled after the session ended");
        if status == JobStatus::Queued {
            self.ctx.hub.jobs.notify_one();
        }
        self.ctx.hub.set_worker_state(WorkerState::Idle);
        self.ctx.hub.changed();
        Ok(())
    }

    pub(super) async fn handle_job_result(
        &mut self,
        result: JobResult,
    ) -> Result<Step, SessionError> {
        if self.job.as_ref().map(|job| job.id) != Some(result.job_id) {
            let error = WireError::new(
                ErrorCode::UnexpectedMessage,
                "the result does not belong to the current job",
            );
            return self.reject_frame(None, error).await;
        }
        self.job = None;
        let status = match result.outcome {
            JobOutcome::Completed => JobStatus::Completed,
            JobOutcome::Failed => JobStatus::Failed,
        };
        tracing::info!(session = %self.id, job = %result.job_id, %status, "job finished");
        self.finish_job(result.job_id, status, Some(result.summary))
            .await?;
        Ok(None)
    }

    pub(super) fn advance_to(&mut self, seq: u32) {
        if let Some(job) = self.job.as_mut() {
            job.seq = seq;
        }
    }
}
