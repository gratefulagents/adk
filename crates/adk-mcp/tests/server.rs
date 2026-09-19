use adk_core::ToolDefinition;
use adk_mcp::{BoxFuture, Error, PROTOCOL_VERSION, server::*};
use axum::http::request::Parts;
use reqwest::{Client, RequestBuilder, StatusCode};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

struct Tenants;
impl TenantResolver for Tenants {
    fn resolve_tenant<'a>(&'a self, request: &'a Parts) -> BoxFuture<'a, Result<String, Error>> {
        Box::pin(async move {
            if request
                .headers
                .get("x-tenant")
                .is_some_and(|v| v == "error")
            {
                return Err(Error::Policy("TOP_SECRET tenant resolution".into()));
            }
            Ok(request
                .headers
                .get("x-tenant")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned())
        })
    }
}
#[derive(Default)]
struct Policy {
    calls: Mutex<Vec<ServerToolRequest>>,
}
impl ServerToolPolicy for Policy {
    fn execute_mcp_tool(
        &self,
        request: ServerToolRequest,
    ) -> BoxFuture<'_, Result<ServerToolResult, Error>> {
        Box::pin(async move {
            let args: Value = serde_json::from_slice(request.arguments()).unwrap();
            self.calls.lock().unwrap().push(request);
            if args["deny"] == true {
                return Err(Error::Policy("TOP_SECRET policy details".into()));
            }
            if args["slow"] == true {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            Ok(ServerToolResult {
                content: if args["large"] == true {
                    "s".repeat(20_000)
                } else {
                    "policy result".into()
                },
                is_error: false,
            })
        })
    }
}
struct Resource;
impl ResourcePolicy for Resource {
    fn read<'a>(&'a self, tenant: &'a str, uri: &'a str) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async move {
            if tenant == "denied" {
                return Err(Error::Policy("TOP_SECRET resource".into()));
            }
            Ok(json!({"contents": [{"uri": uri, "text": format!("{tenant} note")}]}))
        })
    }
}
struct Prompt;
impl PromptPolicy for Prompt {
    fn get<'a>(
        &'a self,
        tenant: &'a str,
        arguments: &'a Map<String, Value>,
    ) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async move {
            if tenant == "denied" {
                return Err(Error::Policy("TOP_SECRET prompt".into()));
            }
            Ok(
                json!({"messages": [{"role": "user", "content": {"type": "text", "text": format!("{tenant} {}", arguments["subject"].as_str().unwrap())}}]}),
            )
        })
    }
}
fn tool(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: "look up a value".into(),
        input_schema: schemars::Schema::try_from(
            json!({"type":"object","properties":{"id":{"type":"string"}}}),
        )
        .unwrap(),
        read_only: true,
        requires_approval: true,
    }
}
fn mode(options: ServerOptions) -> (Arc<ServerMode>, Arc<Policy>) {
    let policy = Arc::new(Policy::default());
    let server = ServerMode::new(
        vec![tool("z-last"), tool("lookup")],
        policy.clone(),
        Arc::new(Tenants),
        vec![ServerResource {
            definition: ResourceDefinition {
                uri: "memory://note/1".into(),
                name: "note".into(),
                description: None,
                mime_type: None,
            },
            policy: Arc::new(Resource),
        }],
        vec![ServerPrompt {
            definition: PromptDefinition {
                name: "summarize".into(),
                description: None,
                arguments: vec![PromptArgument {
                    name: "subject".into(),
                    description: None,
                    required: true,
                }],
            },
            policy: Arc::new(Prompt),
        }],
        options,
    )
    .unwrap();
    (Arc::new(server), policy)
}
struct HttpServer {
    endpoint: String,
    client: Client,
    task: tokio::task::JoinHandle<std::io::Result<()>>,
}
impl HttpServer {
    async fn start(mode: Arc<ServerMode>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/mcp", listener.local_addr().unwrap());
        let task = tokio::spawn(mode.serve(listener));
        Self {
            endpoint,
            client: Client::builder().no_proxy().build().unwrap(),
            task,
        }
    }
    fn post(&self, tenant: &str, session: Option<&str>) -> RequestBuilder {
        let mut request = self
            .client
            .post(&self.endpoint)
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream")
            .header("x-tenant", tenant);
        if let Some(session) = session {
            request = request.header("mcp-session-id", session);
        }
        request
    }
    async fn init(&self, tenant: &str) -> reqwest::Response {
        self.post(tenant, None).json(&json!({"jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
            "protocolVersion": PROTOCOL_VERSION, "capabilities": {}, "clientInfo": {"name":"test", "version":"1"}
        }})).send().await.unwrap()
    }
    async fn connect(&self, tenant: &str) -> String {
        let response = self.init(tenant).await;
        assert_eq!(response.status(), StatusCode::OK);
        let session = response.headers()["mcp-session-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let result: Value = response.json().await.unwrap();
        assert_eq!(result["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(
            self.post(tenant, Some(&session))
                .json(&json!({"jsonrpc":"2.0", "method":"notifications/initialized"}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::ACCEPTED
        );
        session
    }
    async fn rpc(&self, tenant: &str, session: &str, method: &str, params: Value) -> Value {
        let response = self
            .post(tenant, Some(session))
            .json(&json!({"jsonrpc":"2.0", "id":7, "method":method, "params":params}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response.json().await.unwrap()
    }
}
impl Drop for HttpServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn real_http_policy_boundary_and_immutable_digest() {
    let (mode, policy) = mode(ServerOptions::default());
    let http = HttpServer::start(mode).await;
    let session = http.connect("tenant-a").await;
    let list = http
        .rpc("tenant-a", &session, "tools/list", json!({}))
        .await;
    assert_eq!(list["result"]["tools"][0]["name"], "lookup");
    assert_eq!(list["result"]["tools"][1]["name"], "z-last");
    assert_eq!(
        list["result"]["tools"][0]["annotations"]["readOnlyHint"],
        true
    );
    let result = http
        .rpc(
            "tenant-a",
            &session,
            "tools/call",
            json!({"name":"lookup", "arguments":{"id":"42"}}),
        )
        .await;
    assert_eq!(result["result"]["content"][0]["text"], "policy result");
    let requests = policy.calls.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.tenant_id(), "tenant-a");
    assert_eq!(request.tool(), &tool("lookup"));
    assert_eq!(request.arguments(), br#"{"id":"42"}"#);
    let mut expected = Sha256::new();
    for value in [
        b"tenant-a".as_slice(),
        b"lookup".as_slice(),
        br#"{"id":"42"}"#.as_slice(),
    ] {
        expected.update((value.len() as u64).to_be_bytes());
        expected.update(value);
    }
    assert_eq!(
        request.request_sha256(),
        format!("{:x}", expected.finalize())
    );
}

#[tokio::test]
async fn tenant_auth_session_binding_and_cleanup() {
    let (mode, policy) = mode(ServerOptions::default());
    let http = HttpServer::start(mode.clone()).await;
    assert_eq!(http.init("").await.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(http.init("  ").await.status(), StatusCode::UNAUTHORIZED);
    let failed_auth = http.init("error").await;
    assert_eq!(failed_auth.status(), StatusCode::UNAUTHORIZED);
    assert!(!failed_auth.text().await.unwrap().contains("TOP_SECRET"));
    let session = http.connect("tenant-a").await;
    for method in [reqwest::Method::POST, reqwest::Method::DELETE] {
        let response = http
            .client
            .request(method, &http.endpoint)
            .header("x-tenant", "tenant-b")
            .header("mcp-session-id", &session)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    assert_eq!(mode.session_count(), 1);
    let response = http
        .client
        .delete(&http.endpoint)
        .header("x-tenant", "tenant-a")
        .header("mcp-session-id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(mode.session_count(), 0);
    assert_eq!(
        http.post("tenant-a", Some(&session))
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    http.connect("tenant-b").await;
    mode.close();
    assert_eq!(mode.session_count(), 0);
    assert_eq!(
        http.init("tenant-b").await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(policy.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn selected_resources_and_prompts_are_tenant_aware_and_redacted() {
    let (mode, _) = mode(ServerOptions::default());
    let http = HttpServer::start(mode).await;
    for tenant in ["tenant-a", "tenant-b", "denied"] {
        let session = http.connect(tenant).await;
        let resources = http
            .rpc(tenant, &session, "resources/list", json!({}))
            .await;
        assert_eq!(
            resources["result"]["resources"].as_array().unwrap().len(),
            1
        );
        let prompts = http.rpc(tenant, &session, "prompts/list", json!({})).await;
        assert_eq!(prompts["result"]["prompts"].as_array().unwrap().len(), 1);
        let resource = http
            .rpc(
                tenant,
                &session,
                "resources/read",
                json!({"uri":"memory://note/1"}),
            )
            .await;
        let prompt = http
            .rpc(
                tenant,
                &session,
                "prompts/get",
                json!({"name":"summarize", "arguments":{"subject":"prompt"}}),
            )
            .await;
        if tenant == "denied" {
            assert_eq!(
                resource["error"]["message"],
                "resource access denied or failed"
            );
            assert_eq!(prompt["error"]["message"], "prompt access denied or failed");
            assert!(!resource.to_string().contains("TOP_SECRET"));
            assert!(!prompt.to_string().contains("TOP_SECRET"));
        } else {
            assert_eq!(
                resource["result"]["contents"][0]["text"],
                format!("{tenant} note")
            );
            assert_eq!(
                prompt["result"]["messages"][0]["content"]["text"],
                format!("{tenant} prompt")
            );
        }
        assert!(
            http.rpc(
                tenant,
                &session,
                "resources/read",
                json!({"uri":"memory://unselected"})
            )
            .await["error"]
                .is_object()
        );
        assert!(
            http.rpc(
                tenant,
                &session,
                "prompts/get",
                json!({"name":"unselected"})
            )
            .await["error"]
                .is_object()
        );
        assert!(
            http.rpc(
                tenant,
                &session,
                "prompts/get",
                json!({"name":"summarize", "arguments":{}})
            )
            .await["error"]
                .is_object()
        );
    }
}

#[tokio::test]
async fn malformed_messages_never_execute_policy() {
    let (mode, policy) = mode(ServerOptions::default());
    let http = HttpServer::start(mode.clone()).await;
    let session = http.connect("a").await;
    for body in [
        "{",
        "[]",
        "null",
        r#"{"id":1,"method":"tools/call"}"#,
        r#"{"jsonrpc":"2.0","id":{},"method":"tools/call"}"#,
        r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":[]}"#,
    ] {
        let response = http
            .post("a", Some(&session))
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.json::<Value>().await.unwrap()["error"].is_object());
    }
    for params in [
        json!({"name":"unselected"}),
        json!({"name":"lookup","arguments":null}),
        json!({"name":"lookup","arguments":[]}),
    ] {
        assert!(http.rpc("a", &session, "tools/call", params).await["error"].is_object());
    }
    assert_eq!(
        http.post("a", Some(&session))
            .json(&json!({"jsonrpc":"2.0", "method":"tools/call", "params":{"name":"lookup"}}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    assert!(policy.calls.lock().unwrap().is_empty());
    let before = mode.session_count();
    let invalid_init = http
        .post("a", None)
        .json(&json!({"jsonrpc":"2.0", "id":1, "method":"initialize"}))
        .send()
        .await
        .unwrap();
    assert!(invalid_init.json::<Value>().await.unwrap()["error"].is_object());
    assert_eq!(mode.session_count(), before);
    assert_eq!(
        http.post("a", None)
            .json(&json!({"jsonrpc":"2.0", "id":1, "method":"tools/list"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        http.post("a", Some(&session))
            .header("mcp-protocol-version", "wrong")
            .json(&json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn failure_redaction_request_result_bounds_and_timeout() {
    let mut options = ServerOptions::default();
    options.limits.max_message_bytes = 1024;
    options.limits.timeout = Duration::from_millis(100);
    let (mode, policy) = mode(options);
    let http = HttpServer::start(mode).await;
    let session = http.connect("a").await;
    let denied = http
        .rpc(
            "a",
            &session,
            "tools/call",
            json!({"name":"lookup", "arguments":{"deny":true}}),
        )
        .await;
    assert_eq!(
        denied["error"]["message"],
        "tool execution denied or failed"
    );
    assert!(!denied.to_string().contains("TOP_SECRET"));
    let response = http
        .post("a", Some(&session))
        .body("x".repeat(1025))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(policy.calls.lock().unwrap().len(), 1);
    for (args, status) in [
        (json!({"large":true}), StatusCode::PAYLOAD_TOO_LARGE),
        (json!({"slow":true}), StatusCode::GATEWAY_TIMEOUT),
    ] {
        let response = http.post("a", Some(&session)).json(&json!({"jsonrpc":"2.0", "id":1, "method":"tools/call", "params":{"name":"lookup", "arguments":args}})).send().await.unwrap();
        assert_eq!(response.status(), status);
        assert!(response.bytes().await.unwrap().len() <= 1024);
    }
    assert_eq!(policy.calls.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn dns_rebinding_origin_and_transport_matrix() {
    let (mode, _) = mode(ServerOptions {
        allowed_origins: vec!["https://trusted.example".into()],
        ..ServerOptions::default()
    });
    let http = HttpServer::start(mode).await;
    for (host, origin, expected) in [
        ("attacker.example", None, StatusCode::FORBIDDEN),
        ("localhost.attacker.example", None, StatusCode::FORBIDDEN),
        (
            "127.0.0.1",
            Some("https://attacker.example"),
            StatusCode::FORBIDDEN,
        ),
        ("127.0.0.1", Some("null"), StatusCode::FORBIDDEN),
        ("127.0.0.1", Some("https://trusted.example"), StatusCode::OK),
    ] {
        let mut request = http.post("a", None).header("host", host);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        assert_eq!(request.body("{}").send().await.unwrap().status(), expected);
    }
    assert_eq!(
        http.client
            .get(&http.endpoint)
            .header("x-tenant", "a")
            .header("accept", "text/event-stream")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(
        http.post("a", None)
            .header("content-type", "text/plain")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        http.client
            .post(&http.endpoint)
            .header("x-tenant", "a")
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_ACCEPTABLE
    );
    for transport in [ServerTransport::Stdio, ServerTransport::LegacySse] {
        let result = ServerMode::new(
            vec![],
            Arc::new(Policy::default()),
            Arc::new(Tenants),
            vec![],
            vec![],
            ServerOptions {
                transport,
                ..ServerOptions::default()
            },
        );
        assert!(matches!(result, Err(Error::Config(_))));
    }
}

#[tokio::test]
async fn atomic_total_and_tenant_capacity_and_delete_releases_slot() {
    let (mode, _) = mode(ServerOptions::default());
    let http = Arc::new(HttpServer::start(mode.clone()).await);
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..MAX_SESSIONS_PER_TENANT + 8 {
        let http = http.clone();
        tasks.spawn(async move { http.init("a").await });
    }
    let mut accepted = vec![];
    let mut rejected = 0;
    while let Some(response) = tasks.join_next().await {
        let response = response.unwrap();
        if response.status() == StatusCode::OK {
            accepted.push(
                response.headers()["mcp-session-id"]
                    .to_str()
                    .unwrap()
                    .to_owned(),
            );
        } else {
            assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
            rejected += 1;
        }
    }
    assert_eq!(accepted.len(), MAX_SESSIONS_PER_TENANT);
    assert_eq!(rejected, 8);
    assert_eq!(mode.session_count(), MAX_SESSIONS_PER_TENANT);
    assert_eq!(
        http.client
            .delete(&http.endpoint)
            .header("x-tenant", "a")
            .header("mcp-session-id", &accepted[0])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(http.init("a").await.status(), StatusCode::OK);
    for tenant in 1..(MAX_SESSIONS / MAX_SESSIONS_PER_TENANT) {
        for _ in 0..MAX_SESSIONS_PER_TENANT {
            assert_eq!(
                http.init(&format!("tenant-{tenant}")).await.status(),
                StatusCode::OK
            );
        }
    }
    assert_eq!(mode.session_count(), MAX_SESSIONS);
    assert_eq!(
        http.init("different").await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    mode.close();
    assert_eq!(mode.session_count(), 0);
}

#[tokio::test]
async fn handler_bounds_chunked_framing_and_incomplete_bodies() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut options = ServerOptions::default();
    options.limits.max_message_bytes = 1024;
    options.limits.timeout = Duration::from_millis(100);
    let (mode, policy) = mode(options);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = mode.handler();
    let task = tokio::spawn(async move { axum::serve(listener, router).await });
    let http = HttpServer {
        endpoint: format!("http://{addr}/"),
        client: Client::builder().no_proxy().build().unwrap(),
        task,
    };
    let session = http.connect("a").await;
    for (body_headers, body, expected) in [
        (
            "Transfer-Encoding: chunked",
            format!("401\r\n{}\r\n0\r\n\r\n", "x".repeat(1025)),
            "413",
        ),
        ("Content-Length: 20", "{".into(), "504"),
    ] {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let wire = format!(
            "POST / HTTP/1.1\r\nHost: {addr}\r\nX-Tenant: a\r\nMcp-Session-Id: {session}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nConnection: close\r\n{body_headers}\r\n\r\n{body}"
        );
        stream.write_all(wire.as_bytes()).await.unwrap();
        let mut response = vec![0; 4096];
        let size = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut response))
            .await
            .unwrap()
            .unwrap();
        let response = String::from_utf8_lossy(&response[..size]);
        assert!(
            response.starts_with(&format!("HTTP/1.1 {expected}")),
            "{response}"
        );
    }
    assert!(policy.calls.lock().unwrap().is_empty());
}

#[test]
fn selected_tool_names_are_validated() {
    for tools in [vec![tool("lookup"), tool("lookup")], vec![tool(" ")]] {
        assert!(matches!(
            ServerMode::new(
                tools,
                Arc::new(Policy::default()),
                Arc::new(Tenants),
                vec![],
                vec![],
                ServerOptions::default()
            ),
            Err(Error::Config(_))
        ));
    }
}
