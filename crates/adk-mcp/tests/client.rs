use adk_mcp::{
    BoxFuture, Error, Limits, PROTOCOL_VERSION, Transport,
    client::{
        AuditOutcome, Client, ClientManager, HostHooks, HostPolicy, OperationContext, ServerPolicy,
        normalize_input_schema,
    },
    config::ServerConfig,
    tools::ToolManager,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

type Log = Arc<Mutex<Vec<(String, Value)>>>;
struct Mock {
    replies: VecDeque<Result<Value, Error>>,
    log: Log,
    hang: bool,
}
impl Transport for Mock {
    fn diagnostics(&self) -> Option<String> {
        Some("bounded host-only peer diagnostic".into())
    }
    fn request<'a>(
        &'a mut self,
        method: &'a str,
        params: Value,
    ) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async move {
            self.log.lock().unwrap().push((method.into(), params));
            if self.hang && self.replies.is_empty() {
                std::future::pending::<()>().await;
            }
            self.replies.pop_front().expect("unexpected request")
        })
    }
    fn notify<'a>(
        &'a mut self,
        method: &'a str,
        params: Value,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.log.lock().unwrap().push((method.into(), params));
            Ok(())
        })
    }
    fn close(&mut self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            self.log.lock().unwrap().push(("close".into(), Value::Null));
            Ok(())
        })
    }
}
fn transport(replies: Vec<Result<Value, Error>>) -> (Box<dyn Transport>, Log) {
    let log = Log::default();
    (
        Box::new(Mock {
            replies: replies.into(),
            log: log.clone(),
            hang: false,
        }),
        log,
    )
}
fn handshake() -> Result<Value, Error> {
    Ok(
        json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{"tools":{},"resources":{},"prompts":{}}}),
    )
}
fn tool(name: &str, read_only: bool) -> Value {
    json!({"name":name,"description":"untrusted","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":read_only}})
}
fn config() -> ServerConfig {
    serde_json::from_value(json!({"command":"mock","trustReadOnlyHint":true})).unwrap()
}
fn policy() -> HostPolicy {
    HostPolicy {
        tenant_id: "tenant-a".into(),
        servers: BTreeMap::from([(
            "server".into(),
            ServerPolicy {
                enabled: true,
                ..Default::default()
            },
        )]),
        hooks: None,
    }
}
async fn client(replies: Vec<Result<Value, Error>>) -> (Client, Log) {
    let (transport, log) = transport([vec![handshake()], replies].concat());
    let mut client = Client::new(transport, "server", config(), policy()).unwrap();
    client.initialize().await.unwrap();
    (client, log)
}

#[tokio::test]
async fn initializes_once_negotiates_and_gates_capabilities() {
    let (mut client, log) = client(vec![]).await;
    assert!(
        client.capabilities().tools
            && client.capabilities().resources
            && client.capabilities().prompts
    );
    assert!(client.initialize().await.is_err());
    {
        let calls = log.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "initialize");
        assert_eq!(calls[0].1["capabilities"], json!({}));
        assert_eq!(calls[1].0, "notifications/initialized");
    }
    for result in [
        json!({"protocolVersion":"old","capabilities":{}}),
        json!({"protocolVersion":PROTOCOL_VERSION}),
        json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{"tools":true}}),
    ] {
        let (transport, log) = transport(vec![Ok(result)]);
        let mut c = Client::new(transport, "server", config(), policy()).unwrap();
        assert!(c.initialize().await.is_err());
        assert!(!c.is_ready());
        assert_eq!(log.lock().unwrap().len(), 1);
    }
    let (transport, log) = transport(vec![Ok(
        json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{}}),
    )]);
    let mut c = Client::new(transport, "server", config(), policy()).unwrap();
    assert!(c.list_tools().await.is_err());
    c.initialize().await.unwrap();
    assert!(c.list_tools().await.unwrap().is_empty());
    assert_eq!(log.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn tools_paginate_pin_and_only_invoke_discovered_names() {
    let (mut client, log) = client(vec![
        Ok(json!({"tools":[tool("read",true)],"nextCursor":" next "})),
        Ok(json!({"tools":[tool("other",true)]})),
        Ok(json!({"content":[]})),
    ])
    .await;
    assert!(client.call_tool("read", json!({})).await.is_err());
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools.len(), 2);
    assert!(tools.iter().all(|t| t.read_only));
    assert_eq!(log.lock().unwrap()[3].1, json!({"cursor":"next"}));
    assert_eq!(client.list_tools().await.unwrap(), tools);
    assert!(client.call_tool("guessed", json!({})).await.is_err());
    assert!(client.call_tool("read", json!([])).await.is_err());
    client.call_tool("read", json!({"value":1})).await.unwrap();
    assert_eq!(
        log.lock().unwrap().last().unwrap().1,
        json!({"name":"read","arguments":{"value":1}})
    );
}

