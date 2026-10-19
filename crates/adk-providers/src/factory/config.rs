//! Opt-in declarative assembly; credential persistence and acquisition remain host-owned.
use super::{Kind, RouteSpec, default_route};
use crate::{
    auth::{AuthMode, CredentialStore, Refresh, Scope, Secret},
    routing::Routes,
    wire::Protocol,
};
use adk_core::Error;
use serde::Deserialize;
use std::{collections::BTreeMap, sync::Arc};

/// Supported factory subset. Unknown fields (including OAuth files) are rejected
/// on deserialization. Intentionally not `Debug`.
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub provider: String,
    pub default_provider: String,
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    pub auth_mode: String,
    pub api_mode: String,
    pub openai_oauth_account_id: String,
    pub provider_api_keys: BTreeMap<String, String>,
    pub provider_base_urls: BTreeMap<String, String>,
    pub provider_api_modes: BTreeMap<String, String>,
    pub provider_auth_modes: BTreeMap<String, String>,
    pub model_fallbacks: Vec<String>,
    pub routes: Vec<NamedRoute>,
}

/// Named instances never inherit top-level or canonical settings or credentials.
#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NamedRoute {
    pub prefix: String,
    pub provider: String,
    pub base_url: String,
    pub api_key: String,
    pub auth_mode: String,
    pub api_mode: String,
    pub openai_oauth_account_id: String,
}

pub struct ResolvedRoute {
    pub spec: RouteSpec,
    /// Canonical OpenRouter configuration only; named routes do not inherit it.
    pub model_fallbacks: Vec<String>,
    /// Optional seed for the host store, not an eagerly acquired credential.
    pub api_key: Option<Secret>,
}

pub struct ResolvedConfig {
    pub default: String,
    pub routes: Vec<ResolvedRoute>,
}

const CANONICAL: [Kind; 8] = [
    Kind::OpenAi,
    Kind::Anthropic,
    Kind::OpenRouter,
    Kind::Gemini,
    Kind::Groq,
    Kind::Xai,
    Kind::Local,
    Kind::Copilot,
];

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalized(values: &BTreeMap<String, String>) -> Result<BTreeMap<String, String>, Error> {
    let mut result = BTreeMap::new();
    for (key, value) in values {
        let key = key.parse::<Kind>()?.name().to_owned();
        if result.insert(key, value.trim().to_owned()).is_some() {
            return Err(crate::invalid(
                "ambiguous normalized provider override keys",
            ));
        }
    }
    Ok(result)
}

fn selected<'a>(
    values: &'a BTreeMap<String, String>,
    kind: Kind,
    top: &'a str,
    scoped: bool,
) -> &'a str {
    values
        .get(kind.name())
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(if scoped { top.trim() } else { "" })
}

impl Config {
    pub fn resolve(&self) -> Result<ResolvedConfig, Error> {
        let provider = normalize(&self.provider);
        let multi = provider == "multi" || !self.routes.is_empty();
        let kind = if provider.is_empty() || provider == "multi" {
            Kind::OpenAi
        } else {
            provider.parse::<Kind>()?
        };
        // Go spreads these values across canonical legs when Provider is empty.
        // Reject ambiguity instead of widening secret scope.
        if provider.is_empty()
            && multi
            && [
                &self.api_key,
                &self.base_url,
                &self.api_mode,
                &self.auth_mode,
            ]
            .iter()
            .any(|value| !value.trim().is_empty())
        {
            return Err(crate::invalid(
                "multi configuration with top-level settings requires an explicit provider",
            ));
        }
        let default = if multi {
            default_route(Some(&self.default_provider), &self.model, Some(kind))
        } else {
            kind.name().to_owned()
        };
        let keys = normalized(&self.provider_api_keys)?;
        let urls = normalized(&self.provider_base_urls)?;
        let modes = normalized(&self.provider_api_modes)?;
        let auth = normalized(&self.provider_auth_modes)?;
        for (provider, mode) in &auth {
            auth_mode(provider.parse()?, mode)?;
        }
        for (provider, mode) in &modes {
            api_mode(provider.parse()?, mode)?;
        }
        let mut routes = BTreeMap::new();
        for kind in if multi {
            CANONICAL.as_slice()
        } else {
            std::slice::from_ref(&kind)
        } {
            let scoped = provider.is_empty()
                || provider == kind.name()
                || (provider == "multi" && default == kind.name());
            let auth_scoped = provider == kind.name() || default == kind.name();
            let mut route = resolve_route(
                *kind,
                kind.name(),
                selected(&urls, *kind, &self.base_url, scoped),
                selected(&keys, *kind, &self.api_key, scoped),
                selected(&auth, *kind, &self.auth_mode, auth_scoped),
                selected(&modes, *kind, &self.api_mode, scoped),
                &self.openai_oauth_account_id,
            )?;
            if *kind == Kind::OpenRouter {
                route.model_fallbacks = self.model_fallbacks.clone();
            }
            routes.insert(kind.name().to_owned(), route);
        }
        for route in &self.routes {
            if route.provider.trim().is_empty() || normalize(&route.provider) == "multi" {
                return Err(crate::invalid("named route requires a non-multi provider"));
            }
            let kind = route.provider.parse::<Kind>()?;
            let prefix = if route.prefix.trim().is_empty() {
                kind.name().to_owned()
            } else {
                normalize(&route.prefix)
            };
            let resolved = resolve_route(
                kind,
                &prefix,
                route.base_url.trim(),
                route.api_key.trim(),
                route.auth_mode.trim(),
                route.api_mode.trim(),
                &route.openai_oauth_account_id,
            )?;
            routes.insert(prefix, resolved);
        }
        Ok(ResolvedConfig {
            default,
            routes: routes.into_values().collect(),
        })
    }
}

