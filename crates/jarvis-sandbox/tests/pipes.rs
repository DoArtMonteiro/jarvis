//! The named pipe checks behind the RPC interface on Windows: a client
//! refuses a pipe that is not its own user's at medium integrity, and the
//! Core stops impersonating a client as soon as it has read its token.
#![cfg(windows)]

use std::ffi::c_void;
use std::os::windows::io::AsRawHandle;
use std::ptr::null_mut;

use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{ERROR_NO_TOKEN, GetLastError, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY};
use windows_sys::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};

fn name(test: &str) -> String {
    format!(r"\\.\pipe\jarvis-pipes-{test}-{}", std::process::id())
}

/// The current user's SID in `S-1-...` form.
fn user_sid() -> String {
    let output = std::process::Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .unwrap();
    let line = String::from_utf8(output.stdout).unwrap();
    line.trim()
        .rsplit(',')
        .next()
        .unwrap()
        .trim_matches('"')
        .to_owned()
}

/// A pipe server created from an SDDL string, as another program could.
fn pipe_with(name: &str, sddl: &str) -> NamedPipeServer {
    let wide: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: NUL-terminated SDDL; the descriptor is freed below.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        )
    };
    assert_ne!(ok, 0, "invalid SDDL {sddl}");
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    // SAFETY: valid attributes whose descriptor outlives the call.
    let server = unsafe {
        ServerOptions::new()
            .create_with_security_attributes_raw(name, (&raw mut attributes).cast::<c_void>())
    }
    .unwrap();
    // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
    unsafe { LocalFree(descriptor) };
    server
}

#[tokio::test]
async fn a_client_accepts_only_a_pipe_of_its_own_user_at_medium_integrity() {
    let sid = user_sid();
    // Same owner and access; only the label differs. No label counts as
    // medium, as Windows treats it.
    for (index, (sacl, accepted)) in [
        ("S:(ML;;NW;;;ME)", true),
        ("", true),
        ("S:(ML;;NW;;;LW)", false),
    ]
    .into_iter()
    .enumerate()
    {
        let name = name(&format!("label-{index}"));
        let _server = pipe_with(&name, &format!("O:{sid}D:P(A;;GA;;;WD){sacl}"));
        let client = ClientOptions::new().open(&name).unwrap();
        assert_eq!(
            jarvis_sandbox::pipe_belongs_to_current_user(client.as_raw_handle()).unwrap(),
            accepted,
            "SACL {sacl:?}"
        );
    }
    // The Core's own pipe passes.
    let name = name("core");
    let _server = jarvis_sandbox::create_owner_only_pipe(&ServerOptions::new(), &name).unwrap();
    let client = ClientOptions::new().open(&name).unwrap();
    assert!(jarvis_sandbox::pipe_belongs_to_current_user(client.as_raw_handle()).unwrap());
}

/// Runs on one thread (the default test runtime), so the thread that
/// impersonated the client is the one checked afterwards.
#[tokio::test]
async fn reading_a_client_token_leaves_the_thread_as_itself() {
    let name = name("revert");
    let server = jarvis_sandbox::create_owner_only_pipe(&ServerOptions::new(), &name).unwrap();
    let _client = ClientOptions::new().open(&name).unwrap();
    server.connect().await.unwrap();
    assert_eq!(
        jarvis_sandbox::inspect_pipe_client(server.as_raw_handle()).unwrap(),
        jarvis_sandbox::PipeClient::SameUser
    );
    let mut token = null_mut();
    // SAFETY: queries the calling thread's token, if it has one.
    let ok = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) };
    // SAFETY: reads the calling thread's last-error value.
    assert_eq!((ok, unsafe { GetLastError() }), (0, ERROR_NO_TOKEN));
}