#[tokio::test]
async fn malformed_pagination_and_duplicate_original_names_fail_atomically() {
    for bad in [
        json!({"tools":[] ,"nextCursor":null}),
        json!({"tools":[],"nextCursor":7}),
        json!({"tools":[],"nextCursor":{}}),
        json!({"tools":{}}),
        json!({"tools":[{"name":"bad","annotations":{"readOnlyHint":"true"}}]}),
        json!({"tools":[tool("read",true)]}),
        json!({"tools":[],"nextCursor":" cursor "}),
    ] {
        let (mut client, log) = client(vec![
            Ok(json!({"tools":[tool("read",true)],"nextCursor":"cursor"})),
            Ok(bad),
        ])
        .await;
        assert!(client.list_tools().await.is_err());
        assert!(client.call_tool("read", json!({})).await.is_err());
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == "tools/call")
                .count(),
            0
        );
    }
}

#[tokio::test]
async fn exact_default_page_and_item_bounds() {
    assert_eq!(Limits::default().max_pages, 100);
    assert_eq!(Limits::default().max_items, 10_000);
    let pages: Vec<_> = (0..100)
        .map(|i| {
            Ok(if i == 99 {
                json!({"tools":[]})
            } else {
                json!({"tools":[],"nextCursor":i.to_string()})
            })
        })
        .collect();
    let (mut c, log) = client(pages).await;
    assert!(c.list_tools().await.unwrap().is_empty());
    assert_eq!(log.lock().unwrap().len(), 102);
    let pages = (0..100)
        .map(|i| Ok(json!({"tools":[],"nextCursor":i.to_string()})))
        .collect();
    let (mut c, log) = client(pages).await;
    assert_eq!(c.list_tools().await, Err(Error::Limit));
    assert_eq!(log.lock().unwrap().len(), 102);
    for count in [10_000, 10_001] {
        let tools: Vec<_> = (0..count).map(|i| tool(&format!("t{i}"), true)).collect();
        let (mut c, _) = client(vec![Ok(json!({"tools":tools}))]).await;
        if count == 10_000 {
            assert_eq!(c.list_tools().await.unwrap().len(), count);
        } else {
            assert_eq!(c.list_tools().await, Err(Error::Limit));
        }
    }
}

#[tokio::test]
async fn resources_and_prompts_discover_validate_and_enforce_exact_host_allowlists() {
    let (mut c, log) = client(vec![Ok(json!({"resources":[{"uri":"file:///a","name":"a"}]})), Ok(json!({"contents":[{"uri":"file:///a","text":"text"}]})), Ok(json!({"prompts":[{"name":"summarize","arguments":[{"name":"topic","required":true}]}]})), Ok(json!({"messages":[]}))]).await;
    assert!(c.read_resource("file:///a").await.is_err());
    assert!(
        c.get_prompt("summarize", json!({"topic":"a"}))
            .await
            .is_err()
    );
    assert_eq!(c.list_resources().await.unwrap()[0].server, "server");
    assert!(c.read_resource("file:///b").await.is_err());
    c.read_resource("file:///a").await.unwrap();
    c.list_prompts().await.unwrap();
    for args in [
        json!({}),
        json!({"topic":5}),
        json!({"topic":"a","unknown":"b"}),
    ] {
        assert!(c.get_prompt("summarize", args).await.is_err());
    }
    c.get_prompt("summarize", json!({"topic":"a"}))
        .await
        .unwrap();
    assert_eq!(
        log.lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "prompts/get")
            .count(),
        1
    );

    let mut p = policy();
    p.servers.get_mut("server").unwrap().allowed_resources = Some(BTreeSet::new());
    p.servers.get_mut("server").unwrap().allowed_prompts = Some(BTreeSet::new());
    let (transport, _) = transport(vec![
        handshake(),
        Ok(json!({"resources":[{"uri":"a","name":"a"}]})),
        Ok(json!({"prompts":[{"name":"a"}]})),
    ]);
    let mut c = Client::new(transport, "server", config(), p).unwrap();
    c.initialize().await.unwrap();
    assert!(c.list_resources().await.unwrap().is_empty());
    assert!(c.list_prompts().await.unwrap().is_empty());
    assert!(c.read_resource("a").await.is_err());
    assert!(c.get_prompt("a", json!({})).await.is_err());
}

#[derive(Default)]
struct Hooks {
    approval: bool,
    break_glass: bool,
    audit_fails: bool,
    callback_fails: bool,
    events: Mutex<Vec<(OperationContext, String)>>,
}
impl HostHooks for Hooks {
    fn approve<'a>(&'a self, c: &'a OperationContext) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async move {
            self.events
                .lock()
                .unwrap()
                .push((c.clone(), "approval".into()));
            if self.callback_fails {
                Err(Error::Policy("SECRET".into()))
            } else {
                Ok(self.approval)
            }
        })
    }
    fn break_glass<'a>(
        &'a self,
        c: &'a OperationContext,
        _: &'a str,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async move {
            self.events
                .lock()
                .unwrap()
                .push((c.clone(), "breakglass".into()));
            if self.callback_fails {
                Err(Error::Policy("SECRET".into()))
            } else {
                Ok(self.break_glass)
            }
        })
    }
    fn audit<'a>(
        &'a self,
        c: &'a OperationContext,
        outcome: AuditOutcome,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.events
                .lock()
                .unwrap()
                .push((c.clone(), format!("{outcome:?}")));
            if self.audit_fails {
                Err(Error::Policy("SECRET".into()))
            } else {
                Ok(())
            }
        })
    }
}

