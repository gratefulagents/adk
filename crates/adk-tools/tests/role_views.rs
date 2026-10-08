use adk_core::*;
use adk_tools::{
    Config, Features,
    bundle::{BundleBuilder, BundleError},
};
use serde_json::json;
use std::{
    collections::BTreeSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

struct Probe {
    definition: ToolDefinition,
    adaptable: bool,
    rename: bool,
    control: bool,
    adapted: bool,
    policies: Arc<Mutex<Vec<ToolPolicy>>>,
    drops: Arc<AtomicUsize>,
}
impl Drop for Probe {
    fn drop(&mut self) {
        if self.adapted {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
}
impl Tool for Probe {
    fn definition(&self) -> &ToolDefinition {
        &self.definition
    }
    fn is_control_flow(&self) -> bool {
        self.control
    }
    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(7))
    }
    fn for_access(&self, access: AccessMode) -> Option<Arc<dyn Tool>> {
        if !self.adaptable || access != AccessMode::ReadOnly {
            return None;
        }
        let mut definition = self.definition.clone();
        definition.read_only = true;
        if self.rename {
            definition.name = "renamed".into();
        }
        Some(Arc::new(Self {
            definition,
            adaptable: false,
            rename: false,
            control: self.control,
            adapted: true,
            policies: self.policies.clone(),
            drops: self.drops.clone(),
        }))
    }
    fn execute<'a>(
        &'a self,
        context: &'a ToolContext,
        _: ToolCall,
    ) -> BoxFuture<'a, Result<ToolOutput, Error>> {
        Box::pin(async move {
            self.policies.lock().unwrap().push(context.policy.clone());
            Ok(ToolOutput {
                content: vec![],
                is_error: false,
                should_pause: false,
            })
        })
    }
}
fn probe(name: &str, read_only: bool) -> Probe {
    let mut definition = adk_tools::capabilities()
        .iter()
        .find_map(|c| c.definition.clone())
        .unwrap();
    definition.name = name.into();
    definition.read_only = read_only;
    definition.requires_approval = false;
    Probe {
        definition,
        adaptable: false,
        rename: false,
        control: false,
        adapted: false,
        policies: Arc::new(Mutex::new(vec![])),
        drops: Arc::new(AtomicUsize::new(0)),
    }
}
fn config() -> Config {
    Config {
        features: Features::Strict(["ExtraTools".into()].into()),
        access: AccessMode::FullAccess,
        ..Default::default()
    }
}
fn context() -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "role-view".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: ".".into(),
        policy: ToolPolicy {
            access: AccessMode::FullAccess,
            ..Default::default()
        },
        idempotency_key: None,
    }
}
fn call(name: &str) -> ToolCall {
    ToolCall {
        raw_arguments: None,
        id: "call".into(),
        name: name.into(),
        arguments: json!({}),
    }
}

#[tokio::test]
async fn role_view_owns_adapters_clamps_context_and_expires_on_close() {
    let mut adapter = probe("adapt", false);
    adapter.adaptable = true;
    let policies = adapter.policies.clone();
    let drops = adapter.drops.clone();
    let mut control = probe("control", false);
    control.control = true;
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(adapter),
        Arc::new(control),
        Arc::new(probe("write", false)),
        Arc::new(probe("hidden", true)),
    ];
    let mut owner = BundleBuilder::new(config())
        .extra_tools(tools)
        .build(ToolPolicy {
            access: AccessMode::FullAccess,
            allowed_mutating_tools: ["write".into()].into(),
            timeout: Some(Duration::from_secs(3)),
            max_child_turns: std::num::NonZeroU32::new(2),
            ..Default::default()
        })
        .unwrap();
    let view = owner
        .role_view(AccessMode::ReadOnly, &["hidden".into()].into())
        .unwrap();
    assert_eq!(
        view.tools
            .iter()
            .map(|t| t.definition().name.as_str())
            .collect::<Vec<_>>(),
        ["adapt"]
    );
    assert_eq!(view.policy.allowed_tools, Some(["adapt".into()].into()));
    assert!(view.policy.allowed_mutating_tools.is_empty());
    assert_eq!(view.tools[0].timeout(), Some(Duration::from_secs(7)));
    assert_eq!(owner.prepared().tools.len(), 4, "parent remains unchanged");
    let mut ctx = context();
    ctx.policy.approval = ApprovalPolicy::All;
    ctx.policy.allowed_mutating_tools.insert("adapt".into());
    ctx.policy.timeout = Some(Duration::from_secs(10));
    ctx.policy.max_child_turns = std::num::NonZeroU32::new(9);
    view.tools[0].execute(&ctx, call("adapt")).await.unwrap();
    let observed = policies.lock().unwrap()[0].clone();
    assert_eq!(observed.access, AccessMode::ReadOnly);
    assert_eq!(observed.approval, ApprovalPolicy::All);
    assert_eq!(observed.timeout, Some(Duration::from_secs(3)));
    assert_eq!(observed.max_child_turns.unwrap().get(), 2);
    assert!(observed.allowed_mutating_tools.is_empty());
    assert_eq!(observed.allowed_tools, view.policy.allowed_tools);
    for denied in [false, true] {
        let mut ctx = context();
        if denied {
            ctx.policy.denied_tools.insert("adapt".into());
        } else {
            ctx.policy.allowed_tools = Some(BTreeSet::new());
        }
        assert_eq!(
            view.tools[0]
                .execute(&ctx, call("adapt"))
                .await
                .unwrap_err()
                .info
                .category,
            ErrorCategory::PermissionDenied
        );
    }
    assert_eq!(policies.lock().unwrap().len(), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    owner.close().await.unwrap();
    assert_eq!(
        drops.load(Ordering::SeqCst),
        1,
        "saved handles must not own adapters"
    );
    assert_eq!(
        view.tools[0]
            .execute(&context(), call("adapt"))
            .await
            .unwrap_err()
            .info
            .category,
        ErrorCategory::Cancelled
    );
    assert!(matches!(
        owner.role_view(AccessMode::ReadOnly, &BTreeSet::new()),
        Err(BundleError::Closed)
    ));
}

