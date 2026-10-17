use adk_codec::{
    GoJsonDecode, GoJsonEncode,
    dto::{ModeConstraints, ModeRoleRouting, ModeRouting, ModeTemplateSpec},
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
    assert_eq!(cases.len(), 993);
    for case in cases {
        match case["type"].as_str().unwrap() {
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
