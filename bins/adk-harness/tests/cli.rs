use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn computed_outputs_match_all_go_goldens() {
    let result = Command::new(env!("CARGO_BIN_EXE_adk-harness"))
        .arg("--fixtures")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let actual: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    let mut count = 0;
    for text in [
        include_str!("../../../fixtures/sdk.json"),
        include_str!("../../../fixtures/platform.json"),
    ] {
        let document: serde_json::Value = serde_json::from_str(text).unwrap();
        for case in document["cases"].as_array().unwrap() {
            assert_eq!(actual[case["name"].as_str().unwrap()], case["expected"]);
            count += 1;
        }
    }
    assert_eq!(actual.as_object().unwrap().len(), count);
    assert_eq!(count, 7);
}

#[test]
fn stdin_protocol_reports_invalid_operations() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_adk-harness"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"operation":"not-implemented","input":null}"#)
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
}
