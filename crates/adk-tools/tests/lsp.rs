#![cfg(any(target_os = "linux", target_os = "macos"))]
use adk_core::*;
use adk_tools::{Config, Features, Registry, lsp};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

fn sandbox_config(root: &Path) -> adk_sandbox::Config {
    #[allow(unused_mut)]
    let mut config = adk_sandbox::Config::new(root);
    #[cfg(target_os = "macos")]
    config
        .runtime_roots
        .push(adk_sandbox::macos_developer_toolchain_root().unwrap());
    config
}
fn context(path: &Path) -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "lsp-test".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: path.into(),
        policy: Default::default(),
        idempotency_key: None,
    }
}
fn server(mode: &str) -> lsp::ServerConfig {
    lsp::ServerConfig {
        command: [
            "/usr/bin/python3",
            "/opt/homebrew/bin/python3",
            "/usr/local/bin/python3",
        ]
        .into_iter()
        .find(|path| Path::new(path).is_file())
        .expect("host Python 3 is required")
        .into(),
        args: vec!["lsp-server.py".into(), mode.into()],
        env: [("LANG".into(), "C".into())].into(),
        language_id: "plain".into(),
        file_patterns: vec!["**/*.txt".into()],
        startup_timeout: Duration::from_secs(3),
        request_timeout: Duration::from_secs(2),
        ..Default::default()
    }
}
fn make(root: &Path, servers: Vec<lsp::ServerConfig>, limit: usize) -> Arc<lsp::LspTool> {
    let mut config = sandbox_config(root);
    config.output_limit = limit;
    config.term_grace = Duration::from_millis(20);
    lsp::tool(lsp::Config {
        executor: Arc::new(adk_sandbox::Executor::new(config).unwrap()),
        servers,
        discoverer: None,
    })
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("lsp-server.py"),
        include_str!("../../../fixtures/tools/lsp-server.py"),
    )
    .unwrap();
    std::fs::write(root.path().join("sample.txt"), "a😀b\r\n").unwrap();
    root
}
async fn available(root: &Path) -> bool {
    let executor = adk_sandbox::Executor::new(sandbox_config(root)).unwrap();
    let mut request = adk_sandbox::Request::new("/bin/true");
    request.timeout = Some(Duration::from_secs(3));
    let result = executor.run(&context(root).operation, request).await;
    if result.as_ref().is_ok_and(|r| r.status.success()) {
        return true;
    }
    assert!(
        std::env::var_os("ADK_REQUIRE_SANDBOX").is_none(),
        "required sandbox unavailable: {result:?}"
    );
    eprintln!(
        "SKIP confined LSP integration: {result:?}; ADK_REQUIRE_SANDBOX=1 makes this a failure"
    );
    false
}
async fn invoke(tool: &lsp::LspTool, context: &ToolContext, args: Value) -> ToolOutput {
    tool.execute(
        context,
        ToolCall {
            id: "lsp".into(),
            name: "LSP".into(),
            arguments: args,
        },
    )
    .await
    .unwrap()
}
fn text(output: &ToolOutput) -> &str {
    match &output.content[..] {
        [Content::Text { text }] => text,
        _ => panic!("text output required"),
    }
}
fn request(operation: &str) -> Value {
    json!({"operation":operation,"filePath":"sample.txt","languageId":"plain","line":1,"character":4})
}
struct Discover;
impl lsp::ServerDiscoverer for Discover {
    fn discover<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Path,
        request: &'a lsp::Request,
    ) -> BoxFuture<'a, Result<Vec<lsp::ServerConfig>, String>> {
        Box::pin(async move {
            assert_eq!(request.language_id, "plain");
            Ok(vec![server("normal")])
        })
    }
}
#[tokio::test]
async fn discovery_uses_the_same_confined_executor() {
    let root = fixture();
    let mut config = sandbox_config(root.path());
    config.backend = adk_sandbox::Backend::Local;
    let tool = lsp::tool(lsp::Config {
        executor: Arc::new(adk_sandbox::Executor::new(config).unwrap()),
        servers: Vec::new(),
        discoverer: Some(Arc::new(Discover)),
    });
    let output = invoke(&tool, &context(root.path()), request("hover")).await;
    assert!(
        output.is_error && text(&output).contains("local requires"),
        "{output:?}"
    );
}
#[tokio::test]
async fn schema_routing_validation_and_closed_lifecycle() {
    let root = fixture();
    let ctx = context(root.path());
    let tool = make(root.path(), vec![server("normal")], 1 << 20);
    let registry = Registry::build(
        &Config {
            features: Features::Strict(["LSP".into()].into()),
            ..Default::default()
        },
        [tool.clone() as Arc<dyn Tool>],
    )
    .unwrap();
    assert_eq!(registry.names().collect::<Vec<_>>(), ["LSP"]);
    assert!(tool.definition().read_only);
    assert!(!tool.definition().requires_approval);
    for (args, error) in [
        (json!({"operation":"rename"}), "non-read-only"),
        (
            json!({"operation":"hover","filePath":"sample.txt","languageId":"rust"}),
            "no configured",
        ),
        (
            json!({"operation":"hover","filePath":"sample.rs"}),
            "no configured",
        ),
        (json!({"operation":"hover"}), "filePath is required"),
        (
            json!({"operation":"hover","filePath":"../escape.txt"}),
            "outside",
        ),
        (
            json!({"operation":"hover","filePath":"sample.txt","line":0,"character":1}),
            "1-based",
        ),
        (
            json!({"operation":"hover","filePath":"sample.txt","line":1,"character":6}),
            "outside the line",
        ),
        (json!({"operation":"hover","line":1.5}), "Invalid input"),
    ] {
        let output = invoke(&tool, &ctx, args).await;
        assert!(
            output.is_error && text(&output).contains(error),
            "{output:?}"
        );
    }
    let ambiguous = make(
        root.path(),
        vec![server("normal"), server("normal")],
        1 << 20,
    );
    let output = invoke(&ambiguous, &ctx, request("hover")).await;
    assert!(text(&output).contains("ambiguous"));
    let empty = make(root.path(), vec![], 1 << 20);
    assert!(text(&invoke(&empty, &ctx, request("hover")).await).contains("no configured"));
    let mut relative = server("normal");
    relative.command = "python3".into();
    let relative = make(root.path(), vec![relative], 1 << 20);
    assert!(text(&invoke(&relative, &ctx, request("hover")).await).contains("absolute path"));
    tool.close().await.unwrap();
    tool.close().await.unwrap();
    assert!(text(&invoke(&tool, &ctx, request("hover")).await).contains("closed"));
}
#[tokio::test]
async fn source_files_are_confined_regular_single_link_and_utf8() {
    let root = fixture();
    let ctx = context(root.path());
    let tool = make(root.path(), vec![server("normal")], 1 << 20);
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("private"), "secret").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("private"),
        root.path().join("escape.txt"),
    )
    .unwrap();
    std::fs::hard_link(root.path().join("sample.txt"), root.path().join("hard.txt")).unwrap();
    std::fs::write(root.path().join("invalid.txt"), [255]).unwrap();
    std::fs::create_dir(root.path().join("dir.txt")).unwrap();
    std::fs::File::create(root.path().join("large.txt"))
        .unwrap()
        .set_len((16 << 20) + 1)
        .unwrap();
    for path in [
        "escape.txt",
        "hard.txt",
        "invalid.txt",
        "dir.txt",
        "large.txt",
    ] {
        let output = invoke(
            &tool,
            &ctx,
            json!({"operation":"documentSymbol","filePath":path}),
        )
        .await;
        assert!(output.is_error, "{path}: {output:?}");
    }
}
#[tokio::test]
async fn refuses_unconfined_executor_and_unsafe_environment() {
    let root = fixture();
    let mut config = sandbox_config(root.path());
    config.backend = adk_sandbox::Backend::Local;
    let mut forbidden = server("normal");
    forbidden.args = vec![
        "-c".into(),
        "open('unconfined-fallback', 'w').write('unsafe')".into(),
    ];
    let tool = lsp::tool(lsp::Config {
        executor: Arc::new(adk_sandbox::Executor::new(config).unwrap()),
        servers: vec![forbidden],
        discoverer: None,
    });
    let mut args = request("hover");
    args["access"] = json!("FullAccess");
    args["network"] = json!("Allow");
    args["command"] = json!("/bin/sh");
    let output = invoke(&tool, &context(root.path()), args).await;
    assert!(!root.path().join("unconfined-fallback").exists());
    assert!(
        output.is_error && text(&output).contains("local requires"),
        "{output:?}"
    );
    let mut config = server("normal");
    config.env.insert("LD_PRELOAD".into(), "evil".into());
    let tool = make(root.path(), vec![config], 1 << 20);
    let output = invoke(&tool, &context(root.path()), request("hover")).await;
    assert!(
        text(&output).contains("unsafe environment override"),
        "{output:?}"
    );
}
#[tokio::test]
async fn all_operations_sync_and_read_only_server_requests() {
    let root = fixture();
    if !available(root.path()).await {
        return;
    }
    let ctx = context(root.path());
    let tool = make(root.path(), vec![server("normal")], 1 << 20);
    for operation in [
        "goToDefinition",
        "definition",
        "findReferences",
        "references",
        "hover",
        "documentSymbol",
        "workspaceSymbol",
        "implementation",
        "typeDefinition",
        "diagnostics",
    ] {
        let output = invoke(&tool, &ctx, request(operation)).await;
        assert!(!output.is_error, "{operation}: {output:?}");
        let value: Value = serde_json::from_str(text(&output)).unwrap();
        match operation {
            "hover" => assert!(
                value["hover"]["contents"]
                    .as_str()
                    .unwrap()
                    .contains("a😀b")
            ),
            "documentSymbol" => assert_eq!(value["symbols"][0]["children"][0]["name"], "Inner"),
            "workspaceSymbol" => assert_eq!(value["symbols"].as_array().unwrap().len(), 1),
            "diagnostics" => assert_eq!(
                value["diagnostics"][0]["filePath"],
                root.path().join("sample.txt").to_str().unwrap()
            ),
            _ => {
                assert_eq!(value["locations"].as_array().unwrap().len(), 1);
                assert_eq!(value["locations"][0]["range"]["start"]["line"], 1);
            }
        }
    }
    std::fs::write(root.path().join("sample.txt"), "changed text").unwrap();
    let output = invoke(&tool, &ctx, request("hover")).await;
    assert!(
        !output.is_error && text(&output).contains("changed text"),
        "{output:?}"
    );
    let output = invoke(
        &tool,
        &ctx,
        json!({"operation":"workspaceSymbol","query":"edits"}),
    )
    .await;
    assert!(
        !output.is_error && text(&output).contains("opens=10;closes=10"),
        "{output:?}"
    );
    assert!(!root.path().join("forbidden-write").exists());
    tool.close().await.unwrap();
}
#[tokio::test]
async fn pull_push_fallback_and_versioned_diagnostics() {
    let root = fixture();
    if !available(root.path()).await {
        return;
    }
    for mode in ["normal", "push", "fallback", "stale"] {
        let mut config = server(mode);
        config.request_timeout = Duration::from_millis(150);
        let tool = make(root.path(), vec![config], 1 << 20);
        let output = invoke(&tool, &context(root.path()), request("diagnostics")).await;
        if mode == "stale" {
            assert!(
                output.is_error && text(&output).contains("timed out"),
                "{output:?}"
            );
        } else {
            assert!(
                !output.is_error
                    && text(&output).contains(if mode == "normal" {
                        "diagnostic"
                    } else {
                        "published"
                    }),
                "{mode}: {output:?}"
            );
        }
        tool.close().await.unwrap();
    }
}
#[tokio::test]
async fn timeouts_framing_limits_and_restarts() {
    let root = fixture();
    if !available(root.path()).await {
        return;
    }
    for (mode, expected) in [
        ("encoding", "unsupported position encoding"),
        ("bad-frame", "message exceeds"),
        ("stderr", "invalid LSP header"),
        ("startup-timeout", "timed out"),
        ("flood", "truncated"),
    ] {
        let mut config = server(mode);
        config.startup_timeout = Duration::from_millis(200);
        config.max_stderr_bytes = 16;
        let tool = make(
            root.path(),
            vec![config],
            if mode == "flood" { 1 } else { 1 << 20 },
        );
        let output = invoke(&tool, &context(root.path()), request("hover")).await;
        assert!(
            output.is_error && text(&output).contains(expected),
            "{mode}: {output:?}"
        );
        if mode == "stderr" {
            assert!(text(&output).contains("[stderr truncated]"), "{output:?}");
        }
        tool.close().await.unwrap();
    }
    let mut config = server("normal");
    config.request_timeout = Duration::from_millis(150);
    let tool = make(root.path(), vec![config], 1 << 20);
    let flooded = invoke(
        &tool,
        &context(root.path()),
        json!({"operation":"workspaceSymbol","query":"queue"}),
    )
    .await;
    assert!(flooded.is_error, "{flooded:?}");
    let output = invoke(
        &tool,
        &context(root.path()),
        json!({"operation":"workspaceSymbol","query":"hang"}),
    )
    .await;
    assert!(
        output.is_error && text(&output).contains("timed out"),
        "{output:?}"
    );
    assert!(
        !invoke(&tool, &context(root.path()), request("hover"))
            .await
            .is_error
    );
    tool.close().await.unwrap();
    for (field, expected) in [
        ("output", "output limit"),
        ("message", "outbound LSP message"),
    ] {
        let mut config = server("normal");
        if field == "output" {
            config.max_output_bytes = 8;
        } else {
            config.max_message_bytes = 8;
        }
        let tool = make(root.path(), vec![config], 1 << 20);
        let output = invoke(&tool, &context(root.path()), request("hover")).await;
        assert!(
            output.is_error && text(&output).contains(expected),
            "{output:?}"
        );
        tool.close().await.unwrap();
    }
}
#[tokio::test]
async fn cancellation_deadline_and_host_close_interrupt_requests() {
    let root = fixture();
    if !available(root.path()).await {
        return;
    }
    for mode in ["cancel", "deadline", "close"] {
        let tool = make(root.path(), vec![server("normal")], 1 << 20);
        let token = Arc::new(adk_runtime::CancellationToken::new());
        let mut ctx = context(root.path());
        ctx.operation.cancellation = token.clone();
        if mode == "deadline" {
            ctx.operation.deadline = Some(Instant::now() + Duration::from_millis(300));
        }
        let started = Instant::now();
        let operation = tool.execute(
            &ctx,
            ToolCall {
                id: "hang".into(),
                name: "LSP".into(),
                arguments: json!({"operation":"workspaceSymbol","query":"hang"}),
            },
        );
        let stop = async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if mode == "cancel" {
                token.cancel();
            }
            if mode == "close" {
                tool.close().await.unwrap();
            }
        };
        let (output, _) = tokio::join!(operation, stop);
        if mode == "close" {
            assert!(output.unwrap().is_error);
        } else {
            assert!(output.is_err());
        }
        assert!(started.elapsed() < Duration::from_secs(2));
        tool.close().await.unwrap();
    }
}

