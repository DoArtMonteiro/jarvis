//! Contract for the Session Agent: the trusted component that will one day
//! observe the screen, use the clipboard or send input for the person.
//! **Only the contract exists.** There is no Session Agent process, no
//! transport to it and no desktop code; see ADR 0017.
//!
//! The rules the contract fixes, so that an implementation cannot drift from
//! them:
//!
//! - The operations are a closed, typed set ([`DesktopRequest`]). None takes
//!   a command line, a script, a key chord, a window title or a path.
//! - Every operation needs a class B capability ([`DesktopCapability`]):
//!   a person approves each use, and no policy can allow one outright.
//! - The Session Agent acts only on a [`Grant`] the Core issued. A grant
//!   needs a [`Consent`], which can only be made from a [`ConsumedApproval`]
//!   (returned once, by the store, when an approval is consumed) whose
//!   fingerprint is that of this exact request. A grant is used at most once,
//!   short-lived and revocable, and is checked against a [`Caller`]: only
//!   the Core is accepted. The caller is a plain value; nothing yet derives
//!   it from an authenticated transport (see below).
//! - What the worker sends is data. A [`DesktopRequest`] decoded from it
//!   carries no grant, approval or capability, and decoding rejects any
//!   attempt to add one.
//! - Every check ends in an [`Outcome`] with a stable name for the audit log.
//!
//! What the integration must supply, because this module cannot check it, and
//! which is therefore not enforced today: the [`Caller`] from the transport's
//! authentication of the peer, never from a message (any code in this crate
//! can pass `Caller::Core`); and results (images, clipboard text) delivered
//! to the Core only, never to the worker. Recording outcomes in the audit log
//! is not built.
//!
//! Time: every `now_ms` here is the store's clock (`store::now_ms`, epoch
//! milliseconds), the one approvals are consumed and expire on, so a consent's
//! age is measured on one clock. That clock can be set by the user, so
//! [`Grants`] keeps the latest reading it has seen and refuses any earlier
//! one: a step back cannot extend a grant or a consent.

use std::collections::BTreeMap;

use jarvis_protocol::{ActionClass, ApprovalId, Fingerprint, RequestId, TaskId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::store::ConsumedApproval;

/// The longest a grant may live. Approvals already expire; a grant is
/// issued after one is consumed and must be used at once.
pub const MAX_GRANT_TTL_MS: u64 = 60_000;

/// The longest a consent may wait between its approval being consumed and
/// its grant being issued. A consent cannot be kept for later.
pub const MAX_CONSENT_AGE_MS: u64 = 10_000;

/// Live grants the Core may hold at once (invariant 9: everything bounded).
pub const MAX_LIVE_GRANTS: usize = 16;

/// Grants remembered after they are used, revoked or expired, so that a late
/// check is refused with its real reason. Bounded: the oldest finished grants
/// are forgotten first. Forgetting one cannot let its approval back a second
/// grant, because a [`ConsumedApproval`] exists once per approval. That rests
/// on the store: an approval is consumed at most once and its row is never
/// deleted (database triggers refuse both).
pub const MAX_TRACKED_GRANTS: usize = 256;

/// Longest text an operation may carry, in characters.
pub const MAX_TEXT_CHARS: usize = 4096;

/// A window the person chose through the Session Agent's own picker. The
/// Core issues the value; the worker can repeat one it was shown but cannot
/// make one valid, because a grant names its target. (Binding it to one
/// picker session, so a reused window handle cannot pass, is not built.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TargetId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidText {
    #[error("text is longer than {MAX_TEXT_CHARS} characters")]
    TooLong,
    #[error("text contains a control character")]
    Control,
    #[error("text contains an invisible formatting character")]
    Invisible,
}

/// Characters that change how text displays without being seen: bidi
/// overrides and isolates, zero-width characters, the soft hyphen and the
/// byte-order mark. With them, the text a person approves could read
/// differently from the text that is used.
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{2069}'
            | '\u{FEFF}'
    )
}

fn check_text(value: &str, allow_layout: bool) -> Result<(), InvalidText> {
    if value.chars().count() > MAX_TEXT_CHARS {
        return Err(InvalidText::TooLong);
    }
    for c in value.chars() {
        if c.is_control() && !(allow_layout && (c == '\n' || c == '\t')) {
            return Err(InvalidText::Control);
        }
        if is_invisible(c) {
            return Err(InvalidText::Invisible);
        }
    }
    Ok(())
}

