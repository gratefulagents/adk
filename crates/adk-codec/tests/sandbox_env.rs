use std::collections::BTreeMap;

fn decode(text: &str) -> Vec<u8> {
    assert_eq!(text.len() % 2, 0);
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn explicit_environment_expansion_matches_pinned_go() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/sandbox-env/observations.json"
    ))
    .unwrap();
    let environment: BTreeMap<Vec<u8>, Vec<u8>> = fixture["environment"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| (decode(key), decode(value.as_str().unwrap())))
        .collect();
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 540);
    for case in cases {
        let input = decode(case["input"].as_str().unwrap());
        assert_eq!(
            adk_codec::sandbox_env::expand_safe(&input, &environment),
            decode(case["output"].as_str().unwrap()),
            "{input:?}"
        );
    }
    assert_eq!(
        adk_codec::sandbox_env::expand_safe(b"$OPENAI_API_KEY/${HOME}", &BTreeMap::new()),
        b"/"
    );
}

#[test]
fn sandbox_configuration_names_match_pinned_go_and_are_owned() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/sandbox-env/observations.json"
    ))
    .unwrap();
    let names = adk_codec::sandbox_env::config_env_names();
    assert_eq!(
        serde_json::to_value(&names).unwrap(),
        fixture["configEnvNames"]
    );
    assert_eq!(
        adk_codec::sandbox_env::SANDBOX_MODE_ENV,
        fixture["modeEnvName"]
    );
    assert!(
        !names
            .iter()
            .any(|name| name == adk_codec::sandbox_env::SANDBOX_MODE_ENV || name == "GOROOT")
    );
    let mut changed = names.clone();
    changed[0].clear();
    assert_eq!(adk_codec::sandbox_env::config_env_names(), names);
}

#[test]
fn posix_environment_projection_matches_pinned_go() {
    use adk_codec::sandbox_env::{EnvironmentConfig, safe_environment_map_posix};
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../fixtures/sandbox-env/observations.json"
    ))
    .unwrap();
    let map = |value: &serde_json::Value| -> BTreeMap<Vec<u8>, Vec<u8>> {
        value
            .as_object()
            .unwrap()
            .iter()
            .map(|(key, value)| (decode(key), decode(value.as_str().unwrap())))
            .collect()
    };
    let list = |value: &serde_json::Value| -> Vec<Vec<u8>> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|value| decode(value.as_str().unwrap()))
            .collect()
    };
    let cases = fixture["environmentCases"].as_array().unwrap();
    assert_eq!(cases.len(), 106);
    for (index, case) in cases.iter().enumerate() {
        let parent = map(&case["parent"]);
        let config = EnvironmentConfig {
            path: decode(case["config"]["path"].as_str().unwrap()),
            path_prepend: list(&case["config"]["prepend"]),
            path_append: list(&case["config"]["append"]),
            extra_env: map(&case["config"]["extra"]),
            go_root: decode(case["config"]["goRoot"].as_str().unwrap()),
        };
        let actual = safe_environment_map_posix(&parent, &config);
        assert_eq!(actual, map(&case["output"]), "case {index}");
        assert!(!actual.contains_key(b"OPENAI_API_KEY".as_slice()));
        let mut mutated = actual;
        mutated.clear();
        assert_eq!(
            safe_environment_map_posix(&parent, &config),
            map(&case["output"])
        );
    }
    assert_eq!(
        safe_environment_map_posix(&BTreeMap::new(), &EnvironmentConfig::default()),
        BTreeMap::from([(b"PATH".to_vec(), b"/usr/local/bin:/usr/bin:/bin".to_vec())])
    );
}
