//! Bounded OAuth material codecs. The host supplies bytes and persists the returned
//! secret document; no implicit file or environment access takes place here.
use crate::auth::{AuthMode, Material, Secret};
use adk_core::{Error, ErrorCategory};
use base64::{
    Engine,
    engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD},
};
use serde_json::{Value, json};
use std::time::{Duration, SystemTime};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

fn failure() -> Error {
    Error::new(
        ErrorCategory::PermissionDenied,
        "invalid or incomplete OAuth material",
    )
}
fn string(root: &Value, paths: &[&str]) -> Option<String> {
    paths
        .iter()
        .filter_map(|p| root.pointer(p).and_then(Value::as_str))
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(str::to_owned)
}
pub(crate) fn first_time(root: &Value, paths: &[&str]) -> Option<SystemTime> {
    paths
        .iter()
        .filter_map(|p| root.pointer(p))
        .find_map(parse_time)
}
/// JWT claims are used only as unverified token metadata, never as authorization.
pub fn jwt_claims(token: &str) -> Option<Value> {
    if token.len() > 1024 * 1024 {
        return None;
    }
    let mut parts = token.trim().split('.');
    parts.next()?;
    let payload = parts.next()?;
    parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    let raw = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| URL_SAFE.decode(payload))
        .ok()?;
    serde_json::from_slice(&raw).ok().filter(Value::is_object)
}
pub fn account_from_id_token(token: &str) -> Option<String> {
    let claims = jwt_claims(token)?;
    string(
        &claims,
        &["/https:~1~1api.openai.com~1auth/chatgpt_account_id"],
    )
}
pub fn access_token_expiry(token: &str) -> Option<SystemTime> {
    let claims = jwt_claims(token)?;
    let value = claims.get("exp")?;
    let seconds = value
        .as_i64()
        .or_else(|| value.as_str()?.trim().parse().ok())?;
    if seconds < 0 {
        return SystemTime::UNIX_EPOCH.checked_sub(Duration::from_secs(seconds.unsigned_abs()));
    }
    SystemTime::UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64))
}
fn parse_time(value: &Value) -> Option<SystemTime> {
    if let Some(raw) = value.as_str() {
        let raw = raw.trim();
        if let Ok(number) = raw.parse::<i64>() {
            return numeric_time(number as f64);
        }
        let timestamp = OffsetDateTime::parse(raw, &Rfc3339).ok()?;
        let nanos = timestamp.unix_timestamp_nanos();
        let absolute = nanos.unsigned_abs();
        let duration = Duration::new(
            u64::try_from(absolute / 1_000_000_000).ok()?,
            (absolute % 1_000_000_000) as u32,
        );
        return if nanos < 0 {
            SystemTime::UNIX_EPOCH.checked_sub(duration)
        } else {
            SystemTime::UNIX_EPOCH.checked_add(duration)
        };
    }
    numeric_time(value.as_f64()?)
}
fn numeric_time(number: f64) -> Option<SystemTime> {
    if !number.is_finite() || number <= 0.0 || number >= i64::MAX as f64 {
        return None;
    }
    // Matches the baseline flexible clock: second or millisecond integer truncation.
    let duration = if number < 1e12 {
        Duration::from_secs(number as u64)
    } else {
        Duration::from_millis(number as u64)
    };
    SystemTime::UNIX_EPOCH.checked_add(duration)
}
fn format_time(value: SystemTime) -> Result<String, Error> {
    let nanos = match value.duration_since(SystemTime::UNIX_EPOCH) {
        Ok(duration) => i128::try_from(duration.as_nanos()).map_err(|_| failure())?,
        Err(error) => -i128::try_from(error.duration().as_nanos()).map_err(|_| failure())?,
    };
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .map_err(|_| failure())?
        .format(&Rfc3339)
        .map_err(|_| failure())
}

