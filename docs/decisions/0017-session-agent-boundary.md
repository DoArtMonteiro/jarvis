# 0017. Desktop access belongs to a trusted Session Agent, never to the worker

**Status:** Proposed. The Session Agent process, its channel, its grants and their audit are implemented for one synthetic operation by ADR 0019. The one desktop operation implemented is writing approved text to the clipboard (ADR 0020), and the parts listed under "Not built" below remain so unless ADR 0019 or 0020 says otherwise.

## Context

Future features may need the screen, the clipboard or keyboard input: reading a window the person points at, pasting a result, typing into a form. The worker cannot be given these. It is untrusted (ADR 0002), and on Windows its own access to the interactive desktop is not proven blocked (ADR 0013, threat model known gaps).

Two OS mechanisms were evaluated for closing the worker's desktop channels directly:

- **A private window station.** Windows documents that only members of the Administrators group may create a window station with a chosen name, and the Core runs as a standard user. Without a name, Windows derives one from the caller's logon session, so the station is per logon session, not per worker. On a test host that station already existed, created by other software, and opening an existing station would ignore the access control the Core asks for. A named per-worker station is therefore not available to a standard user, and an unnamed one cannot be relied on to be the Core's own. This design does not depend on either.
- **Win32k system call lockdown** is adopted (ADR 0018): the worker is created with win32k system calls (window, GDI and input) disabled, and the kernel reports the policy as set whether or not the Core runs inside a job; no test exercises a particular call. That removes one property. It does not by itself prove the screen, clipboard, keyboard and window-object channels closed, and it has no effect on the registry.

Whichever OS controls are adopted later, a legitimate desktop feature still needs some component to hold these rights.

## Decision

Desktop operations run in a separate **Session Agent**: a medium-integrity process of the same user, outside the worker's isolation, that acts only when the Core tells it to. The worker never holds a desktop right, and the Session Agent never decides anything (invariant 1).

This builds on ADR 0006, which defers personal integrations: it fixes where such features would live, and enables none. It applies invariants 2, 3, 4, 6, 8 and 9 to desktop operations rather than changing them.

The contract, fixed now so that an implementation cannot drift from it:

1. **Closed, typed operations.** `capture_window`, `read_clipboard_text`, `write_clipboard_text` and `type_text`. No operation takes a command line, script, key chord, window title or path (invariant 2). A window is named by a target the person picked through the Session Agent's own picker, never by a value the worker makes up. Text is bounded and may not contain invisible formatting characters (bidi overrides, zero-width characters), so what a person approves reads as what is used. Typed text may contain no control characters at all, because a typed newline or tab is Enter or Tab; clipboard text may contain newlines and tabs.
2. **Class B only.** Each operation maps, through an exhaustive match, to one desktop capability, and every desktop capability is class B: a person approves each use, and no policy can allow one outright (invariant 4).
3. **Grants.** The Session Agent performs an operation only under a grant the Core issued. A grant needs the person's consent, which can only be made from the proof the approval store returns once, when it consumes an approval, and only if that approval's fingerprint is the fingerprint of this exact request. Desktop fingerprints are domain-separated from tool-call fingerprints, so an approval for a tool call is never consent to a desktop operation. The consent cannot be copied, so one approval backs one grant. A grant is named by its approval, so a grant from an earlier run names nothing after a restart. It is used at most once, lives at most 60 seconds, can be revoked singly or all at once, and is held in memory only, so none survives a restart (ADR 0009). The Core holds at most 16 live grants.
4. **Authenticated caller.** The caller is what the transport authenticated (the peer's token, user and integrity level, as the RPC server checks today), not what a message claims. Only the Core is accepted; a client or the worker is refused, and a refused call does not use the grant up. The contract takes the caller and the time as parameters. **Not enforced yet:** the caller is a plain value that no transport constructs, so the integration must take it from the transport. The time is the store's clock, the one approvals are consumed and expire on, so a consent's age and a grant's lifetime are measured on one clock. That clock can be set by the user, so the grants refuse any time earlier than the latest they have seen in this run: stepping the clock back cannot extend a grant or a consent (a step forward can only end grants early), and a consent more than 10 seconds old cannot be turned into a grant.
5. **Worker output is data.** A desktop request decoded from anything the worker sent carries no grant, approval or capability. Decoding rejects unknown fields and unknown operations.
6. **Auditable outcomes.** Every check ends in one outcome with a stable name: authorised (with the capability used), or refused as not from the Core, unknown grant, expired, revoked, already used, out of scope, invalid lifetime, too many grants, approval reused, consent too old, not approved for this request, or clock went back. Recording these outcomes is not built. Finished grants are remembered, within a bound, so a late check is refused with its real reason.
7. **Results go to the Core only.** A captured image or clipboard text is personal data. It is returned to the Core, never to the worker. Passing any of it to the worker would break the rule that personal data stays out of the worker, and needs its own decision.

### Transport (decided, not built)

The Core starts the Session Agent itself and talks to it only over pipes it creates and hands to the child at start, the same way it talks to the worker. There is no named endpoint.

- **Identity by construction.** Only the Core holds the other ends. No other process can connect to, squat on or impersonate the channel.
- **Why not the RPC pipe.** Reusing the RPC pipe was rejected. Any same-user process can open it, so any local client could pose as the Session Agent and claim grants, or read the requests they carry.
- **Isolation of the agent.** The agent runs as the user at medium integrity, outside the AppContainer, in a job that dies with the Core. The worker has no handle to it and cannot name it.
- **Messages.** Line-delimited and strictly decoded, with their own version key.
  - Core to agent: `perform`, carrying one grant ID, its request and its remaining lifetime.
  - Agent to Core: `result`, carrying the grant ID and the outcome. Any data travels to the Core only.
- **Where checks happen.** The Core authorises and uses up a grant before it sends `perform`. The agent refuses a grant ID it has already seen and one whose lifetime has run out by its own monotonic clock.
- **Audit.** The Core is to record each grant, each authorisation or refusal, and each result in the audit log in the same transaction as the state change (invariant 7).
- **Consuming the approval.** The Core passes the desktop request's own fingerprint to the approval store when it consumes the approval, so an approval for a different request is refused before it is used up. The fingerprint check when the consent is bound is a second line of defence; a refusal there comes after the approval was consumed, so that approval is lost. A crash between consuming an approval and issuing its grant loses that approval: the person is asked again.

## Consequences

- The rule that no language model, voice or personal-data integration runs in the worker stands. Desktop features get a place to live that does not depend on closing every channel in the worker.
- The worker's own desktop channels on Windows remain open questions, recorded in the threat model's known gaps. This decision does not close them.
- Not built: the Session Agent process, its transport and its authentication of the Core, the window picker (including binding a target to one picker session, so a reused window handle cannot pass, and checking focus before typing), a fingerprint and a display description for desktop requests (invariant 6), mapping desktop capabilities into the protocol's capability set and policy, the audit events in the store, and any desktop code. Each needs its own review against this contract.
- The contract can tighten without breaking anything, because nothing uses it yet. Loosening it needs a new ADR.
