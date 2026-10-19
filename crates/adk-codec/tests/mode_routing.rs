use adk_codec::{
    dto::{ModeRoleRouting, ModeRouting},
    from_go_json,
    mode::{resolve_mode_routing, resolve_model_for_provider, resolve_role_mode_routing},
    to_go_json_bytes,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[test]
fn pinned_mode_and_role_routing_oracle() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/mode-routing/observations.json"
    ))
    .unwrap();
    assert_eq!(fixture["schema_version"], 1);
    assert_eq!(
        fixture["provenance"]["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (path, expected) in fixture["provenance"]["harnessSHA256"].as_object().unwrap() {
        let digest = Sha256::digest(std::fs::read(root.join(path)).unwrap());
        assert_eq!(format!("{digest:x}"), expected.as_str().unwrap(), "{path}");
    }
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 294);
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let input = &case["input"];
        let routing: Option<ModeRouting> = if input["routing"].is_null() {
            None
        } else {
            Some(from_go_json(&serde_json::to_vec(&input["routing"]).unwrap()).unwrap())
        };
        let before = routing.clone();
        let model = input["base_model"].as_str().unwrap();
        let provider = input["provider"].as_str().unwrap();
        assert_eq!(
            resolve_model_for_provider(model, provider),
            case["resolved_model"].as_str().unwrap(),
            "{id}"
        );
        let top = resolve_mode_routing(model, provider, routing.as_ref());
        assert_eq!(serde_json::to_value(&top).unwrap(), case["top"], "{id}");
        assert_eq!(
            to_go_json_bytes(&top).unwrap(),
            case["top_json"].as_str().unwrap().as_bytes(),
            "{id}"
        );
        let roles = case["roles"].as_array().unwrap();
        assert_eq!(roles.len(), 5);
        for role in roles {
            let name = role["name"].as_str().unwrap();
            let result = resolve_role_mode_routing(model, provider, name, routing.as_ref());
            assert_eq!(
                serde_json::to_value(&result).unwrap(),
                role["result"],
                "{id}/{name}"
            );
            assert_eq!(
                to_go_json_bytes(&result).unwrap(),
                role["output_json"].as_str().unwrap().as_bytes(),
                "{id}/{name}"
            );
        }
        assert_eq!(routing, before, "input mutated: {id}");
    }
}

#[test]
fn explicit_none_clears_budget_and_blank_fallback_entries_clear_inheritance() {
    let routing = ModeRouting {
        reasoning_level: "high".into(),
        fallback_models: Some(vec!["small".into()]),
        role_overrides: Some(BTreeMap::from([(
            "worker".into(),
            ModeRoleRouting {
                reasoning_level: " NONE ".into(),
                fallback_models: Some(vec![" ".into()]),
                ..Default::default()
            },
        )])),
        ..Default::default()
    };
    let result = resolve_role_mode_routing("medium", "anthropic", "worker", Some(&routing));
    assert_eq!(result.model, "claude-sonnet-4-6");
    assert_eq!(result.reasoning_level, "none");
    assert_eq!(result.model_settings.reasoning_effort, "none");
    assert_eq!(result.model_settings.thinking_budget, 0);
    assert_eq!(result.fallback_models, None);
    let unmatched = resolve_role_mode_routing("medium", "anthropic", "worker ", Some(&routing));
    assert_eq!(unmatched.model_settings.thinking_budget, 8192);
    assert_eq!(unmatched.fallback_models.unwrap(), ["claude-haiku-4-5"]);
}

#[test]
fn routing_results_own_fallbacks_and_do_not_share_mutations() {
    let mut routing = ModeRouting {
        fallback_models: Some(vec!["small".into(), "small".into()]),
        ..Default::default()
    };
    let mut first = resolve_mode_routing(" raw ", "unknown", Some(&routing));
    let second = resolve_mode_routing(" raw ", "unknown", Some(&routing));
    routing.fallback_models.as_mut().unwrap()[0] = "changed-input".into();
    first.fallback_models.as_mut().unwrap()[1] = "changed-output".into();
    assert_eq!(second.model, "raw");
    assert_eq!(second.fallback_models.unwrap(), ["small", "small"]);
    assert_eq!(first.fallback_models.unwrap()[0], "small");
    assert_eq!(routing.fallback_models.unwrap()[1], "small");
}

#[test]
fn original_resolve_role_mode_routing_regression() {
    let routing: ModeRouting = from_go_json(
        br#"{
        "DefaultModel":"openai/gpt-5.4",
        "FallbackModels":["anthropic/claude-sonnet-4-6"],
        "ReasoningLevel":"medium",
        "TextVerbosity":"low",
        "RoleOverrides":{"architect":{
            "Model":"anthropic/claude-sonnet-4-6",
            "FallbackModels":["copilot/gpt-4.1"],
            "ReasoningLevel":"high",
            "TextVerbosity":"high"
        }}
    }"#,
    )
    .unwrap();
    let architect = resolve_role_mode_routing(
        "openai/gpt-5.3-codex",
        "openai",
        "architect",
        Some(&routing),
    );
    assert_eq!(architect.model, "anthropic/claude-sonnet-4-6");
    assert_eq!(architect.fallback_models.unwrap(), ["copilot/gpt-4.1"]);
    assert_eq!(architect.reasoning_level, "high");
    assert_eq!(architect.model_settings.thinking_budget, 8192);
    assert_eq!(architect.text_verbosity, "high");
    assert_eq!(architect.model_settings.text_verbosity, "high");
    let planner =
        resolve_role_mode_routing("openai/gpt-5.3-codex", "openai", "planner", Some(&routing));
    assert_eq!(planner.model, "openai/gpt-5.4");
    assert_eq!(
        planner.fallback_models.unwrap(),
        ["anthropic/claude-sonnet-4-6"]
    );
    assert_eq!(planner.reasoning_level, "medium");
    assert_eq!(planner.model_settings.thinking_budget, 4096);
    assert_eq!(planner.text_verbosity, "low");
    assert_eq!(planner.model_settings.text_verbosity, "low");
    let empty = resolve_role_mode_routing(
        "openai/gpt-5.3-codex",
        "openai",
        "executor",
        Some(&ModeRouting::default()),
    );
    assert_eq!(empty.reasoning_level, "");
    assert_eq!(empty.model_settings.thinking_budget, 0);
}

#[test]
fn original_model_resolution_leaves_unknown_providers_alone() {
    assert_eq!(resolve_model_for_provider("small", "local"), "small");
}
