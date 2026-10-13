# 0016. A gaming governor that defers starting jobs

**Status:** Accepted

## Context

The Core can run for days on a machine its owner also plays games on. A queued job starts the Python worker's planner and a few tool calls. That is little work, but it competes with a game for the CPU at the moment frame times matter, and the person who queued the job did not choose that moment. They would rather the work waited until they stopped playing.

The goal is small and is stated here so that it does not grow: **do not start queued jobs while a configured game is running.** The governor does not pause a job that is already running, does not suspend or inspect any process, does not change priorities, and does not budget CPU, GPU or memory. A job that started before the game keeps running, and a job that is claimed in the instant before the game appears runs to its end (see the accepted race below).

The governor is off by default. A Core configured without it behaves as it did before.

## Decision

### Where the decision lives

The Core decides, as with everything else (ADR 0001). A task reads the process list every `poll_ms`, feeds each reading to a pure function, records what must be recorded, and publishes the result to the hub. Every session reads that result before it claims a job. There is one place a job is claimed (`dispatch_next_job`, called only from the session loop), and the gate sits in front of it. While the gate is closed:

- no job is claimed, so no attempt is counted and no `job_timeout` starts;
- the session still watches its worker, shutdown and the gate itself, so a worker that exits, or a shutdown, is handled as usual;
- the session wakes when the gate changes, without any other signal, so a release starts the job even if nothing else happens.

Queued jobs are in the database, so they survive a restart as before (ADR 0015). Nothing about a held job is special in the store.

### The state machine

The function is `governor::machine::step(state, observation, now, timing) -> (state, events)`. It has no clock and no I/O. An observation is one of: `Game(name)` (a configured process runs, named as it was configured), `Clear` (the list was read and none runs) or `Failed` (the list could not be read).

| From | Observation | To | Event |
| --- | --- | --- | --- |
| `Open` | game | `Held`, since now | `governor_hold` (reason `game`) |
| `Open` | failed | `Held`, since now | `governor_hold` (reason `detection_failed`) |
| `Open` | clear | `Open` | |
| `Held` | game or failed | `Held` (reason updated) | |
| `Held` | clear | `Releasing`, until now + `resume_delay_ms` | |
| `Releasing` | game or failed | `Held`, same `since` | |
| `Releasing` | clear, before `until` | `Releasing` | |
| `Releasing` | clear, at or after `until` | `Open` | `governor_released` (`held_ms`) |
| start-up `Held` | clear | `Open` | |
| start-up `Held` | game or failed | `Held`, `since` = start-up | `governor_hold` |

Notes on the table:

- A zero `resume_delay_ms` releases on the first clear reading.
- `held_ms` counts from the beginning of the hold, resume delay included, and a game that returns during the delay does not start a new hold.
- Detection errors are logged and audited once per streak. A streak that starts while the gate is open produces one `governor_hold`, and the readings that fail after it change nothing. A failure while already held changes the reason shown in `health` and nothing else.
- The release is recorded before the gate opens. If that write fails, the gate stays shut, `health` shows `releasing` (also when `resume_delay_ms` is 0, where the step would otherwise have gone straight from held to open) and the next reading tries again; the failure is logged once per streak. A hold is the other way round: the gate closes first and the record is written after, so a slow, locked or failing database cannot keep jobs starting while a game runs. A hold that cannot be recorded still applies, and is retried (see Audit events).

### Start-up

When the governor is enabled the Core starts held, with reason `starting`, and the gate is closed before the RPC endpoint answers and before the first session can run. Nothing is known about the machine yet, and jobs queued before a restart are waiting in the database. Without this rule such a job could start in the moments before the first reading found the game, which is the situation the governor exists to avoid.

The first reading ends it. A clear list opens the gate at once, without a resume delay, because nothing had been held that could need a settling period, and records nothing: it was not a decision. A game or a failure on the first reading is a hold that began when the Core started, and is recorded as one. The `starting` hold itself has no event, because it is neither a decision nor something a person acted on.

### Reads that hang, and a governor that stops

