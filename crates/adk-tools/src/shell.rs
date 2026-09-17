//! Host-owned shell tools. Keep the bundle alive and call `close` before runtime shutdown.
use adk_core::{
    AccessMode, BoxFuture, Content, Error, Tool, ToolCall, ToolContext, ToolDefinition, ToolOutput,
};
use adk_sandbox::{Backend, Executor, Network, OutputMode, Request};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

#[path = "shell_buffer.rs"]
mod buffer;
#[path = "shell_policy.rs"]
mod policy;
#[path = "shell_session.rs"]
mod session;
pub use policy::command_blocked;
use session::{Job, Manager};

#[derive(Debug, Clone)]
pub struct Limits {
    pub default_timeout_ms: u64,
    pub max_timeout_ms: u64,
    pub output_bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self::from_environment(&BTreeMap::new())
    }
}
impl Limits {
    /// Read only host-supplied settings, never tool arguments or child environment.
    pub fn from_environment(environment: &BTreeMap<String, String>) -> Self {
        fn positive(env: &BTreeMap<String, String>, key: &str, fallback: u64) -> u64 {
            env.get(key)
                .and_then(|s| s.trim().parse::<i64>().ok())
                .filter(|n| *n > 0)
                .map(|n| n as u64)
                .unwrap_or(fallback)
        }
        let default_timeout_ms =
            positive(environment, "GRATEFUL_BASH_DEFAULT_TIMEOUT_MS", 120_000).max(1000);
        Self {
            default_timeout_ms,
            max_timeout_ms: positive(environment, "GRATEFUL_BASH_MAX_TIMEOUT_MS", 600_000)
                .max(1000)
                .max(default_timeout_ms),
            output_bytes: positive(environment, "GRATEFUL_BASH_MAX_OUTPUT_BYTES", 100 * 1024)
                .clamp(4096, 10 * 1024 * 1024) as usize,
        }
    }
    fn timeout(&self, requested: i64, asynchronous: bool) -> Duration {
        Duration::from_millis(if requested > 0 {
            (requested as u64).min(self.max_timeout_ms)
        } else if asynchronous {
            self.max_timeout_ms
        } else {
            self.default_timeout_ms
        })
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub sandbox: adk_sandbox::Config,
    pub access: AccessMode,
    pub git_remote_writes: bool,
    pub environment: BTreeMap<String, String>,
}

/// Dynamic definitions are frozen from trusted settings at construction time.
pub fn definition(name: &str, access: AccessMode, limits: &Limits) -> Option<ToolDefinition> {
    let capability = crate::capabilities().iter().find(|c| c.name == name)?;
    if let Some(definition) = &capability.definition {
        return Some(definition.clone());
    }
    let description = match name {
        "Bash" => {
            "Executes a bash command and returns its output (stdout and stderr combined). Use for running shell commands, build tools, git operations, and other CLI tasks. Each call runs in a fresh process: cd and environment variables do not persist between calls, so chain dependent steps with && in one command. Disable pagers (git --no-pager, | cat) for interactive-output commands. For long-running commands use BashStart/BashPoll; for interactive programs use the Terminal tool if available. In restricted sessions (read-only/plan mode, read-only sub-agents, or hosts without an enforcing OS sandbox) commands are statically authorized before running: command substitution $(...) or backticks, heredocs/here-strings, VAR=value prefixes, eval/source/alias, function definitions, `git remote`, dynamically-built `git push` refs, and the gh CLI are rejected — write plain literal commands, split pipelines into separate calls, and use the built-in git/GitHub tools for anything git must not do here."
        }
        "BashStart" => {
            "Starts a long-running bash command in the background and returns a job id. Use with BashPoll for builds, training, simulations, OCR, and other commands that may run for a long time."
        }
        _ => return None,
    };
    Some(ToolDefinition {
        name: name.into(), description: description.into(), read_only: name == "Bash" && access == AccessMode::ReadOnly, requires_approval: false,
        input_schema: serde_json::from_value(json!({"type":"object", "properties": {
            "command":{"type":"string","description":"The bash command to execute"},
            "timeout":{"type":"number","description":format!("Timeout in milliseconds (max {}, default {})",limits.max_timeout_ms,if name == "BashStart" { limits.max_timeout_ms } else { limits.default_timeout_ms })},
            "description":{"type":"string","description":"Description of what the command does"}
        },"required":["command"]})).expect("shell schema"),
    })
}

pub struct ShellBundle {
    state: Arc<State>,
}
struct State {
    executor: Executor,
    readonly_executor: Executor,
    access: AccessMode,
    git_remote_writes: bool,
    enforced: bool,
    limits: Limits,
    manager: Manager,
}
impl ShellBundle {
    pub fn new(config: Config) -> Result<Self, adk_sandbox::Error> {
        let enforced =
            config.sandbox.backend != Backend::Local && config.access != AccessMode::FullAccess;
        let limits = Limits::from_environment(&config.environment);
        let mut readonly_config = config.sandbox.clone();
        readonly_config.backend = Backend::Auto;
        Ok(Self {
            state: Arc::new(State {
                readonly_executor: Executor::new(readonly_config)?,
                executor: Executor::new(config.sandbox)?,
                access: config.access,
                git_remote_writes: config.git_remote_writes,
                enforced,
                limits,
                manager: Manager::new(),
            }),
        })
    }
    pub fn tools(&self) -> Vec<Arc<dyn Tool>> {
        ["Bash", "BashStart", "BashPoll", "BashKill", "Terminal"]
            .into_iter()
            .filter(|name| match *name {
                "BashStart" | "BashPoll" | "BashKill" => self.state.access != AccessMode::ReadOnly,
                "Terminal" => {
                    self.state.access == AccessMode::FullAccess && self.state.git_remote_writes
                }
                _ => true,
            })
            .map(|name| {
                Arc::new(ShellTool {
                    definition: definition(name, self.state.access, &self.state.limits)
                        .expect("shell definition"),
                    state: self.state.clone(),
                    read_only_adapter: false,
                }) as Arc<dyn Tool>
            })
            .collect()
    }
    /// Cancels and reaps every owned job, including interactive sessions.
    pub async fn close(&self) {
        self.state.manager.close().await;
    }
}
impl Drop for ShellBundle {
    fn drop(&mut self) {
        self.state.manager.cancel();
    }
}
struct ShellTool {
    read_only_adapter: bool,
    definition: ToolDefinition,
    state: Arc<State>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct Input {
    command: String,
    timeout: i64,
    description: String,
    id: String,
    wait_ms: i64,
    incremental: Option<bool>,
    op: String,
    session_id: String,
    keystrokes: String,
}
fn output(text: String, is_error: bool) -> ToolOutput {
    ToolOutput {
        content: vec![Content::Text { text }],
        is_error,
        should_pause: false,
    }
}
fn json_output(value: Value) -> String {
    serde_json::to_string_pretty(&value)
        .expect("shell JSON")
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}
impl Tool for ShellTool {
    fn for_access(&self, access: AccessMode) -> Option<Arc<dyn Tool>> {
        if access != AccessMode::ReadOnly || self.definition.name != "Bash" {
            return None;
        }
        Some(Arc::new(Self {
            definition: definition("Bash", AccessMode::ReadOnly, &self.state.limits)
                .expect("Bash definition"),
            state: self.state.clone(),
            read_only_adapter: true,
        }))
    }
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            context.operation.check_active()?;
            if context.policy.decision(&self.definition) == adk_core::ToolDecision::Deny {
                return Err(Error::new(
                    adk_core::ErrorCategory::PermissionDenied,
                    "shell tool denied by host policy",
                ));
            }
            let result = match serde_json::from_value::<Input>(call.arguments) {
                Ok(input) => self.run(context, input).await,
                Err(error) => Err(format!("Invalid input: {error}")),
            };
            context.operation.check_active()?;
            let (text, is_error) = match result {
                Ok(text) => (text, false),
                Err(text) => (text, true),
            };
            Ok(match adk_security::check_secrets(&text) {
                Ok(()) => output(text, is_error),
                Err(error) => output(error.to_string(), true),
            })
        })
    }
}
impl ShellTool {
    fn access(&self, context: &ToolContext) -> AccessMode {
        if self.read_only_adapter {
            return AccessMode::ReadOnly;
        }
        match (self.state.access, context.policy.access) {
            (AccessMode::ReadOnly, _) | (_, AccessMode::ReadOnly) => AccessMode::ReadOnly,
            (AccessMode::WorkspaceWrite, _) | (_, AccessMode::WorkspaceWrite) => {
                AccessMode::WorkspaceWrite
            }
            _ => AccessMode::FullAccess,
        }
    }
    async fn start(
        &self,
        context: &ToolContext,
        input: &Input,
        terminal: bool,
        asynchronous: bool,
    ) -> Result<Arc<Job>, String> {
        let access = self.access(context);
        if !terminal {
            if input.command.is_empty() || (asynchronous && input.command.trim().is_empty()) {
                return Err("command is required".into());
            }
            if let Some(reason) = command_blocked(
                access,
                self.state.git_remote_writes,
                &input.command,
                self.read_only_adapter || self.state.enforced,
            ) {
                return Err(reason);
            }
        }
        let mut request = Request::new("/bin/bash");
        request.args = vec!["--noprofile".into(), "--norc".into()];
        if terminal {
            request.args.push("-i".into());
            request.output = OutputMode::Pty {
                rows: 40,
                cols: 160,
            };
            request.env.insert("TERM".into(), "xterm-256color".into());
        } else {
            request
                .args
                .extend(["-c".into(), format!("exec 2>&1; {}", input.command)]);
            request.timeout = Some(self.state.limits.timeout(input.timeout, asynchronous));
        }
        request.hide_git_credentials = !self.state.git_remote_writes;
        request.cwd = context.work_dir.clone();
        request.access = access;
        request.network = if access == AccessMode::ReadOnly {
            Network::Deny
        } else {
            Network::Allow
        };
        self.state
            .manager
            .start(
                if self.read_only_adapter {
                    &self.state.readonly_executor
                } else {
                    &self.state.executor
                },
                context,
                request,
                input.description.trim(),
                self.state.limits.output_bytes,
                terminal,
                asynchronous,
            )
            .await
    }
    async fn run(&self, context: &ToolContext, input: Input) -> Result<String, String> {
        match self.definition.name.as_str() {
            "Bash" => {
                let job = self.start(context, &input, false, false).await?;
                // A dropped synchronous call must not leave a background command running.
                let guard = session::CancelOnDrop(Some(job.clone()));
                job.done().await;
                let result = job.bash_result();
                drop(guard);
                result
            }
            "BashStart" => {
                if self.access(context) == AccessMode::ReadOnly {
                    return Err("BashStart is unavailable in read-only mode".into());
                }
                let job = self.start(context, &input, false, true).await?;
                Ok(format!("started background bash job {}", job.id))
            }
            "BashPoll" | "BashKill" => {
                let job = self
                    .state
                    .manager
                    .get(input.id.trim(), false)
                    .ok_or_else(|| format!("unknown job id: {}", input.id))?;
                if self.definition.name == "BashKill" {
                    job.kill().await;
                } else {
                    job.wait_for(context, input.wait_ms.clamp(0, 120_000) as u64)
                        .await;
                }
                Ok(json_output(job.bash_snapshot(
                    self.definition.name == "BashPoll" && input.incremental.unwrap_or(true),
                )?))
            }
            "Terminal" => self.terminal(context, input).await,
            _ => unreachable!(),
        }
    }
    async fn terminal(&self, context: &ToolContext, input: Input) -> Result<String, String> {
        if !self.state.git_remote_writes {
            return Err("Terminal is unavailable when GitRemoteWrites is disabled".into());
        }
        if self.access(context) != AccessMode::FullAccess {
            return Err("Terminal requires danger-full-access mode".into());
        }
        let op = input.op.trim();
        if op == "list" {
            return Ok(json_output(Value::Array(
                self.state
                    .manager
                    .list(true)
                    .iter()
                    .map(|s| s.terminal_snapshot(false))
                    .collect::<Result<Vec<_>, _>>()?,
            )));
        }
        if !matches!(op, "start" | "send" | "read" | "kill") {
            return Err(format!(
                "invalid op {:?}: use start, send, read, kill, or list",
                input.op
            ));
        }
        let job = if op == "start" {
            self.start(context, &input, true, true).await?
        } else {
            self.state
                .manager
                .get(input.session_id.trim(), true)
                .ok_or_else(|| {
                    format!(
                        "unknown session_id: {}{}",
                        input.session_id,
                        if op == "kill" {
                            ""
                        } else {
                            " (use op=start first, or op=list to see sessions)"
                        }
                    )
                })?
        };
        if op == "send" {
            if input.keystrokes.is_empty() {
                return Err("keystrokes is required for op=send (end shell commands with \\n to execute them)".into());
            }
            job.send(context, translate_keystrokes(&input.keystrokes))
                .await
                .map_err(|e| format!("failed to send keystrokes: {e}"))?;
        }
        if op == "kill" {
            job.kill().await;
        } else if op != "read" || input.wait_ms > 0 {
            job.wait_for(
                context,
                if input.wait_ms <= 0 {
                    1000
                } else {
                    input.wait_ms.min(60_000) as u64
                },
            )
            .await;
        }
        if op == "start"
            && let Some(error) = job.startup_error()
        {
            return Err(format!("failed to start terminal session: {error}"));
        }
        Ok(json_output(job.terminal_snapshot(true)?))
    }
}
pub fn translate_keystrokes(s: &str) -> &[u8] {
    match s {
        "C-c" => b"\x03",
        "C-d" => b"\x04",
        "C-z" => b"\x1a",
        "C-l" => b"\x0c",
        "Enter" => b"\r",
        "Escape" => b"\x1b",
        "Tab" => b"\t",
        "Up" => b"\x1b[A",
        "Down" => b"\x1b[B",
        "Right" => b"\x1b[C",
        "Left" => b"\x1b[D",
        _ => s.as_bytes(),
    }
}
