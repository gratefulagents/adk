//! Pinned SDK tool contracts and fail-closed registry composition.
//!
//! Selection is separate from construction: missing selected runtime dependencies
//! are construction errors, never model-visible placeholders. Trusted hosts supply
//! stores, executables and external-service adapters. See docs/tools.md for verification.
use adk_core::{AccessMode, Tool, ToolDecision, ToolDefinition, ToolPolicy};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod browser;
pub mod bundle;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod edit;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod edit_diff;
pub mod git;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod git_host;
mod html;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod lifecycle;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod lsp;
pub mod memory;
mod network;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod patch;
pub mod plan;
mod search;
mod search_pattern;
pub mod shell;
pub mod signal;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod skills;
pub mod vision;
mod web;
mod workspace;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod write;

fn json_text(value: &impl serde::Serialize) -> Result<String, serde_json::Error> {
    Ok(serde_json::to_string(value)?
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029"))
}

#[derive(Debug, Clone, Deserialize)]
pub struct Capability {
    pub name: String,
    pub family: String,
    pub feature: String,
    pub mode: String,
    pub classification: String,
    pub legacy: bool,
    pub read_only: bool,
    pub control_flow: bool,
    pub writes_git_remote: bool,
    /// None denotes an environment-dependent Bash schema, not an empty schema.
    pub definition: Option<ToolDefinition>,
    pub source_type: String,
    pub acceptance_id: String,
}

pub fn capabilities() -> &'static [Capability] {
    static CATALOG: OnceLock<Vec<Capability>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("manifest.json")).expect("pinned tool manifest")
    })
}

#[derive(Debug, Clone, Default)]
pub struct LegacyFeatures {
    pub enable_tools: bool,
    pub enable_subagents: bool,
    pub disable_default_tools: bool,
    pub disable_signal_tools: bool,
    pub disable_web_tools: bool,
    pub enable_async_shell: bool,
    pub enable_project_state: bool,
}

#[derive(Debug, Clone)]
pub enum Features {
    /// Exact documented feature paths; empty means no tools.
    Strict(BTreeSet<String>),
    Legacy(LegacyFeatures),
}

impl Default for Features {
    fn default() -> Self {
        Self::Strict(BTreeSet::new())
    }
}

