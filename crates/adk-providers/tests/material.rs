use adk_providers::{auth::AuthMode, material::*};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::json;
use std::time::{Duration, SystemTime};
fn jwt(value: serde_json::Value) -> String {
    format!(
        "header.{}.signature",
        URL_SAFE_NO_PAD.encode(value.to_string())
    )
}
#[test]
fn openai_material_preserves_id_token_account_override_and_jwt_expiry() {
    let id = jwt(json!({"https://api.openai.com/auth":{"chatgpt_account_id":"claim-account"}}));
    let access = jwt(json!({"exp":2000000000}));
    let input = json!({"tokens":{"access_token":access,"refresh_token":"refresh","id_token":id},"last_refresh":"2025-01-01T00:00:00Z"});
    let auth = parse(AuthMode::OpenAiOAuth, input.to_string().as_bytes(), None, 7).unwrap();
    assert_eq!(auth.account.as_deref(), Some("claim-account"));
    assert_eq!(auth.revision, 7);
    assert!(!auth.needs_refresh(
        AuthMode::OpenAiOAuth,
        SystemTime::UNIX_EPOCH + Duration::from_secs(1900000000)
    ));
    assert!(auth.needs_refresh(
        AuthMode::OpenAiOAuth,
        SystemTime::UNIX_EPOCH + Duration::from_secs(2000000000)
    ));
    let serialized = serialize(AuthMode::OpenAiOAuth, &auth, SystemTime::now()).unwrap();
    assert!(!format!("{serialized:?}").contains("refresh"));
    let parsed = parse(
        AuthMode::OpenAiOAuth,
        serialized.expose().as_bytes(),
        Some("explicit-account"),
        8,
    )
    .unwrap();
    assert_eq!(parsed.account.as_deref(), Some("explicit-account"));
    assert_eq!(parsed.id_token.unwrap().expose(), id);
}
#[test]
fn invalid_material_diagnostics_never_echo_input() {
    for raw in [
        "secret-input",
        "[]",
        "null",
        "{}",
        r#"{"tokens":{"access_token":"sensitive"}}"#,
    ] {
        let error = parse(AuthMode::OpenAiOAuth, raw.as_bytes(), None, 0).unwrap_err();
        assert!(!format!("{error:?}").contains("secret-input"));
        assert!(!format!("{error:?}").contains("sensitive"));
    }
    assert!(parse(AuthMode::OpenAiOAuth, &vec![b'a'; 1024 * 1024 + 1], None, 0).is_err());
    assert!(jwt_claims("a.b.c.d").is_none());
}
#[test]
fn anthropic_nested_and_flat_shapes_roundtrip_fractional_timestamps() {
    let input = json!({"claudeAiOauth":{"accessToken":"access","refreshToken":"refresh","expiresAt":1800000000123_u64,
        "tokenAccount":{"uuid":"account","emailAddress":"fixture@example.test"}},"lastRefresh":"2026-01-02T03:04:05.123456789+02:00"});
    let auth = parse(
        AuthMode::AnthropicOAuth,
        input.to_string().as_bytes(),
        None,
        1,
    )
    .unwrap();
    assert_eq!(
        auth.expires_at,
        Some(SystemTime::UNIX_EPOCH + Duration::from_millis(1800000000123))
    );
    assert_eq!(auth.account.as_deref(), Some("account"));
    let flat = serialize(AuthMode::AnthropicOAuth, &auth, SystemTime::now()).unwrap();
    let roundtrip = parse(AuthMode::AnthropicOAuth, flat.expose().as_bytes(), None, 2).unwrap();
    assert_eq!(roundtrip.last_refresh, auth.last_refresh);
    assert_eq!(roundtrip.expires_at, auth.expires_at);
    assert_eq!(roundtrip.email, auth.email);
    assert_eq!(roundtrip.account, auth.account);
}
#[test]
fn flexible_expiry_accepts_seconds_millis_strings_and_skips_invalid_candidates() {
    for expiry in [
        json!(1800000000),
        json!("1800000000"),
        json!(1800000000000_u64),
        json!("2027-01-15T08:00:00Z"),
    ] {
        let input = json!({"access_token":"a","refresh_token":"r","expired":"not-a-time","expiresAt":expiry});
        let auth = parse(
            AuthMode::AnthropicOAuth,
            input.to_string().as_bytes(),
            None,
            0,
        )
        .unwrap();
        assert_eq!(
            auth.expires_at,
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1800000000))
        );
    }
    let input = json!({"access_token":"a","expires_at":-1});
    assert!(
        parse(
            AuthMode::AnthropicOAuth,
            input.to_string().as_bytes(),
            None,
            0
        )
        .unwrap()
        .expires_at
        .is_none()
    );
}
#[test]
fn copilot_host_shapes_and_exact_host_precedence() {
    for key in [
        "github.com",
        "github.com:application-id",
        "enterprise.example.test",
    ] {
        let input = json!({key:{"oauth_token":"github-refresh"}});
        let auth = parse(
            AuthMode::CopilotOAuth,
            input.to_string().as_bytes(),
            None,
            0,
        )
        .unwrap();
        assert_eq!(auth.refresh_token.unwrap().expose(), "github-refresh");
        assert_eq!(auth.access_token.expose(), "");
    }
    let input = json!({"github.com":{"oauth_token":"exact"},"github.com:other":{"oauth_token":"other"},"token":"api","expires_at":1800000000});
    let auth = parse(
        AuthMode::CopilotOAuth,
        input.to_string().as_bytes(),
        None,
        0,
    )
    .unwrap();
    assert_eq!(auth.refresh_token.as_ref().unwrap().expose(), "exact");
    let wire = serialize(AuthMode::CopilotOAuth, &auth, SystemTime::now()).unwrap();
    let roundtrip = parse(AuthMode::CopilotOAuth, wire.expose().as_bytes(), None, 0).unwrap();
    assert_eq!(roundtrip.access_token.expose(), "api");
    assert_eq!(roundtrip.expires_at, auth.expires_at);
}
#[test]
fn copilot_token_can_select_only_allowlisted_https_hosts() {
    for segment in ["individual", "business", "enterprise"] {
        assert_eq!(
            copilot_endpoint(&format!(
                "metadata;proxy-ep=proxy.{segment}.githubcopilot.com;more=1"
            )),
            Some(format!("https://api.{segment}.githubcopilot.com"))
        );
    }
    for endpoint in [
        "http://proxy.individual.githubcopilot.com",
        "proxy.individual.githubcopilot.com/",
        "proxy.individual.githubcopilot.com:443",
        "proxy.individual.githubcopilot.com.evil.test",
        "evil.test",
        "user@proxy.individual.githubcopilot.com",
        "proxy.individual.githubcopilot.com?q=1",
        "proxy.individual.githubcopilot.com#x",
    ] {
        assert_eq!(copilot_endpoint(&format!("proxy-ep={endpoint}")), None);
    }
}

