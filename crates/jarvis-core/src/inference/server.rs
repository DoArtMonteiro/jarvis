//! Starting and stopping the local model server (ADR 0021).
//!
//! The server is started like the Session Agent, as a trusted helper
//! ([`jarvis_sandbox::spawn_helper`]): from an absolute path, never through a
//! shell, with a cleared environment, no child processes, a memory limit and
//! a kill-on-close job, so it dies with the Core. It listens on `127.0.0.1`
//! only, on a port chosen at start, and refuses requests without the key the
//! Core generated for this start. Its web interface, agent tools, slot
//! endpoint and model downloads are switched off on the command line; with
//! the environment cleared, no `LLAMA_ARG_*` variable can switch them back
//! on.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};

use super::http::{self, Request};

/// Commit limit of the server process. With `--load-mode none` the weights
/// are read once and copied to the GPU; the embedding table and some
/// buffers stay in memory.
const SERVER_MEMORY_BYTES: u64 = 20 * 1024 * 1024 * 1024;
/// Lines of the server's log kept for a failure report.
const LOG_LINES: usize = 40;

/// What to start.
#[derive(Debug, Clone)]
pub struct Launch {
    pub program: PathBuf,
    pub model: PathBuf,
    pub context_size: u32,
    pub gpu_layers: u32,
    pub threads: u32,
    pub timeout: Duration,
}

/// A running server. Dropping it kills the server.
pub struct Server {
    pub port: u16,
    pub key: String,
    process: jarvis_sandbox::Process,
    contained: jarvis_sandbox::Contained,
    log: Arc<Mutex<VecDeque<String>>>,
}

/// Without the key, which must not reach a log.
impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server")
            .field("port", &self.port)
            .field("pid", &self.process.id())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LaunchError {
    #[error("the model server could not be started: {0}")]
    Spawn(std::io::Error),
    #[error("the model server exited while loading: {0}")]
    Exited(String),
    #[error("the model server did not become ready within {0} s")]
    Timeout(u64),
}

impl Server {
    /// Start the server without waiting for it, so that its process is
    /// known (and refused by the RPC server) from the start.
    pub fn spawn(launch: &Launch) -> Result<Self, LaunchError> {
        let port = free_port().map_err(LaunchError::Spawn)?;
        let key = random_key();
        let command = jarvis_sandbox::WorkerCommand {
            program: launch.program.clone(),
            args: arguments(launch, port, &key),
            env: environment(),
            cwd: launch.program.parent().map(Path::to_path_buf),
        };
        let spawned = jarvis_sandbox::spawn_helper(&command, SERVER_MEMORY_BYTES)
            .map_err(LaunchError::Spawn)?;
        drop(spawned.stdin);
        let log = Arc::new(Mutex::new(VecDeque::with_capacity(LOG_LINES)));
        tokio::spawn(drain(spawned.stdout, Arc::clone(&log)));
        tokio::spawn(drain(spawned.stderr, Arc::clone(&log)));
        Ok(Self {
            port,
            key,
            process: spawned.process,
            contained: spawned.contained,
            log,
        })
    }