pub fn parse(
    mode: AuthMode,
    bytes: &[u8],
    account_override: Option<&str>,
    revision: u64,
) -> Result<Material, Error> {
    if bytes.len() > 1024 * 1024 {
        return Err(failure());
    }
    let root: Value = serde_json::from_slice(bytes).map_err(|_| failure())?;
    if !root.is_object() {
        return Err(failure());
    }
    let mut material = Material {
        access_token: Secret::new(""),
        refresh_token: None,
        id_token: None,
        account: None,
        email: None,
        expires_at: None,
        last_refresh: None,
        revision,
    };
    let (access, refresh) = match mode {
        AuthMode::OpenAiOAuth => {
            let access = string(&root, &["/tokens/access_token"]);
            let refresh = string(&root, &["/tokens/refresh_token"]);
            material.id_token = string(&root, &["/tokens/id_token"]).map(Secret::new);
            material.account = account_override
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| string(&root, &["/tokens/account_id"]))
                .or_else(|| account_from_id_token(material.id_token.as_ref()?.expose()));
            material.expires_at = access.as_ref().and_then(|s| access_token_expiry(s));
            material.last_refresh = first_time(&root, &["/last_refresh"]);
            if material.account.is_none() && refresh.is_none() {
                return Err(failure());
            }
            (access, refresh)
        }
        AuthMode::AnthropicOAuth => {
            material.account = string(
                &root,
                &[
                    "/account_uuid",
                    "/accountUuid",
                    "/claudeAiOauth/tokenAccount/uuid",
                    "/account/uuid",
                ],
            );
            material.email = string(
                &root,
                &[
                    "/email",
                    "/claudeAiOauth/tokenAccount/emailAddress",
                    "/account/email_address",
                ],
            );
            material.expires_at = first_time(
                &root,
                &[
                    "/claudeAiOauth/expiresAt",
                    "/expired",
                    "/expires_at",
                    "/expiresAt",
                ],
            );
            material.last_refresh = first_time(&root, &["/last_refresh", "/lastRefresh"]);
            (
                string(
                    &root,
                    &[
                        "/claudeAiOauth/accessToken",
                        "/access_token",
                        "/accessToken",
                        "/tokens/access_token",
                    ],
                ),
                string(
                    &root,
                    &[
                        "/claudeAiOauth/refreshToken",
                        "/refresh_token",
                        "/refreshToken",
                        "/tokens/refresh_token",
                    ],
                ),
            )
        }
        AuthMode::CopilotOAuth => {
            let object = root.as_object().unwrap();
            let host = object
                .get("github.com")
                .filter(|v| v.is_object())
                .or_else(|| {
                    object
                        .iter()
                        .find(|(key, value)| key.starts_with("github.com:") && value.is_object())
                        .map(|(_, value)| value)
                })
                .or_else(|| {
                    object
                        .values()
                        .find(|v| string(v, &["/oauth_token", "/token"]).is_some())
                });
            material.expires_at = first_time(
                &root,
                &[
                    "/expires_at",
                    "/expiresAt",
                    "/expires",
                    "/tokens/expires_at",
                    "/copilot/expires_at",
                ],
            );
            material.last_refresh = first_time(
                &root,
                &["/last_refresh", "/lastRefresh", "/copilot/last_refresh"],
            );
            let refresh = string(
                &root,
                &[
                    "/oauth_token",
                    "/oauthToken",
                    "/github_oauth_token",
                    "/tokens/oauth_token",
                ],
            )
            .or_else(|| host.and_then(|v| string(v, &["/oauth_token"])))
            .or_else(|| string(&root, &["/copilot/oauth_token"]));
            (
                string(
                    &root,
                    &[
                        "/token",
                        "/access_token",
                        "/accessToken",
                        "/copilot_token",
                        "/tokens/token",
                        "/tokens/access_token",
                        "/copilot/token",
                    ],
                ),
                refresh,
            )
        }
        _ => return Err(crate::invalid("material codec requires an OAuth mode")),
    };
    if access.is_none() && refresh.is_none() {
        return Err(failure());
    }
    material.access_token = Secret::new(access.unwrap_or_default());
    material.refresh_token = refresh.map(Secret::new);
    Ok(material)
}

/// Serialize an explicit provider material format. This returns a redacted-debug
/// Secret; the host must opt in to exposing it for persistence.
pub fn serialize(mode: AuthMode, material: &Material, now: SystemTime) -> Result<Secret, Error> {
    let access = material.access_token.expose().trim();
    let refresh = material
        .refresh_token
        .as_ref()
        .map(|s| s.expose().trim())
        .unwrap_or_default();
    let last = format_time(material.last_refresh.unwrap_or(now))?;
    let mut body = match mode {
        AuthMode::OpenAiOAuth => json!({"tokens":{"access_token":access,"refresh_token":refresh,
            "id_token":material.id_token.as_ref().map(Secret::expose).unwrap_or_default(),
            "account_id":material.account.as_deref().unwrap_or_default()},"last_refresh":last}),
        AuthMode::AnthropicOAuth => {
            json!({"access_token":access,"refresh_token":refresh,"last_refresh":last,"type":"claude"})
        }
        AuthMode::CopilotOAuth => {
            json!({"oauth_token":refresh,"token":access,"last_refresh":last,"type":"copilot"})
        }
        _ => return Err(crate::invalid("material codec requires an OAuth mode")),
    };
    if mode == AuthMode::AnthropicOAuth {
        if let Some(account) = material
            .account
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            body["account_uuid"] = account.into();
        }
        if let Some(email) = material
            .email
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            body["email"] = email.into();
        }
        if let Some(expiry) = material.expires_at {
            body["expired"] = format_time(expiry)?.into();
        }
    }
    if mode == AuthMode::CopilotOAuth
        && let Some(expiry) = material.expires_at
    {
        body["expires_at"] = expiry
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_err(|_| failure())?
            .as_secs()
            .into();
    }
    Ok(Secret::new(body.to_string()))
}

/// Accept only the three baseline Copilot API hosts. Token text cannot select an
/// arbitrary credential destination, explicit port, path, query or userinfo.
pub fn copilot_endpoint(token: &str) -> Option<String> {
    let raw = token.trim().split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        key.trim()
            .eq_ignore_ascii_case("proxy-ep")
            .then(|| value.trim())
    })?;
    let host = raw
        .strip_prefix("https://")
        .unwrap_or(raw)
        .to_ascii_lowercase();
    let host = host
        .strip_prefix("proxy.")
        .map(|s| format!("api.{s}"))
        .unwrap_or(host);
    match host.as_str() {
        "api.individual.githubcopilot.com"
        | "api.business.githubcopilot.com"
        | "api.enterprise.githubcopilot.com" => Some(format!("https://{host}")),
        _ => None,
    }
}
