//! Opt-in, trusted-host process execution. This is not tool authorization.
//! Never construct policy/configuration directly from model arguments.
//! See the crate README for containment and lifecycle limitations.

use adk_core::{AccessMode, Context};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};

#[cfg(unix)]
mod backend;
mod policy;
#[cfg(unix)]
mod process;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Backend {
    #[default]
    Auto,
    Bubblewrap,
    Seatbelt,
    /// No containment. Requires FullAccess and Network::Allow explicitly.
    Local,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Network {
    #[default]
    Deny,
    Allow,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OutputMode {
    #[default]
    Pipes,
    /// Capture a private terminal; stdout and stderr are merged into stdout.
    /// No interactive input API is provided.
    Pty { rows: u16, cols: u16 },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub workspace: PathBuf,
    pub backend: Backend,
    /// Combined retained stdout/stderr byte budget; excess bytes are drained.
    pub output_limit: usize,
    pub term_grace: Duration,
}

impl Config {
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            backend: Backend::Auto,
            output_limit: 1024 * 1024,
            term_grace: Duration::from_millis(250),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    /// Absolute executable path; no parent PATH lookup takes place.
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Absolute or workspace-relative existing directory.
    pub cwd: PathBuf,
    pub access: AccessMode,
    pub network: Network,
    /// Explicit locale/terminal overrides only; no ambient environment inheritance.
    pub env: BTreeMap<String, String>,
    pub timeout: Option<Duration>,
    pub output: OutputMode,
}

impl Request {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: PathBuf::from("."),
            access: AccessMode::ReadOnly,
            network: Network::Deny,
            env: BTreeMap::new(),
            timeout: None,
            output: OutputMode::Pipes,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    Exited,
    Cancelled,
    TimedOut,
}

#[derive(Debug)]
pub struct RunResult {
    pub status: std::process::ExitStatus,
    pub completion: Completion,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub truncated: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid sandbox configuration: {0}")]
    Invalid(String),
    #[error("sandbox unavailable: {0}")]
    Unavailable(String),
    #[error("operation cancelled before spawn")]
    Cancelled,
    #[error("deadline expired before spawn")]
    TimedOut,
    #[error("subprocess I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("subprocess supervisor failed: {0}")]
    Supervisor(String),
}

#[derive(Debug, Clone)]
pub struct Executor {
    config: Config,
}

/// Owned asynchronous cleanup boundary. Dropping requests cancellation;
/// `cancel_and_wait` additionally waits for process-group cleanup and reaping.
pub struct RunningProcess {
    #[cfg(unix)]
    pub(crate) task: tokio::task::JoinHandle<Result<RunResult, Error>>,
    pub(crate) cancel: tokio_util::sync::CancellationToken,
}

impl RunningProcess {
    pub async fn wait(mut self) -> Result<RunResult, Error> {
        #[cfg(unix)]
        {
            (&mut self.task)
                .await
                .map_err(|e| Error::Supervisor(e.to_string()))?
        }
        #[cfg(not(unix))]
        {
            let _ = &mut self;
            Err(Error::Unavailable(
                "only Unix process lifecycle is implemented".into(),
            ))
        }
    }

    pub async fn cancel_and_wait(self) -> Result<RunResult, Error> {
        self.cancel.cancel();
        self.wait().await
    }
}

impl Drop for RunningProcess {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

impl Executor {
    pub fn new(mut config: Config) -> Result<Self, Error> {
        config.workspace = policy::workspace(&config.workspace)?;
        if config.output_limit > 64 * 1024 * 1024 || config.term_grace > Duration::from_secs(30) {
            return Err(Error::Invalid(
                "output limit exceeds 64 MiB or grace exceeds 30 seconds".into(),
            ));
        }
        Ok(Self { config })
    }

    /// The supervisor owns explicit TERM/grace/KILL/wait cleanup independently of
    /// this future. Dropping the future requests cleanup; keep the Tokio runtime
    /// alive to complete it. Normal returns have reaped the direct child.
    pub async fn run(&self, context: &Context, request: Request) -> Result<RunResult, Error> {
        self.start(context, request)?.wait().await
    }

    /// Start independently supervised work. Keep this handle to explicitly
    /// cancel and await cleanup during host shutdown. Requires a Tokio runtime.
    pub fn start(&self, context: &Context, request: Request) -> Result<RunningProcess, Error> {
        policy::active(context)?;
        let request = policy::validate(&self.config, request)?;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            Ok(process::start(
                self.config.clone(),
                context.clone(),
                request,
            ))
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = request;
            Err(Error::Unavailable(
                "only Unix process lifecycle is implemented".into(),
            ))
        }
    }
}