impl Features {
    fn enabled(&self, capability: &Capability) -> bool {
        match self {
            Self::Strict(features) => features.contains(&capability.feature),
            Self::Legacy(config) => {
                let tools = config.enable_tools && !config.disable_default_tools;
                match capability.feature.as_str() {
                    "ExtraTools" => config.enable_tools || config.enable_subagents,
                    "Signals.AskUserQuestion" | "Signals.PresentPlan" | "Signals.Finish" => {
                        (config.enable_tools || config.enable_subagents)
                            && !config.disable_signal_tools
                    }
                    "ProjectState.TaskTools"
                    | "ProjectState.MemoryTools"
                    | "ProjectState.PrimeTool" => config.enable_project_state,
                    "AsyncShell" => tools && config.enable_async_shell,
                    "WebFetch" => tools && !config.disable_web_tools,
                    _ => tools && capability.legacy,
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub features: Features,
    pub access: AccessMode,
    pub allowed_mutating_tools: BTreeSet<String>,
    pub allowed_names: Option<BTreeSet<String>>,
    pub git_remote_writes: bool,
    pub allow_private_network_urls: bool,
    /// Trusted values used to freeze environment-dependent shell contracts.
    pub shell_environment: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BuildError {
    #[error("unknown tool feature: {0}")]
    UnknownFeature(String),
    #[error("duplicate tool implementation: {0}")]
    Duplicate(String),
    #[error("selected implementations unavailable: {0:?}")]
    Unavailable(Vec<String>),
    #[error("implementation does not match the selected SDK contract: {0}")]
    Contract(String),
    #[error("tool is not in the SDK built-in catalog: {0}")]
    UnknownTool(String),
}

/// Compute the deterministic eligible contract matrix without constructing resources.
/// Host-only entries describe possible injections; selection never invents stores.
pub fn select(config: &Config) -> Result<Vec<&'static Capability>, BuildError> {
    if let Features::Strict(features) = &config.features {
        for feature in features {
            if !capabilities().iter().any(|c| c.feature == *feature) {
                return Err(BuildError::UnknownFeature(feature.clone()));
            }
        }
    }
    Ok(capabilities()
        .iter()
        .filter(|c| {
            if !config.features.enabled(c)
                || config
                    .allowed_names
                    .as_ref()
                    .is_some_and(|names| !names.contains(&c.name))
                || (c.writes_git_remote && !config.git_remote_writes)
                || (c.name == "Browser" && !config.allow_private_network_urls)
                || (c.family == "interactive-terminal" && config.access != AccessMode::FullAccess)
                || (c.family == "async-shell" && config.access == AccessMode::ReadOnly)
                || (c.family == "workspace-filesystem" && config.access == AccessMode::ReadOnly)
            {
                return false;
            }
            let mode_matches = match c.mode.as_str() {
                "any" => true,
                "read_only" => config.access == AccessMode::ReadOnly,
                "workspace_write" => config.access == AccessMode::WorkspaceWrite,
                "full_access" => config.access == AccessMode::FullAccess,
                "write" => config.access != AccessMode::ReadOnly,
                _ => false,
            };
            mode_matches
                && (c.classification == "host-only"
                    || c.family == "project-state"
                    || c.read_only
                    || c.control_flow
                    || config.access != AccessMode::ReadOnly
                    || config.allowed_mutating_tools.contains(&c.name))
        })
        .collect())
}

/// A registry contains only real implementations with matching model contracts.
/// The caller owns execution authorization and resource lifetimes of injected tools.
pub struct Registry {
    tools: BTreeMap<String, Arc<dyn Tool>>,
}

/// A model-visible tool list and its matching dispatch policy. Registration alone
/// never authorizes a mutable state or host-supplied tool in read-only mode.
pub struct PreparedTools {
    pub tools: Vec<Arc<dyn Tool>>,
    pub policy: ToolPolicy,
}

impl Registry {
    /// Compose explicitly host-supplied extensions after the canonical built-ins.
    /// Extensions are enabled only by ExtraTools (or the legacy tool/subagent
    /// switch), and never bypass model preparation or dispatch authorization.
    pub fn build_with_extra_tools(
        config: &Config,
        implementations: impl IntoIterator<Item = Arc<dyn Tool>>,
        extra_tools: impl IntoIterator<Item = Arc<dyn Tool>>,
    ) -> Result<Self, BuildError> {
        let mut registry = Self::build(config, implementations)?;
        let enabled = match &config.features {
            Features::Strict(features) => features.contains("ExtraTools"),
            Features::Legacy(features) => features.enable_tools || features.enable_subagents,
        };
        if enabled {
            for tool in extra_tools {
                let name = tool.definition().name.clone();
                if config
                    .allowed_names
                    .as_ref()
                    .is_some_and(|names| !names.contains(&name))
                {
                    continue;
                }
                if registry.tools.contains_key(&name) {
                    return Err(BuildError::Duplicate(name));
                }
                registry.tools.insert(name, tool);
            }
        }
        Ok(registry)
    }

    pub fn prepare(&self, mut policy: ToolPolicy) -> PreparedTools {
        for tool in self.tools.values() {
            if tool.is_control_flow() {
                policy
                    .allowed_mutating_tools
                    .insert(tool.definition().name.clone());
            }
        }
        let tools = self
            .tools
            .values()
            .map(|tool| {
                if policy
                    .allowed_mutating_tools
                    .contains(&tool.definition().name)
                {
                    tool.clone()
                } else {
                    tool.for_access(policy.access)
                        .unwrap_or_else(|| tool.clone())
                }
            })
            .filter(|tool| policy.decision(tool.definition()) != ToolDecision::Deny)
            .collect();
        PreparedTools { tools, policy }
    }
    pub fn build(
        config: &Config,
        implementations: impl IntoIterator<Item = Arc<dyn Tool>>,
    ) -> Result<Self, BuildError> {
        let selected = select(config)?;
        let mut supplied = BTreeMap::new();
        for tool in implementations {
            let name = tool.definition().name.clone();
            if !capabilities().iter().any(|c| c.name == name) {
                return Err(BuildError::UnknownTool(name));
            }
            if supplied.insert(name.clone(), tool).is_some() {
                return Err(BuildError::Duplicate(name));
            }
        }
        let mut tools = BTreeMap::new();
        let mut missing = Vec::new();
        for capability in selected {
            let expected = capability.definition.clone().or_else(|| {
                shell::definition(
                    &capability.name,
                    config.access,
                    &shell::Limits::from_environment(&config.shell_environment),
                )
            });
            let implementation = supplied
                .remove(&capability.name)
                .or_else(|| signal::builtin(capability))
                .or_else(|| search::builtin(capability))
                .or_else(|| web::builtin(capability, config.allow_private_network_urls))
                .or_else(|| vision::builtin(capability, config.allow_private_network_urls));
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            let implementation = implementation
                .or_else(|| lifecycle::builtin(capability))
                .or_else(|| write::builtin(capability))
                .or_else(|| edit::builtin(capability))
                .or_else(|| patch::builtin(capability));
            let Some(tool) = implementation else {
                if capability.classification != "host-only" {
                    missing.push(capability.name.clone());
                }
                continue;
            };
            if tool.definition().read_only != capability.read_only
                || tool.definition().requires_approval
                || expected.as_ref() != Some(tool.definition())
                || tool.is_control_flow() != capability.control_flow
                || tool.timeout().is_some_and(|timeout| !timeout.is_zero())
            {
                return Err(BuildError::Contract(capability.name.clone()));
            }
            tools.insert(capability.name.clone(), tool);
        }
        if !missing.is_empty() {
            return Err(BuildError::Unavailable(missing));
        }
        Ok(Self { tools })
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.tools.keys().map(String::as_str)
    }
    pub fn tools(&self) -> impl Iterator<Item = &Arc<dyn Tool>> {
        self.tools.values()
    }
}

/// Store identity and actor are supplied by the host, never inferred from arguments.
pub fn project_state_tools(
    store: Arc<dyn adk_project_state::Store>,
    actor: &str,
) -> Vec<Arc<dyn Tool>> {
    adk_project_state::tools::tools(store, actor)
}
