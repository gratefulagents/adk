//! Actual pinned Go interoperability. Run via scripts/mcp-interop/run.py.
//! Intentionally ignored in the hermetic/default suite: requires a built Go helper.
use adk_mcp::{
    BoxFuture, Error, Limits,
    client::{HostPolicy, ServerPolicy},
    config::ConfigSnapshot,
    connection,
    server::*,
    transport::{HeaderProvider, RemoteOptions},
};
use axum::http::{HeaderMap, HeaderValue, request::Parts};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, Command},
};

const TENANT: &str = "interop-tenant";
const URI: &str = "test://interop/resource";
const TOKEN: &str = "Bearer interop-test-only";
fn helper() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("MCP_INTEROP_GO_BINARY")
            .expect("run scripts/mcp-interop/run.py; explicit helper required"),
    );
    assert!(path.is_absolute() && path.is_file());
    path
}
fn command() -> Command {
    let mut cmd = Command::new(helper());
    cmd.env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    cmd
}
fn record(name: &str, value: Value) {
    let root = PathBuf::from(
        std::env::var_os("MCP_INTEROP_RESULTS").expect("explicit evidence directory required"),
    );
    std::fs::write(
        root.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
}
struct Auth;
impl HeaderProvider for Auth {
    fn headers<'a>(
        &'a self,
        tenant: &'a str,
        server: &'a str,
        _: &'a url::Url,
    ) -> BoxFuture<'a, Result<HeaderMap, Error>> {
        Box::pin(async move {
            assert_eq!((tenant, server), (TENANT, "go"));
            let mut h = HeaderMap::new();
            h.insert("authorization", HeaderValue::from_static(TOKEN));
            Ok(h)
        })
    }
}
async fn remote_server(mode: &str) -> (Child, String) {
    let mut child = command().arg(mode).spawn().unwrap();
    let mut line = String::new();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    tokio::time::timeout(Duration::from_secs(8), reader.read_line(&mut line))
        .await
        .expect("Go readiness timed out")
        .unwrap();
    let value: Value = serde_json::from_str(&line).expect("Go readiness JSON");
    let endpoint = value["endpoint"].as_str().unwrap().to_owned();
    assert_eq!(
        url::Url::parse(&endpoint).unwrap().host_str(),
        Some("127.0.0.1")
    );
    // Retain the pipe for the bounded shutdown acknowledgement.
    child.stdout = Some(reader.into_inner());
    (child, endpoint)
}
async fn rust_to_go(mode: &str, operation: &str) {
    tokio::time::timeout(Duration::from_secs(30), async {
        let temp = tempfile::tempdir().unwrap();
        let (mut child, config, origin) = if mode == "stdio" {
            (
                None,
                json!({"command":helper(),"args":["stdio",temp.path().join("go.pid")],"trustReadOnlyHint":true}),
                None,
            )
        } else {
            let (child, endpoint) = remote_server(mode).await;
            let origin = url::Url::parse(&endpoint)
                .unwrap()
                .origin()
                .ascii_serialization();
            (
                Some(child),
                json!({"type":mode,"url":endpoint,"trustReadOnlyHint":true}),
                Some(origin),
            )
        };
        std::fs::write(
            temp.path().join(".mcp.json"),
            json!({"mcpServers":{"go":config}}).to_string(),
        )
        .unwrap();
        let snapshot = ConfigSnapshot::load(temp.path()).unwrap();
        let grant = ServerPolicy {
            enabled: true,
            allowed_origins: origin.into_iter().collect(),
            allowed_tools: Some(BTreeSet::from(["echo".into()])),
            read_only_tools: BTreeSet::from(["echo".into()]),
            ..Default::default()
        };
        let policy = HostPolicy {
            tenant_id: TENANT.into(),
            servers: BTreeMap::from([("go".into(), grant)]),
            ..Default::default()
        };
        let remote = RemoteOptions {
            tenant_id: TENANT.into(),
            allow_private_network: true,
            headers: Some(Arc::new(Auth)),
            ..Default::default()
        };
        let mut client = connection::connect(
            &snapshot,
            "go",
            policy,
            Some(remote),
            &BTreeMap::new(),
            temp.path(),
            Limits {
                timeout: Duration::from_secs(8),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let observed = match operation {
            "tools" => serde_json::to_value(client.list_tools().await.unwrap()).unwrap(),
            "resources" => serde_json::to_value(client.list_resources().await.unwrap()).unwrap(),
            "prompts" => serde_json::to_value(client.list_prompts().await.unwrap()).unwrap(),
            "call" => {
                let name = client.list_tools().await.unwrap()[0].tool_name.clone();
                client
                    .call_tool(&name, json!({"value":"cross-wire"}))
                    .await
                    .unwrap()
            }
            "read" => {
                client.list_resources().await.unwrap();
                client.read_resource(URI).await.unwrap()
            }
            "prompt" => {
                client.list_prompts().await.unwrap();
                client.get_prompt("prompt", json!({})).await.unwrap()
            }
            "shutdown" => json!({"readyBeforeClose":client.is_ready()}),
            _ => panic!("unknown operation"),
        };
        client.close().await.unwrap();
        assert!(!client.is_ready());
        assert_eq!(client.list_tools().await.unwrap_err(), Error::Closed);
        let mut shutdown = json!({"clientClosed":true});
        #[cfg(unix)]
        if mode == "stdio" {
            let raw: i32 = std::fs::read_to_string(temp.path().join("go.pid")).unwrap().parse().unwrap();
            let pid = rustix::process::Pid::from_raw(raw).unwrap();
            let result = rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG);
            assert!(matches!(result, Err(rustix::io::Errno::CHILD)), "Go child not reaped: {result:?}");
            shutdown["goPID"] = json!(raw);
            shutdown["waitpidAfterClose"] = json!("ECHILD");
        }
        if let Some(child) = child.as_mut() {
            child.stdin.take().unwrap().write_all(b"\n").await.unwrap();
            let mut line = String::new();
            BufReader::new(child.stdout.take().unwrap())
                .read_line(&mut line)
                .await
                .unwrap();
            shutdown["goServer"] = serde_json::from_str(&line).unwrap();
            assert_eq!(shutdown["goServer"]["serverClosed"], true);
            if mode == "streamable-http" {
                assert_eq!(shutdown["goServer"]["deleteRequests"], 1);
            }
            let status = child.wait().await.unwrap();
            assert!(status.success());
            shutdown["goExitCode"] = json!(status.code());
        }
        record(
            &format!("rust-to-go-{mode}-{operation}"),
            json!({"observed":observed,"shutdown":shutdown}),
        );
        verify(operation, &observed, false);
    })
    .await
    .expect("cross-wire test timed out");
}
fn verify(operation: &str, observed: &Value, go_client: bool) {
    match operation {
        "tools" => {
            assert_eq!(observed.as_array().unwrap().len(), 1);
            assert_eq!(
                observed[0][if go_client { "ToolName" } else { "tool_name" }],
                "echo"
            );
        }
        "resources" => {
            assert_eq!(observed.as_array().unwrap().len(), 1);
            assert_eq!(observed[0]["uri"], URI);
        }
        "prompts" => {
            assert_eq!(observed.as_array().unwrap().len(), 1);
            assert_eq!(observed[0]["name"], "prompt");
        }
        "call" => assert_eq!(observed["content"][0]["text"], "echo:cross-wire"),
        "read" => assert_eq!(observed["contents"][0]["text"], "resource-value"),
        "prompt" => assert_eq!(observed["messages"][0]["content"]["text"], "prompt-value"),
        "shutdown" => {
            if go_client {
                assert_eq!(observed, &json!(["rust"]))
            } else {
                assert_eq!(observed["readyBeforeClose"], true)
            }
        }
        _ => panic!("unknown operation"),
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
                == Some(TOKEN)
            {
                Ok(TENANT.into())
            } else {
                Err(Error::Policy("unauthorized".into()))
            }
        })
    }
}
struct Policy;
impl ServerToolPolicy for Policy {
    fn execute_mcp_tool(
        &self,
        r: ServerToolRequest,
    ) -> BoxFuture<'_, Result<ServerToolResult, Error>> {
        Box::pin(async move {
            assert_eq!(r.tenant_id(), TENANT);
            assert_eq!(r.tool().name, "echo");
            assert_eq!(r.request_sha256().len(), 64);
            let args: Value = serde_json::from_slice(r.arguments()).unwrap();
            Ok(ServerToolResult {
                content: format!("echo:{}", args["value"].as_str().unwrap()),
                is_error: false,
            })
        })
    }
}
struct Resources;
impl ResourcePolicy for Resources {
    fn read<'a>(&'a self, tenant: &'a str, uri: &'a str) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async move {
            assert_eq!(tenant, TENANT);
            assert_eq!(uri, URI);
            Ok(json!({"contents":[{"uri":uri,"text":"resource-value"}]}))
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
            assert_eq!(tenant, TENANT);
            Ok(
                json!({"messages":[{"role":"user","content":{"type":"text","text":"prompt-value"}}]}),
            )
        })
    }
}
// Abort the Rust listener even when a child fails or an assertion panics.
struct Serving(tokio::task::JoinHandle<()>);
impl Drop for Serving {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn go_to_rust(operation: &str) {
    tokio::time::timeout(Duration::from_secs(30),async {
        let server=Arc::new(ServerMode::new(vec![adk_core::ToolDefinition{name:"echo".into(),description:"interop echo".into(),input_schema:json!({"type":"object","properties":{"value":{"type":"string"}},"required":["value"]}).try_into().unwrap(),read_only:true,requires_approval:false}],Arc::new(Policy),Arc::new(Tenants),vec![ServerResource{definition:ResourceDefinition{uri:URI.into(),name:"resource".into(),description:None,mime_type:None},policy:Arc::new(Resources)}],vec![ServerPrompt{definition:PromptDefinition{name:"prompt".into(),description:None,arguments:vec![]},policy:Arc::new(Prompts)}],ServerOptions::default()).unwrap());
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint=format!("http://{}/mcp",listener.local_addr().unwrap());
        let cloned=server.clone();let _serving=Serving(tokio::spawn(async move {cloned.serve(listener).await.unwrap()}));
        let child=command().args(["client",&endpoint,operation]).spawn().unwrap();
        let output=child.wait_with_output().await.unwrap();assert!(output.status.success(),"Go Manager failed: {}",output.status);
        let result:Value=serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["clientClosed"],true);
        // Actual Go Manager.Close sends DELETE to Rust ServerMode.
        assert_eq!(server.session_count(),0);
        server.close();
        record(&format!("go-to-rust-streamable-http-{operation}"),json!({"observed":result["observed"],"goExitCode":output.status.code(),"clientClosed":result["clientClosed"],"rustSessionsAfterClose":server.session_count()}));
        verify(operation,&result["observed"],true);
    }).await.expect("Go Manager cross-wire test timed out");
}
macro_rules! cases {
    ($module:ident,$run:expr) => {
        mod $module {
            use super::*;
            #[tokio::test]
            #[ignore = "requires pinned Go helper"]
            async fn discovers_tools() {
                ($run)("tools").await
            }
            #[tokio::test]
            #[ignore = "requires pinned Go helper"]
            async fn discovers_resources() {
                ($run)("resources").await
            }
            #[tokio::test]
            #[ignore = "requires pinned Go helper"]
            async fn discovers_prompts() {
                ($run)("prompts").await
            }
            #[tokio::test]
            #[ignore = "requires pinned Go helper"]
            async fn calls_tool() {
                ($run)("call").await
            }
            #[tokio::test]
            #[ignore = "requires pinned Go helper"]
            async fn reads_resource() {
                ($run)("read").await
            }
            #[tokio::test]
            #[ignore = "requires pinned Go helper"]
            async fn gets_prompt() {
                ($run)("prompt").await
            }
            #[tokio::test]
            #[ignore = "requires pinned Go helper"]
            async fn closes_session() {
                ($run)("shutdown").await
            }
        }
    };
}
cases!(go_stdio, |op| rust_to_go("stdio", op));
cases!(go_streamable_http, |op| rust_to_go("streamable-http", op));
cases!(go_legacy_sse, |op| rust_to_go("sse", op));
cases!(rust_streamable_http, go_to_rust);