/// Clipboard text: bounded, visible, and no control characters other than
/// tab and newline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ClipboardText(String);

impl TryFrom<String> for ClipboardText {
    type Error = InvalidText;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        check_text(&value, true)?;
        Ok(Self(value))
    }
}

impl From<ClipboardText> for String {
    fn from(value: ClipboardText) -> Self {
        value.0
    }
}

/// Text to type: bounded, visible, and no control characters at all. Typed,
/// a newline or a tab would be Enter or Tab, which can submit a form or move
/// focus, so neither is allowed; nor can an escape sequence or a key chord be
/// expressed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TypedText(String);

impl TryFrom<String> for TypedText {
    type Error = InvalidText;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        check_text(&value, false)?;
        Ok(Self(value))
    }
}

impl From<TypedText> for String {
    fn from(value: TypedText) -> Self {
        value.0
    }
}

/// One desktop operation, exactly as it would be approved and performed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum DesktopRequest {
    /// One image of one window the person picked; never the whole screen.
    CaptureWindow { target: TargetId },
    /// The clipboard's text, if it holds text.
    ReadClipboardText {},
    /// Replace the clipboard with this text.
    WriteClipboardText { text: ClipboardText },
    /// Type this text into the window the person picked. No key chords, no
    /// other window. (Checking that the target still has focus is the
    /// Session Agent's job and is not built.)
    TypeText { target: TargetId, text: TypedText },
}

/// What a desktop operation needs. Closed and exhaustive, like
/// `policy::required_capabilities`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesktopCapability {
    ScreenObserve,
    ClipboardRead,
    ClipboardWrite,
    InputSend,
}

impl DesktopCapability {
    /// Every desktop capability touches personal data or acts for the
    /// person, so each is class B.
    pub fn class(self) -> ActionClass {
        match self {
            Self::ScreenObserve | Self::ClipboardRead | Self::ClipboardWrite | Self::InputSend => {
                ActionClass::Confirm
            }
        }
    }
}

impl DesktopRequest {
    pub fn capability(&self) -> DesktopCapability {
        match self {
            Self::CaptureWindow { .. } => DesktopCapability::ScreenObserve,
            Self::ReadClipboardText {} => DesktopCapability::ClipboardRead,
            Self::WriteClipboardText { .. } => DesktopCapability::ClipboardWrite,
            Self::TypeText { .. } => DesktopCapability::InputSend,
        }
    }
}

/// SHA-256 over a canonical encoding of one desktop request in its task. The
/// encoding is domain-separated from tool-call fingerprints
/// (`approval::fingerprint`), so an approval for a tool call can never be
/// read as consent to a desktop operation, and the reverse.
pub fn fingerprint(
    task_id: TaskId,
    request_id: &RequestId,
    request: &DesktopRequest,
) -> Fingerprint {
    // serde_json keeps object keys sorted, so this encoding is canonical.
    let canonical = serde_json::json!({
        "v": "desktop-1",
        "task_id": task_id,
        "request_id": request_id,
        "request": request,
        "capability": request.capability(),
    });
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    Fingerprint::from_digest(&digest)
}

/// Who is asking, as the transport will authenticate it (the pipe peer's
/// token, user and integrity level, as the RPC server checks today), never as
/// the message claims. Not enforced yet: this is a plain value, and no
/// transport constructs it. Until one does, the check guards against routing
/// mistakes in the Core, not against code in the Core.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    /// The Core of this user, authenticated on the Session Agent's channel.
    Core,
    /// A local client of the person. It may approve through the Core; it
    /// never instructs the Session Agent directly.
    Client,
    /// The untrusted worker, or anything running in its isolation.
    Worker,
}

/// The person's consent to one exact desktop request: a consumed approval
/// whose fingerprint is that request's. Not `Clone`, and made only by moving
/// a [`ConsumedApproval`] in, so one approval backs at most one grant.
#[derive(Debug)]
pub struct Consent {
    approval: ApprovalId,
    request: DesktopRequest,
    consumed_at_ms: u64,
}

