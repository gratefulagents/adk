//! Host-invoked model discovery metadata; never used on the request hot path.
use crate::{
    auth::{AuthMode, Session},
    error::RequestFailure,
};
use adk_core::{Context, Error, ErrorCategory};
use std::collections::BTreeMap;

pub const DEFAULT_CODEX_CLIENT_VERSION: &str = "0.153.4";

pub fn model_metadata_endpoint(session: &Session) -> Result<reqwest::Url, Error> {
    let scope = session.scope();
    let base = scope
        .endpoint
        .trim_end_matches('/')
        .trim_end_matches("/responses")
        .trim_end_matches("/chat/completions");
    let mut url = reqwest::Url::parse(&format!("{base}/models"))
        .map_err(|_| crate::invalid("invalid model metadata endpoint"))?;
    if scope.mode == AuthMode::OpenAiOAuth
        && url
            .host_str()
            .is_some_and(|host| host == "chatgpt.com" || host.ends_with(".chatgpt.com"))
    {
        url.query_pairs_mut()
            .append_pair("client_version", DEFAULT_CODEX_CLIENT_VERSION);
    }
    Ok(url)
}

/// Explicit, bounded catalog fetch with the session's scoped auth and one OAuth
/// 401 refresh. No catalog or credentials are cached by this API.
pub async fn fetch_model_metadata(
    context: &Context,
    session: &Session,
) -> Result<Vec<ModelMetadata>, Error> {
    context.check_active()?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|_| {
            Error::new(
                ErrorCategory::Provider,
                "cannot initialize metadata transport",
            )
        })?;
    let endpoint = model_metadata_endpoint(session)?;
    for attempt in 0..=1 {
        let material = session.material_for_request(context).await?;
        let headers = crate::auth::headers(session.scope(), &material, false)?;
        let mut response = crate::active(
            context,
            client.get(endpoint.clone()).headers(headers).send(),
        )
        .await?
        .map_err(|error| RequestFailure::transport(&error).into_error())?;
        if response.status().as_u16() == 401
            && attempt == 0
            && matches!(
                session.scope().mode,
                AuthMode::OpenAiOAuth | AuthMode::AnthropicOAuth | AuthMode::CopilotOAuth
            )
        {
            drop(response);
            session.reject(context, &material.access_token).await?;
            continue;
        }
        if !response.status().is_success() {
            return Err(RequestFailure::http(
                response.status().as_u16(),
                response.headers(),
                std::time::SystemTime::now(),
            )
            .into_error());
        }
        let mut body = Vec::new();
        while let Some(chunk) = crate::active(context, response.chunk())
            .await?
            .map_err(|error| RequestFailure::transport(&error).into_error())?
        {
            if body.len().saturating_add(chunk.len()) > 8 * 1024 * 1024 {
                return Err(crate::invalid("model metadata exceeds response limit"));
            }
            body.extend_from_slice(&chunk);
        }
        return parse_model_metadata(&body);
    }
    Err(Error::new(
        ErrorCategory::PermissionDenied,
        "model metadata authentication retry exhausted",
    ))
}

/// Explicit discovery metadata, never fetched or learned during model calls.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(default)]
pub struct ModelMetadata {
    #[serde(alias = "slug")]
    pub id: String,
    pub context_window: Option<u64>,
    pub max_context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub auto_compact_token_limit: Option<u64>,
    pub effective_context_window_percent: Option<u64>,
    pub display_name: String,
    pub description: String,
    pub visibility: String,
    pub priority: Option<i64>,
    pub default_reasoning_level: String,
    #[serde(skip)]
    pub supported_reasoning_levels: Vec<String>,
    #[serde(skip)]
    pub upgrade_model: Option<String>,
}
impl ModelMetadata {
    pub fn hidden(&self) -> bool {
        self.visibility.trim().eq_ignore_ascii_case("hide")
    }
    pub fn resolved_context_window(&self) -> Option<u64> {
        self.context_window
            .filter(|v| *v > 0)
            .or(self.max_context_window.filter(|v| *v > 0))
    }
    /// Baseline compaction trigger and target, in tokens.
    pub fn compaction_defaults(&self) -> Option<(u64, u64)> {
        let mut trigger = self.auto_compact_token_limit.unwrap_or(0);
        let mut target = trigger / 2;
        if let Some(context) = self.resolved_context_window() {
            let limit = (u128::from(context) * 9 / 10) as u64;
            if trigger == 0 || trigger > limit {
                trigger = limit;
            }
            target = context / 2;
        }
        if trigger == 0 {
            return None;
        }
        if target == 0 || target >= trigger {
            target = trigger / 2;
        }
        Some((trigger, target))
    }
}

