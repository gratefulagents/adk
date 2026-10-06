//! Host-neutral composition of native providers, tools and owned runtime lifetimes.
use adk_core::{
    AccessMode, BoxFuture, Cancellation, Context, Error, ErrorCategory, Host, ItemProvenance,
    RunError, RunItem, RunPolicy, RunRequest, StreamingModel, Tool, ToolDefinition, ToolPolicy,
};
use adk_providers::{
    auth::{CredentialStore, Refresh},
    factory::{Kind, RouteSpec, default_route},
    routing::Routes,
};
use adk_runtime::{
    AgentConfig, CancellationToken, Handoff, HandoffInputFilter, ModelBinding, RunOutcome,
    RunStream, Runner, RunnerConfig, SubagentSession, subagent::Scheduler,
};
use adk_tools::bundle::{BundleBuilder, ToolBundle};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::Arc,
    task::Poll,
    time::Duration,
};

mod workspace;
pub use workspace::workspace_context;

#[cfg(feature = "mcp")]
mod mcp;
#[cfg(feature = "project-state")]
mod project_state;
#[cfg(feature = "mcp")]
pub use adk_mcp::session::ConnectionSet as McpInput;

fn invalid(message: impl Into<String>) -> Error {
    Error::new(ErrorCategory::InvalidInput, message)
}

