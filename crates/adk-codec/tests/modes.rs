use adk_codec::mode::ModeRoutingResolution;
use adk_codec::{
    GoJsonDecode, GoJsonEncode,
    dto::{
        ModeConstraints, ModeReasoningLevel, ModeRoleRouting, ModeRouting, ModeTemplateSpec,
        ModeTextVerbosity, ModelRetryAdvice, ModelSettings,
    },
    from_go_json, to_go_json_bytes,
};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn unhex(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0);
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

fn compare<T: GoJsonDecode + GoJsonEncode + Serialize>(case: &Value) {
    let id = case["id"].as_str().unwrap();
    let decoded = from_go_json::<T>(&unhex(case["input_hex"].as_str().unwrap()));
    assert_eq!(
        decoded.is_ok(),
        case["decode_ok"].as_bool().unwrap(),
        "{id}: {:?}",
        decoded.as_ref().err()
    );
    let Ok(decoded) = decoded else { return };
    assert_eq!(
        serde_json::to_value(&decoded).unwrap(),
        case["decoded_fields"],
        "unnormalized state: {id}"
    );
    assert_eq!(case["marshal_ok"], true, "{id}");
    assert_eq!(
        to_go_json_bytes(&decoded).unwrap(),
        unhex(case["marshal_hex"].as_str().unwrap()),
        "exact bytes: {id}"
    );
}

#[test]
fn pinned_mode_codec_oracle() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/mode-codec/observations.json"
    ))
    .unwrap();
    assert_eq!(fixture["schema_version"], 1);
    assert_eq!(fixture["int_size"], 64);
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
    assert_eq!(cases.len(), 1901);
    for case in cases {
        match case["type"].as_str().unwrap() {
            "RetryAdvice" => compare::<ModelRetryAdvice>(case),
            "ModelSettings" => compare_settings(case),
            "RoutingResolution" => compare_resolution(case),
            "ReasoningLevel" => compare::<ModeReasoningLevel>(case),
            "TextVerbosity" => compare::<ModeTextVerbosity>(case),
            "TemplateSpec" => compare::<ModeTemplateSpec>(case),
            "Constraints" => compare::<ModeConstraints>(case),
            "TemplateRouting" | "RunnerRouting" => compare::<ModeRouting>(case),
            "TemplateRoleRouting" | "RunnerRoleRouting" => compare::<ModeRoleRouting>(case),
            other => panic!("unknown target {other}"),
        }
    }
}

