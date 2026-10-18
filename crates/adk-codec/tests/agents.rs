use adk_codec::{agents::*, from_go_json, to_go_json_bytes};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../fixtures/agent-kinds/observations.json"
    ))
    .unwrap()
}
fn bytes(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn governed_kind_catalog_and_resolution_match_pinned_go() {
    let fixture = fixture();
    assert_eq!(
        fixture["provenance"]["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert_eq!(
        serde_json::to_value(built_in_kinds()).unwrap(),
        fixture["kinds"]
    );
    assert_eq!(
        serde_json::to_value(definitions()).unwrap(),
        fixture["definitions"]
    );
    assert_eq!(
        built_in_kinds(),
        [
            Kind::EXPLORE,
            Kind::ANALYST,
            Kind::PLANNER,
            Kind::ARCHITECT,
            Kind::DEBUGGER,
            Kind::EXECUTOR,
            Kind::CRITIC,
            Kind::SECURITY_REVIEWER,
            Kind::DEPENDENCY_EXPERT,
            Kind::DESIGNER,
            Kind::WRITER
        ]
    );
    assert_eq!(fixture["resolved"].as_array().unwrap().len(), 312);
    assert_eq!(fixture["allowed"].as_array().unwrap().len(), 1122);
    for case in fixture["resolved"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        let definition = resolve_kind(input);
        assert_eq!(definition.is_some(), case["ok"], "{input:?}");
        assert_eq!(
            serde_json::to_value(definition.unwrap_or_default()).unwrap(),
            case["definition"],
            "{input:?}"
        );
        let parsed = parse_kind(input);
        assert_eq!(parsed.is_some(), case["parsed_ok"], "{input:?}");
        assert_eq!(
            parsed.unwrap_or_default().to_string(),
            case["parsed"],
            "{input:?}"
        );
    }
    for case in fixture["allowed"].as_array().unwrap() {
        let kind: Kind = serde_json::from_value(case["kind"].clone()).unwrap();
        let allowed: Option<Vec<String>> = serde_json::from_value(case["allowed"].clone()).unwrap();
        assert_eq!(
            is_allowed(&kind, allowed.as_deref().unwrap_or_default()),
            case["result"],
            "{case:?}"
        );
    }
}

#[test]
fn governed_kind_codecs_match_pinned_go_state_and_bytes() {
    let fixture = fixture();
    assert_eq!(fixture["codecs"].as_array().unwrap().len(), 126);
    for case in fixture["codecs"].as_array().unwrap() {
        let input = bytes(case["input_hex"].as_str().unwrap());
        let result = if case["type"] == "Kind" {
            from_go_json::<Kind>(&input)
                .map(|value| (json!(value.to_string()), to_go_json_bytes(&value)))
        } else {
            from_go_json::<Definition>(&input).map(|value| {
                (
                    json!({"kind":value.kind,"readOnly":value.read_only,"aliases":value.aliases}),
                    to_go_json_bytes(&value),
                )
            })
        };
        assert_eq!(result.is_ok(), case["ok"], "{case:?}");
        if let Ok((fields, encoded)) = result {
            assert_eq!(fields, case["fields"], "{case:?}");
            assert_eq!(encoded.is_ok(), case["marshal_ok"], "{case:?}");
            if let Ok(encoded) = encoded {
                assert_eq!(
                    encoded,
                    bytes(case["marshal_hex"].as_str().unwrap()),
                    "{case:?}"
                );
            }
        }
    }
}

#[test]
fn original_governed_kind_regressions() {
    let kinds = built_in_kinds();
    assert!(!kinds.is_empty());
    assert!(kinds.iter().all(|kind| !kind.is_empty()));
    for (input, expected) in [
        ("explore", Kind::EXPLORE),
        ("Explore", Kind::EXPLORE),
        ("Plan", Kind::PLANNER),
        ("security-reviewer", Kind::SECURITY_REVIEWER),
        ("general-purpose", Kind::EXECUTOR),
    ] {
        assert_eq!(resolve_kind(input).unwrap().kind, expected);
    }
    assert!(resolve_kind("unknown-role").is_none());
    assert!(is_allowed(
        &Kind::EXECUTOR,
        &["explore".into(), "general-purpose".into()]
    ));
    assert!(!is_allowed(
        &Kind::EXECUTOR,
        &["explore".into(), "security-reviewer".into()]
    ));
}

#[test]
fn governed_definitions_are_owned_and_unknown_kinds_remain_open() {
    let mut values = definitions();
    values[0].aliases.as_mut().unwrap()[0] = "changed".into();
    values[0].kind = Kind("custom".into());
    assert_eq!(resolve_kind("Explore").unwrap().kind, Kind::EXPLORE);
    assert!(resolve_kind("changed").is_none());
    let custom = Kind("custom".into());
    assert!(is_allowed(&custom, &[]));
    assert!(!is_allowed(&custom, &["custom".into()]));
    assert_eq!(
        from_go_json::<Kind>(&to_go_json_bytes(&custom).unwrap()).unwrap(),
        custom
    );
    assert_eq!(
        std::collections::HashSet::from([custom.clone()]).get(&custom),
        Some(&custom)
    );
    assert!(!is_allowed(&Kind("Explore".into()), &["explore".into()]));
}
