//! Pinned SDK tool contracts and fail-closed registry composition.
//!
//! This crate does not yet implement every catalogued tool. Selection is separate
//! from construction: missing selected implementations are construction errors,
//! never model-visible placeholders. See docs/tools.md for the parity boundary.
use adk_core::{AccessMode, Tool, ToolDefinition};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

pub mod memory;
pub mod plan;
mod search;
mod search_pattern;
pub mod signal;
mod workspace;

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
                && (c.read_only
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

impl Registry {
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
            if capability.definition.is_none() {
                missing.push(capability.name.clone());
                continue;
            }
            let implementation = supplied
                .remove(&capability.name)
                .or_else(|| signal::builtin(capability))
                .or_else(|| search::builtin(capability));
            let Some(tool) = implementation else {
                if capability.classification != "host-only" {
                    missing.push(capability.name.clone());
                }
                continue;
            };
            if tool.definition().read_only != capability.read_only
                || tool.definition().requires_approval
                || capability
                    .definition
                    .as_ref()
                    .is_some_and(|definition| definition != tool.definition())
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
