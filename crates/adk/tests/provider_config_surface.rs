#![cfg(feature = "providers")]

use adk::providers::{
    auth::AuthMode,
    factory::{
        config::{Config, NamedRoute},
        Kind,
    },
    wire::Protocol,
};

#[test]
fn provider_configuration_is_available_without_builder_or_runtime() {
    let resolved = Config::default().resolve().unwrap();
    assert_eq!(resolved.default, "openai");
    assert_eq!(resolved.routes.len(), 1);
    let route = &resolved.routes[0];
    assert_eq!(route.spec.kind, Kind::OpenAi);
    assert_eq!(route.spec.protocol, Some(Protocol::Responses));
    assert_eq!(route.spec.scope().unwrap().mode, AuthMode::ApiKey);
    assert!(route.api_key.is_none());
}

#[test]
fn public_named_route_does_not_inherit_parent_credentials_or_endpoint() {
    let resolved = Config {
        provider: "anthropic".into(),
        api_key: "parent-fixture-only".into(),
        base_url: "https://parent.example".into(),
        routes: vec![NamedRoute {
            prefix: "independent".into(),
            provider: "anthropic".into(),
            ..Default::default()
        }],
        ..Default::default()
    }
    .resolve()
    .unwrap();
    let parent = resolved
        .routes
        .iter()
        .find(|route| route.spec.prefix() == "anthropic")
        .unwrap();
    let named = resolved
        .routes
        .iter()
        .find(|route| route.spec.prefix() == "independent")
        .unwrap();
    assert_eq!(
        parent.api_key.as_ref().unwrap().expose(),
        "parent-fixture-only"
    );
    assert_eq!(
        parent.spec.scope().unwrap().endpoint,
        "https://parent.example"
    );
    assert!(named.api_key.is_none());
    assert_eq!(
        named.spec.scope().unwrap().endpoint,
        "https://api.anthropic.com"
    );
}

#[test]
fn public_configuration_refuses_ambiguous_secret_fanout() {
    let config = Config {
        api_key: "must-not-fan-out".into(),
        routes: vec![NamedRoute {
            prefix: "independent".into(),
            provider: "anthropic".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(config.resolve().is_err());
}

#[test]
fn codex_backend_default_matches_pinned_go_without_changing_api_key_default() {
    use adk::providers::{
        auth::AuthMode,
        factory::{Kind, DEFAULT_CODEX_BACKEND_BASE_URL},
    };
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/provider-config/observations.json"
    ))
    .unwrap();
    assert_eq!(
        reference["codex_backend_base_url"],
        DEFAULT_CODEX_BACKEND_BASE_URL
    );
    assert_eq!(
        Kind::OpenAi.endpoint(AuthMode::OpenAiOAuth),
        DEFAULT_CODEX_BACKEND_BASE_URL
    );
    assert_eq!(
        Kind::OpenAi.endpoint(AuthMode::ApiKey),
        "https://api.openai.com/v1"
    );
}

#[test]
fn canonical_provider_name_constants_match_pinned_go() {
    use adk::providers::factory::*;
    let reference: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/provider-config/observations.json"
    ))
    .unwrap();
    let names: [(&str, &'static str, Kind); 8] = [
        (
            "DefaultProviderOpenAI",
            DEFAULT_PROVIDER_OPENAI,
            Kind::OpenAi,
        ),
        (
            "DefaultProviderAnthropic",
            DEFAULT_PROVIDER_ANTHROPIC,
            Kind::Anthropic,
        ),
        (
            "DefaultProviderOpenRouter",
            DEFAULT_PROVIDER_OPENROUTER,
            Kind::OpenRouter,
        ),
        (
            "DefaultProviderGemini",
            DEFAULT_PROVIDER_GEMINI,
            Kind::Gemini,
        ),
        ("DefaultProviderGroq", DEFAULT_PROVIDER_GROQ, Kind::Groq),
        ("DefaultProviderXAI", DEFAULT_PROVIDER_XAI, Kind::Xai),
        ("DefaultProviderLocal", DEFAULT_PROVIDER_LOCAL, Kind::Local),
        (
            "DefaultProviderCopilot",
            DEFAULT_PROVIDER_COPILOT,
            Kind::Copilot,
        ),
    ];
    assert_eq!(
        reference["provider_names"].as_object().unwrap().len(),
        names.len()
    );
    for (source_name, native, kind) in names {
        assert_eq!(reference["provider_names"][source_name], native);
        assert_eq!(kind.name(), native);
        assert_eq!(native.parse::<Kind>().unwrap(), kind);
    }
}
