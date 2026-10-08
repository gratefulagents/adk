use super::*;
use adk::mcp::{
    client::{HostPolicy, ServerPolicy},
    config::ConnectionConfig,
};
use std::{
    collections::BTreeSet,
    path::Path,
    process::{Command, Stdio},
};

fn input(dir: &Path, names: &[(&str, &str)]) -> McpInput {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/mcp/builder-peer.py");
    let servers: serde_json::Map<String, serde_json::Value> = names.iter().map(|(name, behavior)| (
        (*name).into(), json!({"command":"/usr/bin/python3","args":["-u",script,dir.join(name),behavior],"trustReadOnlyHint":true})
    )).collect();
    McpInput::new(
        ConnectionConfig::inline(serde_json::from_value(json!({"mcpServers":servers})).unwrap())
            .unwrap(),
        HostPolicy {
            tenant_id: "offline".into(),
            servers: names
                .iter()
                .map(|(name, _)| {
                    (
                        (*name).into(),
                        ServerPolicy {
                            enabled: true,
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            ..Default::default()
        },
    )
}
fn config(dir: &Path, mcp: McpFeatures) -> Config {
    Config {
        work_dir: dir.into(),
        features: Some(Features {
            mcp,
            ..Default::default()
        }),
        ..Default::default()
    }
}
fn enabled() -> McpFeatures {
    McpFeatures {
        enabled: true,
        allow_all_servers: true,
        allow_all_tools: true,
        resource_tools: true,
        ..Default::default()
    }
}
async fn pid(dir: &Path, server: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(value) = std::fs::read_to_string(dir.join(format!("{server}.pid"))) {
                if value.ends_with('\n') {
                    break value.trim().to_owned();
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
async fn reaped(pid: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while Command::new("/bin/kill")
            .args(["-0", pid])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}
fn operation(dir: &Path, policy: ToolPolicy) -> ToolContext {
    ToolContext {
        operation: context(),
        work_dir: dir.into(),
        policy,
        idempotency_key: None,
    }
}
fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        raw_arguments: None,
        id: "offline".into(),
        name: name.into(),
        arguments,
    }
}
struct Extra(ToolDefinition);
impl Tool for Extra {
    fn definition(&self) -> &ToolDefinition {
        &self.0
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async { panic!("unselected extra tool executed") })
    }
}
fn extra(name: &str) -> Arc<dyn Tool> {
    Arc::new(Extra(ToolDefinition {
        name: name.into(),
        description: String::new(),
        input_schema: json!({"type":"object"}).try_into().unwrap(),
        read_only: true,
        requires_approval: false,
    }))
}

#[tokio::test]
async fn runtime_selection_matches_pinned_sdk_builder_observations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/mcp");
    let inputs: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("runtime-inputs.json")).unwrap()).unwrap();
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("runtime-observations.json")).unwrap())
            .unwrap();
    let mut actual = vec![];
    for case in inputs.as_array().unwrap() {
        let flag = |name: &str| case[name].as_bool().unwrap_or(false);
        let names = |name: &str| -> BTreeSet<String> {
            case[name]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|v| v.as_str().unwrap().to_owned())
                        .collect()
                })
                .unwrap_or_default()
        };
        let features = McpFeatures {
            enabled: flag("enabled"),
            allow_all_servers: flag("all_servers"),
            allowed_servers: names("servers"),
            allow_all_tools: flag("all_tools"),
            allowed_tools: names("tools"),
            resource_tools: flag("resources"),
        };
        let dir = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel::default());
        let mut bundle = builder(config(dir.path(), features), &model)
            .mcp(input(dir.path(), &[("chosen", "ok"), ("other", "ok")]))
            .build(&context())
            .await
            .unwrap();
        let mut tools: Vec<_> = bundle
            .agent()
            .tools
            .iter()
            .map(|tool| tool.definition().name.clone())
            .collect();
        tools.sort();
        let catalog: Vec<_> = bundle.mcp_catalog().into_iter().map(|entry| json!({"name":entry.definition.name,"server":entry.server_name,"raw":entry.tool_name})).collect();
        assert_eq!(
            bundle.agent().mcp_servers,
            bundle.mcp_servers().keys().cloned().collect::<Vec<_>>()
        );
        actual.push(json!({"name":case["name"],"servers":bundle.mcp_servers().keys().cloned().collect::<Vec<_>>(),"tools":tools,"catalog":catalog}));
        bundle.close().await.unwrap();
    }
    assert_eq!(serde_json::Value::Array(actual), expected["cases"]);
}

