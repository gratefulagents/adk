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
    pub context_window: Option<i64>,
    pub max_context_window: Option<i64>,
    pub max_output_tokens: Option<i64>,
    pub auto_compact_token_limit: Option<i64>,
    pub effective_context_window_percent: Option<i64>,
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
            .map(|value| value as u64)
    }
    /// Baseline compaction trigger and target, in tokens.
    pub fn compaction_defaults(&self) -> Option<(u64, u64)> {
        let mut trigger = self.auto_compact_token_limit.unwrap_or(0);
        let mut target = trigger / 2;
        if let Some(context) = self.resolved_context_window() {
            // Preserve the pinned 64-bit SDK arithmetic for reported metadata limits.
            let limit = (context as i64).wrapping_mul(9) / 10;
            if trigger <= 0 || trigger > limit {
                trigger = limit;
            }
            target = context as i64 / 2;
        }
        if trigger <= 0 {
            return None;
        }
        if target <= 0 || target >= trigger {
            target = trigger / 2;
        }
        Some((trigger as u64, target as u64))
    }
}

// Raw JSON preserves integer -0 without admitting fractions or exponent notation.
fn metadata_integer<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<i64>, D::Error> {
    use serde::Deserialize;
    Option::<Box<serde_json::value::RawValue>>::deserialize(deserializer)?
        .map(|raw| raw.get().parse().map_err(serde::de::Error::custom))
        .transpose()
}

/// Decode the pinned OpenAI, Codex, or Copilot catalog shape. First duplicate
/// wins case-insensitively; a nonempty Codex catalog takes precedence over data.
/// The host owns fetching, authentication, refresh, and any catalog lifetime.
pub fn parse_model_metadata(body: &[u8]) -> Result<Vec<ModelMetadata>, Error> {
    #[derive(Default, serde::Deserialize)]
    #[serde(default)]
    struct Limits {
        #[serde(default, deserialize_with = "metadata_integer")]
        max_context_window_tokens: Option<i64>,
        #[serde(default, deserialize_with = "metadata_integer")]
        max_prompt_tokens: Option<i64>,
        #[serde(default, deserialize_with = "metadata_integer")]
        max_output_tokens: Option<i64>,
    }
    #[derive(Default, serde::Deserialize)]
    #[serde(default)]
    struct Capabilities {
        limits: Option<Limits>,
    }
    #[derive(serde::Deserialize)]
    struct Effort {
        effort: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct Upgrade {
        model: Option<String>,
    }
    #[derive(serde::Deserialize)]
    struct CodexEntry {
        slug: Option<String>,
        #[serde(default, deserialize_with = "metadata_integer")]
        context_window: Option<i64>,
        #[serde(default, deserialize_with = "metadata_integer")]
        max_context_window: Option<i64>,
        #[serde(default, deserialize_with = "metadata_integer")]
        auto_compact_token_limit: Option<i64>,
        #[serde(default, deserialize_with = "metadata_integer")]
        effective_context_window_percent: Option<i64>,
        display_name: Option<String>,
        description: Option<String>,
        visibility: Option<String>,
        #[serde(default, deserialize_with = "metadata_integer")]
        priority: Option<i64>,
        default_reasoning_level: Option<String>,
        supported_reasoning_levels: Option<Vec<Option<Effort>>>,
        upgrade: Option<Upgrade>,
    }
    #[derive(serde::Deserialize)]
    struct DataEntry {
        id: Option<String>,
        capabilities: Option<Capabilities>,
    }
    #[derive(serde::Deserialize)]
    struct Catalog {
        models: Option<Vec<Option<CodexEntry>>>,
        data: Option<Vec<Option<DataEntry>>>,
    }
    if body.len() > 8 * 1024 * 1024 {
        return Err(crate::invalid("model metadata exceeds response limit"));
    }
    let catalog: Catalog = serde_json::from_slice(body)
        .map_err(|_| crate::invalid("invalid model metadata response"))?;
    let models = catalog.models.unwrap_or_default();
    let entries: Vec<ModelMetadata> = if !models.is_empty() {
        models
            .into_iter()
            .flatten()
            .map(|entry| ModelMetadata {
                id: entry.slug.unwrap_or_default(),
                context_window: entry.context_window,
                max_context_window: entry.max_context_window,
                auto_compact_token_limit: entry.auto_compact_token_limit,
                effective_context_window_percent: entry.effective_context_window_percent,
                display_name: entry.display_name.unwrap_or_default().trim().to_owned(),
                description: entry.description.unwrap_or_default().trim().to_owned(),
                visibility: metadata_key(&entry.visibility.unwrap_or_default()),
                priority: entry.priority,
                default_reasoning_level: metadata_key(
                    &entry.default_reasoning_level.unwrap_or_default(),
                ),
                supported_reasoning_levels: entry
                    .supported_reasoning_levels
                    .unwrap_or_default()
                    .into_iter()
                    .flatten()
                    .filter_map(|level| {
                        let effort = metadata_key(&level.effort.unwrap_or_default());
                        (!effort.is_empty()).then_some(effort)
                    })
                    .collect(),
                upgrade_model: entry
                    .upgrade
                    .map(|upgrade| upgrade.model.unwrap_or_default().trim().to_owned()),
                ..Default::default()
            })
            .collect()
    } else {
        catalog
            .data
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .map(|entry| {
                let limits = entry
                    .capabilities
                    .unwrap_or_default()
                    .limits
                    .unwrap_or_default();
                ModelMetadata {
                    id: entry.id.unwrap_or_default(),
                    context_window: limits
                        .max_context_window_tokens
                        .filter(|v| *v != 0)
                        .or(limits.max_prompt_tokens),
                    max_context_window: limits.max_context_window_tokens,
                    max_output_tokens: limits.max_output_tokens,
                    ..Default::default()
                }
            })
            .collect()
    };
    let mut unique = BTreeMap::new();
    for mut meta in entries {
        meta.id = meta.id.trim().to_owned();
        if meta.id.is_empty() {
            continue;
        }
        unique.entry(metadata_key(&meta.id)).or_insert(meta);
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
        .map(|meta| (metadata_key(&meta.id), meta.clone()))
        .collect()
}

pub(crate) fn metadata_key(id: &str) -> String {
    metadata_lower(id.trim())
}

fn metadata_lower(value: &str) -> String {
    // The pinned Go uses Unicode 15 simple mappings, not Rust's expanding lowercase.
    value
        .chars()
        .map(|c| {
            if unicode_general_category::get_general_category(c)
                == unicode_general_category::GeneralCategory::Unassigned
            {
                c
            } else {
                c.to_lowercase().next().unwrap()
            }
        })
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
            metadata_lower(&meta.id),
        )
    });
    models
}

#[cfg(test)]
mod tests {
    use super::metadata_key;
    use sha2::{Digest, Sha256};

    #[test]
    fn lookup_keys_match_every_pinned_go_scalar() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../fixtures/metadata-compaction/observations.json"
        ))
        .unwrap();
        assert_eq!(fixture["unicode_version"], "15.0.0");
        let mut hash = Sha256::new();
        let mut count = 0u64;
        for ch in (0..=0x10ffff).filter_map(char::from_u32) {
            let key = metadata_key(&ch.to_string());
            hash.update((key.len() as u32).to_le_bytes());
            hash.update(key.as_bytes());
            count += 1;
        }
        assert_eq!(count, fixture["scalar_count"].as_u64().unwrap());
        assert_eq!(
            format!("{:x}", hash.finalize()),
            fixture["scalar_sha256"].as_str().unwrap()
        );
    }
}