impl Consent {
    /// Bind a consumed approval to `request`. Refused unless the approval's
    /// fingerprint is exactly this request's in the approval's own task. The
    /// integration should check the fingerprint before consuming, by passing
    /// this request's [`fingerprint`] to `Store::consume_approval`, so that a
    /// mismatch never uses the approval up.
    pub fn bind(consumed: ConsumedApproval, request: DesktopRequest) -> Result<Self, Refusal> {
        let expected = fingerprint(consumed.task_id(), consumed.request_id(), &request);
        if consumed.fingerprint() != &expected {
            return Err(Refusal::NotApproved);
        }
        // A time before the epoch is a broken clock, never "very old" or
        // "brand new".
        let consumed_at_ms =
            u64::try_from(consumed.consumed_at_ms()).map_err(|_| Refusal::ClockWentBack)?;
        Ok(Self {
            approval: consumed.approval_id(),
            request,
            consumed_at_ms,
        })
    }

    pub fn approval(&self) -> ApprovalId {
        self.approval
    }

    pub fn request(&self) -> &DesktopRequest {
        &self.request
    }
}

/// A grant's identity: the approval behind it. Approval IDs are unique and
/// never reused, including across restarts, so a grant ID from an earlier
/// Core run names nothing in this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GrantId(ApprovalId);

/// Permission for the Session Agent to perform one exact request once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub id: GrantId,
    pub request: DesktopRequest,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

/// Why a desktop operation was not performed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("only the Core may instruct the Session Agent")]
    NotCore,
    #[error("the approval is not for this request")]
    NotApproved,
    #[error("no such grant")]
    UnknownGrant,
    #[error("the grant has expired")]
    Expired,
    #[error("the clock went back")]
    ClockWentBack,
    #[error("the grant was revoked")]
    Revoked,
    #[error("the grant was already used")]
    AlreadyUsed,
    #[error("the request is not the one that was approved")]
    ScopeMismatch,
    #[error("the grant lifetime is out of range")]
    InvalidLifetime,
    #[error("too many live grants")]
    TooManyGrants,
    #[error("that approval already backs a grant")]
    ApprovalReused,
    #[error("the consent is too old to issue a grant")]
    ConsentStale,
}

/// The end of every check, as it will be written to the audit log. An
/// authorised outcome carries the capability, which the event records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Authorized(DesktopCapability),
    Refused(Refusal),
}

impl Outcome {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Authorized(_) => "desktop_authorized",
            Self::Refused(Refusal::NotCore) => "desktop_refused_not_core",
            Self::Refused(Refusal::NotApproved) => "desktop_refused_not_approved",
            Self::Refused(Refusal::UnknownGrant) => "desktop_refused_unknown_grant",
            Self::Refused(Refusal::Expired) => "desktop_refused_expired",
            Self::Refused(Refusal::ClockWentBack) => "desktop_refused_clock_went_back",
            Self::Refused(Refusal::Revoked) => "desktop_refused_revoked",
            Self::Refused(Refusal::AlreadyUsed) => "desktop_refused_already_used",
            Self::Refused(Refusal::ScopeMismatch) => "desktop_refused_scope_mismatch",
            Self::Refused(Refusal::InvalidLifetime) => "desktop_refused_invalid_lifetime",
            Self::Refused(Refusal::TooManyGrants) => "desktop_refused_too_many_grants",
            Self::Refused(Refusal::ApprovalReused) => "desktop_refused_approval_reused",
            Self::Refused(Refusal::ConsentStale) => "desktop_refused_consent_stale",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Live,
    Used,
    Revoked,
}

/// The grants one Core has issued. Held in memory only: like approvals
/// (ADR 0009), no grant survives a restart.
#[derive(Debug, Default)]
pub struct Grants {
    grants: BTreeMap<GrantId, (Grant, State)>,
    /// The latest `now_ms` an issue or a check from the Core was given, even
    /// one that was then refused. It only rises: if the clock jumps forward
    /// and back, every grant is refused until the Core restarts. That fails
    /// closed, and only the Core supplies the time.
    latest_ms: u64,
}

fn is_live(grant: &Grant, state: State, now_ms: u64) -> bool {
    state == State::Live && grant.expires_at_ms > now_ms
}

impl Grants {
    /// Refuse a reading earlier than any seen before; otherwise remember it.
    fn observe(&mut self, now_ms: u64) -> Result<(), Refusal> {
        if now_ms < self.latest_ms {
            return Err(Refusal::ClockWentBack);
        }
        self.latest_ms = now_ms;
        Ok(())
    }

