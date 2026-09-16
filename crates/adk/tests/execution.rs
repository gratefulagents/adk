#![cfg(all(feature = "execution", unix))]

use adk::{core::*, execution::run_authorized, sandbox, security};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct NeverCancelled;
impl Cancellation for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
    fn cancelled(&self) -> BoxFuture<'_, ()> {
        Box::pin(std::future::pending())
    }
}
fn context(run: &str) -> Context {
    Context {
        run_id: run.into(),
        cancellation: Arc::new(NeverCancelled),
        deadline: None,
    }
}
struct Approver {
    decision: ApprovalDecision,
    approvals: AtomicUsize,
}
impl Host for Approver {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        self.approvals.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(self.decision) })
    }
}
struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        static ID: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "adk-execution-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn executor(&self) -> sandbox::Executor {
        let mut config = sandbox::Config::new(&self.0);
        config.backend = sandbox::Backend::Local;
        sandbox::Executor::new(config).unwrap()
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "shell".into(),
        description: String::new(),
        input_schema: true.into(),
        read_only: false,
        requires_approval: true,
    }
}
fn policy() -> security::SecurityPolicy {
    security::SecurityPolicy {
        tools: ToolPolicy {
            access: AccessMode::FullAccess,
            ..Default::default()
        },
        allow_network: true,
        ..Default::default()
    }
}
fn call(command: &str) -> security::CommandRequest {
    // All test commands are literal ASCII without JSON metacharacters.
    security::CommandRequest::from_call(ToolCall {
        id: "call-1".into(),
        name: "shell".into(),
        arguments: format!(r#"{{"command":"{command}"}}"#).parse().unwrap(),
    })
    .unwrap()
}

#[tokio::test]
async fn approved_command_executes_and_capability_is_run_bound() {
    let workspace = Workspace::new();
    let host = Approver {
        decision: ApprovalDecision::Approve,
        approvals: AtomicUsize::new(0),
    };
    let ctx = context("run-1");
    let security::Authorization::Approved(command) = policy()
        .authorize(&ctx, &host, &definition(), call("echo authorized"))
        .await
        .unwrap()
    else {
        panic!("expected approval")
    };
    let result = run_authorized(&workspace.executor(), &ctx, command)
        .await
        .unwrap();
    assert_eq!(result.stdout, b"authorized\n");
    assert!(result.status.success());
    assert_eq!(host.approvals.load(Ordering::SeqCst), 1);
    let security::Authorization::Approved(command) = policy()
        .authorize(&ctx, &host, &definition(), call("echo wrong-run"))
        .await
        .unwrap()
    else {
        panic!("expected approval")
    };
    let error = run_authorized(&workspace.executor(), &context("run-2"), command)
        .await
        .unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::PermissionDenied);
}

#[tokio::test]
async fn policy_denial_precedes_approval_and_deferred_calls_do_not_spawn() {
    let workspace = Workspace::new();
    let host = Approver {
        decision: ApprovalDecision::Approve,
        approvals: AtomicUsize::new(0),
    };
    let mut restricted = policy();
    restricted.tools.denied_tools.insert("shell".into());
    assert!(
        restricted
            .authorize(
                &context("run"),
                &host,
                &definition(),
                call("echo denied > marker")
            )
            .await
            .is_err()
    );
    assert_eq!(host.approvals.load(Ordering::SeqCst), 0);
    assert!(!workspace.0.join("marker").exists());
    let host = Approver {
        decision: ApprovalDecision::Defer,
        approvals: AtomicUsize::new(0),
    };
    assert!(matches!(
        policy()
            .authorize(
                &context("run"),
                &host,
                &definition(),
                call("echo deferred > marker")
            )
            .await
            .unwrap(),
        security::Authorization::Deferred(_)
    ));
    assert!(!workspace.0.join("marker").exists());
}

#[tokio::test]
async fn secret_output_is_not_returned_to_tool() {
    let workspace = Workspace::new();
    // Deliberately synthetic token, assembled so repository scanners do not
    // mistake the fixture for a deployed credential.
    let fake = ["ghp_", &"a".repeat(36)].concat();
    std::fs::write(workspace.0.join("fixture"), fake).unwrap();
    let host = Approver {
        decision: ApprovalDecision::Approve,
        approvals: AtomicUsize::new(0),
    };
    let ctx = context("run");
    let security::Authorization::Approved(command) = policy()
        .authorize(&ctx, &host, &definition(), call("cat fixture"))
        .await
        .unwrap()
    else {
        panic!("expected approval")
    };
    let error = run_authorized(&workspace.executor(), &ctx, command)
        .await
        .unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::Guardrail);
    assert!(!error.info.message.contains("ghp_"));
}
