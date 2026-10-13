# 0009. Pending approvals do not survive a restart

**Status:** Accepted. Amended by [0015](0015-durable-jobs.md): the sentences about jobs.

## Context

When the Core stops, requests may be waiting for a person, or may have been approved but not yet used. On the next start the worker that asked is gone, its job context is lost, and the situation the person reviewed may have changed.

## Decision

Start-up recovery closes everything that carries authority, in one transaction, before the Core accepts any client:

- pending and granted approvals become `expired`, each with an `approval_expired` event;
- tasks waiting for confirmation become `expired`, tasks that were received or executing become `interrupted`, each with a `task_recovered` event;
- running jobs are requeued or closed, and queued jobs stay queued (ADR 0015). Their approvals are expired all the same.

The same applies, without a restart, when a session ends: the worker exiting, crashing or being restarted expires the approvals of that session.

## Consequences

- No approval issued by one run of the Core can be used by another, and no approval outlives the worker process that asked for it.
- A person who approved just before a crash has to approve again, after the worker asks again. That is the intended cost.
- Queued jobs stay queued and are handed to the next worker. A job that was waiting for a person asks again, with a new task and a new approval (ADR 0015).
