//! Explicit credential scope and host-owned storage. No environment reads, files,
//! global token caches, or automatic credential fallback across routes.
use adk_core::{BoxFuture, Context, Error, ErrorCategory};
use reqwest::{
    Url,
    header::{HeaderMap, HeaderValue},
};
use sha2::{Digest, Sha256};
use std::{
    fmt,
    sync::Arc,
    time::{Duration, SystemTime},
};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Secret(String);
impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AuthMode {
    ApiKey,
    OpenAiOAuth,
    AnthropicOAuth,
    CopilotOAuth,
    Anonymous,
}
/// Scope includes route identity *and* endpoint. Hosts must not use a provider
/// name alone as their credential lookup key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Scope {
    pub route: String,
    pub endpoint: String,
    pub account: Option<String>,
    pub mode: AuthMode,
}
impl Scope {
    pub fn new(
        route: &str,
        endpoint: &str,
        account: Option<String>,
        mode: AuthMode,
    ) -> Result<Self, Error> {
        let mut url =
            Url::parse(endpoint).map_err(|_| crate::invalid("invalid provider endpoint"))?;
        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(crate::invalid(
                "provider endpoint must not contain credentials, query or fragment",
            ));
        }
        let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
            return Err(crate::invalid(
                "provider endpoint requires HTTPS (except explicit loopback)",
            ));
        }
        if route.trim().is_empty() || route.contains('/') {
            return Err(crate::invalid("invalid credential route"));
        }
        url.set_path(url.path().trim_end_matches('/').to_owned().as_str());
        Ok(Self {
            route: route.to_owned(),
            endpoint: url.to_string().trim_end_matches('/').to_owned(),
            account,
            mode,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Material {
    pub access_token: Secret,
    pub refresh_token: Option<Secret>,
    pub id_token: Option<Secret>,
    pub email: Option<String>,
    pub account: Option<String>,
    pub expires_at: Option<SystemTime>,
    pub last_refresh: Option<SystemTime>,
    /// Host revision for compare-and-swap. Never a credential.
    pub revision: u64,
}
impl Material {
    pub fn needs_refresh(&self, mode: AuthMode, now: SystemTime) -> bool {
        if matches!(mode, AuthMode::ApiKey | AuthMode::Anonymous) {
            return false;
        }
        if self
            .refresh_token
            .as_ref()
            .is_none_or(|v| v.expose().trim().is_empty())
        {
            return false;
        }
        if self.access_token.expose().trim().is_empty() {
            return true;
        }
        if mode == AuthMode::OpenAiOAuth {
            if let Some(expiry) = crate::material::access_token_expiry(self.access_token.expose()) {
                return expiry <= now;
            }
            return self
                .last_refresh
                .and_then(|at| now.duration_since(at).ok())
                .is_some_and(|age| age >= Duration::from_secs(8 * 86400));
        }
        let lead = if mode == AuthMode::CopilotOAuth {
            Duration::from_secs(300)
        } else {
            let nominal = Duration::from_secs(4 * 3600);
            self.last_refresh
                .zip(self.expires_at)
                .and_then(|(last, expires)| expires.duration_since(last).ok())
                .map(|lifetime| nominal.min(lifetime / 2))
                .unwrap_or(nominal)
        };
        self.expires_at
            .is_some_and(|expires| expires <= now.checked_add(lead).unwrap_or(now))
    }
}

/// The host controls persistence, external rotations and atomic revision checks.
/// Implementations must return only sanitized errors and validate scope on lookup.
pub trait CredentialStore: Send + Sync {
    fn load<'a>(
        &'a self,
        context: &'a Context,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<Material, Error>>;
    fn replace<'a>(
        &'a self,
        context: &'a Context,
        scope: &'a Scope,
        expected_revision: u64,
        material: Material,
    ) -> BoxFuture<'a, Result<bool, Error>>;
}
/// Refresh is injectable for host-owned OAuth integrations and deterministic tests.
pub trait Refresh: Send + Sync {
    fn refresh<'a>(
        &'a self,
        context: &'a Context,
        scope: &'a Scope,
        material: Material,
    ) -> BoxFuture<'a, Result<Material, Error>>;
}