/// Runtime selection, distinct from Cargo availability. An explicit default is all off.
#[derive(Clone, Debug, Default)]
pub struct Features {
    pub tools: BTreeSet<String>,
    pub mode_instructions: bool,
    pub mode_model_routing: bool,
    pub compaction: bool,
    pub retry: bool,
    pub approval: bool,
    pub builtin_guardrails: bool,
    pub mcp: McpFeatures,
    pub project_state: ProjectStateFeatures,
    pub parallel_tool_calls: bool,
    pub untrusted_tool_outputs: bool,
    pub force_final_summary_turn: bool,
    pub immediate_input_polling: bool,
    /// Requires an explicitly supplied session with an owned scheduler.
    pub subagents: SubagentFeatures,
    /// Catalog transfers, independent of scheduler-backed subagents.
    pub handoffs: bool,
    /// Tool-less fallback, only when handoffs are enabled and the catalog is empty.
    pub handoff_generic_fallback: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ProjectStateFeatures {
    pub prime_context: bool,
    pub task_tools: bool,
    pub memory_tools: bool,
    pub prime_tool: bool,
}
impl ProjectStateFeatures {
    fn active(&self) -> bool {
        self.prime_context || self.task_tools || self.memory_tools || self.prime_tool
    }
}
#[derive(Clone, Debug, Default)]
pub struct ProjectStateConfig {
    pub state_dir: PathBuf,
    pub project_id: String,
    pub actor: String,
    pub run_id: String,
    pub active_task_id: String,
}

#[derive(Clone, Debug, Default)]
pub struct McpFeatures {
    pub enabled: bool,
    pub allow_all_servers: bool,
    pub allowed_servers: BTreeSet<String>,
    pub allow_all_tools: bool,
    pub allowed_tools: BTreeSet<String>,
    pub resource_tools: bool,
}

impl McpFeatures {
    fn active(&self) -> bool {
        self.enabled
            && (self.allow_all_servers || !self.allowed_servers.is_empty())
            && (self.allow_all_tools || !self.allowed_tools.is_empty() || self.resource_tools)
    }
}

#[derive(Clone, Debug, Default)]
pub struct SubagentFeatures {
    /// Spawn tasks and wait for their completion.
    pub task: bool,
    /// Inspect tasks without enabling spawn or control.
    pub status: bool,
    /// Steer or cancel existing tasks.
    pub control: bool,
}

impl SubagentFeatures {
    fn enabled(&self) -> bool {
        self.task || self.status || self.control
    }
}

#[derive(Clone)]
pub struct Config {
    pub provider: Option<Kind>,
    pub default_provider: Option<String>,
    pub model: String,
    pub fallback_models: Vec<String>,
    pub agent_name: String,
    pub instructions: String,
    pub feature_summary: String,
    pub mode_directive_text: String,
    pub final_check_instructions: String,
    pub reasoning: String,
    pub verbosity: String,
    pub settings: Map<String, Value>,
    pub work_dir: PathBuf,
    pub policy: RunPolicy,
    pub active_mode: Option<String>,
    pub active_role: Option<String>,
    pub mode_snapshot: Option<ModeSpec>,
    pub roles: Vec<RoleSpec>,
    /// None preserves legacy defaults; Some(Features::default()) enables nothing.
    pub features: Option<Features>,
    pub legacy_tools: adk_tools::LegacyFeatures,
    pub enable_compaction: bool,
    pub enable_retry: bool,
    pub enable_approval: bool,
    pub enable_guardrails: bool,
    pub enable_mcp: bool,
    pub enable_project_state: bool,
    pub project_state: ProjectStateConfig,
    pub tool_options: adk_tools::Config,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            provider: None,
            default_provider: None,
            model: String::new(),
            fallback_models: vec![],
            agent_name: "agent".into(),
            instructions: String::new(),
            feature_summary: String::new(),
            mode_directive_text: String::new(),
            final_check_instructions: String::new(),
            reasoning: "medium".into(),
            verbosity: "medium".into(),
            settings: Map::new(),
            work_dir: PathBuf::from("."),
            policy: RunPolicy {
                tools: ToolPolicy {
                    access: AccessMode::WorkspaceWrite,
                    ..Default::default()
                },
                ..Default::default()
            },
            active_mode: None,
            active_role: None,
            mode_snapshot: None,
            roles: vec![],
            features: None,
            legacy_tools: Default::default(),
            enable_compaction: false,
            enable_retry: false,
            enable_approval: false,
            enable_guardrails: false,
            enable_mcp: false,
            enable_project_state: false,
            project_state: Default::default(),
            tool_options: Default::default(),
        }
    }
}
impl Config {
    pub fn resolved_features(&self) -> Features {
        self.features.clone().unwrap_or_else(|| Features {
            mode_instructions: true,
            mode_model_routing: true,
            parallel_tool_calls: true,
            untrusted_tool_outputs: true,
            compaction: self.enable_compaction,
            retry: self.enable_retry,
            approval: self.enable_approval,
            builtin_guardrails: self.enable_guardrails,
            project_state: ProjectStateFeatures {
                prime_context: self.enable_project_state || self.legacy_tools.enable_project_state,
                task_tools: self.enable_project_state || self.legacy_tools.enable_project_state,
                memory_tools: self.enable_project_state || self.legacy_tools.enable_project_state,
                prime_tool: self.enable_project_state || self.legacy_tools.enable_project_state,
            },
            mcp: McpFeatures {
                enabled: self.enable_mcp,
                allow_all_servers: self.enable_mcp,
                allow_all_tools: self.enable_mcp,
                resource_tools: self.enable_mcp,
                ..Default::default()
            },
            ..Default::default()
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct RoleRouting {
    pub model: String,
    pub fallback_models: Option<Vec<String>>,
    pub reasoning_level: String,
    pub text_verbosity: String,
    pub settings: Map<String, Value>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelRouting {
    pub default_model: String,
    pub fallback_models: Option<Vec<String>>,
    pub reasoning_level: String,
    pub text_verbosity: String,
    pub role_overrides: BTreeMap<String, RoleRouting>,
    pub settings: Map<String, Value>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Constraints {
    pub max_turns: Option<NonZeroU32>,
    #[serde(rename = "subAgentMaxTurns")]
    pub subagent_max_turns: Option<NonZeroU32>,
    #[serde(rename = "maxConcurrentSubAgents")]
    pub max_concurrent_subagents: Option<NonZeroU32>,
    pub max_runtime_minutes: Option<NonZeroU32>,
    pub max_retries: Option<u32>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ModeSpec {
    pub name: String,
    pub version: String,
    pub display_name: String,
    pub description: String,
    pub category: String,
    pub autonomous: bool,
    pub tool_access: String,
    pub instructions: String,
    pub model_routing: Option<ModelRouting>,
    pub constraints: Option<Constraints>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RoleSpec {
    pub name: String,
    pub description: String,
    #[serde(alias = "toolAccess")]
    pub tool_access: String,
    #[serde(alias = "model")]
    pub model_override: String,
    /// Programmatic override; Some(empty) explicitly clears inherited fallbacks.
    #[serde(skip)]
    pub fallback_models: Option<Vec<String>>,
    pub instructions: String,
}

/// Immutable build input: hosts can adapt databases or configuration services here.
#[derive(Clone, Default)]
pub struct HostConfig {
    pub modes: Vec<ModeSpec>,
    pub roles: Vec<RoleSpec>,
}
pub trait ConfigSource: Send + Sync {
    fn load<'a>(&'a self, context: &'a Context) -> BoxFuture<'a, Result<HostConfig, Error>>;
}

pub fn builtin_modes() -> Vec<ModeSpec> {
    vec![
        ModeSpec {
            name: "chat".into(), version: "v1".into(), display_name: "Chat".into(),
            description: "Interactive chat and coding. Default mode.".into(), category: "direct".into(),
            ..Default::default()
        },
        ModeSpec {
            name: "plan".into(), version: "v1".into(), display_name: "Plan".into(),
            description: "Read-only planning session.".into(), category: "direct".into(),
            tool_access: "read-only".into(),
            instructions: "PLAN MODE — Read-Only Planning\n\nFocus on understanding the problem, exploring the code, weighing tradeoffs,\nand producing a concrete plan. Use read-only inspection tools freely, but do\nnot modify files, run mutating commands, or take externally visible actions.\nWhen the plan is ready, present it and wait for the user to switch modes\nbefore implementing.".into(),
            ..Default::default()
        },
    ]
}

/// Trusted host configuration only, never automatic repository configuration.
pub struct FileConfigSource {
    root: PathBuf,
    trusted_root: bool,
    active_mode: Option<String>,
}
impl Default for FileConfigSource {
    fn default() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        Self {
            active_mode: None,
            trusted_root: home.is_some(),
            root: home
                .map(|path| path.join(".gratefulagents"))
                .unwrap_or_default(),
        }
    }
}
impl FileConfigSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            active_mode: None,
            root: root.into(),
            trusted_root: true,
        }
    }
    /// Interpret a textual configuration root: trim whitespace, use the guarded
    /// default for an empty value, and expand `~`/`~/...` using an absolute HOME.
    /// `new` remains available for literal native filesystem paths.
    pub fn from_config_root(root: &str) -> Self {
        let root = root.trim();
        if root.is_empty() {
            return Self::default();
        }
        if root == "~" || root.starts_with("~/") {
            let mut source = Self::default();
            if source.trusted_root {
                let home = source.root.parent().expect("default root has a home");
                let suffix = root
                    .strip_prefix("~/")
                    .unwrap_or("")
                    .trim_start_matches('/');
                source.root = if suffix.is_empty() {
                    home.to_owned()
                } else {
                    home.join(suffix)
                };
            }
            return source;
        }
        Self::new(root)
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn mode_dir(&self) -> PathBuf {
        self.root.join("modes")
    }
    pub fn agent_dir(&self) -> PathBuf {
        self.root.join("agents")
    }
    pub fn with_active_mode(mut self, name: impl AsRef<str>) -> Self {
        let name = name.as_ref().trim();
        self.active_mode = (!name.is_empty()).then(|| name.to_owned());
        self
    }
    pub fn active_mode(&self) -> Option<&str> {
        self.active_mode.as_deref()
    }
    fn check_root(&self, context: &Context) -> Result<(), Error> {
        context.check_active()?;
        if !self.trusted_root {
            return Err(invalid(
                "default configuration source requires an absolute HOME",
            ));
        }
        Ok(())
    }
    pub fn load_files(&self, context: &Context) -> Result<HostConfig, Error> {
        Ok(HostConfig {
            modes: self.list_modes(context)?,
            roles: self.load_roles(context)?,
        })
    }
    pub fn list_modes(&self, context: &Context) -> Result<Vec<ModeSpec>, Error> {
        self.check_root(context)?;
        let mut modes: BTreeMap<String, ModeSpec> = builtin_modes()
            .into_iter()
            .map(|m| (m.name.clone(), m))
            .collect();
        let mut seen = BTreeSet::new();
        for path in config_files(&self.mode_dir(), &["yaml", "yml", "json"])? {
            context.check_active()?;
            let mode = parse_mode_file(&path)?;
            let key = mode.name.to_lowercase();
            if !seen.insert(key.clone()) {
                return Err(invalid(format!("duplicate mode: {}", mode.name)));
            }
            modes.insert(key, mode);
        }
        let mut modes: Vec<_> = modes.into_values().collect();
        modes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(modes)
    }
    pub fn get_mode(&self, context: &Context, name: &str) -> Result<ModeSpec, Error> {
        let name = name.trim();
        validate_name(name)?;
        self.check_root(context)?;
        for ext in ["yaml", "yml"] {
            let path = self.mode_dir().join(format!("{name}.{ext}"));
            match std::fs::metadata(&path) {
                Ok(_) => return parse_mode_file(&path),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(config_error(&path, e)),
            }
        }
        self.list_modes(context)?
            .into_iter()
            .find(|mode| {
                mode.name.to_lowercase() == name.to_lowercase()
                    || mode.display_name.to_lowercase() == name.to_lowercase()
            })
            .ok_or_else(|| {
                invalid(format!(
                    "mode {name:?} not found in {}",
                    self.mode_dir().display()
                ))
            })
    }
    pub fn load_roles(&self, context: &Context) -> Result<Vec<RoleSpec>, Error> {
        self.check_root(context)?;
        load_role_catalog(context, self.agent_dir())
    }
}

/// Load a standalone Markdown role directory, ordered by declared role name.
///
/// Missing directories produce an empty catalog. Uses the same strict parser as
/// [`FileConfigSource::load_roles`], without consulting HOME or appending `agents`.
/// Checks cancellation before reading the directory and between files. Reads are
/// synchronous; callers should provide a local configuration directory.
pub fn load_role_catalog(
    context: &Context,
    directory: impl AsRef<Path>,
) -> Result<Vec<RoleSpec>, Error> {
    context.check_active()?;
    let mut roles = BTreeMap::new();
    for path in config_files(directory.as_ref(), &["md"])? {
        context.check_active()?;
        let role = parse_role_file(&path)?;
        if roles.insert(role.name.clone(), role).is_some() {
            return Err(invalid("duplicate role name"));
        }
    }
    Ok(roles.into_values().collect())
}

fn parse_mode_file(path: &Path) -> Result<ModeSpec, Error> {
    let raw = read_config(path)?;
    let value: serde_yaml::Value = serde_yaml::from_str(&raw).map_err(|e| config_error(path, e))?;
    let name = value
        .get("metadata")
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .map(str::to_owned);
    let mut mode: ModeSpec = match value.get("spec") {
        Some(spec) if spec.is_null() => ModeSpec::default(),
        Some(spec) => serde_yaml::from_value(spec.clone()).map_err(|e| config_error(path, e))?,
        None => serde_yaml::from_value(value.clone()).map_err(|e| config_error(path, e))?,
    };
    // The SDK falls back to a plain document when the CRD spec is all-zero.
    // Remove recognized envelope fields only; strict unknown-field validation
    // still applies to the resulting plain mode.
    if value.get("spec").is_some()
        && mode.name.is_empty()
        && mode.version.is_empty()
        && mode.display_name.is_empty()
        && mode.description.is_empty()
        && mode.category.is_empty()
        && !mode.autonomous
        && mode.tool_access.is_empty()
        && mode.instructions.is_empty()
        && mode.model_routing.is_none()
        && mode.constraints.is_none()
    {
        let mut plain = value.clone();
        if let Some(mapping) = plain.as_mapping_mut() {
            for key in ["spec", "metadata", "kind", "apiVersion"] {
                mapping.remove(serde_yaml::Value::String(key.into()));
            }
        }
        mode = serde_yaml::from_value(plain).map_err(|e| config_error(path, e))?;
    }
    if mode.name.trim().is_empty() {
        mode.name = name
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| file_stem(path));
    }
    normalize_file_mode(&mut mode)?;
    Ok(mode)
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RoleFrontmatter {
    name: String,
    description: String,
    tool_access: String,
    #[serde(rename = "toolAccess")]
    tool_access_alt: String,
    model_override: String,
    model: String,
    #[serde(rename = "instructions")]
    _instructions: String,
}

fn parse_role_file(path: &Path) -> Result<RoleSpec, Error> {
    let raw = read_config(path)?.replace("\r\n", "\n");
    let (front, body) = if let Some(rest) = raw.strip_prefix("---\n") {
        let (front, body) = rest
            .split_once("\n---\n")
            .ok_or_else(|| invalid(format!("{}: unterminated role frontmatter", path.display())))?;
        (
            serde_yaml::from_str::<RoleFrontmatter>(front).map_err(|e| config_error(path, e))?,
            body,
        )
    } else {
        (RoleFrontmatter::default(), raw.as_str())
    };
    let name = if front.name.trim().is_empty() {
        file_stem(path).trim().to_owned()
    } else {
        front.name.trim().to_owned()
    };
    validate_name(&name)?;
    let access = if front.tool_access.trim().is_empty() {
        &front.tool_access_alt
    } else {
        &front.tool_access
    };
    let tool_access = normalize_access(access, false)?;
    let model_override = if front.model_override.trim().is_empty() {
        front.model.trim()
    } else {
        front.model_override.trim()
    };
    if body.trim().is_empty() {
        return Err(invalid(format!(
            "{}: role instructions are empty",
            path.display()
        )));
    }
    Ok(RoleSpec {
        name,
        description: front.description,
        tool_access,
        model_override: model_override.into(),
        fallback_models: None,
        instructions: body.trim().into(),
    })
}

impl ConfigSource for FileConfigSource {
    fn load<'a>(&'a self, context: &'a Context) -> BoxFuture<'a, Result<HostConfig, Error>> {
        Box::pin(async move { self.load_files(context) })
    }
}
fn config_error(path: &Path, error: impl std::error::Error + Send + Sync + 'static) -> Error {
    invalid(format!("invalid configuration: {}", path.display())).with_source(error)
}
fn read_config(path: &Path) -> Result<String, Error> {
    std::fs::read_to_string(path).map_err(|e| {
        Error::new(
            ErrorCategory::Host,
            format!("read configuration: {}", path.display()),
        )
        .with_source(e)
    })
}
fn config_files(dir: &Path, extensions: &[&str]) -> Result<Vec<PathBuf>, Error> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => {
            return Err(
                Error::new(ErrorCategory::Host, "read configuration directory").with_source(e),
            );
        }
    };
    let mut paths = vec![];
    for entry in entries {
        let entry = entry.map_err(|e| {
            Error::new(ErrorCategory::Host, "read configuration entry").with_source(e)
        })?;
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| extensions.contains(&s.to_ascii_lowercase().as_str()))
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}
fn file_stem(path: &Path) -> String {
    path.file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':']) {
        return Err(invalid("configuration name must be a single nonempty name"));
    }
    Ok(())
}
fn normalize_mode(mode: &mut ModeSpec) -> Result<(), Error> {
    mode.name = mode.name.trim().into();
    validate_name(&mode.name)?;
    if mode.version.trim().is_empty() {
        mode.version = "v1".into();
    }
    parse_access(&mode.tool_access)?;
    Ok(())
}
fn normalize_file_mode(mode: &mut ModeSpec) -> Result<(), Error> {
    normalize_mode(mode)?;
    mode.version = mode.version.trim().into();
    mode.display_name = mode.display_name.trim().into();
    mode.description = mode.description.trim().into();
    mode.category = mode.category.trim().into();
    mode.instructions = mode.instructions.trim().into();
    mode.tool_access = normalize_access(&mode.tool_access, true)?;
    if let Some(routing) = &mut mode.model_routing {
        routing.default_model = routing.default_model.trim().into();
        routing.reasoning_level = routing.reasoning_level.trim().into();
        routing.text_verbosity = routing.text_verbosity.trim().into();
        normalize_fallbacks(&mut routing.fallback_models);
        for role in routing.role_overrides.values_mut() {
            role.model = role.model.trim().into();
            role.reasoning_level = role.reasoning_level.trim().into();
            role.text_verbosity = role.text_verbosity.trim().into();
            normalize_fallbacks(&mut role.fallback_models);
        }
    }
    Ok(())
}
fn normalize_fallbacks(models: &mut Option<Vec<String>>) {
    if let Some(models) = models {
        *models = models
            .iter()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
    }
}
fn normalize_access(value: &str, inherit: bool) -> Result<String, Error> {
    Ok(match parse_access(value)? {
        None if inherit => "",
        None | Some(AccessMode::WorkspaceWrite) => "full",
        Some(AccessMode::ReadOnly) => "read-only",
        Some(AccessMode::FullAccess) => "full-access",
    }
    .into())
}

fn parse_access(value: &str) -> Result<Option<AccessMode>, Error> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" => Ok(None),
        "read-only" | "read_only" | "readonly" | "analysis" => Ok(Some(AccessMode::ReadOnly)),
        "full" | "execution" | "write" | "workspace-write" | "workspace_write" => {
            Ok(Some(AccessMode::WorkspaceWrite))
        }
        "full-access" | "full_access" => Ok(Some(AccessMode::FullAccess)),
        _ => Err(invalid("unrecognized tool access")),
    }
}
fn narrow_access(current: AccessMode, constraint: Option<AccessMode>) -> AccessMode {
    match (current, constraint) {
        (AccessMode::ReadOnly, _) | (_, Some(AccessMode::ReadOnly)) => AccessMode::ReadOnly,
        (AccessMode::WorkspaceWrite, _) | (_, Some(AccessMode::WorkspaceWrite)) => {
            AccessMode::WorkspaceWrite
        }
        _ => AccessMode::FullAccess,
    }
}