#[tokio::test]
async fn selected_raw_and_final_names_route_without_enabling_extra_tools() {
    for selected in ["a b", "mcp__chosen__a_b_2"] {
        let dir = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel::default());
        let features = McpFeatures {
            enabled: true,
            allowed_servers: BTreeSet::from(["chosen".into()]),
            allowed_tools: BTreeSet::from([selected.into()]),
            ..Default::default()
        };
        let mut bundle = builder(config(dir.path(), features), &model)
            .mcp(input(
                dir.path(),
                &[("chosen", "ok"), ("unselected", "fail")],
            ))
            .extra_tools([extra("unrelated")])
            .build(&context())
            .await
            .unwrap();
        assert_eq!(
            bundle
                .mcp_servers()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["chosen"]
        );
        assert_eq!(bundle.agent().mcp_servers, ["chosen"]);
        assert!(!dir.path().join("unselected.pid").exists());
        assert_eq!(bundle.mcp_catalog()[0].tool_name, "a b");
        assert_eq!(bundle.agent().tools.len(), 1);
        let tool = bundle.agent().tools[0].clone();
        assert_eq!(tool.definition().name, "mcp__chosen__a_b_2");
        let op = operation(dir.path(), bundle.policy().tools.clone());
        let output = tool
            .execute(&op, call(&tool.definition().name, json!({"arg":1})))
            .await
            .unwrap();
        let rendered = serde_json::to_string(&output).unwrap();
        assert!(rendered.contains("a b"));
        bundle
            .run(context(), vec![], Arc::new(TestHost))
            .await
            .unwrap();
        assert_eq!(model.requests.lock().unwrap()[0].tools.len(), 1);
        bundle
            .stream(context(), vec![], Arc::new(TestHost))
            .finish()
            .await
            .unwrap();
        assert!(model.requests.lock().unwrap().iter().all(|request| request.instructions.ends_with("# MCP Servers\n\nConnected MCP servers: chosen\n\nMCP tools are prefixed as mcp__<server>__<tool>.")));

        let process = pid(dir.path(), "chosen").await;
        bundle.close().await.unwrap();
        bundle.close().await.unwrap();
        reaped(&process).await;
        assert!(
            tool.execute(&op, call(&tool.definition().name, json!({})))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn disabled_empty_and_absent_server_selections_have_no_connection_effects() {
    for features in [
        McpFeatures::default(),
        McpFeatures {
            enabled: true,
            ..Default::default()
        },
        McpFeatures {
            enabled: true,
            allow_all_servers: true,
            ..Default::default()
        },
        McpFeatures {
            enabled: true,
            allow_all_tools: true,
            ..Default::default()
        },
        McpFeatures {
            enabled: true,
            allowed_servers: BTreeSet::from(["absent".into()]),
            allow_all_tools: true,
            ..Default::default()
        },
    ] {
        let dir = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel::default());
        let mut bundle = builder(config(dir.path(), features), &model)
            .mcp(input(dir.path(), &[("chosen", "fail")]))
            .build(&context())
            .await
            .unwrap();
        assert!(bundle.agent().tools.is_empty());
        assert!(bundle.mcp_servers().is_empty());
        assert!(!dir.path().join("chosen.pid").exists());
        bundle.close().await.unwrap();
    }
}

#[tokio::test]
async fn all_selected_authority_and_bounds_are_checked_before_any_connection() {
    for failure in ["grant", "count", "zero", "remote"] {
        let dir = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel::default());
        let mut input = input(dir.path(), &[("a", "ok"), ("b", "ok")]);
        match failure {
            "grant" => {
                input.host_policy.servers.remove("b");
            }
            "count" => input.max_servers = 1,
            "zero" => input.max_catalog_items = 0,
            "remote" => {
                input.config = ConnectionConfig::inline(
                    serde_json::from_value(json!({"mcpServers":{
                        "a":input.config.config().server("a").unwrap(),
                        "b":{"type":"streamable-http","url":"https://example.com/mcp"}
                    }}))
                    .unwrap(),
                )
                .unwrap();
                input
                    .host_policy
                    .servers
                    .get_mut("b")
                    .unwrap()
                    .allowed_origins
                    .insert("https://example.com".into());
            }
            _ => unreachable!(),
        }
        assert!(
            builder(config(dir.path(), enabled()), &model)
                .mcp(input)
                .build(&context())
                .await
                .is_err()
        );
        assert!(!dir.path().join("a.pid").exists());
        assert!(!dir.path().join("b.pid").exists());
    }
}

