use adk_codec::{
    GoJsonDecode, GoJsonEncode,
    dto::{RoleCatalog, RoleSpec},
    from_go_json, to_go_json_bytes,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
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
        "unnormalized decoded fields: {id}"
    );
    let encoded = to_go_json_bytes(&decoded);
    assert_eq!(
        encoded.is_ok(),
        case["marshal_ok"].as_bool().unwrap(),
        "marshal success: {id}"
    );
    assert_eq!(
        encoded.unwrap(),
        unhex(case["marshal_hex"].as_str().unwrap()),
        "exact Go bytes: {id}"
    );
}

#[test]
fn pinned_role_catalog_oracle() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/role-catalog/observations.json"
    ))
    .unwrap();
    assert_eq!(fixture["schema_version"], 1);
    assert_eq!(fixture["int_size"], 64);
    assert_eq!(fixture["catalog_kind"], "slice");
    assert_eq!(
        fixture["provenance"]["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert_eq!(
        fixture["role_fields"],
        json!([
            {"name":"Name","type":"string","json_tag":""},
            {"name":"Description","type":"string","json_tag":""},
            {"name":"Instructions","type":"string","json_tag":""},
            {"name":"ToolAccess","type":"string","json_tag":""},
            {"name":"ModelOverride","type":"string","json_tag":""},
            {"name":"FallbackModels","type":"[]string","json_tag":""}
        ])
    );
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (path, expected) in fixture["provenance"]["harnessSHA256"].as_object().unwrap() {
        let digest = Sha256::digest(std::fs::read(root.join(path)).unwrap());
        assert_eq!(format!("{digest:x}"), expected.as_str().unwrap(), "{path}");
    }
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 364);
    for case in cases {
        match case["type"].as_str().unwrap() {
            "RoleSpec" => compare::<RoleSpec>(case),
            "RoleCatalog" => compare::<RoleCatalog>(case),
            other => panic!("unknown oracle type: {other}"),
        }
    }
}

#[test]
fn nil_empty_and_null_elements_remain_distinct() {
    let nil: RoleSpec = from_go_json(b"{}").unwrap();
    let empty: RoleSpec = from_go_json(br#"{"FallbackModels":[]}"#).unwrap();
    let null_element: RoleSpec = from_go_json(br#"{"FallbackModels":[null]}"#).unwrap();
    assert_eq!(nil.fallback_models, None);
    assert_eq!(empty.fallback_models, Some(vec![]));
    assert_eq!(null_element.fallback_models, Some(vec![String::new()]));
    assert_eq!(to_go_json_bytes(&nil).unwrap(), br#"{"Name":"","Description":"","Instructions":"","ToolAccess":"","ModelOverride":"","FallbackModels":null}"#);
    assert_eq!(to_go_json_bytes(&RoleCatalog::default()).unwrap(), b"null");
    assert_eq!(to_go_json_bytes(&RoleCatalog(Some(vec![]))).unwrap(), b"[]");
    let catalog: RoleCatalog = from_go_json(b"[null,{}]").unwrap();
    assert_eq!(catalog.0.unwrap(), vec![nil.clone(), nil]);
}

#[test]
fn repeated_fallback_slices_reuse_slots_until_reset() {
    let role: RoleSpec = from_go_json(br#"{"Name":"keep","NAME":null,"FallbackModels":["a","b"],"fallbackmodels":["c"],"FALLBACKMODELS":[null,null,null]}"#).unwrap();
    assert_eq!(role.name, "keep");
    assert_eq!(
        role.fallback_models,
        Some(vec!["c".into(), "b".into(), "".into()])
    );
    for reset in ["null", "[]"] {
        let input = format!(
            r#"{{"FallbackModels":["old"],"FallbackModels":{reset},"FallbackModels":[null]}}"#
        );
        let role: RoleSpec = from_go_json(input.as_bytes()).unwrap();
        assert_eq!(role.fallback_models, Some(vec![String::new()]));
    }
}

#[test]
fn serde_in_place_preserves_null_scalars_and_catalog_slots() {
    let mut catalog = RoleCatalog(Some(vec![RoleSpec {
        name: "keep".into(),
        fallback_models: Some(vec!["model".into()]),
        ..Default::default()
    }]));
    RoleCatalog::deserialize_in_place(
        &mut serde_json::Deserializer::from_str(r#"[{"Name":null,"FallbackModels":[null]}]"#),
        &mut catalog,
    )
    .unwrap();
    let expected = catalog.clone();
    RoleCatalog::deserialize_in_place(
        &mut serde_json::Deserializer::from_str("[null]"),
        &mut catalog,
    )
    .unwrap();
    assert_eq!(catalog, expected);
    let role = &catalog.0.as_ref().unwrap()[0];
    assert_eq!(role.name, "keep");
    assert_eq!(role.fallback_models, Some(vec!["model".into()]));
    RoleCatalog::deserialize_in_place(
        &mut serde_json::Deserializer::from_str("null"),
        &mut catalog,
    )
    .unwrap();
    assert_eq!(catalog.0, None);
}

#[test]
fn byte_entry_point_replaces_strings_without_normalizing_role_values() {
    assert!(serde_json::from_str::<RoleSpec>(r#"{"Name":"\ud800"}"#).is_err());
    let role: RoleSpec = from_go_json(
        br#"{"Name":"\ud800","ToolAccess":" typo ","FallbackModels":["same","same",""]}"#,
    )
    .unwrap();
    assert_eq!(role.name, "�");
    assert_eq!(role.tool_access, " typo ");
    assert_eq!(
        role.fallback_models,
        Some(vec!["same".into(), "same".into(), "".into()])
    );
    let role: RoleSpec = from_go_json(b"{\"Name\":\"\xe2\x82\"}").unwrap();
    assert_eq!(role.name, "��");
}
