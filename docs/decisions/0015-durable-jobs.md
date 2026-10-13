# 0015. Durable jobs: resume what is safe, quarantine what is not

**Status:** Accepted. Amends the job sentences of 0009.

## Context

Until now a job lived only as long as one worker session. If the Core stopped, the next start marked running jobs `interrupted` and queued jobs `cancelled` (ADR 0009). If the worker crashed, the session marked its job `interrupted`. A person who wanted the work done had to submit the goal again, and steps that had already completed would then run again. That includes writes a person had approved.

Resuming a job is only safe if it never repeats a side effect whose result is already recorded, and never repeats one whose result is not known. Approvals stay bound to one exact request and never outlive their session (ADR 0008, ADR 0009).

## Decision

### Which jobs come back

- A queued job stays queued across a restart.
- A running job is **requeued** when its session ended because the worker exited or crashed (`worker_gone`, `worker_closed`), or because the Core shut down. Start-up recovery applies the same rule to a job it finds running after a crash.
- A running job ends **`failed`** when it exceeded `job_timeout`, broke the protocol (`terminated`), or failed a replay check (below). These are never requeued.
- Every Core shutdown while a job runs spends one of its attempts, because the next start claims it again. A job found running with no attempt counted, left from before attempts were recorded, has no ledger of steps and ends `interrupted`.
- The Core counts attempts per job. A job that has already been claimed `max_job_attempts` times ends `interrupted` instead of being requeued. So the total time spent on a job is at most `max_job_attempts × job_timeout`.
- One store function makes this decision for both start-up recovery and session close. At start-up it runs in the recovery transaction, before tasks left open are rewritten, so it still sees what they were doing.

### Steps and replay

- Every tool request in a job has a position, `seq`, counted from 1 in each attempt. A request that creates a task or is answered from the ledger takes a position. A frame rejected before a task exists does not. Each task created for a job is recorded in an insert-only ledger: `(job_id, attempt, seq, task_id)`.
- A step is **completed** when the task recorded for that `(job_id, seq)` in an earlier attempt has status `completed`. The Core never creates a second task for a completed position, so there is at most one.
- When the worker sends the request at position `seq` and a completed task exists there, the Core checks three things:
  - the request, parsed as a typed call, equals the stored task's call;
  - current policy does not deny it;
  - the stored result decodes strictly.

  If all three hold, the Core answers with the stored result and the original `task_id`, and records `step_replayed`. No task is created, no tool runs and no `execution_started` is written. Otherwise the job ends `failed` and the session ends. A replayed request counts against `max_requests_per_session` like any other.
- A position with no completed task goes through the normal pipeline: a new task, fresh policy evaluation and, for class B, a new approval.

### Uncertain outcomes are quarantined

A task's outcome is **known** only when `execution_started` was recorded and the task then ended `completed`, or ended `failed` with an error the tool itself reported (`tool_failed`). The workspace writer, for example, renames a complete temporary file into place or writes nothing, so a write that reports an error did not happen. Every other task that started is **unknown**: one still `executing` when the Core stopped, `interrupted` after starting, `timed_out`, `cancelled` by shutdown, `failed` because the tool panicked (`internal`), or `failed` because the Core rejected a result after the tool had run (`result_rejected`). In each case the tool may have acted without the Core recording what it did.

A job with any task of unknown outcome is never resumed, whatever the task's class. It ends `interrupted`, its summary names the task, and `job_quarantined` is recorded. A person decides what to do next. A job that ends without a retry (a timeout, a protocol violation, a replay mismatch, or a failure the worker reports) records `job_quarantined` as well when such a task exists, so the audit log says where a person should look.

### Class B never inherits approval

Approvals are still expired when their session ends and at start-up (ADR 0009). A fingerprint covers the task ID and request ID (ADR 0008), and a resumed step at an open position always gets a new task. So an approval from an earlier attempt can match nothing in a later one. A completed class B step is replayed from its stored result and not executed again, so nothing new needs approving. A class B step that did not complete asks a person again.

## Consequences

- The guarantee is *at most once for a recorded completion*: resuming never runs a step whose completion is recorded, and never runs a step whose outcome is unknown. It is **not** exactly-once. A tool can act and the Core can stop before `execution_started` is followed by a result. That job is then quarantined rather than resumed, so the action may have happened once and is never repeated automatically.
- Replay depends on the worker sending the same requests in the same order on every attempt. The current planner is a pure function of the goal, so it does. A planner that is not, such as a language model, will fail the replay check at the first difference. Its resumed jobs then end `failed`, and no completed step runs twice. Resuming such planners needs the protocol to tell the worker which steps are done. That would be a later decision.
- There is no request to cancel a job. Carrying the queue over a restart adds no authority: every class B step still needs a fresh approval. Terminal jobs never change (migration 0002).
- `cancelled` is no longer produced for jobs by recovery. Existing rows keep their status.
- A job that crashes the worker every time uses up its attempts and ends `interrupted`. The worker restart budget still applies on top.