#[tokio::test]
async fn approvals_breakglass_and_audit_are_bound_to_immutable_request_context() {
    let hooks = Arc::new(Hooks {
        approval: true,
        break_glass: true,
        ..Default::default()
    });
    let mut p = policy();
    p.hooks = Some(hooks.clone());
    p.servers.get_mut("server").unwrap().allowed_tools = Some(BTreeSet::new());
    let (transport, log) = transport(vec![
        handshake(),
        Ok(json!({"tools":[tool("write",false)]})),
        Ok(json!({"content":[]})),
        Ok(json!({"content":[]})),
    ]);
    let mut c = Client::new(transport, "server", config(), p).unwrap();
    c.initialize().await.unwrap();
    assert!(c.list_tools().await.unwrap().is_empty());
    assert!(c.call_tool("write", json!({"x":1})).await.is_err());
    c.call_tool_with_break_glass("write", json!({"x":1}), "one call")
        .await
        .unwrap();
    assert!(c.call_tool("write", json!({"x":1})).await.is_err());
    c.call_tool_with_break_glass("write", json!({"x":2}), "another call")
        .await
        .unwrap();
    let events = hooks.events.lock().unwrap();
    let approvals: Vec<_> = events.iter().filter(|(_, e)| e == "approval").collect();
    assert_eq!(approvals.len(), 2);
    assert_ne!(
        approvals[0].0.arguments_sha256(),
        approvals[1].0.arguments_sha256()
    );
    assert_ne!(
        approvals[0].0.request_sha256(),
        approvals[1].0.request_sha256()
    );
    let ctx = &approvals[0].0;
    assert_eq!(ctx.tenant_id(), "tenant-a");
    assert_eq!(ctx.server(), "server");
    assert_eq!(ctx.tool(), Some("write"));
    assert_eq!(ctx.operation(), "tools/call");
    assert!(events.iter().any(|(c, e)| c == ctx && e == "Attempted"));
    assert!(events.iter().any(|(c, e)| c == ctx && e == "Completed"));
    assert!(events.iter().any(|(c, e)| c == ctx && e == "breakglass"));
    assert_eq!(
        log.lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "tools/call")
            .count(),
        2
    );
}