/// Exclusive session owner. Shared handles never keep tasks alive after owner drop.
pub struct SessionState {
    handle: SessionHandle,
    scheduler: Option<Scheduler>,
}
#[derive(Clone)]
pub struct SessionHandle {
    cancellation: CancellationToken,
    subagents: Option<Arc<SubagentSession>>,
}
impl Default for SessionState {
    fn default() -> Self {
        Self::new()
    }
}
impl SessionState {
    pub fn new() -> Self {
        Self {
            handle: SessionHandle {
                cancellation: CancellationToken::new(),
                subagents: None,
            },
            scheduler: None,
        }
    }
    pub fn with_scheduler(scheduler: Scheduler) -> Self {
        let mut state = Self::new();
        state.handle.subagents = Some(Arc::new(SubagentSession::new(scheduler.handle())));
        state.scheduler = Some(scheduler);
        state
    }
    pub fn handle(&self) -> SessionHandle {
        self.handle.clone()
    }
    pub async fn close(&mut self) -> Result<(), Error> {
        self.handle.cancellation.cancel();
        if let Some(scheduler) = self.scheduler.take() {
            scheduler.shutdown().await?;
        }
        Ok(())
    }
}
impl Drop for SessionState {
    fn drop(&mut self) {
        self.handle.cancellation.cancel();
    }
}
impl SessionHandle {
    pub fn is_closed(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    pub fn subagents(&self) -> Option<&Arc<SubagentSession>> {
        self.subagents.as_ref()
    }
    pub(crate) fn context(&self, context: &Context) -> Context {
        Context {
            cancellation: Arc::new(SessionCancellation {
                caller: context.cancellation.clone(),
                session: self.cancellation.clone(),
            }),
            ..context.clone()
        }
    }
}
struct SessionCancellation {
    caller: Arc<dyn Cancellation>,
    session: CancellationToken,
}
impl Cancellation for SessionCancellation {
    fn is_cancelled(&self) -> bool {
        self.caller.is_cancelled() || self.session.is_cancelled()
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let mut caller = self.caller.cancelled();
            let mut session = Box::pin(self.session.cancelled());
            std::future::poll_fn(|cx| {
                if caller.as_mut().poll(cx).is_ready() || session.as_mut().poll(cx).is_ready() {
                    Poll::Ready(())
                } else {
                    Poll::Pending
                }
            })
            .await
        })
    }
}

