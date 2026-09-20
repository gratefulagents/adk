use adk_mcp::{
    BoxFuture, Error, Limits,
    client::{ClientManager, HostPolicy, ServerPolicy},
    config::ConfigSnapshot,
    connection,
    server::*,
    tools::ToolManager,
    transport::{HeaderProvider, RemoteOptions},
};
use axum::http::{HeaderMap, HeaderValue, request::Parts};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Auth;
impl HeaderProvider for Auth {
    fn headers<'a>(
        &'a self,
        tenant: &'a str,
        server: &'a str,
        _: &'a url::Url,
    ) -> BoxFuture<'a, Result<HeaderMap, Error>> {
        Box::pin(async move {
            assert_eq!((tenant, server), ("tenant-a", "test"));
            let mut h = HeaderMap::new();
            h.insert(
                "authorization",
                HeaderValue::from_static("Bearer secret-token"),
            );
            Ok(h)
        })
    }
}
struct Tenants;
impl TenantResolver for Tenants {
    fn resolve_tenant<'a>(&'a self, parts: &'a Parts) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            if parts
                .headers
                .get("authorization")
                .and_then(|v| v.to_str().ok())
                == Some("Bearer secret-token")
            {
                Ok("tenant-a".into())
            } else {
                Err(Error::Policy("unauthorized".into()))
            }
        })
    }
}
struct Executor(AtomicUsize);
impl ServerToolPolicy for Executor {
    fn execute_mcp_tool(
        &self,
        request: ServerToolRequest,
    ) -> BoxFuture<'_, Result<ServerToolResult, Error>> {
        Box::pin(async move {
            assert_eq!(request.tenant_id(), "tenant-a");
            assert_eq!(request.tool().name, "lookup");
            assert_eq!(request.request_sha256().len(), 64);
            self.0.fetch_add(1, Ordering::SeqCst);
            if serde_json::from_slice::<Value>(request.arguments()).unwrap()["uncertain"] == true {
                return Err(Error::ReconciliationRequired {
                    server: "sensitive-downstream".into(),
                    operation: "secret-operation".into(),
                });
            }
            Ok(ServerToolResult {
                content: "verified".into(),
                is_error: false,
            })
        })
    }
}
struct Resources;
impl ResourcePolicy for Resources {
    fn read<'a>(&'a self, tenant: &'a str, uri: &'a str) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async move {
            assert_eq!(tenant, "tenant-a");
            Ok(json!({"contents":[{"uri":uri,"text":"resource"}]}))
        })
    }
}
struct Prompts;
impl PromptPolicy for Prompts {
    fn get<'a>(
        &'a self,
        tenant: &'a str,
        _: &'a serde_json::Map<String, Value>,
    ) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async move {
            assert_eq!(tenant, "tenant-a");
            Ok(json!({"messages":[{"role":"user","content":{"type":"text","text":"prompt"}}]}))
        })
    }
}

#[tokio::test]
async fn configured_http_client_to_policy_server_and_adk_manager() {
    let executor = Arc::new(Executor(AtomicUsize::new(0)));
    let mode = Arc::new(
        ServerMode::new(
            vec![adk_core::ToolDefinition {
                name: "lookup".into(),
                description: "lookup".into(),
                input_schema: json!({"type":"object","properties":{}}).try_into().unwrap(),
                read_only: true,
                requires_approval: false,
            }],
            executor.clone(),
            Arc::new(Tenants),
            vec![ServerResource {
                definition: ResourceDefinition {
                    uri: "test://resource".into(),
                    name: "resource".into(),
                    description: None,
                    mime_type: None,
                },
                policy: Arc::new(Resources),
            }],
            vec![ServerPrompt {
                definition: PromptDefinition {
                    name: "prompt".into(),
                    description: None,
                    arguments: vec![],
                },
                policy: Arc::new(Prompts),
            }],
            ServerOptions::default(),
        )
        .unwrap(),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let serving = {
        let mode = mode.clone();
        tokio::spawn(async move { mode.serve(listener).await.unwrap() })
    };
    let temp = tempfile::tempdir().unwrap();
    std::fs::write(temp.path().join(".mcp.json"), json!({"mcpServers":{"test":{"type":"streamable-http","url":format!("{origin}/mcp"),"trustReadOnlyHint":true}}}).to_string()).unwrap();
    let snapshot = ConfigSnapshot::load(temp.path()).unwrap();
    let policy = HostPolicy {
        tenant_id: "tenant-a".into(),
        servers: BTreeMap::from([(
            "test".into(),
            ServerPolicy {
                enabled: true,
                allowed_origins: BTreeSet::from([origin]),
                read_only_tools: BTreeSet::from(["lookup".into()]),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let remote = RemoteOptions {
        tenant_id: "tenant-a".into(),
        allow_private_network: true,
        headers: Some(Arc::new(Auth)),
        ..Default::default()
    };
    let mut client = connection::connect(
        &snapshot,
        "test",
        policy.clone(),
        Some(remote.clone()),
        &BTreeMap::new(),
        temp.path(),
        Limits::default(),
    )
    .await
    .unwrap();
    assert_eq!(client.list_prompts().await.unwrap()[0].name, "prompt");
    assert!(client.get_prompt("prompt", json!({})).await.unwrap()["messages"].is_array());
    assert_eq!(
        client.list_resources().await.unwrap()[0].uri,
        "test://resource"
    );
    assert_eq!(
        client.read_resource("test://resource").await.unwrap()["contents"][0]["text"],
        "resource"
    );
    let manager = ClientManager::new(vec![client]).await.unwrap();
    let name = manager.definitions()[0].name.clone();
    assert_eq!(
        manager.call(&name, json!({})).await.unwrap()["content"][0]["text"],
        "verified"
    );
    assert_eq!(executor.0.load(Ordering::SeqCst), 1);
    assert_eq!(
        manager
            .list_resources(None)
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // Preserve a downstream uncertain outcome across server and client boundaries.
    let error = manager
        .call(&name, json!({"uncertain":true}))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::ReconciliationRequired { .. }));
    assert!(!error.to_string().contains("secret-operation"));
    assert_eq!(manager.call(&name, json!({})).await, Err(Error::Closed));
    assert_eq!(executor.0.load(Ordering::SeqCst), 2);
    manager.close().await.unwrap();
    assert_eq!(mode.session_count(), 0);
    drop(manager);
    mode.close();
    serving.abort();

    // Reconfiguration cannot redirect an approved pinned command/endpoint.
    std::fs::write(temp.path().join(".mcp.json"), "{}").unwrap();
    assert!(matches!(
        connection::connect(
            &snapshot,
            "test",
            policy,
            Some(remote),
            &BTreeMap::new(),
            temp.path(),
            Limits::default()
        )
        .await,
        Err(Error::ConfigChanged)
    ));
}

#[tokio::test]
async fn composition_rejects_host_denial_before_any_subprocess() {
    let temp = tempfile::tempdir().unwrap();
    let marker = temp.path().join("should-not-exist");
    std::fs::write(temp.path().join(".mcp.json"), json!({"mcpServers":{"test":{"command":"/bin/sh","args":["-c",format!("touch {}",marker.display())]}}}).to_string()).unwrap();
    let snapshot = ConfigSnapshot::load(temp.path()).unwrap();
    assert!(matches!(
        connection::connect(
            &snapshot,
            "test",
            HostPolicy::default(),
            None,
            &BTreeMap::new(),
            temp.path(),
            Limits::default()
        )
        .await,
        Err(Error::Policy(_))
    ));
    assert!(!marker.exists());
}