#[tokio::test]
async fn remote_requires_all_four_readonly_conditions_and_breakglass_cannot_widen() {
    for trust in [false, true] {
        for annotation in [false, true] {
            for host_readonly in [false, true] {
                let hooks = Arc::new(Hooks {
                    approval: true,
                    break_glass: true,
                    ..Default::default()
                });
                let mut p = policy();
                p.hooks = Some(hooks.clone());
                let grant = p.servers.get_mut("server").unwrap();
                grant.allowed_origins.insert("https://example.com".into());
                if host_readonly {
                    grant.read_only_tools.insert("read".into());
                }
                let config = serde_json::from_value(json!({"type":"streamable-http","url":"https://example.com/mcp","trustReadOnlyHint":trust})).unwrap();
                let (transport, log) = transport(vec![
                    handshake(),
                    Ok(json!({"tools":[tool("read",annotation)]})),
                    Ok(json!({"content":[]})),
                ]);
                let mut c = Client::new(transport, "server", config, p).unwrap();
                c.initialize().await.unwrap();
                let allowed = trust && annotation && host_readonly;
                assert_eq!(c.list_tools().await.unwrap().len(), usize::from(allowed));
                assert_eq!(
                    c.call_tool_with_break_glass("read", json!({}), "override")
                        .await
                        .is_ok(),
                    allowed
                );
                assert_eq!(
                    log.lock()
                        .unwrap()
                        .iter()
                        .filter(|(m, _)| m == "tools/call")
                        .count(),
                    usize::from(allowed)
                );
                assert!(
                    !hooks
                        .events
                        .lock()
                        .unwrap()
                        .iter()
                        .any(|(_, e)| e == "breakglass")
                );
            }
        }
    }
    for (enabled, origin) in [(false, true), (true, false)] {
        let mut p = policy();
        let grant = p.servers.get_mut("server").unwrap();
        grant.enabled = enabled;
        if origin {
            grant.allowed_origins.insert("https://example.com".into());
        }
        let cfg = serde_json::from_value(
            json!({"type":"sse","url":"https://example.com/mcp","trustReadOnlyHint":true}),
        )
        .unwrap();
        let (transport, log) = transport(vec![]);
        assert!(Client::new(transport, "server", cfg, p).is_err());
        assert!(log.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn ambiguous_call_is_never_replayed_and_requires_explicit_fresh_transport() {
    let (mut c, old_log) = client(vec![
        Ok(json!({"tools":[tool("read",true)]})),
        Err(Error::ReconciliationRequired {
            server: "secret".into(),
            operation: "secret".into(),
        }),
    ])
    .await;
    c.list_tools().await.unwrap();
    assert_eq!(
        c.call_tool("read", json!({})).await,
        Err(Error::ReconciliationRequired {
            server: "server".into(),
            operation: "tools/call".into()
        })
    );
    assert!(!c.is_ready());
    assert_eq!(c.call_tool("read", json!({})).await, Err(Error::Closed));
    let (fresh, new_log) = transport(vec![
        handshake(),
        Ok(json!({"tools":[tool("read",true)]})),
        Ok(json!({"content":[]})),
    ]);
    c.reconnect(fresh).await.unwrap();
    assert!(c.call_tool("read", json!({})).await.is_err());
    assert_eq!(new_log.lock().unwrap().len(), 2);
    c.list_tools().await.unwrap();
    c.call_tool("read", json!({})).await.unwrap();
    assert_eq!(
        old_log
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "tools/call")
            .count(),
        1
    );
    assert_eq!(
        new_log
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "tools/call")
            .count(),
        1
    );
}

#[tokio::test]
async fn cancelled_or_timed_out_dispatch_poison_session() {
    for cancellation in [false, true] {
        let log = Log::default();
        let mock = Mock {
            replies: vec![handshake(), Ok(json!({"tools":[tool("read",true)]}))].into(),
            log: log.clone(),
            hang: true,
        };
        let limits = Limits {
            timeout: Duration::from_millis(if cancellation { 1000 } else { 5 }),
            ..Default::default()
        };
        let mut c =
            Client::with_limits(Box::new(mock), "server", config(), policy(), limits).unwrap();
        c.initialize().await.unwrap();
        c.list_tools().await.unwrap();
        if cancellation {
            assert!(
                tokio::time::timeout(Duration::from_millis(5), c.call_tool("read", json!({})))
                    .await
                    .is_err()
            );
        } else {
            assert!(matches!(
                c.call_tool("read", json!({})).await,
                Err(Error::ReconciliationRequired { .. })
            ));
        }
        assert_eq!(c.call_tool("read", json!({})).await, Err(Error::Closed));
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == "tools/call")
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn audit_and_callback_errors_are_redacted_and_fail_closed() {
    let mut p = policy();
    p.hooks = Some(Arc::new(Hooks {
        audit_fails: true,
        ..Default::default()
    }));
    let (t, log) = transport(vec![]);
    let mut c = Client::new(t, "server", config(), p).unwrap();
    assert_eq!(
        c.initialize().await,
        Err(Error::Policy("audit unavailable".into()))
    );
    assert!(log.lock().unwrap().is_empty());
    for breakglass in [false, true] {
        let mut p = policy();
        p.hooks = Some(Arc::new(Hooks {
            callback_fails: true,
            ..Default::default()
        }));
        if breakglass {
            p.servers.get_mut("server").unwrap().allowed_tools = Some(BTreeSet::new());
        }
        let (t, log) = transport(vec![
            handshake(),
            Ok(json!({"tools":[tool("write",false)]})),
        ]);
        let mut c = Client::new(t, "server", config(), p).unwrap();
        c.initialize().await.unwrap();
        c.list_tools().await.unwrap();
        let error = c
            .call_tool_with_break_glass("write", json!({}), "reason")
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("SECRET"));
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == "tools/call")
                .count(),
            0
        );
    }
}

#[tokio::test]
async fn manager_adapter_routes_exactly_and_reconnect_rejects_catalog_drift() {
    let (c, log) = client(vec![
        Ok(json!({"tools":[tool("read",true)]})),
        Ok(json!({"content":[]})),
    ])
    .await;
    let manager = ClientManager::new(vec![c]).await.unwrap();
    assert_eq!(manager.definitions()[0].name, "mcp__server__read");
    assert!(manager.call("read", json!({})).await.is_err());
    manager.call("mcp__server__read", json!({})).await.unwrap();
    let (fresh, fresh_log) =
        transport(vec![handshake(), Ok(json!({"tools":[tool("read",false)]}))]);
    assert!(manager.reconnect("server", fresh).await.is_err());
    assert_eq!(
        manager.call("mcp__server__read", json!({})).await,
        Err(Error::Closed)
    );
    assert!(log.lock().unwrap().iter().any(|(m, _)| m == "close"));
    assert!(fresh_log.lock().unwrap().iter().any(|(m, _)| m == "close"));
    manager.close().await.unwrap();
}

#[test]
fn schema_fixture_corpus() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/mcp/schemas.json")).unwrap();
    for case in cases {
        assert_eq!(
            normalize_input_schema(case["input"].clone()),
            case["expected"]
        );
    }
}

