//! Host-neutral composition of native providers, tools and owned runtime lifetimes.
use adk_core::{
    AccessMode, BoxFuture, Cancellation, Context, Error, ErrorCategory, Host, RunError, RunItem,
    RunPolicy, RunRequest, StreamingModel, Tool, ToolPolicy,
};
use adk_providers::{
    auth::{CredentialStore, Refresh},
    factory::{Kind, RouteSpec, default_route},
    routing::Routes,
};
use adk_runtime::{
    AgentConfig, CancellationToken, ModelBinding, RunOutcome, RunStream, Runner, RunnerConfig,
    SubagentSession, subagent::Scheduler,
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
    pub parallel_tool_calls: bool,
    pub untrusted_tool_outputs: bool,
    /// Requires an explicitly supplied session with an owned scheduler.
    pub subagents: bool,
}

#[derive(Clone)]
pub struct Config {
    pub provider: Option<Kind>,
    pub default_provider: Option<String>,
    pub model: String,
    pub fallback_models: Vec<String>,
    pub agent_name: String,
    pub instructions: String,
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
}
impl Default for FileConfigSource {
    fn default() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        Self {
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
            root: root.into(),
            trusted_root: true,
        }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn load_files(&self, context: &Context) -> Result<HostConfig, Error> {
        context.check_active()?;
        if !self.trusted_root {
            return Err(invalid(
                "default configuration source requires an absolute HOME",
            ));
        }
        let mut modes: BTreeMap<String, ModeSpec> = builtin_modes()
            .into_iter()
            .map(|m| (m.name.clone(), m))
            .collect();
        let mut seen = BTreeSet::new();
        for path in config_files(&self.root.join("modes"), &["yaml", "yml", "json"])? {
            context.check_active()?;
            let raw = read_config(&path)?;
            let value: serde_yaml::Value =
                serde_yaml::from_str(&raw).map_err(|e| config_error(&path, e))?;
            let (spec, name) = match value.get("spec") {
                Some(spec) => (
                    spec.clone(),
                    value
                        .get("metadata")
                        .and_then(|v| v.get("name"))
                        .and_then(|v| v.as_str())
                        .map(str::to_owned),
                ),
                None => (value, None),
            };
            let mut mode: ModeSpec =
                serde_yaml::from_value(spec).map_err(|e| config_error(&path, e))?;
            if mode.name.trim().is_empty() {
                mode.name = name.unwrap_or_else(|| file_stem(&path));
            }
            normalize_mode(&mut mode)?;
            let key = mode.name.to_lowercase();
            if !seen.insert(key.clone()) {
                return Err(invalid(format!("duplicate mode: {}", mode.name)));
            }
            modes.insert(key, mode);
        }
        let mut roles = BTreeMap::new();
        for path in config_files(&self.root.join("agents"), &["md"])? {
            context.check_active()?;
            let raw = read_config(&path)?.replace("\r\n", "\n");
            let (mut role, body) = if let Some(rest) = raw.strip_prefix("---\n") {
                let (front, body) = rest.split_once("\n---\n").ok_or_else(|| {
                    invalid(format!("{}: unterminated role frontmatter", path.display()))
                })?;
                (
                    serde_yaml::from_str::<RoleSpec>(front).map_err(|e| config_error(&path, e))?,
                    body,
                )
            } else {
                (RoleSpec::default(), raw.as_str())
            };
            if role.name.trim().is_empty() {
                role.name = file_stem(&path);
            }
            role.name = role.name.trim().into();
            role.instructions = body.trim().into();
            validate_name(&role.name)?;
            parse_access(&role.tool_access)?;
            if role.instructions.is_empty() {
                return Err(invalid(format!(
                    "{}: role instructions are empty",
                    path.display()
                )));
            }
            if roles.insert(role.name.clone(), role).is_some() {
                return Err(invalid("duplicate role name"));
            }
        }
        Ok(HostConfig {
            modes: modes.into_values().collect(),
            roles: roles.into_values().collect(),
        })
    }
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
    fn context(&self, context: &Context) -> Context {
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
    implementations: Vec<Arc<dyn Tool>>,
    extra_tools: Vec<Arc<dyn Tool>>,
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
            implementations: vec![],
            extra_tools: vec![],
            shell: None,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            lsp: None,
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            browser: None,
        }
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

