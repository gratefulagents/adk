#![cfg(feature = "observability")]
use adk::progress::*;

#[test]
fn pinned_progress_helpers_match_independent_go_observations() {
    use serde_json::{Value, json};
    let fixture: Value =
        serde_json::from_str(include_str!("../../../fixtures/progress/helpers.json")).unwrap();
    let cases = fixture["cases"].as_array().expect("source helper cases");
    assert!(cases.len() >= 200);
    for case in cases {
        let input: Vec<u8> = if let Some(bytes) = case["input_bytes"].as_array() {
            bytes
                .iter()
                .map(|byte| u8::try_from(byte.as_u64().unwrap()).unwrap())
                .collect()
        } else {
            case["input"]
                .as_str()
                .unwrap_or_default()
                .as_bytes()
                .to_vec()
        };
        let limit = case["limit"].as_i64().unwrap();
        let actual = match case["function"].as_str().unwrap() {
            "ExtractFilePath" => json!(extract_file_path(&input)),
            "ExtractPath" => json!(extract_path(&input)),
            "ExtractGrepPattern" => json!(extract_grep_pattern(&input)),
            "ExtractGlobPattern" => json!(extract_glob_pattern(&input)),
            "ExtractBashCommand" => json!(extract_bash_command(&input)),
            "ExtractAskUserQuestion" => json!(extract_ask_user_question(&input)),
            "ExtractAgentMeta" => json!(extract_agent_meta(&input)),
            "ExtractLSPOperation" => {
                let (operation, path) = extract_lsp_operation(&input);
                json!({"operation": operation, "file_path": path})
            }
            "Truncate" => json!(truncate(std::str::from_utf8(&input).unwrap(), limit)),
            "TruncateBytes" => json!(truncate_bytes(std::str::from_utf8(&input).unwrap(), limit)),
            "TruncateMiddle" => json!(truncate_middle(std::str::from_utf8(&input).unwrap(), limit)),
            other => panic!("unhandled source helper {other}"),
        };
        assert_eq!(actual, case["expected"], "{}", case["name"]);
    }
}

#[test]
fn tool_extractors_preserve_order_null_and_go_error_rules() {
    assert_eq!(
        extract_file_path(br#"{"FILE_PATH":"first","file_path":null,"file_PATH":"last"}"#),
        "last"
    );
    assert_eq!(
        extract_file_path(br#"{"file_path":false,"FILE_PATH":"last"}"#),
        ""
    );
    assert_eq!(
        extract_lsp_operation(br#"{"operation":false,"filePath":"a","OPERATION":"hover"}"#),
        ("hover".into(), "a".into())
    );
    assert_eq!(
        extract_lsp_operation(br#"{"operation":"hover","filePath":4}"#),
        ("hover".into(), String::new())
    );
    assert_eq!(
        extract_lsp_operation(br#"{"operation":"hover",!}"#),
        (String::new(), String::new())
    );
    assert_eq!(extract_path(br#"{"path":"a","unknown":1e999}"#), "a");
    assert_eq!(extract_path(br#"{"path":"\ud800"}"#), "\u{fffd}");
    assert_eq!(extract_path(b"{\"path\":\"\xff\xff\"}"), "\u{fffd}\u{fffd}");
    assert_eq!(extract_agent_meta(b"null"), AgentMeta::default());
    assert_eq!(
        extract_agent_meta(br#"{"description":2,"prompt":"valid"}"#).description,
        "unknown"
    );
    assert_eq!(
        extract_agent_meta(br#"{"prompt":"  hello\nworld  "}"#).description,
        "hello world"
    );
    assert_eq!(
        extract_agent_meta("{\"deſcription\":\"ok\"}".as_bytes()).description,
        "ok"
    );
}

#[test]
fn extractor_unknown_nesting_obeys_go_limit_without_value_recursion() {
    for (depth, expected) in [(200, "a"), (9999, "a"), (10000, "")] {
        let raw = format!(
            "{{\"path\":\"a\",\"unused\":{}0{}}}",
            "[".repeat(depth),
            "]".repeat(depth)
        );
        assert_eq!(extract_path(raw.as_bytes()), expected, "depth={depth}");
    }
}

#[test]
fn sdk_text_limits_keep_utf8_and_document_marker_reservation() {
    assert_eq!(truncate("  a\nb  ", 0), "a b");
    assert_eq!(truncate("αβγδε", 3), "αβγ");
    assert_eq!(truncate("αβγδε", 4), "α...");
    assert_eq!(truncate_bytes("αβγδε", 3), "α");
    assert_eq!(truncate_bytes("αβγδε", 4), "...");
    assert_eq!(truncate_bytes("αβγδε", 5), "α...");
    let text = "α".repeat(100);
    assert_eq!(truncate_middle(&text, 0), text);
    assert_eq!(truncate_middle(&text, 35), "α".repeat(35));
    let result = truncate_middle(&text, 50);
    assert!(result.starts_with("ααα"));
    assert!(result.ends_with("ααα"));
    assert!(result.contains("[elided "));
    assert!(result.chars().count() <= 50);
}
