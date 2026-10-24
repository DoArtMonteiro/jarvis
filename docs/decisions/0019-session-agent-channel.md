# 0019. The first Session Agent: a Core-owned channel and one synthetic operation

**Status:** Accepted. Implements the transport, grants and audit of ADR 0017 for one operation that touches no desktop data. No desktop operation can be performed here; ADR 0020 later adds writing approved text to the clipboard.

## Context

ADR 0017 decided that desktop access belongs to a trusted Session Agent, never to the worker, and fixed a contract for it. Before any desktop capability is built, the path from a person's approval to an operation in a separate process has to exist and be tested: the channel, how the agent knows a request comes from the Core, how one approval becomes one operation, and what happens when the agent fails halfway.

## Decision

**Operations are tool calls.** A Session Agent operation is a typed `ToolCall` like any other: the worker can only propose it, policy derives its capability by an exhaustive match, and every agent capability is class B, so a person approves each use (invariants 2 to 4). The first operation is `session_agent.healthcheck`, which asks the agent to echo a short label. The four desktop operations of ADR 0017 (`desktop.capture_window`, `desktop.read_clipboard_text`, `desktop.write_clipboard_text`, `desktop.type_text`) are typed and can be described to a person, but cannot be performed:

- the configuration refuses any rule for a desktop capability other than `deny`;
- the Core refuses them before a grant could be issued;
- the agent's operation set has no variant for them, so none can be sent.

**What a person sees.** `approvals show` describes every agent operation in words (`action`), with the window it would act on (`target`) and, for text, its size, SHA-256 and a preview. The approval's fingerprint covers the task, request, tool, arguments and capabilities, so what was shown is what is authorised.

**The channel.** The Core starts `jarvis-session-agent` itself, on first use, as a trusted helper (`jarvis_sandbox::spawn_helper`), and talks to it only over the three stdio pipes it created for that child:

- **Windows.** One `CreateProcessW` with an explicit handle list (only the child's stdio ends are inherited), a job object that kills the agent when the Core closes it and admits one process, and no child processes. The pipes are single-instance, already connected, and owner-only with a medium mandatory label, so no other process can open them.
- **Linux.** A cleared environment, every descriptor except stdio close-on-exec, its own process group, `no_new_privs` and `PR_SET_PDEATHSIG`.
- The agent runs as the user at medium integrity, outside the worker's isolation. It is not a sandbox: it holds the user's authority, which is why it acts only on what the Core sends.

The agent's identity is the channel: an OS object only the Core and that child hold. No message field names a caller. No RPC operation reaches the agent, and the worker inherits none of its handles. A same-user process at medium integrity can start its own copy of the agent, but gains nothing it did not already have: it is inside the trust boundary that ADR 0013 and the RPC authorisation already draw.

**Messages.** Line-delimited JSON with its own version (`v: 1`), at most 16 KiB a frame, decoded strictly. The agent sends `ready` once, then one `result` per `perform`. The Core sends only `perform`, carrying a grant ID, the operation and its remaining lifetime.

**From approval to operation.**

1. Before the approval is used, the Core checks that a grant could be issued (the operation exists, the grant clock, capacity) and that a live agent is ready, starting one if needed within its start budget. A refusal here, including an agent that fails to start, expires the approval unused. Only a race with another operation can refuse after this point; it is recorded as not performed.
2. `consume_approval` checks the fingerprint, marks the approval consumed, moves the task to `executing` and records `approval_consumed`, `execution_started` and `session_agent_dispatched` in one transaction.
3. `Grants::issue` takes the store's `ConsumedApproval` by move and checks its fingerprint against the call again, so one approval backs one grant. A grant lives at most 60 seconds (10 by default) on the Core's monotonic clock; a clock reading earlier than one already seen is refused.
4. `Grants::authorize` uses the grant up and returns an `Authorization`, which nothing else can make. If the Core is shutting down, or the agent has exited since step 1 and cannot be replaced, nothing is sent and the operation is recorded as not performed. Otherwise `perform` is sent.
5. The agent refuses a grant ID it has seen and a lifetime that has run out on its own monotonic clock, performs the operation once, and returns its report to the Core.
6. The Core ends the task in one transaction with its outcome events: `completed` with `session_agent_completed` (which holds the report), or `failed`.

**Results go to the Core only.** The worker receives a receipt naming the operation, never the agent's report; the report is in the audit log for the person. A refusal reaches the worker as a fixed message; its reason (whether the agent runs, why it refused) is only in the audit log. Later operations whose results are personal data follow the same rule.

**Failure is closed.** Each outcome is recorded with the task's final state:

| What happened | Task | Error code | Event |
| --- | --- | --- | --- |
| Refused before the approval was used (no live agent, start failed, no grant possible) | `expired` | none | `approval_expired` with the reason |
| Refused after the approval was used, before anything was sent (a race, shutdown, agent gone and not replaceable, expired) | `failed` | `tool_failed` | `session_agent_refused` |
| The agent refused (replayed grant, expired) | `failed` | `tool_failed` | `session_agent_refused` |
| Sent, then a timeout, a malformed or mismatched reply, the agent exiting or Core shutdown | `failed` | `internal` | `session_agent_outcome_unknown` |

On an unknown outcome the agent is stopped and every grant is revoked, so a late reply has no channel to arrive on. The operation is never sent again. A job holding such a task is quarantined, not resumed (ADR 0015). The Core starts the agent at most `session_agent.max_starts` times per run; after that, agent operations are refused before their approval is used.

**What is not claimed.** Exactly-once holds for this synthetic operation, which has no effect outside the agent. A future desktop operation with a real effect can be left in an unknown state by a crash after `perform` is sent. It is then recorded as unknown and never retried, which is fail-closed, not exactly-once. (After a Core crash, start-up recovery records the unknown outcome; ADR 0020 added this.)

## Consequences

- There is a working, audited, authenticated path from a person's approval to an operation in a separate process, exercised by tests and a local demonstration, with no desktop access through it.
- Adding a desktop operation now means: an `AgentOperation` variant, the agent code behind it, its own review, and lifting the `deny`-only rule for its capability. Each needs its own decision.
- The worker protocol gains five tools and one result type (`agent_operation`); its version is unchanged because both sides ship together and existing tools are untouched.

## Not built

- Any desktop code: capture, clipboard, input, the window picker, binding a target to one picker session, or checking focus before typing. (ADR 0020 later adds clipboard writes and records why typing stays closed.)
- Stopping same-user processes at medium integrity from reading the agent's memory, opening it, or replacing its executable: the Core starts the configured program (or `jarvis-session-agent` next to its own executable) without checking its integrity. That account is trusted by the same rule as for the RPC endpoint. (ADR 0020 later records the SHA-256 of the executable it starts; that is a record, not a check.)
- A start budget that refills: after `session_agent.max_starts` starts, agent operations are refused until the Core restarts.
- Concurrency: one operation at a time. The link holds its lock while it starts the agent and waits for an answer, so a slow agent delays other agent operations and shutdown by up to the configured timeout.
- On Linux, `PR_SET_PDEATHSIG` follows the thread that started the agent. The Core starts it from its runtime's worker threads, which live as long as the Core; the agent also exits when its input closes.
