//! A write-only clipboard primitive.
//!
//! The Core uses it to put text a person approved onto the clipboard. It has
//! no way to read the clipboard: nothing a user copied elsewhere can reach
//! the Core through this module.

/// Why a clipboard write did not complete.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClipboardError {
    /// Another process holds the clipboard open. Nothing was changed.
    #[error("the clipboard is in use by another program")]
    Busy,
    /// This platform has no clipboard support here. Nothing was changed.
    #[error("writing the clipboard is not supported on this platform")]
    Unsupported,
    /// Failed before the clipboard was touched. Nothing was changed.
    #[error("the clipboard could not be prepared: {0}")]
    Unavailable(String),
    /// Failed after the clipboard was emptied or while closing it: its content may have changed.
    #[error("the clipboard write did not complete: {0}")]
    Indeterminate(String),
}

impl ClipboardError {
    /// Whether the clipboard is certain to hold what it held before the call.
    pub fn changed_nothing(&self) -> bool {
        !matches!(self, Self::Indeterminate(_))
    }
}

/// Replace the clipboard's content with `text` as `CF_UNICODETEXT`, exactly as
/// given (no newline conversion). Never reads the clipboard.
///
/// On Windows the call opens the clipboard once and does not retry or wait:
/// if another program holds it, the result is [`ClipboardError::Busy`] and the
/// caller decides what to do. The text is also marked as excluded from
/// clipboard history and cloud sync. Other platforms return
/// [`ClipboardError::Unsupported`].
pub fn write_clipboard_text(text: &str) -> Result<(), ClipboardError> {
    #[cfg(windows)]
    {
        crate::windows::clipboard::write_text(text)
    }
    #[cfg(not(windows))]
    {
        let _ = text;
        Err(ClipboardError::Unsupported)
    }
}

#[cfg(all(test, not(windows)))]
mod tests {
    use super::*;

    #[test]
    fn unsupported_off_windows_changes_nothing() {
        let error = write_clipboard_text("x").unwrap_err();
        assert_eq!(error, ClipboardError::Unsupported);
        assert!(error.changed_nothing());
    }
}