The real source reads the list on a blocking thread, which Tokio cannot cancel. At most one read is in flight. A read that has not finished within `poll_ms` counts as a failed reading, so the jobs are held with reason `detection_failed`, and the next tick goes on waiting for that same read instead of starting another. If it finishes late, its answer is used. A hung operating-system call therefore costs one blocked thread, not one per tick. A read that is merely slower than `poll_ms`, on a machine saturated with other work for example, also causes a `detection_failed` hold until it returns; that is the safe direction. The Core stops waiting for blocking threads five seconds after shutdown, so a hung call delays the exit of the process by at most that long.

If the governor task ends for any reason but a stop request (a panic in a source, the task being dropped), a guard sets the gate to held with reason `detection_failed` and logs an error, and `health` shows it. Nothing would update the gate afterwards, so it must not stay open.

### Time never ends a hold

A hold that lasts `alert_after_ms` is marked overdue and one event, `governor_hold_overdue`, is recorded, once per hold. `health` shows `overdue: true`. The jobs keep waiting until the game is gone.

Two alternatives were rejected.

- **Release at the limit.** A "maximum hold" that releases the jobs would start work while the game is still running, which is what the governor exists to prevent, and it would do so at a time the person did not choose. The setting is named `alert_after_ms`, not a maximum, so that it does not read as "release after".
- **Fail the held jobs at the limit.** This would add a transition from queued to a terminal state, which no job has (ADR 0015), and would lose work the person queued only because they played for a long time.

The alert is raised when a reading is processed, so it can be late by up to `poll_ms`.

### The accepted race

The gate is checked, and then the job is claimed. A game that starts between the two is not seen until the next reading, and the job that was claimed in between runs. Closing the gap would mean claiming and then holding, which counts an attempt and starts a timer for nothing, or pausing a running job, which is outside the scope above. The window is not two adjacent statements: the claim goes through the store (a mutex, a transaction with a flush to disk, a thread hop), so it is tens of milliseconds or so, and longer if the database is slow. A running job is never paused. The same applies to a game that starts a moment after a job begins.

### Detection

Detection lives in `jarvis-sandbox::processes::find_running(names)`, the crate that already holds the operating-system calls.