#[tokio::test]
async fn bundle_close_reaps_active_lsp_before_returning() {
    let root = fixture();
    if !available(root.path()).await {
        return;
    }
    let script = root.path().join("owned-lsp-server.py");
    std::fs::write(
        &script,
        r#"
import json, os, signal, sys, time
signal.signal(signal.SIGTERM, signal.SIG_IGN)
while True:
    size = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line in (b"\r\n", b"\n"):
            break
        name, value = line.split(b":", 1)
        if name.lower() == b"content-length":
            size = int(value)
    request = json.loads(sys.stdin.buffer.read(size))
    method = request.get("method")
    if method == "initialize":
        result = {"capabilities": {"positionEncoding": "utf-16"}}
    elif method == "textDocument/hover":
        result = {"contents": str(os.getpid())}
    elif method == "workspace/symbol":
        time.sleep(30)
        result = []
    else:
        continue
    data = json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(data)}\r\n\r\n".encode() + data)
    sys.stdout.buffer.flush()
    if method == "textDocument/hover":
        time.sleep(30)
"#,
    )
    .unwrap();
    let mut config = server("normal");
    config.args = vec![script.to_str().unwrap().into()];
    let mut sandbox = sandbox_config(root.path());
    sandbox.term_grace = Duration::from_millis(500);
    let mut bundle = adk_tools::bundle::BundleBuilder::new(Config {
        features: Features::Strict(["LSP".into()].into()),
        ..Default::default()
    })
    .lsp(lsp::Config {
        executor: Arc::new(adk_sandbox::Executor::new(sandbox).unwrap()),
        servers: vec![config],
        discoverer: None,
    })
    .build(ToolPolicy::default())
    .unwrap();
    let prepared = bundle.prepared();
    let tool = &prepared.tools[0];
    let ctx = context(root.path());
    let output = tool
        .execute(
            &ctx,
            ToolCall {
                id: "pid".into(),
                name: "LSP".into(),
                arguments: request("hover"),
            },
        )
        .await
        .unwrap();
    assert!(!output.is_error, "{output:?}");
    let result: Value = serde_json::from_str(text(&output)).unwrap();
    let pid: i32 = result["hover"]["contents"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    #[cfg(target_os = "macos")]
    let pids = vec![pid];
    // Bubblewrap gives the server a private PID namespace; locate the host PIDs
    // using the unique script argument and verify the reported namespace PID.
    #[cfg(target_os = "linux")]
    let pids = {
        let mut pids = Vec::new();
        let mut found_server = false;
        for entry in std::fs::read_dir("/proc").unwrap().flatten() {
            let Ok(host_pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
                continue;
            };
            let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
            if !cmdline
                .split(|b| *b == 0)
                .any(|arg| arg == script.as_os_str().as_encoded_bytes())
            {
                continue;
            }
            let status = std::fs::read_to_string(entry.path().join("status")).unwrap();
            found_server |= status
                .lines()
                .find(|line| line.starts_with("NSpid:"))
                .and_then(|line| line.split_whitespace().last())
                .and_then(|value| value.parse::<i32>().ok())
                == Some(pid);
            pids.push(host_pid);
        }
        assert!(
            found_server,
            "server namespace PID {pid} missing from host process table"
        );
        assert!(!pids.is_empty());
        pids
    };
    let mut operation = tool.execute(
        &ctx,
        ToolCall {
            id: "active".into(),
            name: "LSP".into(),
            arguments: json!({"operation":"workspaceSymbol","query":"hang"}),
        },
    );
    std::future::poll_fn(|cx| {
        assert!(operation.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let (closed, output) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(biased; bundle.close(), operation)
    })
    .await
    .unwrap();
    closed.unwrap();
    assert_eq!(output.unwrap_err().info.category, ErrorCategory::Cancelled);
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    for pid in pids {
        assert_eq!(
            unsafe { kill(pid, 0) },
            -1,
            "PID {pid} still exists immediately after close"
        );
        assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(3));
    }
}
