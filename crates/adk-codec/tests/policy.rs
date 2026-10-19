use adk_codec::policy::*;
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/policy/observations.json")).unwrap()
}

#[test]
fn legacy_policy_labels_match_pinned_go() {
    let fixture = fixture();
    assert_eq!(
        fixture["provenance"]["commit"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert_eq!(fixture["cases"].as_array().unwrap().len(), 496);
    for case in fixture["cases"].as_array().unwrap() {
        let input = case["input"].as_str().unwrap();
        assert_eq!(
            normalize_permission_mode(input),
            case["permission"],
            "{case}"
        );
        assert_eq!(normalize_git_remote_writes(input), case["git"], "{case}");
        assert_eq!(allows_write_tools(input), case["write"], "{case}");
        assert_eq!(allows_mcp_tool(input, false), case["mcp_write"], "{case}");
        assert_eq!(allows_mcp_tool(input, true), case["mcp_read"], "{case}");
        assert_eq!(
            tool_access_level_from_permission_mode(input),
            case["access"],
            "{case}"
        );
        assert_eq!(
            serde_json::to_value(tool_policy_from_permission_mode(input)).unwrap(),
            case["run_policy"],
            "{case}"
        );
        assert_eq!(
            ToolPolicy {
                permission_mode: input.into()
            }
            .allows_write_tools(),
            case["write"],
            "{case}"
        );
    }
}

#[test]
fn runtime_policy_normalization_preserves_optional_surface() {
    let fixture = fixture();
    assert_eq!(fixture["policies"].as_array().unwrap().len(), 960);
    for case in fixture["policies"].as_array().unwrap() {
        let raw = &case["input"];
        let input = RuntimePolicy {
            permission_mode: raw["PermissionMode"].as_str().unwrap().into(),
            git_remote_writes: raw["GitRemoteWrites"].as_str().unwrap().into(),
            enable_mcp: raw["EnableMCP"].as_bool().unwrap(),
            enable_hooks: raw["EnableHooks"].as_bool().unwrap(),
            enable_plugins: raw["EnablePlugins"].as_bool().unwrap(),
            allowed_agent_kinds: serde_json::from_value(raw["AllowedAgentKinds"].clone()).unwrap(),
            allowed_skills: serde_json::from_value(raw["AllowedSkills"].clone()).unwrap(),
        };
        let normalized = input.normalize();
        assert_eq!(
            serde_json::to_value(&normalized).unwrap(),
            case["normalized"],
            "{case}"
        );
        assert_eq!(
            serde_json::to_value(normalized.normalize()).unwrap(),
            case["twice"],
            "{case}"
        );
        assert_eq!(serde_json::to_value(&input).unwrap(), *raw);
        let tool = ToolPolicy::new(&input);
        assert_eq!(serde_json::to_value(&tool).unwrap(), case["tool"], "{case}");
        assert_eq!(tool.allows_write_tools(), case["write"], "{case}");
    }
    let defaults = RuntimePolicy::default().normalize();
    assert_eq!(defaults.permission_mode, PERMISSION_MODE_WORKSPACE_WRITE);
    assert_eq!(defaults.git_remote_writes, GIT_REMOTE_WRITES_ENABLED);
    assert!(!defaults.enable_mcp && !defaults.enable_hooks && !defaults.enable_plugins);
    assert_eq!(
        serde_json::to_value(ToolPolicy::default()).unwrap(),
        json!({"PermissionMode":""})
    );
    assert!(ToolPolicy::default().allows_write_tools());
}

#[test]
fn legacy_policy_whitespace_matches_every_go_scalar() {
    let fixture = fixture();
    let expected: Vec<u32> = serde_json::from_value(fixture["trimmed_scalars"].clone()).unwrap();
    let mut count = 0;
    for scalar in 0..=0x10ffff {
        let Some(character) = char::from_u32(scalar) else {
            continue;
        };
        count += 1;
        let text = character.to_string();
        assert_eq!(
            normalize_permission_mode(&text) == PERMISSION_MODE_WORKSPACE_WRITE,
            expected.contains(&scalar),
            "U+{scalar:04X}"
        );
        assert_eq!(
            normalize_git_remote_writes(&text) == GIT_REMOTE_WRITES_ENABLED,
            expected.contains(&scalar),
            "U+{scalar:04X}"
        );
    }
    assert_eq!(count, fixture["scalar_count"].as_u64().unwrap());
}