#[tokio::test]
async fn legacy_protocol_is_only_negotiated_for_stdio_and_sse() {
    for kind in ["stdio", "sse", "streamable-http"] {
        let cfg = serde_json::from_value(if kind == "stdio" {
            json!({"type":kind,"command":"mock"})
        } else {
            json!({"type":kind,"url":"https://example.com/mcp"})
        })
        .unwrap();
        let mut p = policy();
        p.servers
            .get_mut("server")
            .unwrap()
            .allowed_origins
            .insert("https://example.com".into());
        let (transport, log) = transport(vec![Ok(
            json!({"protocolVersion":"2024-11-05","capabilities":{}}),
        )]);
        let mut c = Client::new(transport, "server", cfg, p).unwrap();
        assert_eq!(c.initialize().await.is_ok(), kind != "streamable-http");
        assert_eq!(
            log.lock().unwrap().len(),
            if kind == "streamable-http" { 1 } else { 2 }
        );
    }
}

struct HangingTerminalAudit;
impl HostHooks for HangingTerminalAudit {
    fn audit<'a>(
        &'a self,
        context: &'a OperationContext,
        outcome: AuditOutcome,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            if context.operation() == "tools/call" && outcome == AuditOutcome::Completed {
                std::future::pending::<()>().await;
            }
            Ok(())
        })
    }
}
#[tokio::test]
async fn cancellation_during_terminal_audit_keeps_session_poisoned() {
    let mut p = policy();
    p.hooks = Some(Arc::new(HangingTerminalAudit));
    let (t, log) = transport(vec![
        handshake(),
        Ok(json!({"tools":[tool("read",true)]})),
        Ok(json!({"content":[]})),
    ]);
    let mut c = Client::new(t, "server", config(), p).unwrap();
    c.initialize().await.unwrap();
    c.list_tools().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(5), c.call_tool("read", json!({})))
            .await
            .is_err()
    );
    assert!(!c.is_ready());
    assert_eq!(c.call_tool("read", json!({})).await, Err(Error::Closed));
    assert_eq!(
        log.lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == "tools/call")
            .count(),
        1
    );
}

#[tokio::test]
async fn resources_and_prompts_share_atomic_pagination_validation() {
    for resource in [true, false] {
        let field = if resource { "resources" } else { "prompts" };
        let item = if resource {
            json!({"name":"a","uri":"a"})
        } else {
            json!({"name":"a"})
        };
        for cursor in [json!(7), Value::Null, json!("again")] {
            let (mut c, _) = client(vec![
                Ok(json!({field:[item.clone()],"nextCursor":"again"})),
                Ok(json!({field:[],"nextCursor":cursor})),
            ])
            .await;
            if resource {
                assert!(c.list_resources().await.is_err());
                assert!(c.read_resource("a").await.is_err());
            } else {
                assert!(c.list_prompts().await.is_err());
                assert!(c.get_prompt("a", json!({})).await.is_err());
            }
        }
    }
}

#[tokio::test]
async fn context_digest_separates_tenants_servers_tools_and_arguments() {
    let hooks = Arc::new(Hooks {
        approval: true,
        ..Default::default()
    });
    for (tenant, server, tool_name, args) in [
        ("t1", "s1", "write", json!({"x":1})),
        ("t2", "s1", "write", json!({"x":1})),
        ("t1", "s2", "write", json!({"x":1})),
        ("t1", "s1", "other", json!({"x":1})),
        ("t1", "s1", "write", json!({"x":2})),
    ] {
        let mut p = HostPolicy {
            tenant_id: tenant.into(),
            servers: BTreeMap::from([(
                server.into(),
                ServerPolicy {
                    enabled: true,
                    ..Default::default()
                },
            )]),
            hooks: Some(hooks.clone()),
        };
        let (t, _) = transport(vec![
            handshake(),
            Ok(json!({"tools":[tool(tool_name,false)]})),
            Ok(json!({"content":[]})),
        ]);
        let mut c = Client::new(t, server, config(), p.clone()).unwrap();
        p.tenant_id = "mutated".into();
        p.servers.get_mut(server).unwrap().enabled = false;
        c.initialize().await.unwrap();
        c.list_tools().await.unwrap();
        c.call_tool(tool_name, args).await.unwrap();
    }
    let events = hooks.events.lock().unwrap();
    let approvals: Vec<_> = events
        .iter()
        .filter(|(_, e)| e == "approval")
        .map(|(c, _)| c)
        .collect();
    let digests: BTreeSet<_> = approvals.iter().map(|c| c.request_sha256()).collect();
    assert_eq!(digests.len(), 5);
    assert_eq!(
        approvals[0].arguments_sha256(),
        approvals[1].arguments_sha256()
    );
    assert_eq!(approvals[0].tenant_id(), "t1");
}

