//! Approval support: the fingerprint that binds an approval to a request,
//! the display-safe description a person decides on, and the in-memory
//! signal that wakes a waiting session.
//!
//! The database is the source of truth for every decision. The broker only
//! says "look again"; a session always re-reads the approval before acting.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use jarvis_protocol::{
    ApprovalId, ApprovalView, Capability, Fingerprint, RequestId, TaskId, ToolCall,
};
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

use crate::store::{ApprovalRecord, format_ms};

/// SHA-256 over a canonical encoding of everything an approval authorises:
/// the task, the request, the tool, the normalised arguments and the
/// capabilities. Any difference in any of them gives a different value.
pub fn fingerprint(
    task_id: TaskId,
    request_id: &RequestId,
    call: &ToolCall,
    capabilities: &[Capability],
) -> Fingerprint {
    let mut capabilities = capabilities.to_vec();
    capabilities.sort();
    capabilities.dedup();
    // serde_json keeps object keys sorted, so this encoding is canonical.
    let canonical = serde_json::json!({
        "v": 1,
        "task_id": task_id,
        "request_id": request_id,
        "tool": call.tool_name(),
        "args": call.arguments(),
        "capabilities": capabilities,
    });
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    Fingerprint::from_digest(&digest)
}

/// What a person needs to see to decide on a call, with every value made
/// safe to print: control characters escaped and long text shortened. Every
/// Session Agent operation states its action in words, so a person never
/// decides on a fingerprint alone; text that may be cut in the preview also
/// carries its size and SHA-256, which the fingerprint binds.
pub fn describe(call: &ToolCall) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    match call {
        ToolCall::SessionAgentHealthcheck(args) => {
            out.insert(
                "action".into(),
                "Run the Session Agent's health check. It reads nothing from the desktop.".into(),
            );
            out.insert("label".into(), display_safe(args.label.as_str(), 64));
        }
        ToolCall::CaptureWindow(args) => {
            out.insert(
                "action".into(),
                "Capture one image of one window (not available yet).".into(),
            );
            out.insert("target".into(), args.target.to_string());
        }
        ToolCall::ReadClipboardText(_) => {
            out.insert(
                "action".into(),
                "Read the text on the clipboard (not available yet).".into(),
            );
        }
        ToolCall::WriteClipboardText(args) => {
            // Short enough to show in full: the person approves every
            // character, with line ends and tabs made visible.
            let text = args.text.as_str();
            out.insert(
                "action".into(),
                "Replace the clipboard's content with exactly this text. Nothing is read from \
                 the clipboard; any program can paste it."
                    .into(),
            );
            out.insert("destination".into(), "the clipboard".into());
            out.insert("text".into(), display_safe(text, usize::MAX));
            out.insert("characters".into(), text.chars().count().to_string());
            out.insert("lines".into(), text.split('\n').count().to_string());
            describe_text(&mut out, text);
            out.remove("preview");
        }
        ToolCall::TypeText(args) => {
            out.insert(
                "action".into(),
                "Type text into one window (not available yet).".into(),
            );
            out.insert("target".into(), args.target.to_string());
            describe_text(&mut out, args.text.as_str());
        }
        ToolCall::SystemInfo(_) => {}
        ToolCall::ReadFixture(args) => {
            out.insert("path".into(), display_safe(args.path.as_str(), 255));
        }
        ToolCall::WriteFile(args) => {
            out.insert("path".into(), display_safe(args.path.as_str(), 255));
            describe_text(&mut out, &args.content);
        }
    }
    out
}

/// Size, SHA-256 and a display-safe preview of text a call would write or
/// type.
fn describe_text(out: &mut BTreeMap<String, String>, text: &str) {
    out.insert("bytes".into(), text.len().to_string());
    let digest: [u8; 32] = Sha256::digest(text.as_bytes()).into();
    out.insert(
        "sha256".into(),
        Fingerprint::from_digest(&digest).as_str().to_owned(),
    );
    out.insert("preview".into(), display_safe(text, 160));
}

