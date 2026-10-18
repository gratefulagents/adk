use adk_codec::text::{sanitize_mcp_display, sanitize_mcp_tool_description};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[test]
fn mcp_display_bytes_and_unicode_match_pinned_go() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/mcp-display/observations.json"
    ))
    .unwrap();
    assert_eq!(
        fixture["provenance"]["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 160);
    let mut split_utf8 = false;
    for case in fixture["cases"].as_array().unwrap() {
        let actual = sanitize_mcp_display(
            case["server"].as_str().unwrap(),
            case["body"].as_str().unwrap(),
        );
        let expected: Vec<u8> = serde_json::from_value(case["output"].clone()).unwrap();
        assert_eq!(actual, expected, "{case}");
        let description = sanitize_mcp_tool_description(
            case["server"].as_str().unwrap(),
            case["tool"].as_str().unwrap(),
            case["body"].as_str().unwrap(),
        );
        let expected_description: Vec<u8> =
            serde_json::from_value(case["toolOutput"].clone()).unwrap();
        assert_eq!(description, expected_description, "{case}");
        split_utf8 |= std::str::from_utf8(&actual).is_err();
    }
    assert!(split_utf8);
    let mut hash = Sha256::new();
    let mut count = 0;
    for ch in (0..=0x10ffff).filter_map(char::from_u32) {
        hash.update(sanitize_mcp_display("s", &format!("x{ch}y")));
        hash.update(sanitize_mcp_tool_description(
            "s",
            &format!("t{ch}"),
            &format!("x{ch}y"),
        ));
        count += 1;
    }
    assert_eq!(count, fixture["scalarCount"]);
    assert_eq!(format!("{:x}", hash.finalize()), fixture["scalarSHA256"]);
}