#[tokio::test]
async fn malformed_dispatched_tool_result_is_unknown_not_a_reusable_protocol_error() {
    for malformed in [
        json!({"content":"invalid"}),
        json!({"content":[],"isError":"invalid"}),
        Value::Null,
        json!({"content":[{"type":"tool_result","content":[{"type":"text","icons":3}]}]}),
        json!({"content":[{"type":"resource_link","icons":[{"src":3}]}]}),
        json!({"content":[{"type":"text","text":"bad metadata","_meta":7}]}),
        json!({"content":[{"type":"tool_result","content":[{"type":"text","text":3}]}]}),
    ] {
        let (transport, log) = transport(vec![
            handshake(),
            Ok(json!({"tools":[tool("lookup",true)]})),
            Ok(malformed),
        ]);
        let hooks = Arc::new(Hooks {
            approval: true,
            ..Hooks::default()
        });
        let mut host = policy();
        host.hooks = Some(hooks.clone());
        let mut client = Client::new(transport, "server", config(), host).unwrap();
        client.initialize().await.unwrap();
        client.list_tools().await.unwrap();
        assert!(matches!(
            client.call_tool("lookup", json!({})).await,
            Err(Error::ReconciliationRequired { .. })
        ));
        assert_eq!(
            client.call_tool("lookup", json!({})).await,
            Err(Error::Closed)
        );
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|(method, _)| method == "tools/call")
                .count(),
            1
        );
        let outcomes: Vec<_> = hooks
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|(context, event)| context.operation() == "tools/call" && event != "approval")
            .map(|(_, event)| event.clone())
            .collect();
        assert_eq!(outcomes, ["Attempted", "OutcomeUnknown"]);
    }
}

#[tokio::test]
async fn discovery_cache_defaults_invalidate_both_catalogs_but_keep_tools_pinned() {
    let (mut c, log) = client(vec![
        Ok(json!({"tools":[tool("read",true)]})),
        Ok(json!({"resources":[{"uri":"old","name":"old"}]})),
        Ok(json!({"prompts":[{"name":"old","arguments":[{"name":"arg"}]}]})),
        Ok(json!({"resources":[{"uri":"new","name":"new"}]})),
        Ok(json!({"prompts":[{"name":"new"}]})),
    ])
    .await;
    let tools = c.list_tools().await.unwrap();
    let mut resources = c.list_resources().await.unwrap();
    let mut prompts = c.list_prompts().await.unwrap();
    resources[0].name = "mutated".into();
    prompts[0].arguments[0].name = "mutated".into();
    assert_eq!(c.list_resources().await.unwrap()[0].name, "old");
    assert_eq!(c.list_prompts().await.unwrap()[0].arguments[0].name, "arg");
    assert_eq!(log.lock().unwrap().len(), 5);
    c.invalidate_discovery();
    assert!(c.read_resource("old").await.is_err());
    assert!(c.get_prompt("old", json!({})).await.is_err());
    assert_eq!(c.list_tools().await.unwrap(), tools);
    assert_eq!(c.list_resources().await.unwrap()[0].uri, "new");
    assert_eq!(c.list_prompts().await.unwrap()[0].name, "new");
    assert_eq!(log.lock().unwrap().len(), 7);
}

#[tokio::test]
async fn discovery_ttl_expiry_and_zero_ttl_refresh_resources_and_prompts() {
    for ttl in [Duration::from_millis(1), Duration::ZERO] {
        let (mut c, log) = client(vec![
            Ok(json!({"resources":[{"uri":"old","name":"old"}]})),
            Ok(json!({"prompts":[{"name":"old"}]})),
            Ok(json!({"resources":[{"uri":"new","name":"new"}]})),
            Ok(json!({"prompts":[{"name":"new"}]})),
            Ok(json!({"contents":[]})),
            Ok(json!({"messages":[]})),
        ])
        .await;
        c.set_discovery_cache_ttl(ttl);
        assert_eq!(c.list_resources().await.unwrap()[0].uri, "old");
        assert_eq!(c.list_prompts().await.unwrap()[0].name, "old");
        if !ttl.is_zero() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(c.list_resources().await.unwrap()[0].uri, "new");
        assert_eq!(c.list_prompts().await.unwrap()[0].name, "new");
        assert!(c.read_resource("old").await.is_err());
        assert!(c.get_prompt("old", json!({})).await.is_err());
        c.read_resource("new").await.unwrap();
        c.get_prompt("new", json!({})).await.unwrap();
        assert_eq!(log.lock().unwrap().len(), 8);
    }
}

#[tokio::test]
async fn failed_discovery_refresh_is_not_replayed_or_served_stale() {
    for resources in [true, false] {
        let result = if resources {
            json!({"resources":[{"uri":"old","name":"old"}]})
        } else {
            json!({"prompts":[{"name":"old"}]})
        };
        let (mut c, log) = client(vec![
            Ok(result),
            Err(Error::ReconciliationRequired {
                server: "peer-private".into(),
                operation: "peer-private".into(),
            }),
        ])
        .await;
        assert_eq!(
            c.diagnostics().as_deref(),
            Some("bounded host-only peer diagnostic")
        );
        c.set_discovery_cache_ttl(Duration::ZERO);
        if resources {
            c.list_resources().await.unwrap();
            assert!(matches!(
                c.list_resources().await,
                Err(Error::ReconciliationRequired { .. })
            ));
            assert_eq!(c.list_resources().await, Err(Error::Closed));
            assert_eq!(c.read_resource("old").await, Err(Error::Closed));
        } else {
            c.list_prompts().await.unwrap();
            assert!(matches!(
                c.list_prompts().await,
                Err(Error::ReconciliationRequired { .. })
            ));
            assert_eq!(c.list_prompts().await, Err(Error::Closed));
            assert_eq!(c.get_prompt("old", json!({})).await, Err(Error::Closed));
        }
        assert_eq!(
            c.diagnostics().as_deref(),
            Some("bounded host-only peer diagnostic")
        );
        assert_eq!(log.lock().unwrap().len(), 4);
    }
}