/// Construction is credential-free; registering a route never reads its store.
pub struct Builder {
    config: Config,
    routes: Routes,
    source: Option<Arc<dyn ConfigSource>>,
    session: Option<SessionHandle>,
    owned_session: Option<SessionState>,
    runner: RunnerConfig,
    input_guardrails: Vec<Arc<dyn adk_runtime::Guardrail>>,
    output_guardrails: Vec<Arc<dyn adk_runtime::Guardrail>>,
    implementations: Vec<Arc<dyn Tool>>,
    extra_tools: Vec<Arc<dyn Tool>>,
    #[cfg(feature = "project-state")]
    project_state_store: Option<Arc<dyn adk_project_state::Store>>,
    #[cfg(feature = "project-state")]
    project_state_host: Option<adk_project_state::FilesystemResolutionHost>,
    #[cfg(feature = "project-state")]
    owned_project_state: Option<project_state::Owner>,
    warnings: Vec<String>,
    #[cfg(feature = "mcp")]
    mcp: Option<McpInput>,
    #[cfg(feature = "mcp")]
    owned_mcp: Option<adk_mcp::session::OwnedMcpSession>,
    shell: Option<adk_sandbox::Config>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    lsp: Option<adk_tools::lsp::Config>,
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    browser: Option<adk_tools::browser::Config>,
}
impl Builder {
    pub fn new(config: Config) -> Self {
        let default = default_route(
            config.default_provider.as_deref(),
            &config.model,
            config.provider,
        );
        Self {
            config,
            routes: Routes::new(default),
            source: None,
            session: None,
            owned_session: None,
            runner: RunnerConfig::default(),
            input_guardrails: vec![],
            output_guardrails: vec![],
            implementations: vec![],
            extra_tools: vec![],
            #[cfg(feature = "project-state")]
            project_state_store: None,
            #[cfg(feature = "project-state")]
            project_state_host: None,
            #[cfg(feature = "project-state")]
            owned_project_state: None,
            warnings: vec![],
            #[cfg(feature = "mcp")]
            mcp: None,
            #[cfg(feature = "mcp")]
            owned_mcp: None,
            shell: None,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            lsp: None,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            browser: None,
        }
    }
    #[cfg(feature = "project-state")]
    pub fn project_state_store(mut self, store: Arc<dyn adk_project_state::Store>) -> Self {
        self.project_state_store = Some(store);
        self
    }
    #[cfg(feature = "project-state")]
    pub fn project_state_host(mut self, host: adk_project_state::FilesystemResolutionHost) -> Self {
        self.project_state_host = Some(host);
        self
    }
    pub fn routes(mut self, routes: Routes) -> Self {
        self.routes = routes;
        self
    }
    pub fn route(
        mut self,
        spec: &RouteSpec,
        store: Arc<dyn CredentialStore>,
        refresh: Arc<dyn Refresh>,
    ) -> Result<Self, Error> {
        self.routes.register_spec(spec, store, refresh)?;
        Ok(self)
    }
    pub fn model(
        mut self,
        prefix: &str,
        kind: Kind,
        model: Arc<dyn StreamingModel>,
    ) -> Result<Self, Error> {
        self.routes.register_kind(prefix, kind, model)?;
        Ok(self)
    }
    pub fn source(mut self, source: Arc<dyn ConfigSource>) -> Self {
        self.source = Some(source);
        self
    }
    pub fn session(mut self, session: SessionHandle) -> Self {
        self.session = Some(session);
        self.owned_session = None;
        self
    }
    pub fn owned_session(mut self, session: SessionState) -> Self {
        self.session = Some(session.handle());
        self.owned_session = Some(session);
        self
    }
    pub fn input_guardrails(
        mut self,
        guards: impl IntoIterator<Item = Arc<dyn adk_runtime::Guardrail>>,
    ) -> Self {
        self.input_guardrails.extend(guards);
        self
    }
    pub fn output_guardrails(
        mut self,
        guards: impl IntoIterator<Item = Arc<dyn adk_runtime::Guardrail>>,
    ) -> Self {
        self.output_guardrails.extend(guards);
        self
    }
    pub fn runner_config(mut self, config: RunnerConfig) -> Self {
        self.runner = config;
        self
    }
    pub fn implementations(mut self, tools: impl IntoIterator<Item = Arc<dyn Tool>>) -> Self {
        self.implementations.extend(tools);
        self
    }
    pub fn extra_tools(mut self, tools: impl IntoIterator<Item = Arc<dyn Tool>>) -> Self {
        self.extra_tools.extend(tools);
        self
    }

    #[cfg(feature = "mcp")]
    pub fn mcp(mut self, input: McpInput) -> Self {
        self.mcp = Some(input);
        self
    }

