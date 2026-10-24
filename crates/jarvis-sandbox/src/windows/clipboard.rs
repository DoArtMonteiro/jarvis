//! Writing text to the Windows clipboard.
//!
//! Order of work, chosen so that a failure leaves the clipboard alone
//! whenever possible:
//!
//! 1. Build every global memory block, register the format names and create
//!    the owner window while the clipboard is untouched. A failure here is
//!    [`ClipboardError::Unavailable`].
//! 2. Open the clipboard exactly once. If another program has it, return
//!    [`ClipboardError::Busy`] at once: no retry, no sleep.
//! 3. Empty it, set the text, then set the privacy hints. From the empty on,
//!    any failure is [`ClipboardError::Indeterminate`] (fail closed): the
//!    previous content is gone or the result is not what was promised.
//!
//! The clipboard is never read.
//!
//! Owner window: `OpenClipboard(NULL)` makes `EmptyClipboard` set the owner
//! to NULL, and the documentation says `SetClipboardData` then fails. This
//! module follows the documented contract and uses a message-only window as
//! owner, destroyed after the clipboard is closed.

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{GetLastError, GlobalFree, HANDLE, HWND};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, HWND_MESSAGE};

use crate::ClipboardError;

/// Keeps the text out of clipboard monitors, including clipboard history.
const EXCLUDE_FROM_MONITORS: &str = "ExcludeClipboardContentFromMonitorProcessing";
/// A DWORD of 0 keeps the content out of Win+V history.
const CAN_INCLUDE_IN_HISTORY: &str = "CanIncludeInClipboardHistory";
/// A DWORD of 0 keeps the content out of the cloud clipboard.
const CAN_UPLOAD_TO_CLOUD: &str = "CanUploadToCloudClipboard";

/// A movable global memory block that nobody owns yet. Dropping it frees the
/// block; [`Global::hand_over`] gives it to the system after a successful
/// `SetClipboardData`.
struct Global(*mut c_void);

impl Global {
    /// Allocate a block holding a copy of `bytes`.
    fn with_bytes(bytes: &[u8]) -> Result<Self, ClipboardError> {
        // SAFETY: plain allocation call; a null result is handled below.
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) };
        if handle.is_null() {
            return Err(unavailable("allocating memory", last_error()));
        }
        // From here `block` frees the allocation on every early return.
        let block = Self(handle);
        // SAFETY: `handle` is a live movable block from `GlobalAlloc`.
        let view = unsafe { GlobalLock(handle) };
        if view.is_null() {
            return Err(unavailable("locking memory", last_error()));
        }
        // SAFETY: the block holds `bytes.len()` writable bytes at `view`
        // (it was allocated with that size) and does not overlap the slice.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), view.cast::<u8>(), bytes.len()) };
        // SAFETY: balances the successful lock above. The block is ours, so
        // the lock count drops to zero; the return value says nothing more.
        unsafe { GlobalUnlock(handle) };
        Ok(block)
    }

    fn handle(&self) -> HANDLE {
        self.0
    }

    /// The system owns the block now; do not free it.
    fn hand_over(self) {
        std::mem::forget(self);
    }
}

impl Drop for Global {
    fn drop(&mut self) {
        // SAFETY: the block is ours (never handed to the system) and not
        // locked, so freeing it once here is sound.
        unsafe { GlobalFree(self.0) };
    }
}

/// A message-only window, destroyed on drop.
struct OwnerWindow(HWND);

impl OwnerWindow {
    fn create() -> Result<Self, ClipboardError> {
        let class: Vec<u16> = "STATIC".encode_utf16().chain(Some(0)).collect();
        // SAFETY: NUL-terminated name of a system window class; every other
        // argument is null or zero. The parent `HWND_MESSAGE` makes the
        // window message-only. A null result is handled below.
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                null(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                null_mut(),
                null(),
            )
        };
        if window.is_null() {
            return Err(unavailable("creating the owner window", last_error()));
        }
        Ok(Self(window))
    }
}