/// Escape control characters and backslashes and cut to `max_chars`,
/// marking the cut. A backslash is shown as `\\`, so a newline (`\n`) and a
/// backslash followed by `n` (`\\n`) never look the same.
pub fn display_safe(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (count, c) in text.chars().enumerate() {
        if count == max_chars {
            out.push('…');
            break;
        }
        if c.is_control() || c == '\\' {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

/// The view a client gets of a stored approval.
pub fn view(record: &ApprovalRecord, now_ms: i64) -> ApprovalView {
    let arguments = serde_json::from_str(&record.args)
        .ok()
        .and_then(|args| ToolCall::from_request(&record.tool, &args).ok())
        .map(|call| describe(&call))
        .unwrap_or_else(|| BTreeMap::from([("raw".to_owned(), display_safe(&record.args, 160))]));
    ApprovalView {
        approval_id: record.approval_id,
        status: record.status,
        task_id: record.task_id,
        job_id: record.job_id,
        request_id: record.request_id.clone(),
        tool: record.tool.clone(),
        capabilities: record.capabilities.clone(),
        arguments,
        fingerprint: record.fingerprint.clone(),
        requested_at: record.requested_at.clone(),
        expires_at: format_ms(record.expires_at_ms),
        expires_in_ms: u64::try_from(record.expires_at_ms.saturating_sub(now_ms)).unwrap_or(0),
    }
}

/// Wakes the session that waits on an approval when a decision lands.
///
/// `Notify::notify_one` stores a permit when nobody is waiting yet, so a
/// decision that arrives between registration and the wait is not lost.
#[derive(Debug, Default)]
pub struct ApprovalBroker {
    waiters: Mutex<HashMap<ApprovalId, Arc<Notify>>>,
}

impl ApprovalBroker {
    pub fn register(&self, approval_id: ApprovalId) -> Arc<Notify> {
        let mut waiters = self
            .waiters
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        Arc::clone(waiters.entry(approval_id).or_default())
    }

    pub fn notify(&self, approval_id: ApprovalId) {
        let waiters = self
            .waiters
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(notify) = waiters.get(&approval_id) {
            notify.notify_one();
        }
    }

    pub fn remove(&self, approval_id: ApprovalId) {
        let mut waiters = self
            .waiters
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        waiters.remove(&approval_id);
    }
}

#[cfg(test)]
mod tests {
    use jarvis_protocol::{RelativePath, WriteFileArgs};

    use super::*;

    fn write(path: &str, content: &str) -> ToolCall {
        ToolCall::WriteFile(WriteFileArgs {
            path: RelativePath::try_from(path.to_owned()).unwrap(),
            content: content.to_owned(),
        })
    }

    fn request(id: &str) -> RequestId {
        RequestId::try_from(id.to_owned()).unwrap()
    }

    #[test]
    fn fingerprint_changes_with_anything_it_binds() {
        let task = TaskId::new();
        let caps = [Capability::WorkspaceWrite];
        let base = fingerprint(task, &request("r1"), &write("a.txt", "x"), &caps);
        assert_eq!(
            base,
            fingerprint(task, &request("r1"), &write("a.txt", "x"), &caps),
            "deterministic"
        );
        for other in [
            fingerprint(TaskId::new(), &request("r1"), &write("a.txt", "x"), &caps),
            fingerprint(task, &request("r2"), &write("a.txt", "x"), &caps),
            fingerprint(task, &request("r1"), &write("b.txt", "x"), &caps),
            fingerprint(task, &request("r1"), &write("a.txt", "y"), &caps),
            fingerprint(task, &request("r1"), &write("a.txt", "x"), &[]),
        ] {
            assert_ne!(base, other);
        }
    }

    #[test]
    fn capability_order_does_not_matter() {
        let task = TaskId::new();
        let call = write("a.txt", "x");
        assert_eq!(
            fingerprint(
                task,
                &request("r"),
                &call,
                &[Capability::SystemInfo, Capability::WorkspaceWrite]
            ),
            fingerprint(
                task,
                &request("r"),
                &call,
                &[Capability::WorkspaceWrite, Capability::SystemInfo]
            ),
        );
    }

    #[test]
    fn description_is_safe_to_print() {
        let described = describe(&write("a.txt", "line one\n\u{1b}[2Jline two"));
        let preview = &described["preview"];
        assert!(!preview.chars().any(char::is_control), "{preview}");
        assert!(preview.contains("\\n"));
        assert_eq!(described["bytes"], "21");
        assert_eq!(described["sha256"].len(), 64);
        let long = describe(&write("a.txt", &"x".repeat(500)));
        assert!(long["preview"].ends_with('…'));
        assert_eq!(long["preview"].chars().count(), 161);
    }

    #[test]
    fn a_person_never_decides_on_a_hash_alone() {
        for call in crate::policy::tests::one_of_each() {
            let described = describe(&call);
            if call.is_agent_operation() {
                let action = described.get("action").expect("an action in words");
                assert!(!action.is_empty());
            }
            for value in described.values() {
                assert!(!value.chars().any(char::is_control), "{value}");
            }
            match &call {
                ToolCall::CaptureWindow(_) | ToolCall::TypeText(_) => {
                    assert_eq!(described["target"], "window #7");
                }
                ToolCall::SessionAgentHealthcheck(_) => assert_eq!(described["label"], "ping"),
                _ => {}
            }
            if let ToolCall::WriteClipboardText(_) = call {
                assert_eq!(described["text"], "hello");
                assert_eq!(described["characters"], "5");
                assert_eq!(described["destination"], "the clipboard");
                assert!(!described.contains_key("preview"));
            }
            if let ToolCall::TypeText(_) = call {
                assert_eq!(described["preview"], "hello");
            }
            if let ToolCall::TypeText(_) | ToolCall::WriteClipboardText(_) = call {
                assert_eq!(described["bytes"], "5");
                assert_eq!(described["sha256"].len(), 64);
            }
        }
    }

    #[test]
    fn different_texts_never_display_the_same() {
        // A real newline and a backslash followed by `n`.
        assert_eq!(display_safe("a\nb", 10), "a\\nb");
        assert_eq!(display_safe("a\\nb", 10), "a\\\\nb");
        assert_eq!(display_safe("a\tb", 10), "a\\tb");
        assert_eq!(display_safe("a\\tb", 10), "a\\\\tb");
        let call = |text: &str| {
            ToolCall::WriteClipboardText(jarvis_protocol::WriteClipboardTextArgs {
                text: jarvis_protocol::ClipboardText::try_from(text.to_owned()).unwrap(),
            })
        };
        let newline = describe(&call("ls\nrm -rf ~"));
        let literal = describe(&call("ls\\nrm -rf ~"));
        assert_ne!(newline["text"], literal["text"]);
        assert_eq!(newline["lines"], "2");
        assert_eq!(literal["lines"], "1");
    }

    #[tokio::test]
    async fn decision_before_the_wait_is_not_lost() {
        let broker = ApprovalBroker::default();
        let id = ApprovalId::new();
        let notify = broker.register(id);
        broker.notify(id);
        tokio::time::timeout(std::time::Duration::from_secs(1), notify.notified())
            .await
            .expect("stored permit");
        broker.remove(id);
        broker.notify(id);
    }
}