fn auth_mode(kind: Kind, value: &str) -> Result<AuthMode, Error> {
    match normalize(value).as_str() {
        "" | "api-key" | "api_key" | "oauth" if kind == Kind::Copilot => Ok(AuthMode::CopilotOAuth),
        "" | "api-key" | "api_key" => Ok(AuthMode::ApiKey),
        "oauth" if kind == Kind::Anthropic => Ok(AuthMode::AnthropicOAuth),
        "oauth" if kind == Kind::OpenAi => Ok(AuthMode::OpenAiOAuth),
        _ => Err(crate::invalid(
            "unsupported provider authentication configuration",
        )),
    }
}

fn api_mode(kind: Kind, value: &str) -> Result<Protocol, Error> {
    match normalize(value).as_str() {
        "" => Ok(kind.protocol()),
        "auto" | "responses" if !matches!(kind, Kind::Anthropic | Kind::Copilot) => {
            Ok(Protocol::Responses)
        }
        "chat-completions" if !matches!(kind, Kind::Anthropic | Kind::Copilot) => {
            Ok(Protocol::Chat)
        }
        _ => Err(crate::invalid("unsupported provider API mode")),
    }
}

fn resolve_route(
    kind: Kind,
    prefix: &str,
    endpoint: &str,
    key: &str,
    auth: &str,
    api: &str,
    account: &str,
) -> Result<ResolvedRoute, Error> {
    let mode = auth_mode(kind, auth)?;
    let protocol = api_mode(kind, api)?;
    if mode == AuthMode::OpenAiOAuth && (!key.is_empty() || protocol != Protocol::Responses) {
        return Err(crate::invalid(
            "OpenAI OAuth requires host-owned credentials and Responses protocol",
        ));
    }
    let mut spec = RouteSpec::new(kind, mode);
    if mode == AuthMode::OpenAiOAuth && !account.trim().is_empty() {
        spec.account = Some(account.trim().to_owned());
    }
    spec.prefix = Some(prefix.to_owned());
    spec.protocol = (kind != Kind::Copilot).then_some(protocol);
    spec.endpoint = Some(
        if endpoint.is_empty()
            || (kind == Kind::OpenAi && endpoint == Kind::OpenAi.endpoint(AuthMode::OpenAiOAuth))
        {
            kind.endpoint(mode).to_owned()
        } else {
            endpoint.to_owned()
        },
    );
    spec.scope()?;
    let key = if key.is_empty() && kind == Kind::Local {
        "local-key"
    } else {
        key
    };
    Ok(ResolvedRoute {
        spec,
        model_fallbacks: Vec::new(),
        api_key: (!key.is_empty()).then(|| Secret::new(key)),
    })
}

impl ResolvedConfig {
    /// Bind each final route to a host-owned store. `bind` must not acquire
    /// credentials: it receives a validated full scope and optional explicit seed.
    /// Missing seeds are allowed; only a request to that scope loads its store.
    pub fn build(
        &self,
        mut bind: impl FnMut(&Scope, Option<&Secret>) -> Result<Arc<dyn CredentialStore>, Error>,
        refresh: Arc<dyn Refresh>,
    ) -> Result<Routes, Error> {
        let mut routes = Routes::new(&self.default);
        for route in &self.routes {
            let scope = route.spec.scope()?;
            let store = bind(&scope, route.api_key.as_ref())?;
            routes.register_spec_with_model_fallbacks(
                &route.spec,
                store,
                refresh.clone(),
                &route.model_fallbacks,
            )?;
        }
        Ok(routes)
    }
}
