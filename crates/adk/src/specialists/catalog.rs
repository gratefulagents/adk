use super::{filter_tools_by_access, strip_signal_tools};
use adk_core::{AccessMode, Error, Tool};
use adk_runtime::{AgentAsTool, AgentConfig, ModelBinding, SubagentSession, settings};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{collections::BTreeMap, sync::Arc};

/// SDK-native role input, independent of the builder's validation and defaults.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct SpecialistRoleSpec {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub tool_access: String,
    pub model_override: String,
    pub fallback_models: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct SpecialistRoleRouting {
    pub model: String,
    pub fallback_models: Vec<String>,
    pub reasoning_level: String,
    pub text_verbosity: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct SpecialistModeRouting {
    pub default_model: String,
    pub fallback_models: Vec<String>,
    pub reasoning_level: String,
    pub text_verbosity: String,
    pub role_overrides: BTreeMap<String, SpecialistRoleRouting>,
}

/// Copies SDK routing labels without validation, normalization, or model binding.
/// Native arbitrary settings maps are not part of this SDK-label conversion.
#[cfg(feature = "builder")]
pub fn mode_routing_from_template_spec(
    spec: Option<&crate::builder::ModeSpec>,
) -> Option<SpecialistModeRouting> {
    let routing = spec?.model_routing.as_ref()?;
    Some(SpecialistModeRouting {
        default_model: routing.default_model.clone(),
        fallback_models: routing.fallback_models.clone().unwrap_or_default(),
        reasoning_level: routing.reasoning_level.clone(),
        text_verbosity: routing.text_verbosity.clone(),
        role_overrides: routing
            .role_overrides
            .iter()
            .map(|(name, role)| {
                (
                    name.clone(),
                    SpecialistRoleRouting {
                        model: role.model.clone(),
                        fallback_models: role.fallback_models.clone().unwrap_or_default(),
                        reasoning_level: role.reasoning_level.clone(),
                        text_verbosity: role.text_verbosity.clone(),
                    },
                )
            })
            .collect(),
    })
}

/// An unbound agent value: model strings are not yet executable providers.
#[derive(Clone, Default)]
pub struct SpecialistAgentSpec {
    pub name: String,
    pub instructions: String,
    pub description: String,
    pub model: String,
    pub fallback_models: Vec<String>,
    pub settings: Map<String, Value>,
    pub tools: Vec<Arc<dyn Tool>>,
}

/// Raw conversion retains accepted names, duplicates and model strings verbatim.
pub fn build_specialists_from_catalog(catalog: &[SpecialistRoleSpec]) -> Vec<SpecialistAgentSpec> {
    catalog
        .iter()
        .filter(|role| !role.name.trim().is_empty())
        .map(|role| SpecialistAgentSpec {
            name: role.name.clone(),
            instructions: role.instructions.clone(),
            description: role.description.clone(),
            model: role.model_override.clone(),
            fallback_models: role.fallback_models.clone(),
            ..Default::default()
        })
        .collect()
}

#[derive(Default)]
pub struct SpecialistCatalogOptions {
    pub tools: Vec<Arc<dyn Tool>>,
    pub base_model: String,
    pub fallback_models: Vec<String>,
    pub provider: String,
    pub base_settings: Map<String, Value>,
    pub routing: Option<SpecialistModeRouting>,
}

/// Pure selection, with first normalized case-sensitive name winning.
/// This does not bind models, register runners, or create a scheduler.
pub fn prepare_specialist_catalog(
    catalog: &[SpecialistRoleSpec],
    options: &SpecialistCatalogOptions,
) -> Vec<SpecialistAgentSpec> {
    let mut seen = std::collections::BTreeSet::new();
    let mut agents = Vec::new();
    for role in catalog {
        let name = role.name.trim();
        if name.is_empty() || !seen.insert(name) {
            continue;
        }
        let mut agent = SpecialistAgentSpec {
            name: name.into(),
            instructions: role.instructions.clone(),
            description: role.description.clone(),
            model: options.base_model.clone(),
            fallback_models: options.fallback_models.clone(),
            settings: options.base_settings.clone(),
            ..Default::default()
        };
        if !role.model_override.trim().is_empty() {
            agent.model = role.model_override.trim().into();
        }
        if !role.fallback_models.is_empty() {
            agent.fallback_models = role.fallback_models.clone();
        }
        if let Some(routing) = &options.routing {
            let mut model = agent.model.trim();
            if !routing.default_model.trim().is_empty() {
                model = routing.default_model.trim();
            }
            let mut fallbacks = &routing.fallback_models;
            let mut routed_settings =
                settings::routing_settings(&routing.reasoning_level, &routing.text_verbosity);
            if let Some(role) = routing.role_overrides.get(name) {
                if !role.model.trim().is_empty() {
                    model = role.model.trim();
                }
                if !role.fallback_models.is_empty() {
                    fallbacks = &role.fallback_models;
                }
                let overrides =
                    settings::routing_settings(&role.reasoning_level, &role.text_verbosity);
                if overrides.get("reasoning_effort") == Some(&Value::from("none")) {
                    // The inner routing merge clears its budget; the outer SDK merge
                    // ignores zero, preserving any caller-supplied base budget.
                    routed_settings.remove("thinking_budget");
                }
                routed_settings.extend(overrides);
            }
            let model = resolve_model(model, &options.provider);
            if !model.trim().is_empty() {
                agent.model = model;
            }
            let fallbacks: Vec<_> = fallbacks
                .iter()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| resolve_model(s, &options.provider))
                .collect();
            if !fallbacks.is_empty() {
                agent.fallback_models = fallbacks;
            }
            agent.settings.extend(routed_settings);
        }
        agent.tools =
            strip_signal_tools(&filter_tools_by_access(&options.tools, &role.tool_access));
        agents.push(agent);
    }
    agents
}

