#![cfg(any(target_os = "linux", target_os = "macos"))]
use adk_core::*;
use adk_tools::{Config, Features, Registry, browser};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
#[derive(Default)]
struct Runner {
    requests: Mutex<Vec<browser::Request>>,
    output: Vec<u8>,
    status: i32,
    screenshot: bool,
    fail: bool,
}
impl browser::Runner for Runner {
    fn run<'a>(
        &'a self,
        _: &'a ToolContext,
        mut request: browser::Request,
    ) -> BoxFuture<'a, Result<browser::Execution, String>> {
        Box::pin(async move {
            if self.screenshot {
                std::fs::write(
                    request.writable_paths[0].join("screenshot.png"),
                    b"PNG fixture",
                )
                .unwrap();
            }
            request.scratch.take();
            self.requests.lock().unwrap().push(request);
            if self.fail {
                return Err("runner failed".into());
            }
            Ok(browser::Execution {
                output: self.output.clone(),
                exit_code: self.status,
                timed_out: false,
            })
        })
    }
}
fn context(path: PathBuf) -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "browser".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: path,
        policy: Default::default(),
        idempotency_key: None,
    }
}
fn make(runner: Arc<Runner>, access: AccessMode, private: bool, dir: PathBuf) -> Arc<dyn Tool> {
    browser::tool(browser::Config {
        runner,
        executable: Some("/usr/bin/chromium".into()),
        screenshot_dir: dir,
        access,
        allow_private_network_urls: private,
    })
}
async fn invoke(tool: &Arc<dyn Tool>, ctx: &ToolContext, args: Value) -> ToolOutput {
    tool.execute(
        ctx,
        ToolCall {
            id: "b".into(),
            name: "Browser".into(),
            arguments: args,
        },
    )
    .await
    .unwrap()
}
fn text(output: &ToolOutput) -> &str {
    match &output.content[..] {
        [Content::Text { text }] => text,
        _ => panic!("text"),
    }
}
#[tokio::test]
async fn navigation_text_and_launch_contracts() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path().into());
    let runner = Arc::new(Runner {
        output: b"<TITLE class='x'> A &amp; B </TITLE><p>Hello</p>".to_vec(),
        ..Default::default()
    });
    let tool = make(
        runner.clone(),
        AccessMode::WorkspaceWrite,
        true,
        root.path().into(),
    );
    let result = invoke(
        &tool,
        &ctx,
        json!({"action":"navigate","url":"http://example.com/path","width":400,"height":500}),
    )
    .await;
    assert_eq!(
        text(&result),
        "Navigated to http://example.com/path\nTitle: A &amp; B"
    );
    let result = invoke(
        &tool,
        &ctx,
        json!({"action":"get_text","url":"http://example.com/path"}),
    )
    .await;
    assert_eq!(text(&result), "A & B\n\nHello");
    let requests = runner.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].access, AccessMode::ReadOnly);
    assert_eq!(requests[0].timeout, Duration::from_secs(30));
    assert!(requests[0].writable_paths.is_empty());
    assert_eq!(
        requests[0].args,
        vec![
            "--headless",
            "--disable-gpu",
            "--disable-dev-shm-usage",
            "--no-sandbox",
            "--dump-dom",
            "--window-size=400,500",
            "http://example.com/path"
        ]
    );
    assert!(requests[1].args.contains(&"--window-size=1280,720".into()));
}
#[tokio::test]
async fn policy_and_invalid_inputs_never_launch() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path().into());
    let runner = Arc::new(Runner::default());
    for args in [
        Value::Null,
        json!({"url":3}),
        json!({"url":"file:///etc/passwd","action":"navigate"}),
        json!({"url":"http://user:pass@example.com","action":"navigate"}),
        json!({"url":"http://example.com","action":"click"}),
        json!({"url":"http://example.com","action":"navigate","width":99}),
        json!({"url":"http://example.com","action":"navigate","height":4097}),
    ] {
        assert!(
            invoke(
                &make(
                    runner.clone(),
                    AccessMode::ReadOnly,
                    true,
                    root.path().into()
                ),
                &ctx,
                args
            )
            .await
            .is_error
        );
    }
    let screenshot = json!({"url":"http://example.com","action":"screenshot"});
    assert_eq!(
        text(
            &invoke(
                &make(
                    runner.clone(),
                    AccessMode::ReadOnly,
                    true,
                    root.path().into()
                ),
                &ctx,
                screenshot.clone()
            )
            .await
        ),
        "screenshot requires workspace-write access"
    );
    let result = invoke(
        &make(
            runner.clone(),
            AccessMode::WorkspaceWrite,
            false,
            root.path().into(),
        ),
        &ctx,
        screenshot,
    )
    .await;
    assert!(result.is_error);
    assert!(text(&result).contains("public-only networking"));
    assert!(runner.requests.lock().unwrap().is_empty());
    let registry = Registry::build(
        &Config {
            features: Features::Strict(["Browser".into()].into()),
            allow_private_network_urls: true,
            access: AccessMode::ReadOnly,
            ..Default::default()
        },
        [make(runner, AccessMode::ReadOnly, true, root.path().into())],
    )
    .unwrap();
    assert!(registry.get("Browser").unwrap().definition().read_only);
}
#[tokio::test]
async fn screenshots_publish_atomically_preserve_modes_and_remove_temporaries() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let implicit = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let ctx = context(root.path().into());
    let runner = Arc::new(Runner {
        screenshot: true,
        ..Default::default()
    });
    let tool = make(
        runner.clone(),
        AccessMode::WorkspaceWrite,
        true,
        implicit.path().into(),
    );
    let result = invoke(
        &tool,
        &ctx,
        json!({"url":"http://example.com","action":"screenshot","output_path":"sub/picture.png"}),
    )
    .await;
    assert_eq!(
        text(&result),
        "Screenshot saved to sub/picture.png (1280x720)"
    );
    assert_eq!(
        std::fs::read(root.path().join("sub/picture.png")).unwrap(),
        b"PNG fixture"
    );
    assert_eq!(
        std::fs::metadata(root.path().join("sub"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(root.path().join("sub/picture.png"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    std::fs::set_permissions(
        root.path().join("sub/picture.png"),
        std::fs::Permissions::from_mode(0o640),
    )
    .unwrap();
    assert!(!invoke(&tool,&ctx,json!({"url":"http://example.com","action":"screenshot","output_path":"sub/picture.png"})).await.is_error);
    assert_eq!(
        std::fs::metadata(root.path().join("sub/picture.png"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o640
    );
    let result = invoke(
        &tool,
        &ctx,
        json!({"url":"http://example.com","action":"screenshot"}),
    )
    .await;
    assert!(!result.is_error);
    assert!(text(&result).contains(&implicit.path().display().to_string()));
    symlink(outside.path(), root.path().join("escape")).unwrap();
    assert!(
        invoke(
            &tool,
            &ctx,
            json!({"url":"http://example.com","action":"screenshot","output_path":"escape/out.png"})
        )
        .await
        .is_error
    );
    assert!(
        invoke(
            &tool,
            &ctx,
            json!({"url":"http://example.com","action":"screenshot","output_path":"../out.png"})
        )
        .await
        .is_error
    );
    assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    for request in runner.requests.lock().unwrap().iter() {
        assert_eq!(request.access, AccessMode::WorkspaceWrite);
        assert!(!request.writable_paths[0].exists());
    }
}
#[tokio::test]
async fn failures_and_truncation_have_sdk_results() {
    let root = tempfile::tempdir().unwrap();
    let ctx = context(root.path().into());
    for (runner, expected) in [
        (
            Runner {
                status: 1,
                output: b"stderr".to_vec(),
                ..Default::default()
            },
            "Navigation failed: <nil>\nstderr",
        ),
        (
            Runner {
                fail: true,
                ..Default::default()
            },
            "Navigation failed: runner failed\n",
        ),
    ] {
        assert_eq!(
            text(
                &invoke(
                    &make(
                        Arc::new(runner),
                        AccessMode::ReadOnly,
                        true,
                        root.path().into()
                    ),
                    &ctx,
                    json!({"url":"http://example.com","action":"navigate"})
                )
                .await
            ),
            expected
        );
    }
    let tool = make(
        Arc::new(Runner {
            output: vec![b'x'; 50001],
            ..Default::default()
        }),
        AccessMode::ReadOnly,
        true,
        root.path().into(),
    );
    let result = invoke(
        &tool,
        &ctx,
        json!({"url":"http://example.com","action":"get_text"}),
    )
    .await;
    assert_eq!(
        text(&result),
        format!(
            "{}\n\n--- Content truncated at 50000 characters ---",
            "x".repeat(50000)
        )
    );
    let tool = browser::tool(browser::Config {
        runner: Arc::new(Runner::default()),
        executable: None,
        screenshot_dir: root.path().into(),
        access: AccessMode::ReadOnly,
        allow_private_network_urls: true,
    });
    assert!(
        text(
            &invoke(
                &tool,
                &ctx,
                json!({"url":"http://example.com","action":"navigate"})
            )
            .await
        )
        .contains("No Chromium/Chrome binary found")
    );
}

#[tokio::test]
async fn cancelled_browser_releases_runner_and_private_staging() {
    struct Pending {
        started: tokio::sync::Notify,
        active: std::sync::atomic::AtomicBool,
        path: Mutex<Option<PathBuf>>,
    }
    impl browser::Runner for Pending {
        fn run<'a>(
            &'a self,
            _: &'a ToolContext,
            request: browser::Request,
        ) -> BoxFuture<'a, Result<browser::Execution, String>> {
            Box::pin(async move {
                struct Guard<'a>(&'a std::sync::atomic::AtomicBool);
                impl Drop for Guard<'_> {
                    fn drop(&mut self) {
                        self.0.store(false, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                self.active.store(true, std::sync::atomic::Ordering::SeqCst);
                let _guard = Guard(&self.active);
                *self.path.lock().unwrap() = Some(request.writable_paths[0].clone());
                self.started.notify_one();
                std::future::pending().await
            })
        }
    }
    let root = tempfile::tempdir().unwrap();
    let runner = Arc::new(Pending {
        started: tokio::sync::Notify::new(),
        active: std::sync::atomic::AtomicBool::new(false),
        path: Mutex::new(None),
    });
    let tool = browser::tool(browser::Config {
        runner: runner.clone(),
        executable: Some("/usr/bin/chromium".into()),
        screenshot_dir: root.path().into(),
        access: AccessMode::WorkspaceWrite,
        allow_private_network_urls: true,
    });
    let token = Arc::new(adk_runtime::CancellationToken::new());
    let mut ctx = context(root.path().into());
    ctx.operation.cancellation = token.clone();
    let operation = tokio::spawn(async move {
        tool.execute(&ctx,ToolCall{id:"cancel".into(),name:"Browser".into(),arguments:json!({"url":"http://example.com","action":"screenshot","output_path":"unpublished.png"})}).await
    });
    tokio::time::timeout(Duration::from_secs(1), runner.started.notified())
        .await
        .unwrap();
    token.cancel();
    assert_eq!(
        operation.await.unwrap().unwrap_err().info.category,
        ErrorCategory::Cancelled
    );
    assert!(!runner.active.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!runner.path.lock().unwrap().as_ref().unwrap().exists());
    assert!(!root.path().join("unpublished.png").exists());
}

#[tokio::test]
async fn prepared_browser_adapts_and_dispatches_the_same_readonly_contract() {
    let root = tempfile::tempdir().unwrap();
    let runner = Arc::new(Runner::default());
    let original = make(
        runner.clone(),
        AccessMode::WorkspaceWrite,
        true,
        root.path().into(),
    );
    let registry = Registry::build(
        &Config {
            features: Features::Strict(["Browser".into()].into()),
            access: AccessMode::WorkspaceWrite,
            allow_private_network_urls: true,
            ..Default::default()
        },
        [original],
    )
    .unwrap();
    let prepared = registry.prepare(ToolPolicy::default());
    assert_eq!(prepared.tools.len(), 1);
    let adapted = &prepared.tools[0];
    assert!(adapted.definition().read_only);
    assert_eq!(
        prepared.policy.decision(adapted.definition()),
        ToolDecision::Allow
    );
    let result = invoke(
        adapted,
        &context(root.path().into()),
        json!({"action":"screenshot","url":"https://example.com"}),
    )
    .await;
    assert!(result.is_error);
    assert!(runner.requests.lock().unwrap().is_empty());
    let granted = registry.prepare(ToolPolicy {
        allowed_mutating_tools: ["Browser".into()].into(),
        ..Default::default()
    });
    assert!(!granted.tools[0].definition().read_only);
    let denied = registry.prepare(ToolPolicy {
        denied_tools: ["Browser".into()].into(),
        ..Default::default()
    });
    assert!(denied.tools.is_empty());
}

#[tokio::test]
async fn enforcing_browser_runner_publishes_only_owned_screenshot() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    for dir in [".git/hooks", ".codex", ".claude", ".gemini", ".agents"] {
        std::fs::create_dir_all(root.path().join(dir)).unwrap();
    }
    for file in [".git/config", ".mcp.json"] {
        std::fs::write(root.path().join(file), b"").unwrap();
    }
    let ctx = context(root.path().into());
    let executor =
        Arc::new(adk_sandbox::Executor::new(adk_sandbox::Config::new(root.path())).unwrap());
    let mut probe = adk_sandbox::Request::new("/bin/sh");
    probe.args = vec!["-c".into(), "exit 0".into()];
    probe.access = AccessMode::WorkspaceWrite;
    let available = executor.run(&ctx.operation, probe).await;
    if !available.as_ref().is_ok_and(|r| r.status.success()) {
        assert!(
            std::env::var_os("ADK_REQUIRE_SANDBOX").is_none(),
            "required enforcement unavailable: {available:?}"
        );
        eprintln!("SKIP native browser transport: {available:?}");
        return;
    }
    // A deterministic executable fixture exercises the real sandbox transport,
    // scratch grant, screenshot publication and process reaping, not rendering.
    let executable = root.path().join("chromium-fixture");
    std::fs::write(&executable, "#!/bin/sh\nfor arg; do case \"$arg\" in --screenshot=*) printf PNG > \"${arg#--screenshot=}\";; esac; done\nprintf '<title>native</title><p>fixture</p>'\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let tool = browser::tool(browser::Config {
        runner: executor,
        executable: Some(executable),
        screenshot_dir: root.path().join("images"),
        access: AccessMode::WorkspaceWrite,
        allow_private_network_urls: true,
    });
    let result = invoke(
        &tool,
        &ctx,
        json!({"action":"screenshot","url":"https://example.com","output_path":"shot.png"}),
    )
    .await;
    assert!(!result.is_error, "{}", text(&result));
    assert_eq!(std::fs::read(root.path().join("shot.png")).unwrap(), b"PNG");
}

struct NativeProcessFixture {
    executor: adk_sandbox::Executor,
    finished: tokio::sync::Notify,
}
impl NativeProcessFixture {
    fn new(root: &std::path::Path) -> Arc<Self> {
        let mut config = adk_sandbox::Config::new(root);
        config.backend = adk_sandbox::Backend::Local;
        config.term_grace = Duration::from_millis(150);
        Arc::new(Self {
            executor: adk_sandbox::Executor::new(config).unwrap(),
            finished: tokio::sync::Notify::new(),
        })
    }
}
impl browser::Runner for NativeProcessFixture {
    fn run<'a>(
        &'a self,
        context: &'a ToolContext,
        mut request: browser::Request,
    ) -> BoxFuture<'a, Result<browser::Execution, String>> {
        Box::pin(async move {
            // Explicit FullAccess Local fixture tests lifecycle, not confinement.
            request.executable = "/bin/sh".into();
            request.args = vec!["-c".into(), "echo $$ > leader; exec sleep 30".into()];
            request.access = AccessMode::FullAccess;
            let result = browser::Runner::run(&self.executor, context, request).await;
            self.finished.notify_one();
            result
        })
    }
}
fn native_browser_config(runner: Arc<dyn browser::Runner>) -> browser::Config {
    browser::Config {
        runner,
        executable: Some("/bin/sh".into()),
        screenshot_dir: PathBuf::new(),
        access: AccessMode::ReadOnly,
        allow_private_network_urls: true,
    }
}
fn native_browser_call() -> ToolCall {
    ToolCall {
        id: "native-cleanup".into(),
        name: "Browser".into(),
        arguments: json!({"action":"navigate","url":"https://example.com"}),
    }
}
async fn native_pid(root: &std::path::Path) -> i32 {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(pid) = std::fs::read_to_string(root.join("leader"))
                && let Ok(pid) = pid.trim().parse()
            {
                return pid;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
fn assert_native_reaped(pid: i32) {
    unsafe extern "C" {
        fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
        fn kill(pid: i32, signal: i32) -> i32;
    }
    assert_eq!(unsafe { waitpid(pid, std::ptr::null_mut(), 1) }, -1);
    assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(10));
    assert_eq!(
        unsafe { kill(pid, 0) },
        -1,
        "child still exists after close"
    );
    assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(3));
}

#[tokio::test]
async fn managed_browser_close_reaps_active_cancelled_and_abandoned_calls() {
    for mode in ["active", "cancelled", "abandoned"] {
        let root = tempfile::tempdir().unwrap();
        let runner = Arc::new(browser::ManagedRunner::new(NativeProcessFixture::new(
            root.path(),
        )));
        let tool = browser::tool(native_browser_config(runner.clone()));
        let mut ctx = context(root.path().into());
        let token = Arc::new(adk_runtime::CancellationToken::new());
        ctx.operation.cancellation = token.clone();
        let operation =
            tokio::spawn(async move { tool.execute(&ctx, native_browser_call()).await });
        let pid = native_pid(root.path()).await;
        match mode {
            "cancelled" => token.cancel(),
            "abandoned" => operation.abort(),
            _ => {}
        }
        runner.close().await.unwrap();
        assert_native_reaped(pid);
        runner.close().await.unwrap();
        let result = operation.await;
        match mode {
            "cancelled" => assert_eq!(
                result.unwrap().unwrap_err().info.category,
                ErrorCategory::Cancelled
            ),
            "abandoned" => assert!(result.unwrap_err().is_cancelled()),
            _ => assert!(result.unwrap().unwrap().is_error),
        }
        let tool = browser::tool(native_browser_config(runner));
        let output = tool
            .execute(&context(root.path().into()), native_browser_call())
            .await
            .unwrap();
        assert!(text(&output).contains("browser runner is closed"));
    }
}

#[tokio::test]
async fn managed_browser_close_can_be_retried_after_its_future_is_dropped() {
    let root = tempfile::tempdir().unwrap();
    let runner = Arc::new(browser::ManagedRunner::new(NativeProcessFixture::new(
        root.path(),
    )));
    let tool = browser::tool(native_browser_config(runner.clone()));
    let ctx = context(root.path().into());
    let operation = tokio::spawn(async move { tool.execute(&ctx, native_browser_call()).await });
    let pid = native_pid(root.path()).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(10), runner.close())
            .await
            .is_err()
    );
    let (first, second) = tokio::join!(runner.close(), runner.close());
    first.unwrap();
    second.unwrap();
    assert_native_reaped(pid);
    assert!(operation.await.unwrap().unwrap().is_error);
}

