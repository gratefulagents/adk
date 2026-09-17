use adk_core::*;
use adk_tools::{
    BuildError, Config, Features,
    bundle::{BundleBuilder, BundleError},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};

fn config(features: &[&str]) -> Config {
    Config {
        features: Features::Strict(features.iter().map(|s| (*s).into()).collect()),
        ..Default::default()
    }
}
fn context() -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "bundle-test".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: ".".into(),
        policy: ToolPolicy::default(),
        idempotency_key: None,
    }
}
fn call(name: &str) -> ToolCall {
    ToolCall {
        id: "call".into(),
        name: name.into(),
        arguments: json!({}),
    }
}
#[test]
fn absent_host_dependencies_fail_closed() {
    for feature in ["Bash", "LSP"] {
        assert!(matches!(
            BundleBuilder::new(config(&[feature])).build(ToolPolicy::default()),
            Err(BundleError::Build(BuildError::Unavailable(_)))
        ));
    }
    let mut config = config(&["Browser"]);
    config.allow_private_network_urls = true;
    assert!(matches!(
        BundleBuilder::new(config).build(ToolPolicy::default()),
        Err(BundleError::Build(BuildError::Unavailable(_)))
    ));
}
#[tokio::test]
async fn prepare_filters_once_and_close_invalidates_existing_handles() {
    let mut bundle = BundleBuilder::new(config(&["Think", "Signals.Finish"]))
        .build(ToolPolicy {
            denied_tools: ["think".into()].into(),
            ..Default::default()
        })
        .unwrap();
    let prepared = bundle.prepared();
    assert_eq!(
        prepared
            .tools
            .iter()
            .map(|t| t.definition().name.as_str())
            .collect::<Vec<_>>(),
        ["finish"]
    );
    assert_eq!(
        prepared.policy.allowed_tools,
        Some(["finish".into()].into())
    );
    assert!(prepared.policy.allowed_mutating_tools.contains("finish"));
    let ctx = context();
    let owned = bundle.context(&ctx.operation);
    bundle.close().await.unwrap();
    bundle.close().await.unwrap();
    assert!(owned.cancellation.is_cancelled());
    assert!(!ctx.operation.cancellation.is_cancelled());
    assert_eq!(
        prepared.tools[0]
            .execute(&ctx, call("finish"))
            .await
            .unwrap_err()
            .info
            .category,
        ErrorCategory::Cancelled
    );
}

struct PendingTool {
    definition: ToolDefinition,
    started: Arc<tokio::sync::Notify>,
    dropped: Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for PendingTool {
    fn drop(&mut self) {
        self.dropped
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
impl Tool for PendingTool {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(42))
    }
    fn execute<'a>(
        &'a self,
        _: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            self.started.notify_one();
            std::future::pending().await
        })
    }
}
#[tokio::test]
async fn dropping_owner_cancels_active_calls_and_releases_tools() {
    let started = Arc::new(tokio::sync::Notify::new());
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut definition = adk_tools::capabilities()
        .iter()
        .find_map(|c| c.definition.clone())
        .unwrap();
    definition.name = "custom".into();
    definition.read_only = true;
    let tool: Arc<dyn Tool> = Arc::new(PendingTool {
        definition,
        started: started.clone(),
        dropped: dropped.clone(),
    });
    let config = config(&["ExtraTools"]);
    let bundle = BundleBuilder::new(config)
        .extra_tools([tool])
        .build(ToolPolicy::default())
        .unwrap();
    let prepared = bundle.prepared();
    assert_eq!(prepared.tools[0].timeout(), Some(Duration::from_secs(42)));
    let handle = prepared.tools[0].clone();
    let task = tokio::spawn(async move { handle.execute(&context(), call("custom")).await });
    started.notified().await;
    drop(bundle);
    let error = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::Cancelled);
    assert!(dropped.load(std::sync::atomic::Ordering::SeqCst));
}

#[cfg(unix)]
#[tokio::test]
async fn owned_shell_is_real_and_teardown_closes_background_jobs() {
    let root = tempfile::tempdir().unwrap();
    let mut sandbox = adk_sandbox::Config::new(root.path());
    sandbox.backend = adk_sandbox::Backend::Local;
    sandbox.term_grace = Duration::from_millis(20);
    let mut cfg = config(&["Bash", "AsyncShell"]);
    cfg.access = AccessMode::FullAccess;
    cfg.git_remote_writes = true;
    let mut bundle = BundleBuilder::new(cfg)
        .shell(sandbox)
        .build(ToolPolicy {
            access: AccessMode::FullAccess,
            ..Default::default()
        })
        .unwrap();
    let prepared = bundle.prepared();
    let mut ctx = context();
    ctx.work_dir = root.path().into();
    ctx.policy = prepared.policy;
    let bash = prepared
        .tools
        .iter()
        .find(|tool| tool.definition().name == "Bash")
        .unwrap();
    let out = bash
        .execute(
            &ctx,
            ToolCall {
                arguments: json!({"command":"printf bundle"}),
                ..call("Bash")
            },
        )
        .await
        .unwrap();
    assert!(!out.is_error, "{:?}", out.content);
    assert_eq!(
        out.content,
        vec![Content::Text {
            text: "bundle".into()
        }]
    );
    let start = prepared
        .tools
        .iter()
        .find(|tool| tool.definition().name == "BashStart")
        .unwrap();
    let out = start
        .execute(
            &ctx,
            ToolCall {
                arguments: json!({"command":"sleep 60"}),
                ..call("BashStart")
            },
        )
        .await
        .unwrap();
    assert!(!out.is_error, "{:?}", out.content);
    tokio::time::timeout(Duration::from_secs(3), bundle.close())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bash.execute(&ctx, call("Bash"))
            .await
            .unwrap_err()
            .info
            .category,
        ErrorCategory::Cancelled
    );
}