fn resolve_model(model: &str, provider: &str) -> String {
    let model = model.trim();
    match (
        provider
            .trim()
            .chars()
            .map(|c| c.to_lowercase().next().unwrap())
            .collect::<String>()
            .as_str(),
        model
            .chars()
            .map(|c| c.to_lowercase().next().unwrap())
            .collect::<String>()
            .as_str(),
    ) {
        ("anthropic", "" | "medium") => "claude-sonnet-4-6",
        ("anthropic", "small") => "claude-haiku-4-5",
        ("anthropic", "large") => "claude-opus-4-6",
        ("" | "openai", "small") => "gpt-5.6-luna",
        ("" | "openai", "medium") => "gpt-5.6-terra",
        ("" | "openai", "large") => "gpt-5.6-sol",
        _ => model,
    }
    .into()
}

/// Bind prepared strings without adding provider defaults or builder policy.
/// Pass the unmodified output of prepare_specialist_catalog, not the raw helper.
/// The callback must retain the requested model name in its returned binding.
pub fn bind_specialist_catalog(
    prepared: &[SpecialistAgentSpec],
    mut bind: impl FnMut(&str) -> Result<ModelBinding, Error>,
) -> Result<SpecialistCatalog, Error> {
    let mut catalog = SpecialistCatalog::default();
    for spec in prepared {
        let mut agent = AgentConfig::new(&spec.name, bind(&spec.model)?);
        agent.instructions = spec.instructions.clone();
        agent.handoff_description = spec.description.clone();
        agent.fallbacks = spec
            .fallback_models
            .iter()
            .map(|model| bind(model))
            .collect::<Result<_, _>>()?;
        agent.settings = spec.settings.clone();
        agent.tools = spec.tools.clone();
        catalog.order.push(spec.name.clone());
        catalog.agents.insert(spec.name.clone(), Arc::new(agent));
    }
    Ok(catalog)
}

/// Immutable registration snapshots; register these exact agents before attachment.
#[derive(Default)]
pub struct SpecialistCatalog {
    agents: BTreeMap<String, Arc<AgentConfig>>,
    order: Vec<String>,
}

impl SpecialistCatalog {
    pub fn agents(&self) -> &BTreeMap<String, Arc<AgentConfig>> {
        &self.agents
    }

    /// Attach named wrappers to a caller-owned, already registered session.
    /// Name collisions are retained, not rejected or disambiguated.
    pub fn attach_tools(&self, session: Arc<SubagentSession>) -> Vec<Arc<dyn Tool>> {
        self.order
            .iter()
            .map(|name| {
                let agent = &self.agents[name];
                let tool: Arc<dyn Tool> = Arc::new(AgentAsTool::new(
                    wrapper_name(name),
                    &agent.handoff_description,
                    name,
                    session.clone(),
                ));
                if agent.tools.iter().all(|tool| tool.definition().read_only) {
                    tool.for_access(AccessMode::ReadOnly).unwrap()
                } else {
                    tool
                }
            })
            .collect()
    }
}

/// Convenience composition. No session means no filtering, binding or adapters.
/// For registration before attachment, use prepare → bind → attach instead.
pub fn build_specialist_tools_from_catalog(
    catalog: &[SpecialistRoleSpec],
    options: &SpecialistCatalogOptions,
    session: Option<Arc<SubagentSession>>,
    bind: impl FnMut(&str) -> Result<ModelBinding, Error>,
) -> Result<(SpecialistCatalog, Vec<Arc<dyn Tool>>), Error> {
    let Some(session) = session else {
        return Ok((SpecialistCatalog::default(), Vec::new()));
    };
    let catalog = bind_specialist_catalog(&prepare_specialist_catalog(catalog, options), bind)?;
    let tools = catalog.attach_tools(session);
    Ok((catalog, tools))
}

fn wrapper_name(name: &str) -> String {
    let mut result = String::new();
    let mut replaced = false;
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            result.push(c);
            replaced = false;
        } else if !replaced {
            result.push('_');
            replaced = true;
        }
    }
    let result = result.trim_matches(['_', '-']);
    format!("agent_{}", if result.is_empty() { "agent" } else { result })
}
