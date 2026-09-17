#![cfg(any(target_os = "linux", target_os = "macos"))]
use adk_core::*;
use adk_tools::{Config, Features, Registry, skills};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    sync::Arc,
};
fn registry(root: &Path, access: AccessMode) -> Registry {
    let entries =
        serde_json::from_str(include_str!("../../../fixtures/tools/skill-catalog.json")).unwrap();
    Registry::build(
        &Config {
            access,
            features: Features::Strict(["ExtraTools".into()].into()),
            ..Default::default()
        },
        skills::tools(entries, root.into(), BTreeSet::new()),
    )
    .unwrap()
}
fn context(root: &Path) -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "skills".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: root.into(),
        policy: Default::default(),
        idempotency_key: None,
    }
}
async fn call(root: &Path, name: &str, args: Value) -> ToolOutput {
    registry(root, AccessMode::WorkspaceWrite)
        .get(name)
        .unwrap()
        .execute(
            &context(root),
            ToolCall {
                id: "call".into(),
                name: name.into(),
                arguments: args,
            },
        )
        .await
        .unwrap()
}
fn text(output: &ToolOutput) -> &str {
    assert!(!output.should_pause);
    match &output.content[..] {
        [Content::Text { text }] => text,
        _ => panic!("text"),
    }
}
#[tokio::test]
async fn search_preserves_order_query_precedence_and_environment_status() {
    let root = tempfile::tempdir().unwrap();
    let all = call(root.path(), "skill_search", json!({})).await;
    assert!(text(&all).starts_with("Found 3 skill(s):"));
    assert!(text(&all).contains("• **demo** (v1.2.3, Search ✓)"));
    assert!(text(&all).contains("DEMO_TOKEN (NOT set)"));
    let query = call(
        root.path(),
        "skill_search",
        json!({"query":"LOOKUP","category":"absent"}),
    )
    .await;
    assert!(text(&query).starts_with("Found 1 skill(s):"));
    let category = call(root.path(), "skill_search", json!({"category":"SEARCH"})).await;
    assert_eq!(text(&query), text(&category));
    assert_eq!(
        text(&call(root.path(), "skill_search", json!({"query":"missing"})).await),
        "No skills found matching your criteria."
    );
    assert!(
        call(root.path(), "skill_search", json!({"query":1}))
            .await
            .is_error
    );
}
#[tokio::test]
async fn install_merges_hardening_and_lists_without_starting_servers() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(".mcp.json");
    fs::write(&path,serde_json::to_vec(&json!({"mcpServers":{"demo":{"command":"old","env":{"DEFAULT":"existing","CUSTOM":"preserved"},"allowEnv":["OTHER","DEMO_TOKEN","OTHER",""],"allowedTools":["lookup"],"enabled":false,"trustReadOnlyHint":true},"other":{"command":"other"}}})).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    let result = call(root.path(), "skill_install", json!({"name":"demo"})).await;
    assert!(!result.is_error, "{result:?}");
    assert!(text(&result).contains("NOT available in this session"));
    assert!(
        text(&result).contains("WARNING: required environment variable(s) not set: DEMO_TOKEN")
    );
    let config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(
        config["mcpServers"]["demo"],
        json!({"type":"stdio","command":"demo-server","args":["serve"],"env":{"DEFAULT":"catalog","CUSTOM":"preserved"},"allowEnv":["OTHER","DEMO_TOKEN"],"allowedTools":["lookup"],"enabled":false})
    );
    assert_eq!(config["mcpServers"]["other"]["command"], "other");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let result = call(root.path(), "skill_list_installed", json!(["ignored"])).await;
    assert_eq!(
        text(&result),
        "Installed skills: demo\nOther MCP servers in .mcp.json (not from the skill catalog): other"
    );
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
}
#[tokio::test]
async fn absence_errors_confinement_and_read_only_surface() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        text(&call(root.path(), "skill_list_installed", Value::Null).await),
        "No skills currently installed in .mcp.json."
    );
    assert!(
        call(root.path(), "skill_install", json!({"name":"unknown"}))
            .await
            .is_error
    );
    assert!(!root.path().join(".mcp.json").exists());
    let readonly = registry(root.path(), AccessMode::ReadOnly);
    assert!(readonly.get("skill_search").is_some());
    assert!(readonly.get("skill_install").is_some());
    assert!(
        !readonly
            .prepare(ToolPolicy::default())
            .tools
            .iter()
            .any(|tool| tool.definition().name == "skill_install")
    );
    fs::write(root.path().join(".mcp.json"), "{broken").unwrap();
    assert!(
        call(root.path(), "skill_install", json!({"name":"demo"}))
            .await
            .is_error
    );
    assert_eq!(
        fs::read_to_string(root.path().join(".mcp.json")).unwrap(),
        "{broken"
    );
    fs::remove_file(root.path().join(".mcp.json")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("config"), "{}").unwrap();
    symlink(outside.path().join("config"), root.path().join(".mcp.json")).unwrap();
    assert!(
        call(root.path(), "skill_install", json!({"name":"demo"}))
            .await
            .is_error
    );
    assert!(
        call(root.path(), "skill_list_installed", json!({}))
            .await
            .is_error
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("config")).unwrap(),
        "{}"
    );
}
#[tokio::test]
async fn cancellation_and_host_environment_names_are_respected() {
    let root = tempfile::tempdir().unwrap();
    let token = Arc::new(adk_runtime::CancellationToken::new());
    token.cancel();
    let mut ctx = context(root.path());
    ctx.operation.cancellation = token;
    assert!(
        registry(root.path(), AccessMode::WorkspaceWrite)
            .get("skill_install")
            .unwrap()
            .execute(
                &ctx,
                ToolCall {
                    id: "call".into(),
                    name: "skill_install".into(),
                    arguments: json!({"name":"demo"})
                }
            )
            .await
            .is_err()
    );
    assert!(!root.path().join(".mcp.json").exists());
    let entries =
        serde_json::from_str(include_str!("../../../fixtures/tools/skill-catalog.json")).unwrap();
    let tools = skills::tools(entries, root.path().into(), ["DEMO_TOKEN".into()].into());
    let ctx = context(Path::new(""));
    let output = tools
        .iter()
        .find(|tool| tool.definition().name == "skill_install")
        .unwrap()
        .execute(
            &ctx,
            ToolCall {
                id: "call".into(),
                name: "skill_install".into(),
                arguments: json!({"name":"demo"}),
            },
        )
        .await
        .unwrap();
    assert!(!output.is_error, "{output:?}");
    assert!(!text(&output).contains("WARNING"));
    assert!(root.path().join(".mcp.json").is_file());
}
