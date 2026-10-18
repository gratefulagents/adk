#![cfg(feature = "providers")]

use adk::providers::{
    auth::AuthMode,
    factory::{
        Kind,
        config::{Config, NamedRoute},
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