    /// Issue a grant for the request `consent` was bound to.
    pub fn issue(&mut self, consent: Consent, now_ms: u64, ttl_ms: u64) -> Result<Grant, Refusal> {
        if ttl_ms == 0 || ttl_ms > MAX_GRANT_TTL_MS {
            return Err(Refusal::InvalidLifetime);
        }
        self.observe(now_ms)?;
        if now_ms < consent.consumed_at_ms {
            return Err(Refusal::ClockWentBack);
        }
        if now_ms - consent.consumed_at_ms > MAX_CONSENT_AGE_MS {
            return Err(Refusal::ConsentStale);
        }
        let id = GrantId(consent.approval);
        if self.grants.contains_key(&id) {
            return Err(Refusal::ApprovalReused);
        }
        let live = self
            .grants
            .values()
            .filter(|(grant, state)| is_live(grant, *state, now_ms))
            .count();
        if live >= MAX_LIVE_GRANTS {
            return Err(Refusal::TooManyGrants);
        }
        // Forget the oldest finished grants beyond the bound.
        while self.grants.len() >= MAX_TRACKED_GRANTS {
            let oldest = self
                .grants
                .values()
                .filter(|(grant, state)| !is_live(grant, *state, now_ms))
                .min_by_key(|(grant, _)| grant.issued_at_ms)
                .map(|(grant, _)| grant.id);
            match oldest {
                Some(id) => self.grants.remove(&id),
                None => return Err(Refusal::TooManyGrants),
            };
        }
        let grant = Grant {
            id,
            request: consent.request,
            issued_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(ttl_ms),
        };
        self.grants.insert(id, (grant.clone(), State::Live));
        Ok(grant)
    }

    /// Revoke one grant. Revoking an unknown, used or revoked grant changes
    /// nothing.
    pub fn revoke(&mut self, id: GrantId) {
        if let Some((_, state)) = self.grants.get_mut(&id)
            && *state == State::Live
        {
            *state = State::Revoked;
        }
    }

    /// Revoke every live grant: on shutdown, when the worker's session ends,
    /// or whenever the person withdraws consent.
    pub fn revoke_all(&mut self) {
        for (_, state) in self.grants.values_mut() {
            if *state == State::Live {
                *state = State::Revoked;
            }
        }
    }

