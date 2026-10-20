# 0010. Local RPC over a Unix socket or a named pipe

**Status:** Accepted

## Context

A long-lived Core needs a way for local clients (the CLI today, a dashboard later) to submit goals, watch jobs and decide on approvals. Whatever can reach this interface can approve class B actions, so it must not be reachable from the network, from other users, or from the worker.

## Decision

- **Transport.** A Unix domain socket on Unix, a named pipe on Windows. No TCP listener exists.
- **Unix.** The socket lives in a directory created with mode `0700`; an existing directory with group or other permissions, or owned by someone else, is refused rather than changed. The socket itself is `0600`. The peer's user ID (from `SO_PEERCRED`) must match the Core's. A stale socket is removed only if nothing answers on it.
- **Windows.** The pipe is created with an explicit security descriptor: owned by the current user, with a DACL that grants access to that user only (the default DACL would let every account open it for reading), and a medium mandatory label with no-read-up, no-write-up and no-execute-up, so no process below medium integrity can open it. It uses `FILE_FLAG_FIRST_PIPE_INSTANCE`, so start-up fails if another process already owns the name, and it rejects remote clients. The client process ID comes from `GetNamedPipeClientProcessId`.
- **Windows, the Core's own check of the client.** The DACL is enforced by Windows; the Core also checks the client itself, as `SO_PEERCRED` does on Unix. Once a client connects, the Core impersonates it with `ImpersonateNamedPipeClient` just long enough to open the client's token, then reverts (and stops the process if reverting fails). The token's user SID must equal the Core's, and the token must be neither an AppContainer nor below medium integrity. This reads the token the client connected with, not a token found through a process ID that may already have been reused. A token that cannot be read is refused: a client that connects at anonymous impersonation level has none, and another account's token usually does not let the Core's user query it, so in practice such a client is refused as unreadable rather than as a different user. Both are refused, and the DACL normally stops another account first.
- **Windows, the client's check of the Core.** Before sending anything, the client reads the owner and the mandatory label of the pipe it opened, and refuses it unless the owner is its own user and the label is medium or above (no label counts as medium). While the Core is stopped, another account, or a process of the same user below medium integrity, could create a pipe under the Core's name; the client then stops instead of sending requests and approval fingerprints to it.
- **The worker is not a client.** A connection whose process or token cannot be identified is refused, and so is one from the worker: its process ID, its process group or a descendant on Linux, its job object on Windows. On Windows the worker's low integrity level also stops it opening the pipe. Refusals are audited.
- **A small typed protocol.** Nine operations, the same strict decoding rules as the worker protocol, its own version key (`rpc`), requests limited to 16 KiB, an idle timeout, at most 16 concurrent connections, long polls capped at 60 s. `shutdown` is refused unless the configuration allows it.

## Consequences

- Any process running as the same user and able to open the socket or pipe is a trusted client. That matches the rest of the system, where the same account can edit the config and the database, and it is written down in the threat model.
- On Windows the per-request checks for the worker and for a restricted process still go through the client's process ID; only the user and token check at connection time reads the client's own token.
- An administrator can change any of this (take ownership of the pipe, read the Core's memory). Administrators are outside what this boundary defends against, like the same user.
- The worker check on Linux relies on process ancestry. A worker that detaches a process from both its process group and its parent can reach the socket like any other process of the user. Closing that gap needs a separate user or a sandbox for the worker.
- A dashboard can be built on the same interface without new authority: it can only do what the CLI can do.
