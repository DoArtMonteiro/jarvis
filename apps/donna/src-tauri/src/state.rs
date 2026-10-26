//! Shared application state and the Core connection helpers.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use jarvis_core::config::Config;
use jarvis_core::rpc::Endpoint;
use jarvis_core::rpc::client::Client;
use jarvis_protocol::{RpcRequest, RpcResponse};
use tokio::process::Child;
use tokio::sync::Notify;

use crate::core_proc;
use crate::error::DonnaError;
use crate::log;
use crate::paths;

/// What the app is doing about starting the Core.
#[derive(Debug, Clone)]
pub enum Boot {
    Idle,
    Starting,
    Failed(String),
}

#[derive(Debug)]
pub struct AppState {
    pub config_path: PathBuf,
    endpoint: Mutex<Option<Endpoint>>,
    pub boot: Mutex<Boot>,
    /// The Core process, when this app started it.
    pub child: tokio::sync::Mutex<Option<Child>>,
    /// Serialises start attempts.
    pub launch: tokio::sync::Mutex<()>,
    /// Turns that have a polling task.
    pub following: Mutex<HashSet<String>>,
    pub status_wake: Notify,
    pub approvals_wake: Notify,
    pub closing: AtomicBool,
}

/// Lock a std mutex; a poisoned lock still holds usable data.
pub fn locked<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl AppState {
    pub fn new() -> Self {
        Self {
            config_path: paths::config_path(),
            endpoint: Mutex::new(None),
            boot: Mutex::new(Boot::Idle),
            child: tokio::sync::Mutex::new(None),
            launch: tokio::sync::Mutex::new(()),
            following: Mutex::new(HashSet::new()),
            status_wake: Notify::new(),
            approvals_wake: Notify::new(),
            closing: AtomicBool::new(false),
        }
    }

    pub fn is_closing(&self) -> bool {
        self.closing.load(Ordering::Relaxed)
    }

    /// Read the configuration again and remember the Core's endpoint.
    pub fn reload_endpoint(&self) -> Result<Endpoint, DonnaError> {
        let config = Config::load(&self.config_path).map_err(|error| DonnaError::Config {
            message: error.to_string(),
            path: self.config_path.display().to_string(),
        })?;
        let endpoint = Endpoint::from_config(&config.rpc);
        *locked(&self.endpoint) = Some(endpoint.clone());
        Ok(endpoint)
    }

    pub fn endpoint(&self) -> Result<Endpoint, DonnaError> {
        if let Some(endpoint) = locked(&self.endpoint).clone() {
            return Ok(endpoint);
        }
        self.reload_endpoint()
    }

    pub async fn connect(&self) -> Result<Client, DonnaError> {
        Ok(Client::connect(&self.endpoint()?).await?)
    }

    /// One request on a short-lived connection.
    pub async fn call(&self, request: &RpcRequest) -> Result<RpcResponse, DonnaError> {
        let mut client = self.connect().await?;
        into_result(client.call(request).await?)
    }

    /// Start the Core unless it already answers.
    pub async fn ensure_core(&self) -> Result<(), DonnaError> {
        let _guard = self.launch.lock().await;
        let result = self.ensure_core_locked().await;
        *locked(&self.boot) = match &result {
            Ok(()) => Boot::Idle,
            Err(error) => Boot::Failed(error.to_string()),
        };
        self.status_wake.notify_one();
        result
    }

    async fn ensure_core_locked(&self) -> Result<(), DonnaError> {
        self.reload_endpoint()?;
        match self.call(&RpcRequest::Health {}).await {
            Ok(_) => return Ok(()),
            Err(error) if error.is_unreachable() => {}
            Err(error) => return Err(error),
        }
        *locked(&self.boot) = Boot::Starting;
        self.status_wake.notify_one();
        let exe = paths::core_exe().ok_or_else(|| DonnaError::Launch {
            message: format!(
                "jarvis-core.exe not found in {}: start Donna from its installed folder \
                 (<profile>\\app\\donna.exe) or set DONNA_CORE_EXE",
                paths::exe_dir_display()
            ),
        })?;
        // Keep the child before waiting, so closing during start-up stops it.
        let (child, stdout) = core_proc::spawn(&exe, &self.config_path)?;
        *self.child.lock().await = Some(child);
        if let Err(error) = core_proc::wait_ready(stdout).await {
            if let Some(mut child) = self.child.lock().await.take() {
                let _ = child.kill().await;
            }
            return Err(error);
        }
        self.call(&RpcRequest::Health {}).await.map(|_| ())
    }

    /// On exit: stop the Core only if this app started it.
    pub async fn stop_core(&self) {
        self.closing.store(true, Ordering::Relaxed);
        let Some(child) = self.child.lock().await.take() else {
            return;
        };
        let asked =
            tokio::time::timeout(Duration::from_secs(2), self.call(&RpcRequest::Shutdown {})).await;
        let grace = if matches!(asked, Ok(Ok(_))) {
            Duration::from_secs(5)
        } else {
            // Refused (allow_shutdown is off) or unreachable: kill it.
            log::line(&format!("the Core did not accept Shutdown: {asked:?}"));
            Duration::ZERO
        };
        core_proc::stop(child, grace).await;
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// An error reply becomes an error; anything else passes through.
pub fn into_result(response: RpcResponse) -> Result<RpcResponse, DonnaError> {
    match response {
        RpcResponse::Error(error) => Err(error.into()),
        other => Ok(other),
    }
}