    /// Decide whether `caller` may have `request` performed under grant `id`
    /// now. An authorised check uses the grant up.
    pub fn authorize(
        &mut self,
        caller: Caller,
        id: GrantId,
        request: &DesktopRequest,
        now_ms: u64,
    ) -> Outcome {
        if caller != Caller::Core {
            return Outcome::Refused(Refusal::NotCore);
        }
        if let Err(refusal) = self.observe(now_ms) {
            return Outcome::Refused(refusal);
        }
        let Some((grant, state)) = self.grants.get_mut(&id) else {
            return Outcome::Refused(Refusal::UnknownGrant);
        };
        let refusal = match *state {
            State::Used => Some(Refusal::AlreadyUsed),
            State::Revoked => Some(Refusal::Revoked),
            State::Live if now_ms < grant.issued_at_ms => Some(Refusal::ClockWentBack),
            State::Live if now_ms >= grant.expires_at_ms => Some(Refusal::Expired),
            State::Live if grant.request != *request => Some(Refusal::ScopeMismatch),
            State::Live => None,
        };
        match refusal {
            Some(refusal) => Outcome::Refused(refusal),
            None => {
                *state = State::Used;
                Outcome::Authorized(request.capability())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_id() -> RequestId {
        RequestId::try_from("r1".to_owned()).unwrap()
    }

    /// A consumed approval whose fingerprint is that of `request`.
    fn consumed_for(request: &DesktopRequest) -> ConsumedApproval {
        let task = TaskId::new();
        ConsumedApproval::for_tests(
            ApprovalId::new(),
            task,
            request_id(),
            fingerprint(task, &request_id(), request),
            0,
        )
    }

    fn consent(request: DesktopRequest) -> Consent {
        Consent::bind(consumed_for(&request), request).unwrap()
    }

    fn clip(value: &str) -> ClipboardText {
        ClipboardText::try_from(value.to_owned()).unwrap()
    }

    fn text(value: &str) -> TypedText {
        TypedText::try_from(value.to_owned()).unwrap()
    }

    fn capture(target: u64) -> DesktopRequest {
        DesktopRequest::CaptureWindow {
            target: TargetId(target),
        }
    }

    fn issue(grants: &mut Grants, request: DesktopRequest, now_ms: u64) -> Grant {
        grants.issue(consent(request), now_ms, 5_000).unwrap()
    }

    #[test]
    fn every_desktop_capability_needs_a_person() {
        let requests = [
            capture(1),
            DesktopRequest::ReadClipboardText {},
            DesktopRequest::WriteClipboardText { text: clip("hi") },
            DesktopRequest::TypeText {
                target: TargetId(1),
                text: text("hi"),
            },
        ];
        for request in requests {
            assert_eq!(request.capability().class(), ActionClass::Confirm);
        }
    }

    #[test]
    fn consent_needs_an_approval_for_this_exact_request() {
        // Approved: capture window 7. Asked: capture window 8.
        let consumed = consumed_for(&capture(7));
        assert_eq!(
            Consent::bind(consumed, capture(8)).err(),
            Some(Refusal::NotApproved)
        );
        // Approved: type "a". Asked: type "b".
        let typed = |t| DesktopRequest::TypeText {
            target: TargetId(1),
            text: text(t),
        };
        assert_eq!(
            Consent::bind(consumed_for(&typed("a")), typed("b")).err(),
            Some(Refusal::NotApproved)
        );
    }

    #[test]
    fn a_tool_call_approval_is_not_desktop_consent() {
        use jarvis_protocol::{Capability, RelativePath, ToolCall, WriteFileArgs};
        let task = TaskId::new();
        let call = ToolCall::WriteFile(WriteFileArgs {
            path: RelativePath::try_from("notes.txt".to_owned()).unwrap(),
            content: "hi".to_owned(),
        });
        let tool_fingerprint =
            crate::approval::fingerprint(task, &request_id(), &call, &[Capability::WorkspaceWrite]);
        let consumed =
            ConsumedApproval::for_tests(ApprovalId::new(), task, request_id(), tool_fingerprint, 0);
        assert_eq!(
            Consent::bind(consumed, capture(1)).err(),
            Some(Refusal::NotApproved)
        );
    }

    #[test]
    fn a_grant_authorises_its_exact_request_once() {
        let mut grants = Grants::default();
        let grant = issue(&mut grants, capture(7), 1_000);
        let first = grants.authorize(Caller::Core, grant.id, &capture(7), 2_000);
        assert_eq!(first, Outcome::Authorized(DesktopCapability::ScreenObserve));
        assert_eq!(first.name(), "desktop_authorized");
        assert_eq!(
            grants.authorize(Caller::Core, grant.id, &capture(7), 2_001),
            Outcome::Refused(Refusal::AlreadyUsed)
        );
        // A used grant cannot be revoked back into some other state.
        grants.revoke(grant.id);
        assert_eq!(
            grants.authorize(Caller::Core, grant.id, &capture(7), 2_002),
            Outcome::Refused(Refusal::AlreadyUsed)
        );
    }

    #[test]
    fn one_approval_backs_one_grant_while_it_is_tracked() {
        // The store hands out one ConsumedApproval per approval (see the
        // store's consume tests); this is the defence behind it, inside the
        // tracking window.
        let mut grants = Grants::default();
        let request = capture(7);
        let consumed = consumed_for(&request);
        let (approval, task, fp) = (
            consumed.approval_id(),
            consumed.task_id(),
            consumed.fingerprint().clone(),
        );
        grants
            .issue(Consent::bind(consumed, request.clone()).unwrap(), 0, 5_000)
            .unwrap();
        let forged_twin = ConsumedApproval::for_tests(approval, task, request_id(), fp, 0);
        assert_eq!(
            grants.issue(Consent::bind(forged_twin, request).unwrap(), 0, 5_000),
            Err(Refusal::ApprovalReused)
        );
    }

    #[test]
    fn a_grant_id_from_an_earlier_run_names_nothing() {
        let mut before_restart = Grants::default();
        let stale = issue(&mut before_restart, capture(7), 0);
        let mut after_restart = Grants::default();
        issue(&mut after_restart, capture(7), 0);
        assert_eq!(
            after_restart.authorize(Caller::Core, stale.id, &capture(7), 1),
            Outcome::Refused(Refusal::UnknownGrant)
        );
    }

    #[test]
    fn only_the_core_is_accepted() {
        let mut grants = Grants::default();
        let grant = issue(&mut grants, capture(7), 0);
        let unknown = issue(&mut Grants::default(), capture(7), 0).id;
        for caller in [Caller::Worker, Caller::Client] {
            assert_eq!(
                grants.authorize(caller, grant.id, &capture(7), 1),
                Outcome::Refused(Refusal::NotCore)
            );
            // The caller is checked first: nothing about grants leaks.
            assert_eq!(
                grants.authorize(caller, unknown, &capture(7), 1),
                Outcome::Refused(Refusal::NotCore)
            );
        }
        // A refused caller does not use the grant up.
        assert!(matches!(
            grants.authorize(Caller::Core, grant.id, &capture(7), 1),
            Outcome::Authorized(_)
        ));
    }

    #[test]
    fn an_expired_grant_is_refused() {
        let mut grants = Grants::default();
        let grant = issue(&mut grants, capture(7), 1_000);
        assert_eq!(
            grants.authorize(Caller::Core, grant.id, &capture(7), 6_000),
            Outcome::Refused(Refusal::Expired)
        );
    }

    #[test]
    fn a_clock_that_goes_back_does_not_extend_a_grant() {
        let mut grants = Grants::default();
        let grant = issue(&mut grants, capture(7), 10_000);
        assert_eq!(
            grants.authorize(Caller::Core, grant.id, &capture(7), 9_999),
            Outcome::Refused(Refusal::ClockWentBack)
        );
    }

    #[test]
    fn a_consent_cannot_be_kept_for_later() {
        let mut grants = Grants::default();
        assert_eq!(
            grants.issue(consent(capture(1)), MAX_CONSENT_AGE_MS + 1, 5_000),
            Err(Refusal::ConsentStale)
        );
        let task = TaskId::new();
        let later = ConsumedApproval::for_tests(
            ApprovalId::new(),
            task,
            request_id(),
            fingerprint(task, &request_id(), &capture(1)),
            1_000,
        );
        let consent = Consent::bind(later, capture(1)).unwrap();
        assert_eq!(
            Grants::default().issue(consent, 999, 5_000),
            Err(Refusal::ClockWentBack)
        );
    }

    #[test]
    fn a_consent_from_before_the_epoch_is_refused() {
        let task = TaskId::new();
        let broken = ConsumedApproval::for_tests(
            ApprovalId::new(),
            task,
            request_id(),
            fingerprint(task, &request_id(), &capture(1)),
            -1,
        );
        assert_eq!(
            Consent::bind(broken, capture(1)).err(),
            Some(Refusal::ClockWentBack)
        );
    }

    #[test]
    fn stepping_the_clock_back_after_a_check_does_not_extend_a_grant() {
        let mut grants = Grants::default();
        let grant = issue(&mut grants, capture(7), 10_000);
        // Any check at 12 000 records that time, even a refused one.
        assert_eq!(
            grants.authorize(Caller::Core, grant.id, &capture(8), 12_000),
            Outcome::Refused(Refusal::ScopeMismatch)
        );
        // The clock is set back to just after the issue: still refused.
        assert_eq!(
            grants.authorize(Caller::Core, grant.id, &capture(7), 10_001),
            Outcome::Refused(Refusal::ClockWentBack)
        );
        // Issuing on the stepped-back clock is refused too.
        assert_eq!(
            grants.issue(consent(capture(1)), 10_001, 5_000),
            Err(Refusal::ClockWentBack)
        );
        // Back on time, the grant is still live and used once.
        assert!(matches!(
            grants.authorize(Caller::Core, grant.id, &capture(7), 12_001),
            Outcome::Authorized(_)
        ));
    }

    #[test]
    fn a_revoked_grant_is_refused() {
        let mut grants = Grants::default();
        let one = issue(&mut grants, capture(1), 0);
        let two = issue(&mut grants, capture(2), 0);
        grants.revoke(one.id);
        assert_eq!(
            grants.authorize(Caller::Core, one.id, &capture(1), 1),
            Outcome::Refused(Refusal::Revoked)
        );
        grants.revoke_all();
        assert_eq!(
            grants.authorize(Caller::Core, two.id, &capture(2), 1),
            Outcome::Refused(Refusal::Revoked)
        );
    }

    #[test]
    fn a_finished_grant_keeps_its_reason_after_more_are_issued() {
        let mut grants = Grants::default();
        let used = issue(&mut grants, capture(1), 0);
        let revoked = issue(&mut grants, capture(2), 0);
        grants.authorize(Caller::Core, used.id, &capture(1), 1);
        grants.revoke(revoked.id);
        issue(&mut grants, capture(3), 10);
        assert_eq!(
            grants.authorize(Caller::Core, used.id, &capture(1), 11),
            Outcome::Refused(Refusal::AlreadyUsed)
        );
        assert_eq!(
            grants.authorize(Caller::Core, revoked.id, &capture(2), 11),
            Outcome::Refused(Refusal::Revoked)
        );
    }

    #[test]
    fn a_different_request_is_out_of_scope() {
        let mut grants = Grants::default();
        let grant = issue(&mut grants, capture(7), 0);
        for request in [capture(8), DesktopRequest::ReadClipboardText {}] {
            assert_eq!(
                grants.authorize(Caller::Core, grant.id, &request, 1),
                Outcome::Refused(Refusal::ScopeMismatch)
            );
        }
        let approved = DesktopRequest::TypeText {
            target: TargetId(7),
            text: text("approved"),
        };
        let typed = issue(&mut grants, approved, 1);
        let changed_text = DesktopRequest::TypeText {
            target: TargetId(7),
            text: text("something else"),
        };
        assert_eq!(
            grants.authorize(Caller::Core, typed.id, &changed_text, 2),
            Outcome::Refused(Refusal::ScopeMismatch)
        );
    }

    #[test]
    fn grant_lifetimes_and_counts_are_bounded() {
        let mut grants = Grants::default();
        for ttl in [0, MAX_GRANT_TTL_MS + 1] {
            assert_eq!(
                grants.issue(consent(capture(1)), 0, ttl),
                Err(Refusal::InvalidLifetime)
            );
        }
        for _ in 0..MAX_LIVE_GRANTS {
            issue(&mut grants, capture(1), 0);
        }
        assert_eq!(
            grants.issue(consent(capture(1)), 0, 5_000),
            Err(Refusal::TooManyGrants)
        );
        // Expired grants no longer count as live.
        assert!(grants.issue(consent(capture(1)), 5_000, 5_000).is_ok());
    }

    #[test]
    fn finished_grants_are_tracked_within_a_bound() {
        let mut grants = Grants::default();
        for i in 0..(MAX_TRACKED_GRANTS as u64 * 2) {
            let grant = issue(&mut grants, capture(1), i);
            grants.authorize(Caller::Core, grant.id, &capture(1), i);
        }
        assert!(grants.grants.len() <= MAX_TRACKED_GRANTS);
    }

    #[test]
    fn worker_data_cannot_carry_authority() {
        let smuggled = [
            r#"{"op":"capture_window","target":1,"grant":1}"#,
            r#"{"op":"read_clipboard_text","approval_id":"x"}"#,
            r#"{"op":"type_text","target":1,"text":"hi","capability":"input_send"}"#,
            r#"{"op":"capture_screen"}"#,
            r#"{"op":"run","command":"cmd.exe"}"#,
        ];
        for json in smuggled {
            assert!(
                serde_json::from_str::<DesktopRequest>(json).is_err(),
                "{json}"
            );
        }
        let plain: DesktopRequest =
            serde_json::from_str(r#"{"op":"capture_window","target":1}"#).unwrap();
        assert_eq!(plain, capture(1));
    }

    #[test]
    fn typed_text_cannot_press_keys_or_hide_characters() {
        for bad in ["\u{1b}[A", "a\u{8}", "\r", "\u{7f}", "send\n", "a\tb"] {
            assert_eq!(
                TypedText::try_from(bad.to_owned()),
                Err(InvalidText::Control),
                "{bad:?}"
            );
        }
        for hidden in ["pay\u{202E}evil", "a\u{200B}b", "\u{FEFF}x", "x\u{2066}y"] {
            assert_eq!(
                TypedText::try_from(hidden.to_owned()),
                Err(InvalidText::Invisible)
            );
            assert_eq!(
                ClipboardText::try_from(hidden.to_owned()),
                Err(InvalidText::Invisible)
            );
        }
        assert!(TypedText::try_from("plain text, ok".to_owned()).is_ok());
        assert!(ClipboardText::try_from("line\n\tindented".to_owned()).is_ok());
        assert_eq!(
            ClipboardText::try_from("\r".to_owned()),
            Err(InvalidText::Control)
        );
        assert_eq!(
            TypedText::try_from("x".repeat(MAX_TEXT_CHARS + 1)),
            Err(InvalidText::TooLong)
        );
    }
}