    /// Supply shell resources without changing the frozen tool selection.
    pub fn shell(mut self, config: adk_sandbox::Config) -> Self {
        self.shell = Some(config);
        self
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn lsp(mut self, config: adk_tools::lsp::Config) -> Self {
        self.lsp = Some(config);
        self
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub fn browser(mut self, config: adk_tools::browser::Config) -> Self {
        self.browser = Some(config);
        self
    }

    /// Assemble tool resources without provider construction, config-source loading,
    /// mode/role resolution, runtime priming, handoffs or scheduler setup.
    pub async fn build_tools(mut self, context: &Context) -> Result<ToolRuntime, Error> {
        let result = self.assemble_tool_runtime(context).await;
        #[cfg(feature = "mcp")]
        if result.is_err()
            && let Some(mcp) = &self.owned_mcp
        {
            let _ = mcp.close().await;
        }
        if result.is_err()
            && let Some(session) = &mut self.owned_session
        {
            session.close().await?;
        }
        result
    }

    async fn assemble_tool_runtime(&mut self, context: &Context) -> Result<ToolRuntime, Error> {
        context.check_active()?;
        let features = self.config.resolved_features();
        self.validate_integrations(&features)?;
        let policy = self.config.policy.tools.clone();
        let config = self.tool_config(&features, &policy);
        #[cfg(feature = "project-state")]
        if features.project_state.active() {
            let store = project_state::open(
                self.project_state_store.take(),
                self.project_state_host.take(),
                &self.config,
            )
            .await?;
            self.owned_project_state = Some(project_state::Owner::new(
                store,
                project_state::actor(&self.config),
            ));
        }
        let tools = self
            .assemble_tools(context, &features, config, policy)
            .await?;
        Ok(ToolRuntime {
            tools,
            session: self.session.take(),
            owned_session: self.owned_session.take(),
            #[cfg(feature = "mcp")]
            owned_mcp: self.owned_mcp.take(),
            #[cfg(feature = "project-state")]
            owned_project_state: self.owned_project_state.take(),
        })
    }

    fn tool_config(&self, features: &Features, policy: &ToolPolicy) -> adk_tools::Config {
        let mut tool_config = self.config.tool_options.clone();
        tool_config.features = match &self.config.features {
            Some(_) => adk_tools::Features::Strict(features.tools.clone()),
            None => adk_tools::Features::Legacy(self.config.legacy_tools.clone()),
        };
        if let adk_tools::Features::Legacy(legacy) = &mut tool_config.features {
            legacy.enable_project_state = false;
        }
        tool_config.access = policy.access;
        tool_config.allowed_mutating_tools = policy.allowed_mutating_tools.clone();
        tool_config
    }

    fn validate_integrations(&self, features: &Features) -> Result<(), Error> {
        #[cfg(not(feature = "project-state"))]
        if features.project_state.active() {
            return Err(invalid(
                "project-state selection requires the project-state Cargo feature",
            ));
        }
        #[cfg(not(feature = "mcp"))]
        if features.mcp.active() {
            return Err(invalid("MCP selection requires the mcp Cargo feature"));
        }
        #[cfg(feature = "mcp")]
        if features.mcp.active() && self.mcp.is_none() {
            return Err(invalid(
                "MCP selection requires explicit configuration and host authority",
            ));
        }
        Ok(())
    }

    async fn assemble_tools(
        &mut self,
        context: &Context,
        features: &Features,
        tool_config: adk_tools::Config,
        policy: ToolPolicy,
    ) -> Result<ToolBundle, Error> {
        let mut tool_builder = BundleBuilder::new(tool_config)
            .implementations(std::mem::take(&mut self.implementations))
            .extra_tools(std::mem::take(&mut self.extra_tools));
        if features.mcp.active() {
            #[cfg(feature = "mcp")]
            {
                let owned = mcp::assemble(
                    self.mcp.take().expect("preflight MCP input"),
                    &features.mcp,
                    &self.config.work_dir,
                    context,
                )
                .await?;
                let composed = adk_mcp::tools::build_tools(Arc::new(owned.handle()));
                self.owned_mcp = Some(owned);
                tool_builder = tool_builder.composed_tools(composed);
            }
        }
        #[cfg(feature = "project-state")]
        if let Some(owner) = &self.owned_project_state {
            tool_builder = tool_builder.composed_tools(owner.tools(&features.project_state));
        }
        if let Some(config) = self.shell.take() {
            tool_builder = tool_builder.shell(config);
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if let Some(config) = self.lsp.take() {
            tool_builder = tool_builder.lsp(config);
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        if let Some(config) = self.browser.take() {
            tool_builder = tool_builder.browser(config);
        }
        context.check_active()?;
        tool_builder
            .build(policy)
            .map_err(|e| invalid("tool bundle construction failed").with_source(e))
    }

    pub async fn build(mut self, context: &Context) -> Result<Bundle, Error> {
        let result = self.assemble(context).await;
        #[cfg(feature = "mcp")]
        if result.is_err()
            && let Some(mcp) = &self.owned_mcp
        {
            let _ = mcp.close().await;
        }
        if result.is_err()
            && let Some(session) = &mut self.owned_session
        {
            session.close().await?;
        }
        result
    }
    async fn assemble(&mut self, context: &Context) -> Result<Bundle, Error> {
        context.check_active()?;
        let features = self.config.resolved_features();
        self.validate_integrations(&features)?;
        let workspace_dir = self
            .config
            .work_dir
            .to_str()
            .ok_or_else(|| invalid("workspace prompt path must be UTF-8"))?
            .to_owned();
        let host = match &self.source {
            Some(source) => source.load(context).await?,
            None => HostConfig::default(),
        };
        context.check_active()?;
        let mut modes: BTreeMap<String, ModeSpec> = builtin_modes()
            .into_iter()
            .map(|m| (m.name.clone(), m))
            .collect();
        for mut mode in host.modes {
            normalize_mode(&mut mode)?;
            modes.insert(mode.name.to_lowercase(), mode);
        }
        let mode = match &self.config.mode_snapshot {
            Some(mode) => Some(mode.clone()),
            None => match &self.config.active_mode {
                Some(name) => {
                    validate_name(name.trim())?;
                    Some(
                        modes
                            .get(&name.trim().to_lowercase())
                            .or_else(|| {
                                modes
                                    .values()
                                    .find(|m| m.display_name.eq_ignore_ascii_case(name.trim()))
                            })
                            .ok_or_else(|| invalid("active mode not found"))?
                            .clone(),
                    )
                }
                None => None,
            },
        };
        let mut roles = Vec::<RoleSpec>::new();
        for catalog in [host.roles.as_slice(), self.config.roles.as_slice()] {
            let mut seen = BTreeSet::new();
            for role in catalog {
                let mut role = role.clone();
                role.name = role.name.trim().to_owned();
                if role.name.is_empty() || role.instructions.trim().is_empty() {
                    return Err(invalid("role name and instructions must not be empty"));
                }
                if !seen.insert(role.name.clone()) {
                    return Err(invalid(format!("duplicate role: {}", role.name)));
                }
                if let Some(existing) = roles.iter_mut().find(|r| r.name == role.name) {
                    *existing = role;
                } else {
                    roles.push(role);
                }
            }
        }
        let role = self
            .config
            .active_role
            .as_ref()
            .map(|name| {
                roles
                    .iter()
                    .find(|r| r.name == name.trim())
                    .ok_or_else(|| invalid("active role not found"))
            })
            .transpose()?;
        let mut model = self.config.model.trim().to_owned();
        let mut fallbacks = self.config.fallback_models.clone();
        let mut settings = adk_runtime::settings::routing_settings(
            if self.config.reasoning.trim().is_empty() {
                "medium"
            } else {
                &self.config.reasoning
            },
            if self.config.verbosity.trim().is_empty() {
                "medium"
            } else {
                &self.config.verbosity
            },
        );
        settings.extend(self.config.settings.clone());
        let base_settings = settings.clone();
        let mut policy = self.config.policy.clone();
        let mut instructions = vec![self.config.instructions.trim().to_owned()];
        if features.mode_instructions {
            let label = mode
                .as_ref()
                .and_then(|mode| {
                    [&mode.display_name, &mode.name]
                        .into_iter()
                        .find(|s| !s.is_empty())
                        .map(String::as_str)
                })
                .or_else(|| {
                    self.config
                        .active_mode
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                })
                .unwrap_or("chat");
            instructions.push(format!("Active mode: {label}"));
        }
        if let Some(mode) = &mode {
            policy.tools.access =
                narrow_access(policy.tools.access, parse_access(&mode.tool_access)?);
            if features.mode_instructions {
                instructions.push(mode.instructions.trim().into());
            }
            if let Some(constraints) = &mode.constraints {
                if let Some(limit) = constraints.max_concurrent_subagents
                    && self
                        .session
                        .as_ref()
                        .and_then(SessionHandle::subagents)
                        .is_some_and(|session| {
                            session.scheduler.max_concurrency() > limit.get() as usize
                        })
                {
                    return Err(invalid(
                        "injected scheduler exceeds mode maxConcurrentSubAgents",
                    ));
                }
                if let Some(limit) = constraints.max_turns {
                    policy.max_turns = policy.max_turns.min(limit);
                }
                if let Some(limit) = constraints.subagent_max_turns {
                    self.runner.subagent_max_turns = Some(
                        self.runner
                            .subagent_max_turns
                            .map_or(limit, |current| current.min(limit)),
                    );
                }
                if let Some(retries) = constraints.max_retries {
                    self.runner.retry.max_retries = retries;
                }
            }
            if features.mode_model_routing
                && let Some(routing) = &mode.model_routing
            {
                apply_routing(
                    &mut model,
                    &mut fallbacks,
                    &mut settings,
                    &routing.default_model,
                    &routing.fallback_models,
                    &routing.reasoning_level,
                    &routing.text_verbosity,
                    &routing.settings,
                );
            }
        }
        if let Some(role) = role {
            if role.instructions.trim().is_empty() {
                return Err(invalid("role instructions are empty"));
            }
            policy.tools.access =
                narrow_access(policy.tools.access, parse_access(&role.tool_access)?);
            instructions.push(role.instructions.trim().into());
            if !role.model_override.trim().is_empty() {
                model = role.model_override.trim().into();
            }
            if let Some(selected) = &role.fallback_models {
                fallbacks = selected.clone();
            }
            if features.mode_model_routing
                && let Some(routing) = mode
                    .as_ref()
                    .and_then(|m| m.model_routing.as_ref())
                    .and_then(|r| r.role_overrides.get(&role.name))
            {
                apply_routing(
                    &mut model,
                    &mut fallbacks,
                    &mut settings,
                    &routing.model,
                    &routing.fallback_models,
                    &routing.reasoning_level,
                    &routing.text_verbosity,
                    &routing.settings,
                );
            }
        }
        if self.session.is_none() {
            let state = SessionState::new();
            self.session = Some(state.handle());
            self.owned_session = Some(state);
        }
        let session = self.session.as_ref().unwrap().clone();
        if session.is_closed() {
            return Err(Error::new(ErrorCategory::Cancelled, "session is closed"));
        }
        // Resolve every binding before constructing tool resources, including fallbacks.
        let routes = Arc::new(std::mem::take(&mut self.routes));
        let (_, resolved) = routes.resolve(&model)?;
        if model.is_empty() {
            model = resolved;
        }
        for fallback in &fallbacks {
            routes.resolve(fallback)?;
        }
        let name = role
            .map(|r| r.name.as_str())
            .unwrap_or(self.config.agent_name.trim());
        let mut agent = AgentConfig::new(
            if name.is_empty() { "agent" } else { name },
            ModelBinding::streaming(model, routes.clone()),
        );
        agent.input_guardrails = self.input_guardrails.clone();
        agent.output_guardrails = self.output_guardrails.clone();
        agent.instructions = instructions
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        agent.fallbacks = fallbacks
            .into_iter()
            .map(|name| ModelBinding::streaming(name, routes.clone()))
            .collect();
        settings.insert(
            "parallel_tool_calls".into(),
            features.parallel_tool_calls.into(),
        );
        agent.settings = settings;
        let mut targets = Vec::new();
        if features.handoffs {
            let mut names = BTreeSet::new();
            for role in &roles {
                let transfer = format!("transfer_to_{}", sanitize_handoff_name(&role.name));
                if !names.insert(transfer.clone()) {
                    return Err(invalid(format!("duplicate handoff name: {transfer}")));
                }
                let mut model = self.config.model.trim().to_owned();
                let mut fallbacks = self.config.fallback_models.clone();
                let mut settings = base_settings.clone();
                apply_routing(
                    &mut model,
                    &mut fallbacks,
                    &mut settings,
                    &role.model_override,
                    &role.fallback_models,
                    "",
                    "",
                    &Map::new(),
                );
                if features.mode_model_routing
                    && let Some(routing) = mode.as_ref().and_then(|m| m.model_routing.as_ref())
                {
                    apply_routing(
                        &mut model,
                        &mut fallbacks,
                        &mut settings,
                        &routing.default_model,
                        &routing.fallback_models,
                        &routing.reasoning_level,
                        &routing.text_verbosity,
                        &routing.settings,
                    );
                    if let Some(routing) = routing.role_overrides.get(&role.name) {
                        apply_routing(
                            &mut model,
                            &mut fallbacks,
                            &mut settings,
                            &routing.model,
                            &routing.fallback_models,
                            &routing.reasoning_level,
                            &routing.text_verbosity,
                            &routing.settings,
                        );
                    }
                }
                let (_, resolved) = routes.resolve(&model)?;
                if model.is_empty() {
                    model = resolved;
                }
                for fallback in &fallbacks {
                    routes.resolve(fallback)?;
                }
                let mut target =
                    AgentConfig::new(&role.name, ModelBinding::streaming(model, routes.clone()));
                target.instructions = role.instructions.clone();
                target.fallbacks = fallbacks
                    .into_iter()
                    .map(|name| ModelBinding::streaming(name, routes.clone()))
                    .collect();
                settings.insert(
                    "parallel_tool_calls".into(),
                    features.parallel_tool_calls.into(),
                );
                target.settings = settings;
                target.input_guardrails = self.input_guardrails.clone();
                target.output_guardrails = self.output_guardrails.clone();
                target.tool_access_ceiling = Some(narrow_access(
                    policy.tools.access,
                    parse_access(&role.tool_access)?,
                ));
                let description = if role.description.trim().is_empty() {
                    format!("Transfer the conversation to the {} specialist.", role.name)
                } else {
                    role.description.trim().to_owned()
                };
                targets.push((transfer, description, target));
            }
            if roles.is_empty() && features.handoff_generic_fallback {
                let mut model = self.config.model.trim().to_owned();
                let (_, resolved) = routes.resolve(&model)?;
                if model.is_empty() {
                    model = resolved;
                }
                let mut target =
                    AgentConfig::new("specialist", ModelBinding::streaming(model, routes.clone()));
                target.instructions = "# System context\nYou are part of a multi-agent system designed to make agent coordination and execution easy. Agents use two primary abstractions: tools and handoffs. Handoffs transfer control to another agent that is better suited for the task, and are achieved by calling a handoff function, generally named `transfer_to_<agent_name>`. Transfers between agents are handled seamlessly in the background; do not mention or draw attention to these transfers in your conversation with the user.\n\nYou are the handoff specialist. Resolve the delegated request and explain the result briefly.".into();
                target.settings = base_settings;
                target.settings.insert(
                    "parallel_tool_calls".into(),
                    features.parallel_tool_calls.into(),
                );
                target.tool_access_ceiling = Some(policy.tools.access);
                target.input_guardrails = self.input_guardrails.clone();
                target.output_guardrails = self.output_guardrails.clone();
                targets.push((
                    "transfer_to_specialist".into(),
                    "Transfer to a specialist agent.".into(),
                    target,
                ));
            }
        }
        let mut excluded: BTreeSet<String> = ["finish", "present_plan", "AskUserQuestion"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut tool_config = self.tool_config(&features, &policy.tools);
        if features.subagents.enabled() {
            let children = session
                .subagents
                .clone()
                .ok_or_else(|| invalid("subagents require a session-owned scheduler"))?;
            self.runner.subagents = Some(children.clone());
            // Managed tools use ExtraTools registration, but cannot enable other extensions.
            let mut managed = adk_runtime::build_subagent_task_tools(children, agent.name.clone());
            excluded.extend(managed.iter().map(|tool| tool.definition().name.clone()));
            managed.retain(|tool| match tool.definition().name.as_str() {
                "subagent" | "subagent_wait" => features.subagents.task,
                "subagent_status" => features.subagents.status,
                "subagent_control" => features.subagents.control,
                _ => false,
            });
            let extras_enabled = match &tool_config.features {
                adk_tools::Features::Strict(f) => f.contains("ExtraTools"),
                adk_tools::Features::Legacy(f) => f.enable_tools || f.enable_subagents,
            };
            if extras_enabled {
                managed.append(&mut self.extra_tools);
            }
            self.extra_tools = managed;
            if let adk_tools::Features::Strict(f) = &mut tool_config.features {
                f.insert("ExtraTools".into());
            }
        } else {
            self.runner.subagents = None;
        }
        let extras_enabled = match &tool_config.features {
            adk_tools::Features::Strict(f) => f.contains("ExtraTools"),
            adk_tools::Features::Legacy(f) => f.enable_tools || f.enable_subagents,
        };
        if extras_enabled {
            for (name, _, _) in &targets {
                if self
                    .extra_tools
                    .iter()
                    .any(|t| t.definition().name == *name)
                    && tool_config
                        .allowed_names
                        .as_ref()
                        .is_none_or(|names| names.contains(name))
                {
                    return Err(invalid(format!(
                        "handoff collides with parent tool: {name}"
                    )));
                }
            }
        }
        #[cfg(feature = "project-state")]
        if features.project_state.active() {
            let store = project_state::open(
                self.project_state_store.take(),
                self.project_state_host.take(),
                &self.config,
            )
            .await?;
            if features.project_state.prime_context
                && project_state::prime(
                    store.clone(),
                    &self.config,
                    &mut self.runner.working_state_context,
                )
                .await
                .is_err()
            {
                self.warnings.push("project-state priming failed".into());
            }
            context.check_active()?;
            self.owned_project_state = Some(project_state::Owner::new(
                store,
                project_state::actor(&self.config),
            ));
        }
        let mut tools = self
            .assemble_tools(context, &features, tool_config, policy.tools.clone())
            .await?;
        let prepared = tools.prepared();
        policy.tools = prepared.policy;
        agent.tools = prepared.tools;
        #[cfg(feature = "mcp")]
        if let Some(mcp) = &self.owned_mcp {
            agent.mcp_servers = mcp.connected_servers().keys().cloned().collect();
        }

        let composed = (|| -> Result<BTreeMap<String, Arc<AgentConfig>>, Error> {
            let mut specialists = BTreeMap::new();
            let mut graph_names: BTreeSet<_> = agent
                .tools
                .iter()
                .map(|t| t.definition().name.clone())
                .collect();
            for (name, description, mut target) in targets {
                if graph_names.contains(&name) {
                    return Err(invalid(format!(
                        "handoff collides with parent tool: {name}"
                    )));
                }
                if !roles.is_empty() {
                    let view = tools
                        .role_view(target.tool_access_ceiling.unwrap(), &excluded)
                        .map_err(|e| {
                            invalid("specialist tool preparation failed").with_source(e)
                        })?;
                    target.tools = view.tools;
                    graph_names.extend(target.tools.iter().map(|t| t.definition().name.clone()));
                }
                graph_names.insert(name.clone());
                let target = Arc::new(target);
                specialists.insert(target.name.clone(), target.clone());
                agent.handoffs.push(Handoff {
                    definition: ToolDefinition {
                        name, description,
                        input_schema: serde_json::json!({"type":"object", "properties":{}, "additionalProperties":false}).try_into().expect("object schema"),
                        read_only: true,
                        requires_approval: false,
                    },
                    target,
                    input_filter: HandoffInputFilter::RemoveTools,
                });
            }
            graph_names.retain(|name| {
                !self.config.policy.tools.denied_tools.contains(name)
                    && self
                        .config
                        .policy
                        .tools
                        .allowed_tools
                        .as_ref()
                        .is_none_or(|allowed| allowed.contains(name))
            });
            policy.tools.allowed_tools = Some(graph_names);
            let available: Vec<_> = agent
                .handoffs
                .iter()
                .filter(|h| policy.tools.decision(&h.definition) != adk_core::ToolDecision::Deny)
                .map(|h| format!("- {}: {}", h.definition.name, h.definition.description))
                .collect();
            if !available.is_empty() {
                if !agent.instructions.is_empty() {
                    agent.instructions.push_str("\n\n");
                }
                agent.instructions.push_str("Delegate by transferring the conversation to an available specialist. A transfer gives that specialist ownership of the response; it does not run a nested task or return control.\n");
                agent.instructions.push_str(&available.join("\n"));
            }
            let workspace = if self.config.features.is_some() {
                workspace::selected_workspace_context(
                    &workspace_dir,
                    policy.tools.access,
                    &agent
                        .tools
                        .iter()
                        .map(|t| t.definition().name.clone())
                        .collect::<Vec<_>>(),
                )
            } else {
                workspace_context(&workspace_dir, policy.tools.access)
            };
            if !workspace.is_empty() {
                if !agent.instructions.is_empty() {
                    agent.instructions.push_str("\n\n");
                }
                agent.instructions.push_str(&workspace);
            }
            Ok(specialists)
        })();
        let specialists = match composed {
            Ok(specialists) => specialists,
            Err(error) => {
                tools.close().await.map_err(|e| {
                    Error::new(ErrorCategory::Host, "tool teardown failed").with_source(e)
                })?;
                return Err(error);
            }
        };
        let summary = if self.config.feature_summary.trim().is_empty() {
            String::new()
        } else {
            format!("Runtime surface: {}", self.config.feature_summary)
        };
        let additional = [
            summary.as_str(),
            &self.config.mode_directive_text,
            &self.config.final_check_instructions,
        ]
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_owned();
        if !additional.is_empty() {
            self.runner.additional_instructions = additional;
        }
        if self.runner.working_state_context.trim().is_empty() {
            self.runner.working_state_context =
                "Runtime state is maintained by the host adapter.".into();
        }
        self.runner.work_dir = self.config.work_dir.clone();
        self.runner.output.work_dir = Some(self.config.work_dir.clone());
        self.runner.output.untrusted = features.untrusted_tool_outputs;
        self.runner.approve_mutating_tools = features.approval;
        if features.builtin_guardrails {
            let mut input = crate::guardrails::builtin_tool_input_guardrails();
            input.append(&mut self.runner.tool_input_guardrails);
            self.runner.tool_input_guardrails = input;
            let mut output = crate::guardrails::builtin_tool_output_guardrails();
            output.append(&mut self.runner.tool_output_guardrails);
            self.runner.tool_output_guardrails = output;
        }
        self.runner.force_final_summary_turn = features.force_final_summary_turn;
        if self.config.features.is_some() && !features.immediate_input_polling {
            self.runner.immediate_input_poller = None;
            self.runner.immediate_input_signal = None;
            self.runner.immediate_input_finalizer = None;
        }
        if !features.retry {
            self.runner.retry.max_retries = 0;
        } else if self.runner.retry.max_retries == 0
            && mode
                .as_ref()
                .and_then(|m| m.constraints.as_ref())
                .and_then(|c| c.max_retries)
                .is_none()
        {
            self.runner.retry.max_retries = 3;
            self.runner.retry.initial_delay = Duration::from_millis(250);
            self.runner.retry.max_delay = Duration::from_millis(2000);
        }
        self.runner.local_compaction.enabled = features.compaction;
        if !features.compaction {
            self.runner.compaction = None;
        }
        if self.runner.cost_estimator.is_none() {
            self.runner.cost_estimator =
                Some(Arc::new(adk_providers::runtime::BaselineCosts(routes)));
        }
        let runner = match Runner::new(agent.clone(), std::mem::take(&mut self.runner)) {
            Ok(runner) => runner,
            Err(error) => {
                tools.close().await.map_err(|e| {
                    Error::new(ErrorCategory::Host, "tool teardown failed").with_source(e)
                })?;
                return Err(error);
            }
        };
        Ok(Bundle {
            runner,
            agent,
            specialists,
            policy,
            tools,
            session,
            owned_session: self.owned_session.take(),
            #[cfg(feature = "mcp")]
            owned_mcp: self.owned_mcp.take(),
            #[cfg(feature = "project-state")]
            owned_project_state: self.owned_project_state.take(),
            warnings: std::mem::take(&mut self.warnings),
        })
    }
}

fn sanitize_handoff_name(name: &str) -> String {
    let name: String = name
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            'a'..='z' | '0'..='9' => Some(c),
            ' ' | '-' | '_' | '.' => Some('_'),
            _ => None,
        })
        .collect();
    let name = name.trim_matches('_');
    if name.is_empty() {
        "specialist".into()
    } else {
        name.into()
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_routing(
    model: &mut String,
    fallbacks: &mut Vec<String>,
    settings: &mut Map<String, Value>,
    selected: &str,
    selected_fallbacks: &Option<Vec<String>>,
    reasoning: &str,
    verbosity: &str,
    overrides: &Map<String, Value>,
) {
    if !selected.trim().is_empty() {
        *model = selected.trim().into();
    }
    if let Some(selected) = selected_fallbacks {
        *fallbacks = selected
            .iter()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect();
    }
    settings.extend(adk_runtime::settings::routing_settings(
        reasoning, verbosity,
    ));
    settings.extend(overrides.clone());
}

/// Keep alive for runs, streams and continuations; close before executor shutdown.
pub struct Bundle {
    #[cfg(feature = "project-state")]
    owned_project_state: Option<project_state::Owner>,
    warnings: Vec<String>,
    runner: Runner,
    agent: AgentConfig,
    specialists: BTreeMap<String, Arc<AgentConfig>>,
    policy: RunPolicy,
    tools: ToolBundle,
    session: SessionHandle,
    owned_session: Option<SessionState>,
    #[cfg(feature = "mcp")]
    owned_mcp: Option<adk_mcp::session::OwnedMcpSession>,
}
impl Bundle {
    /// Sanitized nonfatal construction diagnostics; raw store errors are never exposed.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    #[cfg(feature = "mcp")]
    pub fn mcp_catalog(&self) -> Vec<adk_mcp::client::CatalogEntry> {
        self.owned_mcp
            .as_ref()
            .map_or_else(Vec::new, |mcp| mcp.catalog())
    }

    #[cfg(feature = "mcp")]
    pub fn mcp_servers(&self) -> BTreeMap<String, adk_mcp::client::Capabilities> {
        self.owned_mcp
            .as_ref()
            .map_or_else(BTreeMap::new, |mcp| mcp.connected_servers().clone())
    }
    pub fn agent(&self) -> &AgentConfig {
        &self.agent
    }
    pub fn specialists(&self) -> &BTreeMap<String, Arc<AgentConfig>> {
        &self.specialists
    }
    pub fn policy(&self) -> &RunPolicy {
        &self.policy
    }
    pub fn session(&self) -> SessionHandle {
        self.session.clone()
    }
    pub async fn run(
        &self,
        context: Context,
        input: Vec<RunItem>,
        host: Arc<dyn Host>,
    ) -> Result<RunOutcome, RunError> {
        self.run_with_provenance(context, input, Vec::new(), host)
            .await
    }
    /// Preserve explicit historical authorship. An empty sidecar means Unknown;
    /// otherwise provide one entry per item, independent of message role.
    pub async fn run_with_provenance(
        &self,
        context: Context,
        input: Vec<RunItem>,
        input_provenance: Vec<ItemProvenance>,
        host: Arc<dyn Host>,
    ) -> Result<RunOutcome, RunError> {
        self.runner
            .run(
                self.tools.context(&self.session.context(&context)),
                RunRequest {
                    input_provenance,
                    input,
                    policy: self.policy.clone(),
                },
                host,
            )
            .await
    }
    pub fn stream(&self, context: Context, input: Vec<RunItem>, host: Arc<dyn Host>) -> RunStream {
        self.stream_with_provenance(context, input, Vec::new(), host)
    }
    /// Streaming counterpart of [`Self::run_with_provenance`].
    pub fn stream_with_provenance(
        &self,
        context: Context,
        input: Vec<RunItem>,
        input_provenance: Vec<ItemProvenance>,
        host: Arc<dyn Host>,
    ) -> RunStream {
        self.runner.stream(
            self.tools.context(&self.session.context(&context)),
            RunRequest {
                input_provenance,
                input,
                policy: self.policy.clone(),
            },
            host,
        )
    }
    pub async fn close(&mut self) -> Result<(), Error> {
        #[cfg(feature = "project-state")]
        if let Some(owner) = &self.owned_project_state {
            owner.begin_close();
        }

        #[cfg(feature = "mcp")]
        if let Some(mcp) = &self.owned_mcp {
            mcp.begin_close();
        }
        let tools =
            self.tools.close().await.map_err(|e| {
                Error::new(ErrorCategory::Host, "tool teardown failed").with_source(e)
            });
        let session = match &mut self.owned_session {
            Some(state) => state.close().await,
            None => Ok(()),
        };
        #[cfg(feature = "mcp")]
        let mcp = match &self.owned_mcp {
            Some(mcp) => mcp.close().await.map_err(|error| {
                Error::new(ErrorCategory::Host, "MCP teardown failed").with_source(error)
            }),
            None => Ok(()),
        };
        #[cfg(feature = "project-state")]
        if let Some(owner) = &self.owned_project_state {
            owner.close().await;
        }
        let result = tools.and(session);
        #[cfg(feature = "mcp")]
        let result = result.and(mcp);
        result
    }
}

/// Provider-free tool composition owner. Close before shutting down the executor.
pub struct ToolRuntime {
    tools: ToolBundle,
    session: Option<SessionHandle>,
    owned_session: Option<SessionState>,
    #[cfg(feature = "mcp")]
    owned_mcp: Option<adk_mcp::session::OwnedMcpSession>,
    #[cfg(feature = "project-state")]
    owned_project_state: Option<project_state::Owner>,
}
impl ToolRuntime {
    pub fn prepared(&self) -> adk_tools::PreparedTools {
        self.tools.prepared()
    }
    pub fn context(&self, context: &Context) -> Context {
        match &self.session {
            Some(session) => self.tools.context(&session.context(context)),
            None => self.tools.context(context),
        }
    }
    #[cfg(feature = "mcp")]
    pub fn mcp_catalog(&self) -> Vec<adk_mcp::client::CatalogEntry> {
        self.owned_mcp
            .as_ref()
            .map_or_else(Vec::new, |mcp| mcp.catalog())
    }
    #[cfg(feature = "mcp")]
    pub fn mcp_servers(&self) -> BTreeMap<String, adk_mcp::client::Capabilities> {
        self.owned_mcp
            .as_ref()
            .map_or_else(BTreeMap::new, |mcp| mcp.connected_servers().clone())
    }
    pub async fn close(&mut self) -> Result<(), Error> {
        #[cfg(feature = "project-state")]
        if let Some(owner) = &self.owned_project_state {
            owner.begin_close();
        }
        #[cfg(feature = "mcp")]
        if let Some(owner) = &self.owned_mcp {
            owner.begin_close();
        }
        let result =
            self.tools.close().await.map_err(|e| {
                Error::new(ErrorCategory::Host, "tool teardown failed").with_source(e)
            });
        let result = result.and(match &mut self.owned_session {
            Some(session) => session.close().await,
            None => Ok(()),
        });
        #[cfg(feature = "project-state")]
        if let Some(owner) = &self.owned_project_state {
            owner.close().await;
        }
        #[cfg(feature = "mcp")]
        let result = result.and(match &self.owned_mcp {
            Some(owner) => owner
                .close()
                .await
                .map_err(|e| Error::new(ErrorCategory::Host, "MCP teardown failed").with_source(e)),
            None => Ok(()),
        });
        result
    }
}