#[tokio::test]
async fn composed_mcp_tools_obey_readonly_specialist_and_prepared_policy() {
    let dir = tempfile::tempdir().unwrap();
    let model = Arc::new(RecordingModel::default());
    let mut config = config(dir.path(), enabled());
    config.roles = vec![RoleSpec {
        name: "reader".into(),
        instructions: "Inspect without changing state.".into(),
        tool_access: "read-only".into(),
        ..Default::default()
    }];
    config.features.as_mut().unwrap().handoffs = true;
    let mut bundle = builder(config, &model)
        .mcp(input(dir.path(), &[("chosen", "ok")]))
        .build(&context())
        .await
        .unwrap();
    assert!(
        bundle
            .agent()
            .tools
            .iter()
            .any(|tool| tool.definition().name == "mcp__chosen__write")
    );
    let target = bundle.specialists().get("reader").unwrap();
    assert!(
        !target
            .tools
            .iter()
            .any(|tool| tool.definition().name == "mcp__chosen__write")
    );
    assert!(
        target
            .tools
            .iter()
            .any(|tool| tool.definition().name == "mcp__chosen__a_b")
    );
    bundle.close().await.unwrap();
}

#[tokio::test]
async fn restrictive_tool_grants_do_not_hide_raw_catalog_budget_usage() {
    let dir = tempfile::tempdir().unwrap();
    let model = Arc::new(RecordingModel::default());
    let mut input = input(dir.path(), &[("a", "ok"), ("b", "ok")]);
    input.max_catalog_items = 4;
    for grant in input.host_policy.servers.values_mut() {
        grant.allowed_tools = Some(BTreeSet::from(["a b".into()]));
    }
    assert!(
        builder(config(dir.path(), enabled()), &model)
            .mcp(input)
            .build(&context())
            .await
            .is_err()
    );
    for name in ["a", "b"] {
        reaped(&pid(dir.path(), name).await).await;
    }
}

#[tokio::test]
async fn scoped_resources_share_budget_and_restrictive_grants_still_charge_raw_entries() {
    for restricted in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel::default());
        let names = if restricted {
            vec![("a", "resources_many")]
        } else {
            vec![("a", "ok"), ("b", "ok")]
        };
        let mut input = input(dir.path(), &names);
        input.max_catalog_items = if restricted { 6 } else { 7 };
        if restricted {
            input
                .host_policy
                .servers
                .get_mut("a")
                .unwrap()
                .allowed_resources = Some(BTreeSet::from(["test://0".into()]));
        }
        let mut bundle = builder(
            config(
                dir.path(),
                McpFeatures {
                    allow_all_tools: false,
                    ..enabled()
                },
            ),
            &model,
        )
        .mcp(input)
        .build(&context())
        .await
        .unwrap();
        let list = bundle
            .agent()
            .tools
            .iter()
            .find(|tool| tool.definition().name == "ListMcpResourcesTool")
            .unwrap();
        let op = operation(dir.path(), bundle.policy().tools.clone());
        if restricted {
            assert!(
                list.execute(&op, call("ListMcpResourcesTool", json!({"server":"a"})))
                    .await
                    .is_err()
            );
        } else {
            list.execute(&op, call("ListMcpResourcesTool", json!({"server":"a"})))
                .await
                .unwrap();
            assert!(
                list.execute(&op, call("ListMcpResourcesTool", json!({"server":"b"})))
                    .await
                    .is_err()
            );
            list.execute(&op, call("ListMcpResourcesTool", json!({"server":"a"})))
                .await
                .unwrap();
        }
        bundle.close().await.unwrap();
    }
}

