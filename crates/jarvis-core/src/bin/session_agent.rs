//! `jarvis-session-agent`: the Session Agent process (ADR 0019).
//!
//! Started only by the Core, which holds the other ends of its standard
//! input and output. It takes no arguments and reads no environment. It
//! performs only the operations in `jarvis_protocol::AgentOperation`, today a
//! synthetic health check, and touches no desktop, clipboard or input.

use std::io::{self, BufReader};
use std::process::ExitCode;

fn main() -> ExitCode {
    if std::env::args_os().len() > 1 {
        return ExitCode::from(64);
    }
    let mut input = BufReader::new(io::stdin().lock());
    let mut output = io::stdout().lock();
    match jarvis_core::session_agent::serve::serve(
        &mut input,
        &mut output,
        env!("CARGO_PKG_VERSION"),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::from(2),
    }
}