/// Decode the pinned OpenAI, Codex, or Copilot catalog shape. First duplicate
/// wins case-insensitively; a nonempty Codex catalog takes precedence over data.
/// The host owns fetching, authentication, refresh, and any catalog lifetime.
pub fn parse_model_metadata(body: &[u8]) -> Result<Vec<ModelMetadata>, Error> {
    #[derive(Default, serde::Deserialize)]
    #[serde(default)]
    struct Limits {
        max_context_window_tokens: Option<u64>,
        max_prompt_tokens: Option<u64>,
        max_output_tokens: Option<u64>,
    }
    #[derive(Default, serde::Deserialize)]
    #[serde(default)]
    struct Capabilities {
        limits: Limits,
    }
    #[derive(serde::Deserialize)]
    struct Effort {
        effort: String,
    }
    #[derive(serde::Deserialize)]
    struct Upgrade {
        model: String,
    }
    #[derive(serde::Deserialize)]
    struct Entry {
        #[serde(flatten)]
        metadata: ModelMetadata,
        #[serde(default)]
        capabilities: Capabilities,
        supported_reasoning_levels: Option<Vec<Effort>>,
        upgrade: Option<Upgrade>,
    }
    #[derive(serde::Deserialize)]
    struct Catalog {
        models: Option<Vec<Entry>>,
        data: Option<Vec<Entry>>,
    }
    if body.len() > 8 * 1024 * 1024 {
        return Err(crate::invalid("model metadata exceeds response limit"));
    }
    let catalog: Catalog = serde_json::from_slice(body)
        .map_err(|_| crate::invalid("invalid model metadata response"))?;
    let models = catalog.models.unwrap_or_default();
    let codex = !models.is_empty();
    let entries = if codex {
        models
    } else {
        catalog.data.unwrap_or_default()
    };
    let mut unique = BTreeMap::new();
    for entry in entries {
        let mut meta = entry.metadata;
        meta.id = meta.id.trim().to_owned();
        if meta.id.is_empty() {
            continue;
        }
        if codex {
            meta.display_name = meta.display_name.trim().to_owned();
            meta.description = meta.description.trim().to_owned();
            meta.visibility = meta.visibility.trim().to_ascii_lowercase();
            meta.default_reasoning_level = meta.default_reasoning_level.trim().to_ascii_lowercase();
            meta.supported_reasoning_levels = entry
                .supported_reasoning_levels
                .unwrap_or_default()
                .into_iter()
                .map(|level| level.effort.trim().to_ascii_lowercase())
                .filter(|level| !level.is_empty())
                .collect();
            meta.upgrade_model = entry.upgrade.map(|upgrade| upgrade.model.trim().to_owned());
        } else {
            let limits = entry.capabilities.limits;
            meta.context_window = limits
                .max_context_window_tokens
                .filter(|v| *v > 0)
                .or(limits.max_prompt_tokens);
            meta.max_context_window = limits.max_context_window_tokens;
            meta.max_output_tokens = limits.max_output_tokens;
        }
        unique.entry(meta.id.to_ascii_lowercase()).or_insert(meta);
    }
    if unique.is_empty() {
        return Err(crate::invalid("provider returned no models"));
    }
    Ok(unique.into_values().collect())
}

pub async fn fetch_model_metadata_by_id(
    context: &Context,
    session: &Session,
) -> Result<BTreeMap<String, ModelMetadata>, Error> {
    Ok(model_metadata_by_id(
        &fetch_model_metadata(context, session).await?,
    ))
}

/// Case-insensitive ID keys only; no synthesized provider-prefixed aliases.
pub fn model_metadata_by_id(models: &[ModelMetadata]) -> BTreeMap<String, ModelMetadata> {
    models
        .iter()
        .filter(|meta| !meta.id.trim().is_empty())
        .map(|meta| (meta.id.trim().to_ascii_lowercase(), meta.clone()))
        .collect()
}

pub fn picker_model_metadata(models: &[ModelMetadata]) -> Vec<ModelMetadata> {
    let mut models: Vec<_> = models
        .iter()
        .filter(|meta| !meta.hidden())
        .cloned()
        .collect();
    models.sort_by_key(|meta| {
        (
            meta.priority.filter(|priority| *priority > 0).is_none(),
            meta.priority.filter(|priority| *priority > 0).unwrap_or(0),
            meta.id.to_ascii_lowercase(),
        )
    });
    models
}