    /// Wait until the server answers, it exits, or `timeout` passes. The
    /// exit is checked again after the first answer, so that another program
    /// that took the port while this server failed to bind is not trusted.
    pub async fn wait_ready(&mut self, timeout: Duration) -> Result<(), LaunchError> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.has_exited() {
                return Err(LaunchError::Exited(self.recent_log()));
            }
            if self.healthy().await {
                tokio::time::sleep(Duration::from_millis(100)).await;
                if self.has_exited() {
                    return Err(LaunchError::Exited(self.recent_log()));
                }
                return Ok(());
            }
            if Instant::now() >= deadline {
                self.stop();
                return Err(LaunchError::Timeout(timeout.as_secs()));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Whether the server answers `/health` with 200 (it answers 503 while
    /// loading).
    pub async fn healthy(&self) -> bool {
        let request = Request {
            method: "GET",
            path: "/health",
            key: &self.key,
            body: None,
        };
        let check = async {
            http::send(self.port, &request)
                .await
                .is_ok_and(|response| response.status == 200)
        };
        tokio::time::timeout(Duration::from_secs(2), check)
            .await
            .unwrap_or(false)
    }

    pub fn pid(&self) -> u32 {
        self.process.id()
    }

    pub fn has_exited(&mut self) -> bool {
        !matches!(self.process.try_wait(), Ok(None))
    }

    /// The last lines the server logged, on one line, for an error message.
    pub fn recent_log(&self) -> String {
        let log = self.log.lock().unwrap_or_else(|p| p.into_inner());
        let skip = log.len().saturating_sub(4);
        let tail: Vec<&str> = log.iter().skip(skip).map(String::as_str).collect();
        super::one_line(&tail.join(" | "), 400)
    }

    pub fn stop(&mut self) {
        let _ = self.contained.kill_all();
        let _ = self.process.start_kill();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
    }
}

fn arguments(launch: &Launch, port: u16, key: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["-m".into(), launch.model.clone().into_os_string()];
    for (flag, value) in [
        ("--host", "127.0.0.1".to_owned()),
        ("--port", port.to_string()),
        ("--api-key", key.to_owned()),
        ("-ngl", launch.gpu_layers.to_string()),
        ("-c", launch.context_size.to_string()),
        ("-t", launch.threads.to_string()),
        ("-np", "1".to_owned()),
        ("-fa", "on".to_owned()),
        ("-lm", "none".to_owned()),
        ("--reasoning", "off".to_owned()),
        ("-lv", "2".to_owned()),
    ] {
        args.push(flag.into());
        args.push(value.into());
    }
    for flag in [
        "--jinja",
        "--no-webui",
        "--no-agent",
        "--no-slots",
        "--offline",
    ] {
        args.push(flag.into());
    }
    args
}

/// Only what Windows and the GPU driver need: no `LLAMA_*` variable
/// reaches the server.
fn environment() -> Vec<(OsString, OsString)> {
    [
        "SYSTEMROOT",
        "WINDIR",
        "SYSTEMDRIVE",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
    ]
    .into_iter()
    .filter_map(|name| std::env::var_os(name).map(|value| (name.into(), value)))
    .collect()
}

fn free_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    Ok(listener.local_addr()?.port())
}

/// 244 random bits (two version 4 UUIDs) as hexadecimal.
fn random_key() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// Read the server's output in pieces of at most `MAX_LOG_LINE` bytes, so a
/// line without an end cannot grow without limit.
async fn drain(stream: impl AsyncRead + Unpin, log: Arc<Mutex<VecDeque<String>>>) {
    const MAX_LOG_LINE: u64 = 4096;
    let mut reader = BufReader::new(stream);
    let mut piece = Vec::new();
    loop {
        piece.clear();
        match (&mut reader)
            .take(MAX_LOG_LINE)
            .read_until(b'\n', &mut piece)
            .await
        {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let line = super::one_line(&String::from_utf8_lossy(&piece), 300);
        tracing::debug!(target: "jarvis_core::inference::server", "{line}");
        let mut log = log.lock().unwrap_or_else(|p| p.into_inner());
        if log.len() == LOG_LINES {
            log.pop_front();
        }
        log.push_back(line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_is_local_keyed_and_has_no_extras() {
        let launch = Launch {
            program: PathBuf::from("llama-server"),
            model: PathBuf::from("model.gguf"),
            context_size: 16384,
            gpu_layers: 99,
            threads: 8,
            timeout: Duration::from_secs(1),
        };
        let args: Vec<String> = arguments(&launch, 4242, "k")
            .into_iter()
            .map(|a| a.into_string().unwrap())
            .collect();
        let joined = args.join(" ");
        assert!(joined.contains("--host 127.0.0.1 --port 4242 --api-key k"));
        for flag in ["--no-webui", "--no-agent", "--no-slots", "--offline"] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        for absent in ["--tools", "--mcp", "--metrics", "--props", "0.0.0.0"] {
            assert!(!joined.contains(absent), "{absent}");
        }
        assert!(
            environment()
                .iter()
                .all(|(name, _)| !name.to_string_lossy().starts_with("LLAMA"))
        );
    }

    #[test]
    fn keys_are_long_and_fresh() {
        let a = random_key();
        assert_eq!(a.len(), 64);
        assert_ne!(a, random_key());
    }
}