#[tokio::test]
async fn collision_suffixes_follow_discovery_order_and_route_exact_originals() {
    for reverse in [false, true] {
        let mut clients = Vec::new();
        let mut logs = Vec::new();
        for (server, names) in [
            ("a b", vec!["z?x", "z x", "z_x_2", "z_x"]),
            ("a?b", vec!["z_x"]),
        ] {
            let mut p = policy();
            let mut grant = p.servers.remove("server").unwrap();
            grant.allowed_tools = Some(names.iter().map(|name| (*name).to_owned()).collect());
            p.servers.insert(server.into(), grant);
            let mut first_page = vec![tool("z/x", true), tool(names[0], true)];
            if server == "a?b" {
                first_page.remove(0);
            }
            let mut replies = vec![
                handshake(),
                Ok(json!({"tools":first_page,"nextCursor":"next"})),
                Ok(
                    json!({"tools":names[1..].iter().map(|name| tool(name,true)).collect::<Vec<_>>()}),
                ),
            ];
            replies.extend(names.iter().map(|_| Ok(json!({"content":[]}))));
            let (t, log) = transport(replies);
            let mut c = Client::new(t, server, config(), p).unwrap();
            c.initialize().await.unwrap();
            let discovered = c.list_tools().await.unwrap();
            assert_eq!(discovered[0].tool_name, names[0]);
            assert_eq!(discovered[0].qualified_name, "mcp__a_b__z_x");
            assert!(c.call_tool("z/x", json!({})).await.is_err());
            clients.push(c);
            logs.push(log);
        }
        if reverse {
            clients.reverse();
        }
        let manager = ClientManager::new(clients).await.unwrap();
        let expected = [
            ("mcp__a_b__z_x", 0, "z?x"),
            ("mcp__a_b__z_x_2", 0, "z x"),
            ("mcp__a_b__z_x_2_2", 0, "z_x_2"),
            ("mcp__a_b__z_x_3", 0, "z_x"),
            ("mcp__a_b__z_x_4", 1, "z_x"),
        ];
        assert_eq!(
            manager
                .definitions()
                .iter()
                .map(|d| d.name.as_str())
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|(name, _, _)| *name)
                .collect::<Vec<_>>()
        );
        for (qualified, server_index, original) in expected {
            manager
                .call(qualified, json!({"route":qualified}))
                .await
                .unwrap();
            assert_eq!(
                logs[server_index].lock().unwrap().last().unwrap().1,
                json!({"name":original,"arguments":{"route":qualified}})
            );
        }
        assert!(manager.call("mcp__a_b__z_x_5", json!({})).await.is_err());
        assert!(manager.call("z?x", json!({})).await.is_err());
        assert_eq!(
            logs[0]
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == "tools/call")
                .count(),
            4
        );
        assert_eq!(
            logs[1]
                .lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == "tools/call")
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn manager_discovery_partial_success_preserves_scoped_errors_and_reconciliation() {
    for field in ["resources", "prompts"] {
        let method = format!("{field}/list");
        let healthy = Ok(json!({field:[{"uri":"healthy", "name":"healthy"}]}));
        let empty = Ok(json!({field:[]}));
        let failure = Error::Remote { code: -32603 };
        let other_failure = Error::Remote { code: -32000 };
        let unknown = |server: &str| Error::ReconciliationRequired {
            server: server.into(),
            operation: method.clone(),
        };
        for (case, replies, scope, expected, requests) in [
            (
                "failure before healthy",
                [Err(failure.clone()), healthy.clone()],
                None,
                Ok(1),
                [1, 1],
            ),
            (
                "failure after healthy",
                [healthy.clone(), Err(failure.clone())],
                None,
                Ok(1),
                [1, 1],
            ),
            (
                "failure before empty",
                [Err(failure.clone()), empty.clone()],
                None,
                Err(failure.clone()),
                [1, 1],
            ),
            (
                "failure after empty",
                [empty.clone(), Err(failure.clone())],
                None,
                Err(failure.clone()),
                [1, 1],
            ),
            (
                "all failed",
                [Err(failure.clone()), Err(other_failure)],
                None,
                Err(failure.clone()),
                [1, 1],
            ),
            (
                "scoped failure before healthy",
                [Err(failure.clone()), healthy.clone()],
                Some("a"),
                Err(failure.clone()),
                [1, 0],
            ),
            (
                "scoped failure after healthy",
                [healthy.clone(), Err(failure.clone())],
                Some("b"),
                Err(failure.clone()),
                [0, 1],
            ),
            ("all empty", [empty.clone(), empty], None, Ok(0), [1, 1]),
            (
                "reconciliation before healthy",
                [Err(unknown("a")), healthy.clone()],
                None,
                Err(unknown("a")),
                [1, 0],
            ),
            (
                "reconciliation after healthy",
                [healthy, Err(unknown("b"))],
                None,
                Err(unknown("b")),
                [1, 1],
            ),
            (
                "reconciliation after ordinary failure",
                [Err(failure), Err(unknown("b"))],
                None,
                Err(unknown("b")),
                [1, 1],
            ),
        ] {
            let mut clients = Vec::new();
            let mut logs = Vec::new();
            for (server, reply) in ["a", "b"].into_iter().zip(replies) {
                let mut p = policy();
                let grant = p.servers.remove("server").unwrap();
                p.servers.insert(server.into(), grant);
                let (t, log) = transport(vec![handshake(), Ok(json!({"tools":[]})), reply]);
                clients.push(Client::new(t, server, config(), p).unwrap());
                logs.push(log);
            }
            let manager = ClientManager::new(clients).await.unwrap();
            let result = if field == "resources" {
                manager.list_resources(scope).await
            } else {
                manager
                    .list_prompts(scope)
                    .await
                    .map(|items| serde_json::to_value(items).unwrap())
            };
            if let Ok(items) = &result {
                for item in items.as_array().unwrap() {
                    assert_eq!(item["name"], "healthy", "{field}: {case}");
                    assert_eq!(
                        item["server"],
                        if case == "failure before healthy" {
                            "b"
                        } else {
                            "a"
                        },
                        "{field}: {case}"
                    );
                }
            }
            assert_eq!(
                result.map(|items| items.as_array().unwrap().len()),
                expected,
                "{field}: {case}"
            );
            for (log, expected_requests) in logs.iter().zip(requests) {
                assert_eq!(
                    log.lock()
                        .unwrap()
                        .iter()
                        .filter(|(m, _)| m == &method)
                        .count(),
                    expected_requests,
                    "{field}: {case}"
                );
            }
        }
    }
}