#[tokio::test]
async fn managed_browser_drop_cancels_abandoned_native_process() {
    let root = tempfile::tempdir().unwrap();
    let fixture = NativeProcessFixture::new(root.path());
    let runner = Arc::new(browser::ManagedRunner::new(fixture.clone()));
    let tool = browser::tool(native_browser_config(runner.clone()));
    let ctx = context(root.path().into());
    let operation = tokio::spawn(async move { tool.execute(&ctx, native_browser_call()).await });
    let pid = native_pid(root.path()).await;
    operation.abort();
    assert!(operation.await.unwrap_err().is_cancelled());
    drop(runner);
    tokio::time::timeout(Duration::from_secs(5), fixture.finished.notified())
        .await
        .unwrap();
    assert_native_reaped(pid);
}

#[tokio::test]
async fn bundle_browser_close_reaps_native_process_without_eventual_wait() {
    for abandoned in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let config = Config {
            features: Features::Strict(["Browser".into()].into()),
            allow_private_network_urls: true,
            ..Default::default()
        };
        let mut bundle = adk_tools::bundle::BundleBuilder::new(config)
            .browser(native_browser_config(NativeProcessFixture::new(
                root.path(),
            )))
            .build(ToolPolicy::default())
            .unwrap();
        let tool = bundle.prepared().tools[0].clone();
        let ctx = context(root.path().into());
        let operation =
            tokio::spawn(async move { tool.execute(&ctx, native_browser_call()).await });
        let pid = native_pid(root.path()).await;
        if abandoned {
            operation.abort();
        }
        bundle.close().await.unwrap();
        assert_native_reaped(pid);
        let _ = operation.await;
    }
}