#[test]
fn pinned_auth_material_fixtures_preserve_alias_precedence() {
    let fixtures: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/providers/auth-material.json"
    ))
    .unwrap();
    assert_eq!(
        fixtures["baseline"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    for case in fixtures["cases"].as_array().unwrap() {
        let mode = match case["mode"].as_str().unwrap() {
            "openai" => AuthMode::OpenAiOAuth,
            "anthropic" => AuthMode::AnthropicOAuth,
            "copilot" => AuthMode::CopilotOAuth,
            _ => unreachable!(),
        };
        let value = parse(mode, case["body"].to_string().as_bytes(), None, 42).unwrap();
        assert_eq!(
            value.access_token.expose(),
            case["access"].as_str().unwrap()
        );
        assert_eq!(
            value.refresh_token.as_ref().unwrap().expose(),
            case["refresh"].as_str().unwrap()
        );
        assert_eq!(value.account.as_deref(), case["account"].as_str());
        assert_eq!(
            value.expires_at,
            case["expiry"]
                .as_u64()
                .map(|v| SystemTime::UNIX_EPOCH + Duration::from_secs(v))
        );
        assert_eq!(value.revision, 42);
    }
}

#[test]
fn zero_anthropic_lifetime_keeps_default_lead_and_serializer_omits_blank_identity() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1800000000);
    let mut value = parse(
        AuthMode::AnthropicOAuth,
        br#"{"access_token":"access","refresh_token":"refresh"}"#,
        None,
        0,
    )
    .unwrap();
    value.expires_at = Some(now + Duration::from_secs(60));
    value.last_refresh = value.expires_at;
    assert!(value.needs_refresh(AuthMode::AnthropicOAuth, now));
    value.account = Some(" ".into());
    value.email = Some(" ".into());
    let serialized = serialize(AuthMode::AnthropicOAuth, &value, now).unwrap();
    let body: serde_json::Value = serde_json::from_str(serialized.expose()).unwrap();
    assert!(body.get("account_uuid").is_none());
    assert!(body.get("email").is_none());
}