#[tokio::test]
async fn composed_resource_catalog_obeys_aggregate_bound() {
    let dir = tempfile::tempdir().unwrap();
    let model = Arc::new(RecordingModel::default());
    let mut input = input(dir.path(), &[("chosen", "resources_many")]);
    input.max_catalog_items = 3;
    let mut bundle = builder(
        config(
            dir.path(),
            McpFeatures {
                allow_all_tools: false,
                ..enabled()
            },
        ),
        &model,
    )
    .mcp(input)
    .build(&context())
    .await
    .unwrap();
    let list = bundle
        .agent()
        .tools
        .iter()
        .find(|tool| tool.definition().name == "ListMcpResourcesTool")
        .unwrap();
    assert!(
        list.execute(
            &operation(dir.path(), bundle.policy().tools.clone()),
            call("ListMcpResourcesTool", json!({}))
        )
        .await
        .is_err()
    );
    bundle.close().await.unwrap();
}

#[tokio::test]
async fn resources_only_selection_and_owner_drop_revoke_retained_handles() {
    let dir = tempfile::tempdir().unwrap();
    let model = Arc::new(RecordingModel::default());
    let bundle = builder(
        config(
            dir.path(),
            McpFeatures {
                allow_all_tools: false,
                ..enabled()
            },
        ),
        &model,
    )
    .mcp(input(dir.path(), &[("chosen", "ok")]))
    .build(&context())
    .await
    .unwrap();
    assert!(bundle.mcp_catalog().is_empty());
    assert_eq!(bundle.agent().tools.len(), 2);
    let list = bundle
        .agent()
        .tools
        .iter()
        .find(|t| t.definition().name == "ListMcpResourcesTool")
        .unwrap()
        .clone();
    let read = bundle
        .agent()
        .tools
        .iter()
        .find(|t| t.definition().name == "ReadMcpResourceTool")
        .unwrap()
        .clone();
    let op = operation(dir.path(), bundle.policy().tools.clone());
    list.execute(&op, call("ListMcpResourcesTool", json!({})))
        .await
        .unwrap();
    let output = read
        .execute(
            &op,
            call(
                "ReadMcpResourceTool",
                json!({"server":"chosen","uri":"test://allowed"}),
            ),
        )
        .await
        .unwrap();
    assert!(serde_json::to_string(&output).unwrap().contains("resource"));
    assert!(
        read.execute(
            &op,
            call(
                "ReadMcpResourceTool",
                json!({"server":"chosen","uri":"test://denied"})
            )
        )
        .await
        .is_err()
    );
    let process = pid(dir.path(), "chosen").await;
    drop(bundle);
    assert!(
        list.execute(&op, call("ListMcpResourcesTool", json!({})))
            .await
            .is_err()
    );
    reaped(&process).await;
}

#[tokio::test]
async fn connection_discovery_and_registry_failures_rollback_acquired_processes() {
    for failure in ["initialize", "catalog", "registry"] {
        let dir = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel::default());
        let mut input = input(
            dir.path(),
            &[
                ("a", "ok"),
                (
                    "b",
                    if failure == "initialize" {
                        "fail"
                    } else {
                        "ok"
                    },
                ),
            ],
        );
        if failure == "catalog" {
            input.max_catalog_items = 4;
        }
        let mut config = config(dir.path(), enabled());
        let mut extras = vec![];
        if failure == "registry" {
            config
                .features
                .as_mut()
                .unwrap()
                .tools
                .insert("ExtraTools".into());
            extras.push(extra("mcp__a__a_b"));
        }
        assert!(
            builder(config, &model)
                .mcp(input)
                .extra_tools(extras)
                .build(&context())
                .await
                .is_err()
        );
        for server in ["a", "b"] {
            reaped(&pid(dir.path(), server).await).await;
        }
    }
}

