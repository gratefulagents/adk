//! OAuth refresh exchanges only. Interactive authorization and secret persistence
//! remain host-owned; this module never opens browsers or writes credentials.
use crate::auth::{AuthMode, Material, Refresh, Scope, Secret};
use adk_core::{BoxFuture, Context, Error, ErrorCategory};
use reqwest::Client;
use serde_json::{Value, json};
use std::time::{Duration, SystemTime};

pub struct OAuthRefresh {
    client: Client,
}
impl OAuthRefresh {
    pub fn new() -> Result<Self, Error> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| {
                Error::new(ErrorCategory::Provider, "cannot initialize OAuth transport")
            })?;
        Ok(Self { client })
    }
    fn request(&self, mode: AuthMode, token: &str) -> Result<reqwest::Request, Error> {
        let builder = match mode {
            AuthMode::OpenAiOAuth => self.client.post("https://auth.openai.com/oauth/token")
                .json(&json!({"grant_type":"refresh_token", "refresh_token":token,
                    "client_id":"app_EMoamEEZ73f0CkXaXp7hrann", "scope":"openid profile email offline_access"})),
            AuthMode::AnthropicOAuth => self.client.post("https://platform.claude.com/v1/oauth/token")
                .json(&json!({"grant_type":"refresh_token","refresh_token":token,
                    "client_id":"9d1c250a-e61b-44d9-88ed-5944d1962f5e",
                    "scope":"user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload"})),
            AuthMode::CopilotOAuth => self.client.get("https://api.github.com/copilot_internal/v2/token")
                .header("authorization", format!("token {token}"))
                .header("accept", "application/json").header("editor-version", "gratefulagents-sdk/unknown")
                .header("editor-plugin-version", "gratefulagents-sdk/unknown").header("user-agent", "gratefulagents-sdk"),
            _ => return Err(crate::invalid("credential mode has no OAuth refresh exchange")),
        };
        let mut request = builder
            .build()
            .map_err(|_| crate::invalid("invalid OAuth refresh request"))?;
        if let Some(value) = request.headers_mut().get_mut("authorization") {
            value.set_sensitive(true);
        }
        Ok(request)
    }
}
impl Refresh for OAuthRefresh {
    fn refresh<'a>(
        &'a self,
        context: &'a Context,
        scope: &'a Scope,
        mut material: Material,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async move {
            let token = material
                .refresh_token
                .as_ref()
                .ok_or_else(|| {
                    Error::new(
                        ErrorCategory::PermissionDenied,
                        "refresh credential unavailable",
                    )
                })?
                .expose();
            let request = self.request(scope.mode, token)?;
            // No transparent retry: refresh tokens can be single-use.
            let mut response = crate::active(context, self.client.execute(request))
                .await?
                .map_err(|e| crate::error::RequestFailure::transport(&e).into_error())?;
            if !response.status().is_success() {
                return Err(crate::error::RequestFailure::http(
                    response.status().as_u16(),
                    response.headers(),
                    SystemTime::now(),
                )
                .into_error());
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = crate::active(context, response.chunk())
                .await?
                .map_err(|e| crate::error::RequestFailure::transport(&e).into_error())?
            {
                if bytes.len().saturating_add(chunk.len()) > 1024 * 1024 {
                    return Err(Error::new(
                        ErrorCategory::Provider,
                        "OAuth response exceeds limit",
                    ));
                }
                bytes.extend_from_slice(&chunk);
            }
            let body: Value = serde_json::from_slice(&bytes)
                .map_err(|_| Error::new(ErrorCategory::Provider, "invalid OAuth response"))?;
            let access_key = if scope.mode == AuthMode::CopilotOAuth {
                "token"
            } else {
                "access_token"
            };
            let access = body[access_key]
                .as_str()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .ok_or_else(|| {
                    Error::new(
                        ErrorCategory::Provider,
                        "OAuth response missing access token",
                    )
                })?;
            material.access_token = Secret::new(access);
            if scope.mode != AuthMode::CopilotOAuth
                && let Some(refresh) = body["refresh_token"]
                    .as_str()
                    .map(str::trim)
                    .filter(|v| !v.is_empty())
            {
                material.refresh_token = Some(Secret::new(refresh));
            }
            let now = SystemTime::now();
            material.expires_at = if scope.mode == AuthMode::CopilotOAuth {
                body["expires_at"]
                    .as_u64()
                    .and_then(|v| SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(v)))
            } else {
                body["expires_in"]
                    .as_u64()
                    .filter(|v| *v > 0)
                    .and_then(|v| now.checked_add(Duration::from_secs(v)))
            };
            material.last_refresh = Some(now);
            // Anthropic account response is authoritative; Session validates it.
            if let Some(account) = body.pointer("/account/uuid").and_then(Value::as_str) {
                material.account = Some(account.to_owned());
            }
            Ok(material)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refresh_requests_match_reference_methods_scope_and_content_type() {
        let client = OAuthRefresh::new().unwrap();
        let openai = client
            .request(AuthMode::OpenAiOAuth, "fixture-refresh")
            .unwrap();
        assert_eq!(openai.method(), reqwest::Method::POST);
        assert_eq!(openai.url().as_str(), "https://auth.openai.com/oauth/token");
        assert_eq!(openai.headers()["content-type"], "application/json");
        let body: Value =
            serde_json::from_slice(openai.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"grant_type":"refresh_token", "refresh_token":"fixture-refresh",
            "client_id":"app_EMoamEEZ73f0CkXaXp7hrann", "scope":"openid profile email offline_access"})
        );
        let anthropic = client
            .request(AuthMode::AnthropicOAuth, "fixture-refresh")
            .unwrap();
        assert_eq!(
            anthropic.url().as_str(),
            "https://platform.claude.com/v1/oauth/token"
        );
        let body: Value =
            serde_json::from_slice(anthropic.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(
            body["scope"],
            "user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload"
        );
        assert_eq!(body["refresh_token"], "fixture-refresh");
        let copilot = client
            .request(AuthMode::CopilotOAuth, "fixture-github")
            .unwrap();
        assert_eq!(copilot.method(), reqwest::Method::GET);
        assert_eq!(
            copilot.url().as_str(),
            "https://api.github.com/copilot_internal/v2/token"
        );
        assert_eq!(copilot.headers()["authorization"], "token fixture-github");
        assert!(copilot.headers()["authorization"].is_sensitive());
        assert!(copilot.body().is_none());
        assert!(client.request(AuthMode::ApiKey, "fixture").is_err());
    }
}
