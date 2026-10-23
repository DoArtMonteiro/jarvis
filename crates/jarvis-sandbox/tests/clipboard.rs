//! `write_clipboard_text` against a real Windows clipboard, in a window
//! station of its own.
//!
//! The clipboard belongs to a window station, so each test re-runs itself in
//! a child process that first moves into a fresh, non-interactive window
//! station and desktop. Only there does the test read the clipboard back, as
//! an observer. The child refuses to continue unless its window station is
//! not the interactive one, so a failed isolation can never touch the
//! developer's clipboard.
#![cfg(windows)]

use std::ffi::c_void;
use std::process::Command;
use std::ptr::{null, null_mut};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use jarvis_sandbox::{ClipboardError, write_clipboard_text};
use windows_sys::Win32::Foundation::{GENERIC_ALL, HANDLE};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW,
};
use windows_sys::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows_sys::Win32::System::Ole::CF_UNICODETEXT;
use windows_sys::Win32::System::StationsAndDesktops::{
    CreateDesktopW, CreateWindowStationW, GetProcessWindowStation, GetUserObjectInformationW,
    SetProcessWindowStation, SetThreadDesktop, UOI_NAME,
};

const CHILD_MARKER: &str = "JARVIS_CLIPBOARD_TEST_CHILD";

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Run `body` inside an isolated window station: in this process when it is
/// the marked child, otherwise in a child that re-runs the named test.
fn isolated(test: &str, body: impl FnOnce()) {
    if std::env::var_os(CHILD_MARKER).is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test, "--nocapture", "--test-threads=1"])
            .env(CHILD_MARKER, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "child failed
{}
{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        // The child must really have run the body, not filtered it out.
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed"),
            "child ran no test"
        );
        return;
    }
    enter_isolated_station();
    body();
}

/// Switch this process and thread to a new window station and desktop, and
/// stop unless the process is no longer in the station it started in.
///
/// An unnamed station is used because creating a named one is refused for a
/// standard user. Windows names it after the logon session; it is not the
/// interactive station and nothing else runs in it, so the developer's
/// clipboard is untouched. Every scenario runs inside one child process,
/// because a second child would share this station.
fn enter_isolated_station() {
    let original = process_station_name();
    // SAFETY: a null name asks for the logon session's non-interactive
    // station; the other arguments are flags and null.
    let station = unsafe { CreateWindowStationW(null(), 0, GENERIC_ALL, null()) };
    assert!(!station.is_null(), "CreateWindowStationW failed");
    // SAFETY: `station` is a valid window station handle.
    assert_ne!(unsafe { SetProcessWindowStation(station) }, 0);
    let current = process_station_name();
    assert!(
        current != original && !current.eq_ignore_ascii_case("WinSta0"),
        "not in an isolated window station: {current}"
    );
    enter_desktop();
}

/// Give the calling thread a desktop of the isolated station.
fn enter_desktop() {
    let desktop_name = wide("isolated");
    // SAFETY: NUL-terminated name; the process is in the isolated station.
    let desktop = unsafe {
        CreateDesktopW(
            desktop_name.as_ptr(),
            null(),
            null(),
            0,
            GENERIC_ALL,
            null(),
        )
    };
    assert!(!desktop.is_null(), "CreateDesktopW failed");
    // SAFETY: `desktop` is valid and this thread has no windows or hooks yet.
    assert_ne!(unsafe { SetThreadDesktop(desktop) }, 0, "SetThreadDesktop");
}

fn process_station_name() -> String {
    let mut buffer = [0_u16; 128];
    let mut needed = 0_u32;
    // SAFETY: the buffer length in bytes is passed; the handle is the
    // process's current window station.
    let ok = unsafe {
        GetUserObjectInformationW(
            GetProcessWindowStation(),
            UOI_NAME,
            buffer.as_mut_ptr().cast::<c_void>(),
            size_of_val(&buffer) as u32,
            &mut needed,
        )
    };
    assert_ne!(ok, 0);
    let length = buffer.iter().position(|&unit| unit == 0).unwrap();
    String::from_utf16(&buffer[..length]).unwrap()
}

/// What the observer found under one format.
fn read_format(format: u32) -> Option<Vec<u8>> {
    // SAFETY: the observer opens, reads and closes the clipboard of the
    // isolated station on this thread.
    unsafe {
        assert_ne!(OpenClipboard(null_mut()), 0, "observer could not open");
        let handle: HANDLE = GetClipboardData(format);
        let bytes = if handle.is_null() {
            None
        } else {
            let size = GlobalSize(handle);
            let view = GlobalLock(handle).cast::<u8>();
            assert!(!view.is_null());
            let bytes = std::slice::from_raw_parts(view, size).to_vec();
            GlobalUnlock(handle);
            Some(bytes)
        };
        CloseClipboard();
        bytes
    }
}

/// The clipboard text up to its terminating NUL, as UTF-16 units.
fn read_text() -> Option<Vec<u16>> {
    let bytes = read_format(u32::from(CF_UNICODETEXT))?;
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    let end = units.iter().position(|&unit| unit == 0).unwrap();
    assert_eq!(end + 1, units.len(), "exactly one terminator, at the end");
    Some(units[..end].to_vec())
}

fn registered(name: &str) -> u32 {
    let name = wide(name);
    // SAFETY: NUL-terminated format name.
    unsafe { RegisterClipboardFormatW(name.as_ptr()) }
}

fn round_trips_text_exactly() {
    {
        for text in [
            "hello",
            "",
            "line one\nline two\ttabbed\n",
            "caf\u{e9} \u{2713} \u{1F600}",
        ] {
            write_clipboard_text(text).unwrap();
            let expected: Vec<u16> = text.encode_utf16().collect();
            assert_eq!(read_text(), Some(expected), "round trip of {text:?}");
        }
    }
}

fn marks_text_out_of_history_and_cloud() {
    {
        write_clipboard_text("secret").unwrap();
        for name in [
            "ExcludeClipboardContentFromMonitorProcessing",
            "CanIncludeInClipboardHistory",
            "CanUploadToCloudClipboard",
        ] {
            // SAFETY: plain query of a registered format.
            assert_ne!(unsafe { IsClipboardFormatAvailable(registered(name)) }, 0);
        }
        for name in ["CanIncludeInClipboardHistory", "CanUploadToCloudClipboard"] {
            let value = read_format(registered(name)).unwrap();
            assert_eq!(value[..4], 0_u32.to_le_bytes(), "{name} is a DWORD 0");
        }
    }
}

fn reports_busy_and_leaves_content_unchanged() {
    {
        write_clipboard_text("before").unwrap();

        let (opened_tx, opened_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let holder = thread::spawn(move || {
            enter_desktop();
            // SAFETY: opened and closed on this thread.
            unsafe {
                assert_ne!(OpenClipboard(null_mut()), 0);
                opened_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(30)).unwrap();
                CloseClipboard();
            }
        });
        opened_rx.recv_timeout(Duration::from_secs(30)).unwrap();

        let error = write_clipboard_text("after").unwrap_err();
        assert_eq!(error, ClipboardError::Busy);
        assert!(error.changed_nothing());

        release_tx.send(()).unwrap();
        holder.join().unwrap();
        let expected: Vec<u16> = "before".encode_utf16().collect();
        assert_eq!(read_text(), Some(expected));
    }
}

#[test]
fn writes_clipboard_text_in_an_isolated_window_station() {
    isolated(
        "writes_clipboard_text_in_an_isolated_window_station",
        || {
            round_trips_text_exactly();
            marks_text_out_of_history_and_cloud();
            reports_busy_and_leaves_content_unchanged();
        },
    );
}