- **Windows** reads a Toolhelp snapshot (`CreateToolhelp32Snapshot` with `TH32CS_SNAPPROCESS`), checks the result against `INVALID_HANDLE_VALUE`, sets `dwSize`, walks the list with `Process32FirstW` and `Process32NextW`, and treats only `ERROR_NO_MORE_FILES` as the end. Any other failure is an error. The file name is the text before the first NUL in `szExeFile`, and it includes the extension, so a configured name on Windows must include `.exe`, as the Details tab of Task Manager shows it. The snapshot handle is closed by the existing RAII `Handle`.
- **No process is opened.** There is no `OpenProcess`; the Core opens no handle to any process, only to the snapshot. It has not been tried against any game.
- **Linux** reads `/proc/<pid>/comm` for each numeric entry of `/proc`. A process that exits between the listing and the read (`ENOENT` or `ESRCH`) is skipped. Failing to read `/proc`, or any other failure on an entry, is an error. An unreadable entry (for example under `hidepid=1`) is an error and the governor holds. But `/proc` can also simply show fewer processes: with `hidepid=2`, or when the Core runs in a different PID namespace (a container), a game outside what the Core sees is not seen, and nothing reports it.
- **Other Unix systems** return `Unsupported`. An enabled governor there holds with reason `detection_failed` until the configuration turns it off; `health` shows the reason and the log says why.
- Names are compared ASCII case-insensitively. A configured name is 1 to 64 printable ASCII bytes with no `/` or `\`, and at most 16 are accepted. The Linux kernel keeps only 15 bytes of a process name (`comm`), so on Linux the configured name is cut to 15 bytes before the comparison. Two different programs whose names share their first 15 bytes cannot be told apart, and a configured name longer than that can match another program. The names are not paths and are never used as one.
- `find_running` returns the first configured name, in configuration order, that matches, spelled as configured. It never returns the list.

On the Core's side the real source runs `find_running` on a blocking thread. Tests substitute a source that does not, so that Tokio's paused clock stays exact.

### Cost

Measured on a Windows machine with 1019 processes, a release build, over 60 s of real time: with the governor off the Core used no measurable CPU; on, with `poll_ms = 2000`, it used 687.5 ms of CPU, about 1.15% of one core. One Toolhelp snapshot costs about 17 ms of CPU there. That cost is inherent to listing the process table and grows with the number of processes. The cheaper Windows ways to ask whether a named process runs need `OpenProcess` on the target, which this design excludes. The default `poll_ms` is therefore 5000. At that setting two 60 s runs on the same machine used 234.4 ms and 265.6 ms of CPU, about 0.4% of one core. A longer poll also means a game can run for that long before the gate closes.

### Privacy

The audit log records the configured name that matched and nothing else: not the process ID, not the path, not the user, not any other process. `health` shows the same name. The process list is read to answer one question and is not kept.

### Health and the RPC version

`health` carries a `governor` object: `state` (`off`, `open`, `held`, `releasing`), `reason`, `process`, `held_ms` and `overdue`. The reply type rejects unknown and missing fields, so a client built before the change cannot read it. ADR 0010 gives the RPC the same strict decoding rules as the worker protocol, and ADR 0007 and the doc comment on `ProtocolVersion` in the protocol crate make any change to the wire format a version change, so the local RPC version goes from 1 to 2. The worker protocol is unchanged. No operation, tool or capability was added, and the worker does not know the governor exists.

### Audit events

`governor_hold` (`reason`, and `process` for a game), `governor_released` (`held_ms`) and `governor_hold_overdue` (`held_ms`).

The contract for recording them:

- Each event is recorded at least once, in the order it happened, as soon as the database accepts writes. A failed write is never counted as done, and an event is written again only after such a failure. An event that cannot be written waits in memory and is tried again at every later reading, oldest first, before anything newer. It leaves the list only after a successful write.
- The gate closes before a hold is recorded. A hold or an alert that cannot be recorded does not delay or undo the state it describes.
- The gate opens only after everything before the release, and the release itself, is recorded. Until then `health` shows `releasing` and the jobs stay held. The release is not queued; the state machine emits it again at the next reading.
- While anything waits to be written the gate cannot open, so at most one hold and one alert can wait. The list is capped at eight and an error is logged if that is ever exceeded.
- A failing database is logged once per streak of failures, not once per reading.
- Limitation: events that wait when the Core stops are lost, because they are only in memory. The next start begins held (reason `starting`) and records afresh, so the log can be missing a hold or an alert from the run that ended, and nothing says so.

### If the queue stays held

A held queue loses nothing: jobs stay queued, with no attempt counted and no timer running. To find out why it is held, run `jarvis-core health` and read the governor line: the reason (`game`, `detection_failed` or `starting`) and, for a game, the configured name that matched. Then, in the order that disturbs least:

1. Close the process that is named. A process of the same user with that name holds the queue as well as the game does (see Consequences).
2. Or stop the Core, set `[governor] enabled = false` in its configuration (or correct `processes`), and start it again. The queued jobs then start.

For `detection_failed`, `jarvis-core health` reports it and the Core log says why the list could not be read; it is also what an enabled governor reports on a platform without detection.

## Consequences

- A person sees a held queue in `health` and in the audit log, with the reason and how long. Nothing is hidden, and nothing is released behind their back.
- **A process on the same account named like a configured game holds jobs indefinitely.** Copying a harmless program to `cs2.exe` and running it is enough. The alert makes this visible after `alert_after_ms`, and the audit log names the configured name that matched. It does not prevent it. The same account is already inside the trust boundary and can edit the configuration, approve requests and change the database (threat model, known gaps), so the governor adds no authority to it. It does not bound spoofing and is not meant to.
- Holding is the answer to every doubt: a failed or missing detection, a release that cannot be recorded, a start-up that has not looked yet. The cost is a queue that waits when it could have run. The alert is how a person finds out.
- A job that is held never has an attempt counted and never has its timer started, so a long game session does not use up `max_job_attempts` or `job_timeout`.
- Approvals are not affected. They belong to a running job and a running job is not touched.
- The Linux comm truncation is a real false-positive risk for long names and is documented in the example configuration.
- The governor reads the process list on every poll, which is the only cost while it is on (see Cost). While it is off, nothing is read.
