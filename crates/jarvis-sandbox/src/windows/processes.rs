//! Process names from a Toolhelp snapshot.
//!
//! A snapshot copies the system's process table. Reading it needs no access
//! to any process in it: nothing here calls `OpenProcess`, so there is no
//! handle to a target, and the only handle this code owns is the snapshot.

use std::io;
use std::mem::{size_of, zeroed};

use windows_sys::Win32::Foundation::{ERROR_NO_MORE_FILES, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

use super::{Handle, last_error};
use crate::processes::ProcessListError;

/// Windows keeps the whole file name.
pub(crate) const PROCESS_NAME_LIMIT: Option<usize> = None;

/// Call `visit` with the executable file name of every process in a snapshot.
/// A list that cannot be read to its end is an error, not a shorter list.
pub(crate) fn visit_process_names(visit: &mut dyn FnMut(&str)) -> Result<(), ProcessListError> {
    // SAFETY: takes no pointers; returns a new handle or INVALID_HANDLE_VALUE.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw == INVALID_HANDLE_VALUE {
        return Err(ProcessListError::Os(last_error("CreateToolhelp32Snapshot")));
    }
    let snapshot = Handle::new(raw)
        .ok_or_else(|| ProcessListError::Os(last_error("CreateToolhelp32Snapshot")))?;

    // SAFETY: PROCESSENTRY32W is plain data; all zero bytes are a valid value.
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;

    // SAFETY: a valid snapshot handle owned by `snapshot`, and an entry whose
    // `dwSize` is set, as the API requires.
    let mut more = unsafe { Process32FirstW(snapshot.0, &mut entry) };
    loop {
        if more == 0 {
            // Read the error at once, before any other call can replace it.
            let error = io::Error::last_os_error();
            return if error.raw_os_error() == Some(ERROR_NO_MORE_FILES as i32) {
                Ok(())
            } else {
                Err(ProcessListError::Os(io::Error::new(
                    error.kind(),
                    format!("Process32NextW: {error}"),
                )))
            };
        }
        visit(&file_name(&entry.szExeFile));
        // SAFETY: as above; the same snapshot and entry.
        more = unsafe { Process32NextW(snapshot.0, &mut entry) };
    }
}

/// The text before the first NUL of `szExeFile`, or all of it if there is
/// none.
fn file_name(units: &[u16]) -> String {
    let end = units
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(units.len());
    String::from_utf16_lossy(&units[..end])
}

#[cfg(test)]
mod tests {
    use super::file_name;

    #[test]
    fn the_name_ends_at_the_first_nul() {
        let mut units = [0u16; 260];
        for (slot, unit) in units.iter_mut().zip("cs2.exe".encode_utf16()) {
            *slot = unit;
        }
        units[10] = u16::from(b'x');
        assert_eq!(file_name(&units), "cs2.exe");
    }

    #[test]
    fn a_name_that_fills_the_buffer_is_read_whole() {
        let units = [u16::from(b'a'); 260];
        assert_eq!(file_name(&units).len(), 260);
        assert_eq!(file_name(&[0u16; 260]), "");
    }
}
