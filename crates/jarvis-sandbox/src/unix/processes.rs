//! Process names from `/proc` on Linux.
//!
//! Each numeric directory under `/proc` is a process, and its `comm` file is
//! the name the kernel keeps for it: the file name of the executable, cut to
//! 15 bytes. Nothing here signals, traces or opens a target process; it only
//! reads one small world-readable file per entry.

use crate::processes::ProcessListError;
#[cfg(target_os = "linux")]
use crate::processes::{comm_name, is_pid};

/// The kernel keeps at most 15 bytes of a process name (`TASK_COMM_LEN` - 1).
#[cfg(target_os = "linux")]
pub(crate) const PROCESS_NAME_LIMIT: Option<usize> = Some(15);
#[cfg(not(target_os = "linux"))]
pub(crate) const PROCESS_NAME_LIMIT: Option<usize> = None;

/// Call `visit` with the `comm` of every process. A process that exits while
/// the list is read is skipped. Failing to read `/proc`, or any entry for a
/// reason other than the process having gone, is an error: a shorter list
/// must never pass for "nothing is running".
#[cfg(target_os = "linux")]
pub(crate) fn visit_process_names(visit: &mut dyn FnMut(&str)) -> Result<(), ProcessListError> {
    use std::io;

    let entries = std::fs::read_dir("/proc").map_err(|error| {
        ProcessListError::Os(io::Error::new(error.kind(), format!("read /proc: {error}")))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            ProcessListError::Os(io::Error::new(error.kind(), format!("read /proc: {error}")))
        })?;
        let file_name = entry.file_name();
        let Some(pid) = file_name.to_str().filter(|name| is_pid(name)) else {
            continue;
        };
        match std::fs::read(format!("/proc/{pid}/comm")) {
            Ok(bytes) => visit(&comm_name(&bytes)),
            Err(error) if process_vanished(&error) => {}
            Err(error) => {
                return Err(ProcessListError::Os(io::Error::new(
                    error.kind(),
                    format!("read /proc/{pid}/comm: {error}"),
                )));
            }
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn visit_process_names(_visit: &mut dyn FnMut(&str)) -> Result<(), ProcessListError> {
    Err(ProcessListError::Unsupported)
}

/// Whether reading a process's file failed because the process exited
/// between listing `/proc` and reading it (`ENOENT`, or `ESRCH` when the
/// process was found but is already gone).
#[cfg(target_os = "linux")]
fn process_vanished(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::NotFound || error.raw_os_error() == Some(libc::ESRCH)
}