impl Drop for OwnerWindow {
    fn drop(&mut self) {
        // SAFETY: this thread created the window and nothing else destroys
        // it. A failure leaves an inert message-only window until the thread
        // ends.
        unsafe { DestroyWindow(self.0) };
    }
}

/// The clipboard, open for this thread. Closed by [`OpenClipboardGuard::close`]
/// or, on an early return, on drop.
struct OpenClipboardGuard {
    open: bool,
}

impl OpenClipboardGuard {
    /// One attempt, no retry.
    fn open(owner: HWND) -> Result<Self, ClipboardError> {
        // SAFETY: `owner` is a live window of this thread.
        if unsafe { OpenClipboard(owner) } == 0 {
            return Err(ClipboardError::Busy);
        }
        Ok(Self { open: true })
    }

    fn close(mut self) -> Result<(), ClipboardError> {
        self.open = false;
        // SAFETY: this thread has the clipboard open.
        if unsafe { CloseClipboard() } == 0 {
            return Err(indeterminate("closing the clipboard", last_error()));
        }
        Ok(())
    }
}

impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        if self.open {
            // SAFETY: this thread has the clipboard open. Callers that need
            // the result use `close`.
            unsafe { CloseClipboard() };
        }
    }
}

fn last_error() -> u32 {
    // SAFETY: reads the calling thread's last-error value.
    unsafe { GetLastError() }
}

fn unavailable(step: &str, code: u32) -> ClipboardError {
    ClipboardError::Unavailable(format!("{step} failed (error {code})"))
}

fn indeterminate(step: &str, code: u32) -> ClipboardError {
    ClipboardError::Indeterminate(format!("{step} failed (error {code})"))
}

fn register_format(name: &str) -> Result<u32, ClipboardError> {
    let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    // SAFETY: NUL-terminated format name.
    let format = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
    if format == 0 {
        return Err(unavailable("registering a clipboard format", last_error()));
    }
    Ok(format)
}

/// One extra clipboard format: its id and the data to put under it.
struct Hint {
    format: u32,
    data: Global,
}

fn hint(name: &str, bytes: &[u8]) -> Result<Hint, ClipboardError> {
    Ok(Hint {
        format: register_format(name)?,
        data: Global::with_bytes(bytes)?,
    })
}

/// Hand `data` to the clipboard under `format`. The clipboard must be open
/// and emptied by this thread.
fn set(format: u32, data: Global, step: &str) -> Result<(), ClipboardError> {
    // SAFETY: the clipboard is open and owned by a window of this thread, and
    // `data` is a movable block that holds the format's data.
    let result = unsafe { SetClipboardData(format, data.handle()) };
    if result.is_null() {
        // The system did not take the block, so dropping `data` frees it.
        return Err(indeterminate(step, last_error()));
    }
    data.hand_over();
    Ok(())
}

pub(crate) fn write_text(text: &str) -> Result<(), ClipboardError> {
    // Step 1: everything that can fail without touching the clipboard.
    let mut bytes: Vec<u8> = Vec::new();
    for unit in text.encode_utf16().chain(Some(0)) {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    let content = Global::with_bytes(&bytes)?;
    let hints = [
        hint(EXCLUDE_FROM_MONITORS, &0_u32.to_le_bytes())?,
        hint(CAN_INCLUDE_IN_HISTORY, &0_u32.to_le_bytes())?,
        hint(CAN_UPLOAD_TO_CLOUD, &0_u32.to_le_bytes())?,
    ];
    // Declared before the clipboard guard, so it is destroyed after the
    // clipboard is closed.
    let owner = OwnerWindow::create()?;

    // Step 2: one attempt to open.
    let clipboard = OpenClipboardGuard::open(owner.0)?;

    // Step 3: from here on, a failure may have changed the clipboard.
    // SAFETY: this thread has the clipboard open.
    if unsafe { EmptyClipboard() } == 0 {
        return Err(indeterminate("emptying the clipboard", last_error()));
    }
    set(
        u32::from(CF_UNICODETEXT),
        content,
        "setting the clipboard text",
    )?;
    for hint in hints {
        set(hint.format, hint.data, "setting a clipboard privacy hint")?;
    }
    clipboard.close()
}