#[tokio::test]
async fn manager_scoped_and_all_invalidation_and_prompt_routing_respect_capabilities() {
    let mut clients = Vec::new();
    let mut logs = Vec::new();
    for server in ["a", "b", "unsupported"] {
        let mut p = policy();
        let grant = p.servers.remove("server").unwrap();
        p.servers.insert(server.into(), grant);
        let mut replies = if server == "unsupported" {
            vec![Ok(
                json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{}}),
            )]
        } else {
            vec![handshake(), Ok(json!({"tools":[]}))]
        };
        if server != "unsupported" {
            for version in 0..if server == "a" { 3 } else { 2 } {
                replies.push(Ok(
                    json!({"resources":[{"uri":format!("{server}-{version}"),"name":"resource"}]}),
                ));
                replies.push(Ok(json!({"prompts":[{"name":format!("{server}-{version}"),"arguments":[{"name":"topic","required":true}]}]})));
            }
            replies.push(Ok(json!({"messages":[]})));
        }
        let (t, log) = transport(replies);
        clients.push(Client::new(t, server, config(), p).unwrap());
        logs.push(log);
    }
    let manager = ClientManager::new(clients).await.unwrap();
    for invalid in ["missing", "unsupported"] {
        assert!(manager.list_resources(Some(invalid)).await.is_err());
        assert!(manager.list_prompts(Some(invalid)).await.is_err());
        assert!(manager.get_prompt(invalid, "a-0", json!({})).await.is_err());
    }
    for _ in 0..2 {
        assert_eq!(
            manager
                .list_resources(None)
                .await
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(manager.list_prompts(None).await.unwrap().len(), 2);
    }
    manager.invalidate_discovery("missing").await;
    manager.invalidate_discovery("a").await;
    assert!(
        manager
            .get_prompt("a", "a-0", json!({"topic":"test"}))
            .await
            .is_err()
    );
    let resources = manager.list_resources(None).await.unwrap();
    let prompts = manager.list_prompts(None).await.unwrap();
    assert_eq!(resources[0]["uri"], "a-1");
    assert_eq!(resources[1]["uri"], "b-0");
    assert_eq!(prompts[0].name, "a-1");
    assert_eq!(prompts[1].name, "b-0");
    manager.invalidate_discovery("").await;
    assert_eq!(manager.list_resources(None).await.unwrap()[1]["uri"], "b-1");
    assert_eq!(manager.list_prompts(None).await.unwrap()[0].name, "a-2");
    for (server, name) in [("a", "a-2"), ("b", "b-1")] {
        assert!(manager.get_prompt(server, name, json!({})).await.is_err());
        manager
            .get_prompt(server, name, json!({"topic":"test"}))
            .await
            .unwrap();
    }
    for (i, log) in logs.iter().enumerate().take(2) {
        assert_eq!(
            log.lock().unwrap().last().unwrap().1,
            json!({"name":if i == 0 {"a-2"} else {"b-1"},"arguments":{"topic":"test"}})
        );
        assert_eq!(
            log.lock()
                .unwrap()
                .iter()
                .filter(|(m, _)| m == "tools/list")
                .count(),
            1
        );
    }
    assert_eq!(logs[2].lock().unwrap().len(), 2);
}