/// Share one session per credential scope to serialize refresh and external-rotation
/// reloads. Independent routes must have independent sessions.
pub struct Session {
    scope: Scope,
    store: Arc<dyn CredentialStore>,
    refresh: Arc<dyn Refresh>,
    gate: Mutex<Option<Secret>>,
}
impl Session {
    pub fn new(
        scope: Scope,
        store: Arc<dyn CredentialStore>,
        refresh: Arc<dyn Refresh>,
    ) -> Result<Self, Error> {
        let scope = Scope::new(&scope.route, &scope.endpoint, scope.account, scope.mode)?;
        Ok(Self {
            scope,
            store,
            refresh,
            gate: Mutex::new(None),
        })
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    /// Invalidate the exact credential rejected on the wire, not whichever token
    /// happens to be current after another request/host has rotated it.
    pub async fn reject(&self, context: &Context, token: &Secret) -> Result<(), Error> {
        let mut rejected = crate::active(context, self.gate.lock()).await?;
        let current = crate::active(context, self.store.load(context, &self.scope)).await??;
        self.validate(&current)?;
        // A late 401 for an older token must not erase rejection of the current one.
        if current.access_token.expose() == token.expose() {
            *rejected = Some(token.clone());
        }
        Ok(())
    }
    pub async fn material(&self, context: &Context) -> Result<Material, Error> {
        let mut rejected = crate::active(context, self.gate.lock()).await?;
        let mut material = crate::active(context, self.store.load(context, &self.scope)).await??;
        self.validate(&material)?;
        let rejected_current = rejected
            .as_ref()
            .is_some_and(|token| token.expose() == material.access_token.expose());
        let now = SystemTime::now();
        let needs_refresh = if self.scope.mode == AuthMode::AnthropicOAuth {
            material.access_token.expose().is_empty()
                || material
                    .expires_at
                    .is_some_and(|at| at <= now + Duration::from_secs(120))
        } else {
            material.needs_refresh(self.scope.mode, now)
        };
        let can_refresh = matches!(
            self.scope.mode,
            AuthMode::OpenAiOAuth | AuthMode::AnthropicOAuth | AuthMode::CopilotOAuth
        ) && material
            .refresh_token
            .as_ref()
            .is_some_and(|token| !token.expose().trim().is_empty());
        if (needs_refresh || rejected_current) && can_refresh {
            let revision = material.revision;
            let original = material.clone();
            match crate::active(
                context,
                self.refresh.refresh(context, &self.scope, material),
            )
            .await?
            {
                Ok(updated) => {
                    material = updated;
                    self.validate(&material)?;
                    crate::active(
                        context,
                        self.store
                            .replace(context, &self.scope, revision, material.clone()),
                    )
                    .await??;
                    // CAS may lose to an external rotation; never return the losing token.
                    material =
                        crate::active(context, self.store.load(context, &self.scope)).await??;
                    self.validate(&material)?;
                }
                Err(error) => {
                    if matches!(
                        error.info.category,
                        ErrorCategory::Cancelled | ErrorCategory::DeadlineExceeded
                    ) {
                        return Err(error);
                    }
                    // A single-use token may have been rotated by the host during the
                    // exchange. Reload before failing or attempting any further exchange.
                    let fresh =
                        crate::active(context, self.store.load(context, &self.scope)).await??;
                    self.validate(&fresh)?;
                    if fresh.access_token.expose() != original.access_token.expose() {
                        material = fresh;
                    } else if self.scope.mode == AuthMode::AnthropicOAuth
                        && !rejected_current
                        && !original.access_token.expose().is_empty()
                        && original.expires_at.is_none_or(|at| at > now)
                    {
                        material = original;
                    } else {
                        return Err(error);
                    }
                }
            }
        } else if rejected_current {
            return Err(Error::new(
                ErrorCategory::PermissionDenied,
                "provider rejected credential and refresh is unavailable",
            ));
        }
        if rejected
            .as_ref()
            .is_some_and(|token| token.expose() != material.access_token.expose())
        {
            *rejected = None;
        }
        if self.scope.mode != AuthMode::Anonymous
            && material.access_token.expose().trim().is_empty()
        {
            return Err(Error::new(
                ErrorCategory::PermissionDenied,
                "provider credential is unavailable",
            ));
        }
        Ok(material)
    }
    fn validate(&self, material: &Material) -> Result<(), Error> {
        if self.scope.account.is_some() && self.scope.account != material.account {
            return Err(Error::new(
                ErrorCategory::PermissionDenied,
                "provider credential account mismatch",
            ));
        }
        Ok(())
    }
}
/// Headers are marked sensitive; do not serialize or log the returned map.
pub fn headers(scope: &Scope, material: &Material, anthropic: bool) -> Result<HeaderMap, Error> {
    let mut out = HeaderMap::new();
    if scope.mode != AuthMode::Anonymous {
        let (key, token) = if anthropic && scope.mode == AuthMode::ApiKey {
            (
                "x-api-key",
                material.access_token.expose().trim().to_owned(),
            )
        } else {
            (
                "authorization",
                format!("Bearer {}", material.access_token.expose().trim()),
            )
        };
        let mut value = HeaderValue::from_str(&token)
            .map_err(|_| crate::invalid("credential is not a valid HTTP header"))?;
        value.set_sensitive(true);
        out.insert(key, value);
    }
    if scope.mode == AuthMode::OpenAiOAuth {
        let account = material
            .account
            .as_ref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                Error::new(
                    ErrorCategory::PermissionDenied,
                    "OpenAI OAuth account is unavailable",
                )
            })?;
        out.insert(
            "openai-beta",
            HeaderValue::from_static("responses=experimental"),
        );
        let mut value = HeaderValue::from_str(account)
            .map_err(|_| crate::invalid("account is not a valid HTTP header"))?;
        value.set_sensitive(true);
        out.insert("chatgpt-account-id", value);
    }
    if scope.mode == AuthMode::CopilotOAuth {
        for (key, value) in [
            ("copilot-integration-id", "vscode-chat"),
            ("editor-version", "vscode/1.107.0"),
            ("editor-plugin-version", "copilot-chat/0.35.0"),
            ("user-agent", "GitHubCopilotChat/0.35.0"),
            ("openai-intent", "conversation-edits"),
            ("x-github-api-version", "2026-06-01"),
            ("x-initiator", "user"),
        ] {
            out.insert(key, HeaderValue::from_static(value));
        }
        if anthropic {
            out.insert(
                "anthropic-beta",
                HeaderValue::from_static("interleaved-thinking-2025-05-14"),
            );
        }
    }
    let openrouter = Url::parse(&scope.endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| host == "openrouter.ai" || host.ends_with(".openrouter.ai"));
    if openrouter
        && scope.mode == AuthMode::ApiKey
        && !material.access_token.expose().trim().is_empty()
    {
        out.insert(
            "http-referer",
            HeaderValue::from_static("https://github.com/gratefulagents/sdk"),
        );
        out.insert(
            "x-openrouter-title",
            HeaderValue::from_static("gratefulagents/sdk"),
        );
    }
    if anthropic {
        out.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        if scope.mode == AuthMode::AnthropicOAuth {
            out.insert(
                "anthropic-beta",
                HeaderValue::from_static("oauth-2025-04-20"),
            );
            out.insert(
                "user-agent",
                HeaderValue::from_static("claude-cli/2.1.158 (external, cli)"),
            );
        }
    }
    Ok(out)
}
/// Internal cache namespace, not a public credential fingerprint. Length prefixes
/// prevent concatenation ambiguity; refresh and endpoint changes invalidate it.
pub fn cache_scope(scope: &Scope, material: &Material, prompt_key: &str) -> String {
    let mut hash = Sha256::new();
    for value in [
        &scope.route,
        &scope.endpoint,
        &format!("{:?}", scope.mode),
        material.account.as_deref().unwrap_or(""),
        material.access_token.expose(),
        prompt_key,
    ] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    format!("{:x}", hash.finalize())
}