#[test]
fn pointer_objects_merge_and_null_resets_them() {
    let mode: ModeTemplateSpec =
        from_go_json(br#"{"Constraints":{"MaxTurns":7},"CONSTRAINTS":{"MaxRetries":2}}"#).unwrap();
    let constraints = mode.constraints.unwrap();
    assert_eq!(constraints.max_turns, 7);
    assert_eq!(constraints.max_retries, 2);
    let mode: ModeTemplateSpec = from_go_json(
        br#"{"Constraints":{"MaxTurns":7},"Constraints":null,"Constraints":{"MaxRetries":2}}"#,
    )
    .unwrap();
    assert_eq!(mode.constraints.unwrap().max_turns, 0);
}

#[test]
fn repeated_pointer_objects_retain_slice_backing_slots() {
    let mode: ModeTemplateSpec = from_go_json(br#"{"ModelRouting":{"FallbackModels":["old","tail"]},"ModelRouting":{"FallbackModels":["short"]},"ModelRouting":{"FallbackModels":[null,null,null]}}"#).unwrap();
    assert_eq!(
        mode.model_routing.unwrap().fallback_models.unwrap(),
        ["short", "tail", ""]
    );
}

#[test]
fn maps_merge_but_each_key_value_decodes_from_zero() {
    let routing: ModeRouting = from_go_json(br#"{"RoleOverrides":{"keep":{"Model":"keep"},"same":{"Model":"old"}},"RoleOverrides":{"same":{"ReasoningLevel":"high"}},"RoleOverrides":{}}"#).unwrap();
    let roles = routing.role_overrides.unwrap();
    assert_eq!(roles["keep"].model, "keep");
    assert_eq!(roles["same"].model, "");
    assert_eq!(roles["same"].reasoning_level, "high");
    let routing: ModeRouting = from_go_json(
        br#"{"RoleOverrides":{"old":{}},"RoleOverrides":null,"RoleOverrides":{"new":null}}"#,
    )
    .unwrap();
    let roles = routing.role_overrides.unwrap();
    assert_eq!(roles.len(), 1);
    assert_eq!(roles["new"], ModeRoleRouting::default());
}

#[test]
fn mode_nil_empty_and_zero_fields_have_exact_go_bytes() {
    let nil = ModeRouting::default();
    let empty: ModeRouting = from_go_json(br#"{"FallbackModels":[],"RoleOverrides":{}}"#).unwrap();
    assert_ne!(nil, empty);
    assert_eq!(to_go_json_bytes(&nil).unwrap(), br#"{"DefaultModel":"","FallbackModels":null,"ReasoningLevel":"","TextVerbosity":"","RoleOverrides":null}"#);
    assert_eq!(to_go_json_bytes(&empty).unwrap(), br#"{"DefaultModel":"","FallbackModels":[],"ReasoningLevel":"","TextVerbosity":"","RoleOverrides":{}}"#);
    assert_eq!(to_go_json_bytes(&ModeConstraints::default()).unwrap(), br#"{"MaxTurns":0,"SubAgentMaxTurns":0,"MaxConcurrentSubAgents":0,"MaxRetries":0,"MaxRuntimeMinutes":0}"#);
}

#[test]
fn mode_routing_conversion_preserves_go_bytes_and_owns_the_result() {
    use adk_codec::mode::mode_routing_from_template_spec;

    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/specialist-catalog/observations.json"
    ))
    .unwrap();
    assert_eq!(
        fixture["provenance"]["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (path, expected) in fixture["provenance"]["harnessSHA256"].as_object().unwrap() {
        let digest = Sha256::digest(std::fs::read(root.join(path)).unwrap());
        assert_eq!(format!("{digest:x}"), expected.as_str().unwrap(), "{path}");
    }
    let cases = fixture["routing_cases"].as_array().unwrap();
    assert_eq!(cases.len(), 6);
    for case in cases {
        let mut input: Option<ModeTemplateSpec> = if case["input"].is_null() {
            None
        } else {
            Some(from_go_json(&serde_json::to_vec(&case["input"]).unwrap()).unwrap())
        };
        let original = input.clone();
        let output = mode_routing_from_template_spec(input.as_ref());
        assert_eq!(input, original);
        assert_eq!(serde_json::to_value(&output).unwrap(), case["output"]);
        let bytes = output.as_ref().map_or_else(
            || b"null".to_vec(),
            |value| to_go_json_bytes(value).unwrap(),
        );
        assert_eq!(bytes, case["output_json"].as_str().unwrap().as_bytes());
        if let Some(routing) = input.as_mut().and_then(|spec| spec.model_routing.as_mut()) {
            routing.default_model.clear();
            routing.reasoning_level.clear();
            routing.text_verbosity.clear();
            if let Some(models) = &mut routing.fallback_models {
                models.clear();
            }
            if let Some(roles) = &mut routing.role_overrides {
                for role in roles.values_mut() {
                    role.model.clear();
                    role.reasoning_level.clear();
                    role.text_verbosity.clear();
                    if let Some(models) = &mut role.fallback_models {
                        models.clear();
                    }
                }
                roles.clear();
            }
        }
        assert_eq!(serde_json::to_value(&output).unwrap(), case["output"]);
    }
}

#[test]
fn named_mode_labels_preserve_constants_unknown_values_and_string_identity() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/mode-codec/observations.json"
    ))
    .unwrap();
    let constants = fixture["constants"].as_object().unwrap();
    assert_eq!(constants.len(), 10);
    for (name, value) in [
        ("ReasoningNone", ModeReasoningLevel::NONE),
        ("ReasoningMinimal", ModeReasoningLevel::MINIMAL),
        ("ReasoningLow", ModeReasoningLevel::LOW),
        ("ReasoningMedium", ModeReasoningLevel::MEDIUM),
        ("ReasoningHigh", ModeReasoningLevel::HIGH),
        ("ReasoningXHigh", ModeReasoningLevel::XHIGH),
        ("ReasoningMax", ModeReasoningLevel::MAX),
    ] {
        assert_eq!(value.to_string(), constants[name].as_str().unwrap());
        let decoded: ModeReasoningLevel = from_go_json(&to_go_json_bytes(&value).unwrap()).unwrap();
        assert_eq!(decoded, value);
        assert!(std::collections::HashSet::from([value]).contains(&decoded));
    }
    for (name, value) in [
        ("TextVerbosityLow", ModeTextVerbosity::LOW),
        ("TextVerbosityMedium", ModeTextVerbosity::MEDIUM),
        ("TextVerbosityHigh", ModeTextVerbosity::HIGH),
    ] {
        assert_eq!(value.to_string(), constants[name].as_str().unwrap());
        let decoded: ModeTextVerbosity = from_go_json(&to_go_json_bytes(&value).unwrap()).unwrap();
        assert_eq!(decoded, value);
        assert!(std::collections::HashSet::from([value]).contains(&decoded));
    }
    let unknown: ModeReasoningLevel = from_go_json(br#"" HIGH ""#).unwrap();
    assert_eq!(&*unknown, " HIGH ");
    assert_ne!(unknown, ModeReasoningLevel::HIGH);
    assert_eq!(
        from_go_json::<ModeReasoningLevel>(b"null").unwrap(),
        ModeReasoningLevel::default()
    );
    assert_eq!(
        from_go_json::<ModeTextVerbosity>(b"null").unwrap(),
        ModeTextVerbosity::default()
    );
    assert_ne!(ModeReasoningLevel::NONE, ModeReasoningLevel::default());
}

fn compare_settings(case: &Value) {
    let id = case["id"].as_str().unwrap();
    let decoded = from_go_json::<ModelSettings>(&unhex(case["input_hex"].as_str().unwrap()));
    assert_eq!(
        decoded.is_ok(),
        case["decode_ok"].as_bool().unwrap(),
        "{id}: {decoded:?}"
    );
    let Ok(decoded) = decoded else { return };
    let state = settings_state(&decoded);
    assert_eq!(state, case["settings_state"], "unnormalized state: {id}");
    assert_eq!(case["marshal_ok"], true, "{id}");
    assert_eq!(
        to_go_json_bytes(&decoded).unwrap(),
        unhex(case["marshal_hex"].as_str().unwrap()),
        "exact bytes: {id}"
    );
}

#[test]
fn settings_preserve_pointer_zero_and_nil_slice_state() {
    let settings: ModelSettings =
        from_go_json(br#"{"temperature":-0,"parallel_tool_calls":false,"stop_sequences":[]}"#)
            .unwrap();
    assert_eq!(
        settings.temperature.unwrap().to_bits(),
        (-0.0_f64).to_bits()
    );
    assert_eq!(settings.parallel_tool_calls, Some(false));
    assert_eq!(settings.stop_sequences, Some(vec![]));
    assert_eq!(
        to_go_json_bytes(&settings).unwrap(),
        br#"{"temperature":-0,"parallel_tool_calls":false}"#
    );
    let settings: ModelSettings = from_go_json(
        br#"{"temperature":-0,"temperature":null,"stop_sequences":[],"stop_sequences":null}"#,
    )
    .unwrap();
    assert_eq!(settings, ModelSettings::default());
    assert_eq!(to_go_json_bytes(&settings).unwrap(), b"{}");
}

#[test]
fn settings_reject_nonfinite_encoding() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/mode-codec/observations.json"
    ))
    .unwrap();
    let cases = fixture["nonfinite"].as_array().unwrap();
    assert_eq!(cases.len(), 6);
    for case in cases {
        let value =
            f64::from_bits(u64::from_str_radix(case["bits"].as_str().unwrap(), 16).unwrap());
        assert!(!value.is_finite());
        let mut settings = ModelSettings::default();
        match case["field"].as_str().unwrap() {
            "Temperature" => settings.temperature = Some(value),
            "TopP" => settings.top_p = Some(value),
            other => panic!("unknown field {other}"),
        }
        assert_eq!(case["marshal_ok"], false);
        assert!(to_go_json_bytes(&settings).is_err());
    }
}

fn settings_state(decoded: &ModelSettings) -> Value {
    serde_json::json!({
        "Temperature": decoded.temperature.map(|v| format!("{:016x}", v.to_bits())),
        "MaxTokens": decoded.max_tokens,
        "TopP": decoded.top_p.map(|v| format!("{:016x}", v.to_bits())),
        "ToolChoice": decoded.tool_choice,
        "ParallelToolCalls": decoded.parallel_tool_calls,
        "ThinkingBudget": decoded.thinking_budget,
        "ReasoningEffort": decoded.reasoning_effort,
        "TextVerbosity": decoded.text_verbosity,
        "StopSequences": decoded.stop_sequences,
    })
}

fn compare_resolution(case: &Value) {
    let id = case["id"].as_str().unwrap();
    let decoded =
        from_go_json::<ModeRoutingResolution>(&unhex(case["input_hex"].as_str().unwrap()));
    assert_eq!(
        decoded.is_ok(),
        case["decode_ok"].as_bool().unwrap(),
        "{id}: {decoded:?}"
    );
    let Ok(decoded) = decoded else { return };
    let state = serde_json::json!({
        "Model": decoded.model,
        "FallbackModels": decoded.fallback_models,
        "ReasoningLevel": decoded.reasoning_level,
        "TextVerbosity": decoded.text_verbosity,
        "ModelSettings": settings_state(&decoded.model_settings),
    });
    assert_eq!(state, case["resolution_state"], "unnormalized state: {id}");
    assert_eq!(case["marshal_ok"], true, "{id}");
    assert_eq!(
        to_go_json_bytes(&decoded).unwrap(),
        unhex(case["marshal_hex"].as_str().unwrap()),
        "exact bytes: {id}"
    );
}

#[test]
fn nested_settings_null_is_noop_and_retains_backing_slots() {
    let resolution: ModeRoutingResolution = from_go_json(br#"{"Model":"raw","ModelSettings":{"temperature":-0,"stop_sequences":["old","tail"]},"ModelSettings":{"stop_sequences":["short"]},"ModelSettings":null,"ModelSettings":{"stop_sequences":[null,null,null]}}"#).unwrap();
    assert_eq!(
        resolution.model_settings.temperature.unwrap().to_bits(),
        (-0.0_f64).to_bits()
    );
    assert_eq!(
        resolution.model_settings.stop_sequences.as_ref().unwrap(),
        &["short", "tail", ""]
    );
    assert_eq!(to_go_json_bytes(&resolution).unwrap(), br#"{"Model":"raw","FallbackModels":null,"ReasoningLevel":"","TextVerbosity":"","ModelSettings":{"temperature":-0,"stop_sequences":["short","tail",""]}}"#);
}

#[test]
fn settings_merge_preserves_pinned_go_state_and_bytes() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/mode-codec/observations.json"
    ))
    .unwrap();
    let cases = fixture["settings_merges"].as_array().unwrap();
    assert_eq!(cases.len(), 534);
    for case in cases {
        let base: ModelSettings = from_go_json(case["base"].as_str().unwrap().as_bytes()).unwrap();
        let overrides: ModelSettings =
            from_go_json(&unhex(case["override_hex"].as_str().unwrap())).unwrap();
        let merged = base.merge(&overrides);
        assert_eq!(settings_state(&merged), case["state"], "{case}");
        assert_eq!(
            to_go_json_bytes(&merged).unwrap(),
            unhex(case["marshal_hex"].as_str().unwrap()),
            "{case}"
        );
    }
}

#[test]
fn settings_merge_copies_override_stops_and_retains_empty_base_allocation() {
    let base: ModelSettings = from_go_json(br#"{"stop_sequences":[]}"#).unwrap();
    assert_eq!(
        base.merge(&ModelSettings::default()).stop_sequences,
        Some(vec![])
    );
    let mut overrides: ModelSettings = from_go_json(br#"{"stop_sequences":["override"]}"#).unwrap();
    let merged = base.merge(&overrides);
    overrides.stop_sequences.as_mut().unwrap()[0].clear();
    assert_eq!(merged.stop_sequences.unwrap(), vec!["override"]);
}

#[test]
fn retry_advice_preserves_signed_delay_and_go_field_spelling() {
    let advice: ModelRetryAdvice = from_go_json(br#"{"ShouldRetry":true,"RetryAfterMS":-1,"Reason":" raw ","shouldretry":null,"retryafterms":null,"reason":null}"#).unwrap();
    assert_eq!(advice.retry_after_ms, -1);
    assert_eq!(
        to_go_json_bytes(&advice).unwrap(),
        br#"{"ShouldRetry":true,"RetryAfterMS":-1,"Reason":" raw "}"#
    );
}