    pub async fn build(mut self, context: &Context) -> Result<Bundle, Error> {
        let result = self.assemble(context).await;
        if result.is_err()
            && let Some(session) = &mut self.owned_session
        {
            session.close().await?;
        }
        result
    }
    async fn assemble(&mut self, context: &Context) -> Result<Bundle, Error> {
        context.check_active()?;
        let host = match &self.source {
            Some(source) => source.load(context).await?,
            None => HostConfig::default(),
        };
        context.check_active()?;
        let features = self.config.resolved_features();
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
        let mut roles: BTreeMap<_, _> = host
            .roles
            .into_iter()
            .map(|r| (r.name.clone(), r))
            .collect();
        for role in &self.config.roles {
            roles.insert(role.name.clone(), role.clone());
        }
        let role = self
            .config
            .active_role
            .as_ref()
            .map(|name| {
                roles
                    .get(name)
                    .ok_or_else(|| invalid("active role not found"))
            })
            .transpose()?;
        let mut model = self.config.model.trim().to_owned();
        let mut fallbacks = self.config.fallback_models.clone();
        let mut settings = self.config.settings.clone();
        let mut policy = self.config.policy.clone();
        let mut instructions = vec![self.config.instructions.trim().to_owned()];
        if let Some(mode) = &mode {
            policy.tools.access =
                narrow_access(policy.tools.access, parse_access(&mode.tool_access)?);
            if features.mode_instructions {
                instructions.push(format!(
                    "Mode: {}",
                    if mode.display_name.is_empty() {
                        &mode.name
                    } else {
                        &mode.display_name
                    }
                ));
                instructions.push(mode.instructions.trim().into());
            }
            if let Some(constraints) = &mode.constraints {
                if let Some(limit) = constraints.max_turns {
                    policy.max_turns = policy.max_turns.min(limit);
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
        let mut tool_config = self.config.tool_options.clone();
        tool_config.features = match &self.config.features {
            Some(_) => adk_tools::Features::Strict(features.tools.clone()),
            None => adk_tools::Features::Legacy(self.config.legacy_tools.clone()),
        };
        tool_config.access = policy.tools.access;
        tool_config.allowed_mutating_tools = policy.tools.allowed_mutating_tools.clone();
        if features.subagents {
            let children = session
                .subagents
                .clone()
                .ok_or_else(|| invalid("subagents require a session-owned scheduler"))?;
            self.runner.subagents = Some(children.clone());
            // Managed tools use ExtraTools registration, but cannot enable other extensions.
            let mut managed = adk_runtime::build_subagent_task_tools(children, agent.name.clone());
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
        let mut tool_builder = BundleBuilder::new(tool_config)
            .implementations(std::mem::take(&mut self.implementations))
            .extra_tools(std::mem::take(&mut self.extra_tools));
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
        let mut tools = tool_builder
            .build(policy.tools.clone())
            .map_err(|e| invalid("tool bundle construction failed").with_source(e))?;
        let prepared = tools.prepared();
        policy.tools = prepared.policy;
        agent.tools = prepared.tools;
        self.runner.work_dir = self.config.work_dir.clone();
        self.runner.output.work_dir = Some(self.config.work_dir.clone());
        self.runner.output.untrusted = features.untrusted_tool_outputs;
        self.runner.approve_mutating_tools = features.approval;
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
            policy,
            tools,
            session,
            owned_session: self.owned_session.take(),
        })
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
    if !reasoning.trim().is_empty() {
        settings.insert("reasoning_effort".into(), reasoning.trim().into());
    }
    if !verbosity.trim().is_empty() {
        settings.insert("text_verbosity".into(), verbosity.trim().into());
    }
    settings.extend(overrides.clone());
}

/// Keep alive for runs, streams and continuations; close before executor shutdown.
pub struct Bundle {
    runner: Runner,
    agent: AgentConfig,
    policy: RunPolicy,
    tools: ToolBundle,
    session: SessionHandle,
    owned_session: Option<SessionState>,
}
impl Bundle {
    pub fn agent(&self) -> &AgentConfig {
        &self.agent
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
        self.runner
            .run(
                self.tools.context(&self.session.context(&context)),
                RunRequest {
                    input,
                    policy: self.policy.clone(),
                },
                host,
            )
            .await
    }
    pub fn stream(&self, context: Context, input: Vec<RunItem>, host: Arc<dyn Host>) -> RunStream {
        self.runner.stream(
            self.tools.context(&self.session.context(&context)),
            RunRequest {
                input,
                policy: self.policy.clone(),
            },
            host,
        )
    }
    pub async fn close(&mut self) -> Result<(), Error> {
        let tools =
            self.tools.close().await.map_err(|e| {
                Error::new(ErrorCategory::Host, "tool teardown failed").with_source(e)
            });
        let session = match &mut self.owned_session {
            Some(state) => state.close().await,
            None => Ok(()),
        };
        tools.and(session)
    }
}
