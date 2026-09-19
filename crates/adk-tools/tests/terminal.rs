#![cfg(unix)]
use adk_core::*;
use adk_tools::shell;
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
fn setup() -> (tempfile::TempDir, shell::ShellBundle, ToolContext) {
    let dir = tempfile::tempdir().unwrap();
    let mut sandbox = adk_sandbox::Config::new(dir.path());
    sandbox.backend = adk_sandbox::Backend::Local;
    sandbox.term_grace = Duration::from_millis(20);
    let bundle = shell::ShellBundle::new(shell::Config {
        sandbox,
        access: AccessMode::FullAccess,
        git_remote_writes: true,
        environment: BTreeMap::new(),
    })
    .unwrap();
    let ctx = ToolContext {
        operation: Context {
            run_id: "terminal-test".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: dir.path().into(),
        policy: ToolPolicy {
            access: AccessMode::FullAccess,
            ..Default::default()
        },
        idempotency_key: None,
    };
    (dir, bundle, ctx)
}
async fn execute(bundle: &shell::ShellBundle, ctx: &ToolContext, args: Value) -> ToolOutput {
    let tool = bundle
        .tools()
        .into_iter()
        .find(|t| t.definition().name == "Terminal")
        .unwrap();
    tool.execute(
        ctx,
        ToolCall {
            id: "term".into(),
            name: "Terminal".into(),
            arguments: args,
        },
    )
    .await
    .unwrap()
}
fn text(out: &ToolOutput) -> &str {
    match &out.content[..] {
        [Content::Text { text }] => text,
        _ => panic!("text"),
    }
}
async fn call(bundle: &shell::ShellBundle, ctx: &ToolContext, args: Value) -> Value {
    let out = execute(bundle, ctx, args).await;
    assert!(!out.is_error, "{}", text(&out));
    serde_json::from_str(text(&out)).unwrap()
}
#[test]
fn all_keys_and_verbatim_strings() {
    for (key, bytes) in [
        ("C-c", &b"\x03"[..]),
        ("C-d", b"\x04"),
        ("C-z", b"\x1a"),
        ("C-l", b"\x0c"),
        ("Enter", b"\r"),
        ("Escape", b"\x1b"),
        ("Tab", b"\t"),
        ("Up", b"\x1b[A"),
        ("Down", b"\x1b[B"),
        ("Right", b"\x1b[C"),
        ("Left", b"\x1b[D"),
        ("C-c\n", b"C-c\n"),
        ("echo x\n", b"echo x\n"),
    ] {
        assert_eq!(shell::translate_keystrokes(key), bytes);
    }
}
#[tokio::test]
async fn pty_start_send_read_list_kill_and_persistent_state() {
    let (_dir, bundle, ctx) = setup();
    assert_eq!(call(&bundle, &ctx, json!({"op":"list"})).await, json!([]));
    let snap = call(&bundle, &ctx, json!({"op":"start","wait_ms":80})).await;
    assert_eq!(snap["session_id"], "term-1");
    assert_eq!(snap["status"], "running");
    let snap = call(&bundle,&ctx,json!({"op":"send","session_id":"term-1","keystrokes":"export CHECK_VALUE=remembered; stty size; printf 'VALUE=%s\\n' \"$CHECK_VALUE\"\n","wait_ms":100})).await;
    assert!(snap["output"].as_str().unwrap().contains("40 160"));
    assert!(
        snap["output"]
            .as_str()
            .unwrap()
            .contains("VALUE=remembered")
    );
    let snap = call(&bundle,&ctx,json!({"op":"send","session_id":"term-1","keystrokes":"printf 'PERSIST=%s\\n' \"$CHECK_VALUE\"\n","wait_ms":100})).await;
    assert!(
        snap["output"]
            .as_str()
            .unwrap()
            .contains("PERSIST=remembered")
    );
    let snap = call(&bundle, &ctx, json!({"op":"read","session_id":"term-1"})).await;
    assert!(snap.get("output").is_none());
    assert!(snap["note"].as_str().unwrap().contains("no new output"));
    let list = call(&bundle, &ctx, json!({"op":"list"})).await;
    assert_eq!(list[0]["status"], "running");
    assert!(list[0].get("output").is_none());
    assert_eq!(
        call(&bundle, &ctx, json!({"op":"kill","session_id":"term-1"})).await["status"],
        "exited"
    );
    let out = execute(
        &bundle,
        &ctx,
        json!({"op":"send","session_id":"term-1","keystrokes":"true\n"}),
    )
    .await;
    assert!(out.is_error && text(&out).contains("session has exited"));
    bundle.close().await;
}
#[tokio::test]
async fn control_c_interrupts_foreground_and_eof_exits_shell() {
    let (_dir, bundle, ctx) = setup();
    call(&bundle, &ctx, json!({"op":"start","wait_ms":50})).await;
    call(
        &bundle,
        &ctx,
        json!({"op":"send","session_id":"term-1","keystrokes":"sleep 30\n","wait_ms":40}),
    )
    .await;
    let snap = call(
        &bundle,
        &ctx,
        json!({"op":"send","session_id":"term-1","keystrokes":"C-c","wait_ms":100}),
    )
    .await;
    assert_eq!(snap["status"], "running");
    let snap = call(&bundle,&ctx,json!({"op":"send","session_id":"term-1","keystrokes":"printf RECOVERED\\n\n","wait_ms":100})).await;
    assert!(snap["output"].as_str().unwrap().contains("RECOVERED"));
    let snap = call(
        &bundle,
        &ctx,
        json!({"op":"send","session_id":"term-1","keystrokes":"C-d","wait_ms":1000}),
    )
    .await;
    assert_eq!(snap["status"], "exited");
    bundle.close().await;
}
#[tokio::test]
async fn invalid_operations_and_access_are_rejected() {
    let (_dir, bundle, mut ctx) = setup();
    for (args, message) in [
        (json!({"op":"what"}), "invalid op"),
        (
            json!({"op":"read","session_id":"missing"}),
            "unknown session_id",
        ),
        (json!({"op":3}), "Invalid input"),
    ] {
        let out = execute(&bundle, &ctx, args).await;
        assert!(out.is_error && text(&out).contains(message));
    }
    call(&bundle, &ctx, json!({"op":"start","wait_ms":20})).await;
    let out = execute(&bundle, &ctx, json!({"op":"send","session_id":"term-1"})).await;
    assert!(out.is_error && text(&out).contains("keystrokes is required"));
    ctx.policy.access = AccessMode::WorkspaceWrite;
    let out = execute(&bundle, &ctx, json!({"op":"list"})).await;
    assert!(out.is_error && text(&out).contains("requires danger-full-access"));
    bundle.close().await;
}
#[tokio::test]
async fn terminal_output_is_capped_and_teardown_is_awaited() {
    let (_dir, bundle, ctx) = setup();
    call(&bundle, &ctx, json!({"op":"start","wait_ms":50})).await;
    let snap = call(&bundle,&ctx,json!({"op":"send","session_id":"term-1","keystrokes":"head -c 50000 /dev/zero | tr '\\0' x; printf TAIL\\n\n","wait_ms":300})).await;
    let out = snap["output"].as_str().unwrap();
    assert!(out.contains("output limited to 10000 bytes"));
    assert!(out.contains("TAIL"));
    assert!(out.len() < 11_000);
    tokio::time::timeout(Duration::from_secs(3), bundle.close())
        .await
        .unwrap();
    assert_eq!(
        call(&bundle, &ctx, json!({"op":"list"})).await[0]["status"],
        "exited"
    );
}
#[test]
fn no_terminal_without_full_access_and_remote_permission() {
    let dir = tempfile::tempdir().unwrap();
    for (access, remote) in [
        (AccessMode::ReadOnly, true),
        (AccessMode::WorkspaceWrite, true),
        (AccessMode::FullAccess, false),
    ] {
        let bundle = shell::ShellBundle::new(shell::Config {
            sandbox: adk_sandbox::Config::new(dir.path()),
            access,
            git_remote_writes: remote,
            environment: BTreeMap::new(),
        })
        .unwrap();
        assert!(
            !bundle
                .tools()
                .iter()
                .any(|t| t.definition().name == "Terminal")
        );
    }
}