#[tokio::test]
async fn cancelled_and_timed_out_multiserver_builds_cleanup_every_acquired_process() {
    for timeout in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let model = Arc::new(RecordingModel::default());
        let mut input = input(dir.path(), &[("a", "ok"), ("b", "hang")]);
        if timeout {
            input.build_timeout = Duration::from_millis(500);
        }
        let builder = builder(config(dir.path(), enabled()), &model).mcp(input);
        let building = tokio::spawn(async move { builder.build(&context()).await });
        let a = pid(dir.path(), "a").await;
        let b = pid(dir.path(), "b").await;
        if !timeout {
            building.abort();
        }
        let result = building.await;
        if timeout {
            assert!(result.unwrap().is_err());
        } else {
            assert!(result.is_err());
        }
        reaped(&a).await;
        reaped(&b).await;
    }
}

#[tokio::test]
async fn tool_only_selection_matches_pinned_sdk_builder_observations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/mcp");
    let inputs: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("runtime-inputs.json")).unwrap()).unwrap();
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("runtime-observations.json")).unwrap())
            .unwrap();
    let mut actual = vec![];
    for case in inputs.as_array().unwrap() {
        let flag = |name: &str| case[name].as_bool().unwrap_or(false);
        let names = |name: &str| -> BTreeSet<String> {
            case[name]
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .map(|v| v.as_str().unwrap().to_owned())
                        .collect()
                })
                .unwrap_or_default()
        };
        let features = McpFeatures {
            enabled: flag("enabled"),
            allow_all_servers: flag("all_servers"),
            allowed_servers: names("servers"),
            allow_all_tools: flag("all_tools"),
            allowed_tools: names("tools"),
            resource_tools: flag("resources"),
        };
        let dir = tempfile::tempdir().unwrap();
        let mut bundle = Builder::new(config(dir.path(), features))
            .mcp(input(dir.path(), &[("chosen", "ok"), ("other", "ok")]))
            .build_tools(&context())
            .await
            .unwrap();
        let mut tools: Vec<_> = bundle
            .prepared()
            .tools
            .iter()
            .map(|tool| tool.definition().name.clone())
            .collect();
        tools.sort();
        let catalog: Vec<_> = bundle.mcp_catalog().into_iter().map(|entry| json!({"name":entry.definition.name,"server":entry.server_name,"raw":entry.tool_name})).collect();
        actual.push(json!({"name":case["name"],"servers":bundle.mcp_servers().keys().cloned().collect::<Vec<_>>(),"tools":tools,"catalog":catalog}));
        bundle.close().await.unwrap();
    }
    assert_eq!(serde_json::Value::Array(actual), expected["cases"]);
}

#[tokio::test]
async fn tool_only_owner_closes_mcp_and_revokes_retained_tools() {
    let dir = tempfile::tempdir().unwrap();
    let mut bundle = Builder::new(config(dir.path(), enabled()))
        .mcp(input(dir.path(), &[("chosen", "ok")]))
        .build_tools(&context())
        .await
        .unwrap();
    let prepared = bundle.prepared();
    let tool = prepared
        .tools
        .iter()
        .find(|t| t.definition().name == "mcp__chosen__a_b")
        .unwrap()
        .clone();
    let op = operation(dir.path(), prepared.policy);
    let process = pid(dir.path(), "chosen").await;
    bundle.close().await.unwrap();
    assert_eq!(
        category(tool.execute(&op, call("mcp__chosen__a_b", json!({}))).await),
        ErrorCategory::Cancelled
    );
    reaped(&process).await;
}
