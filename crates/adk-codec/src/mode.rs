use crate::dto::{ModeRouting, ModeTemplateSpec};

pub fn mode_routing_from_template_spec(spec: Option<&ModeTemplateSpec>) -> Option<ModeRouting> {
    let mut routing = spec?.model_routing.as_ref()?.clone();
    // The Go adapter appends into nil slices and allocates only nonempty maps.
    routing.fallback_models = routing.fallback_models.filter(|models| !models.is_empty());
    routing.role_overrides = routing.role_overrides.filter(|roles| !roles.is_empty());
    if let Some(roles) = &mut routing.role_overrides {
        for role in roles.values_mut() {
            role.fallback_models = role
                .fallback_models
                .take()
                .filter(|models| !models.is_empty());
        }
    }
    Some(routing)
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "PascalCase")]
pub struct ModeRoutingResolution {
    pub model: String,
    pub fallback_models: Option<Vec<String>>,
    pub reasoning_level: String,
    pub text_verbosity: String,
    pub model_settings: crate::dto::ModelSettings,
}

pub fn resolve_mode_routing(
    base_model: &str,
    provider: &str,
    routing: Option<&ModeRouting>,
) -> ModeRoutingResolution {
    let model = routing
        .map(|routing| routing.default_model.trim())
        .filter(|model| !model.is_empty())
        .unwrap_or(base_model);
    let mut result = ModeRoutingResolution {
        model: resolve_model_for_provider(model, provider),
        ..Default::default()
    };
    if let Some(routing) = routing {
        result.fallback_models = resolve_fallbacks(routing.fallback_models.as_deref(), provider);
        apply_behavior(
            &mut result,
            &routing.reasoning_level,
            &routing.text_verbosity,
        );
    }
    result
}

pub fn resolve_role_mode_routing(
    base_model: &str,
    provider: &str,
    role_name: &str,
    routing: Option<&ModeRouting>,
) -> ModeRoutingResolution {
    let mut result = resolve_mode_routing(base_model, provider, routing);
    if let Some(role) = routing
        .and_then(|routing| routing.role_overrides.as_ref())
        .and_then(|roles| roles.get(role_name))
    {
        if !role.model.trim().is_empty() {
            result.model = resolve_model_for_provider(&role.model, provider);
        }
        if role
            .fallback_models
            .as_ref()
            .is_some_and(|models| !models.is_empty())
        {
            result.fallback_models = resolve_fallbacks(role.fallback_models.as_deref(), provider);
        }
        apply_behavior(&mut result, &role.reasoning_level, &role.text_verbosity);
    }
    result
}

fn resolve_fallbacks(models: Option<&[String]>, provider: &str) -> Option<Vec<String>> {
    let models: Vec<_> = models
        .unwrap_or_default()
        .iter()
        .map(|model| model.trim())
        .filter(|model| !model.is_empty())
        .map(|model| resolve_model_for_provider(model, provider))
        .collect();
    (!models.is_empty()).then_some(models)
}

fn lowercase(value: &str) -> String {
    // Go uses simple lowercase rather than expanding Unicode mappings.
    value
        .trim()
        .chars()
        .map(|c| c.to_lowercase().next().unwrap())
        .collect()
}

pub fn resolve_model_for_provider(model: &str, provider: &str) -> String {
    match (lowercase(provider).as_str(), lowercase(model).as_str()) {
        ("anthropic", "" | "medium") => "claude-sonnet-4-6",
        ("anthropic", "small") => "claude-haiku-4-5",
        ("anthropic", "large") => "claude-opus-4-6",
        ("" | "openai", "small") => "gpt-5.6-luna",
        ("" | "openai", "medium") => "gpt-5.6-terra",
        ("" | "openai", "large") => "gpt-5.6-sol",
        _ => model.trim(),
    }
    .into()
}

fn apply_behavior(result: &mut ModeRoutingResolution, reasoning: &str, verbosity: &str) {
    let reasoning = lowercase(reasoning);
    let budget = match reasoning.as_str() {
        "none" => Some(0),
        "minimal" => Some(1024),
        "low" => Some(2048),
        "medium" => Some(4096),
        "high" => Some(8192),
        "xhigh" => Some(16384),
        "max" => Some(24576),
        _ => None,
    };
    if let Some(budget) = budget {
        result.model_settings.thinking_budget = budget;
        result
            .model_settings
            .reasoning_effort
            .clone_from(&reasoning);
        result.reasoning_level = reasoning;
    }
    let verbosity = lowercase(verbosity);
    if matches!(verbosity.as_str(), "low" | "medium" | "high") {
        result.model_settings.text_verbosity.clone_from(&verbosity);
        result.text_verbosity = verbosity;
    }
}