#[tokio::test]
async fn managed_browser_preserves_scratch_and_secret_output_guards() {
    use browser::Runner as _;
    let root = tempfile::tempdir().unwrap();
    let mut config = adk_sandbox::Config::new(root.path());
    config.backend = adk_sandbox::Backend::Local;
    config.term_grace = Duration::from_millis(5);
    let runner = browser::ManagedRunner::new(Arc::new(adk_sandbox::Executor::new(config).unwrap()));
    let ctx = context(root.path().into());
    for case in ["scratch", "stdout", "stderr"] {
        let secret = format!("ghp_{}", "A".repeat(36));
        let script = if case == "stderr" {
            format!("printf '{secret}' >&2")
        } else {
            format!("printf '{secret}'")
        };
        let request = browser::Request {
            executable: "/bin/sh".into(),
            args: vec!["-c".into(), script],
            work_dir: root.path().into(),
            access: AccessMode::FullAccess,
            writable_paths: if case == "scratch" {
                vec![root.path().into()]
            } else {
                vec![]
            },
            scratch: None,
            timeout: Duration::from_secs(5),
        };
        let error = runner
            .run(&ctx, request)
            .await
            .err()
            .expect("guard must reject");
        if case == "scratch" {
            assert!(error.contains("owned scratch grants"), "{error}");
        } else {
            assert!(error.contains("payload blocked"), "{error}");
            assert!(!error.contains(&secret));
        }
    }
    runner.close().await.unwrap();
}