#[tokio::test]
async fn requested_full_view_cannot_broaden_host_and_drop_invalidates_handles() {
    let mut owner = BundleBuilder::new(config())
        .extra_tools([Arc::new(probe("read", true)) as Arc<dyn Tool>])
        .build(ToolPolicy::default())
        .unwrap();
    let view = owner
        .role_view(AccessMode::FullAccess, &BTreeSet::new())
        .unwrap();
    assert_eq!(view.policy.access, AccessMode::ReadOnly);
    drop(owner);
    assert_eq!(
        view.tools[0]
            .execute(&context(), call("read"))
            .await
            .unwrap_err()
            .info
            .category,
        ErrorCategory::Cancelled
    );
}

#[test]
fn renamed_adapters_fail_without_replacing_parent_tools() {
    let mut tool = probe("original", false);
    tool.adaptable = true;
    tool.rename = true;
    let mut owner = BundleBuilder::new(config())
        .extra_tools([Arc::new(tool) as Arc<dyn Tool>])
        .build(ToolPolicy {
            access: AccessMode::FullAccess,
            ..Default::default()
        })
        .unwrap();
    assert!(matches!(
        owner.role_view(AccessMode::ReadOnly, &BTreeSet::new()),
        Err(BundleError::Build(adk_tools::BuildError::Contract(_)))
    ));
    assert_eq!(owner.prepared().tools[0].definition().name, "original");
}

#[test]
fn configuration_access_and_explicit_empty_allowlist_are_also_ceilings() {
    let mut cfg = config();
    cfg.access = AccessMode::ReadOnly;
    for names in [None, Some(BTreeSet::new())] {
        let empty = names.is_some();
        // This is an extension, not an enabled built-in; its name alone must
        // not apply the workspace-filesystem capability selection rules.
        let mut owner = BundleBuilder::new(cfg.clone())
            .extra_tools([Arc::new(probe("Write", true)) as Arc<dyn Tool>])
            .build(ToolPolicy {
                access: AccessMode::FullAccess,
                allowed_tools: names,
                ..Default::default()
            })
            .unwrap();
        let view = owner
            .role_view(AccessMode::FullAccess, &BTreeSet::new())
            .unwrap();
        assert_eq!(view.policy.access, AccessMode::ReadOnly);
        assert_eq!(view.tools.len(), usize::from(!empty));
        if empty {
            assert_eq!(view.policy.allowed_tools, Some(BTreeSet::new()));
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[tokio::test]
async fn real_bash_adapter_excludes_async_tools_and_rejects_mutation() {
    let root = tempfile::tempdir().unwrap();
    let mut sandbox = adk_sandbox::Config::new(root.path());
    sandbox.backend = adk_sandbox::Backend::Local;
    let mut owner = BundleBuilder::new(Config {
        features: Features::Strict(["Bash".into(), "AsyncShell".into()].into()),
        access: AccessMode::FullAccess,
        ..Default::default()
    })
    .shell(sandbox)
    .build(ToolPolicy {
        access: AccessMode::FullAccess,
        ..Default::default()
    })
    .unwrap();
    let view = owner
        .role_view(AccessMode::ReadOnly, &BTreeSet::new())
        .unwrap();
    assert_eq!(view.tools.len(), 1);
    let bash = &view.tools[0];
    assert_eq!(bash.definition().name, "Bash");
    assert!(bash.definition().read_only);
    let mut ctx = context();
    ctx.work_dir = root.path().into();
    let safe = bash
        .execute(
            &ctx,
            ToolCall {
                arguments: json!({"command":"printf role-view"}),
                ..call("Bash")
            },
        )
        .await;
    let succeeded = safe.as_ref().is_ok_and(|out| {
        !out.is_error
            && out
                .content
                .iter()
                .any(|c| matches!(c, Content::Text { text } if text == "role-view"))
    });
    if !succeeded {
        assert_ne!(
            std::env::var("ADK_REQUIRE_SANDBOX").as_deref(),
            Ok("1"),
            "enforcing backend unavailable: {safe:?}"
        );
        eprintln!("Native Bash confinement unavailable; checking fail-closed result: {safe:?}");
    }
    let result = bash
        .execute(
            &ctx,
            ToolCall {
                arguments: json!({"command":"touch forbidden"}),
                ..call("Bash")
            },
        )
        .await;
    assert!(
        !root.path().join("forbidden").exists(),
        "unexpected mutation: {result:?}"
    );
    // Bash reports ordinary nonzero process exits in content, not as tool errors.
    if let Ok(output) = result {
        assert!(output.is_error || output.content.iter().any(|c| {
            matches!(c, Content::Text { text } if text.rsplit_once("Exit code: ")
                .and_then(|(_, code)| code.trim().parse::<i32>().ok()).is_some_and(|code| code != 0))
        }), "mutation command unexpectedly succeeded: {output:?}");
    }
    owner.close().await.unwrap();
}
