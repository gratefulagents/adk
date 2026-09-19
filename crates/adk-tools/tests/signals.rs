use adk_core::*;
use adk_tools::{Config, Features, Registry};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

fn config() -> Config {
    Config {
        features: Features::Strict(
            [
                "Think",
                "Signals.AskUserQuestion",
                "Signals.PresentPlan",
                "Signals.Finish",
                "ExtraTools",
            ]
            .map(String::from)
            .into(),
        ),
        ..Default::default()
    }
}
fn context() -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "signals".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: Default::default(),
        policy: Default::default(),
        idempotency_key: None,
    }
}
async fn invoke(registry: &Registry, name: &str, input: Value) -> ToolOutput {
    registry
        .get(name)
        .unwrap()
        .execute(
            &context(),
            ToolCall {
                id: "call".into(),
                name: name.into(),
                arguments: input,
            },
        )
        .await
        .unwrap()
}
fn text(output: &ToolOutput) -> &str {
    match &output.content[..] {
        [Content::Text { text }] => text,
        _ => panic!("expected text"),
    }
}

#[tokio::test]
async fn deterministic_source_derived_signal_results() {
    let registry = Registry::build(&config(), []).unwrap();
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/tools/signals.json")).unwrap();
    for fixture in fixtures {
        let result = invoke(
            &registry,
            fixture["name"].as_str().unwrap(),
            fixture["input"].clone(),
        )
        .await;
        assert_eq!(text(&result), fixture["text"], "{fixture:?}");
        assert_eq!(result.is_error, fixture["is_error"]);
        assert_eq!(result.should_pause, fixture["should_pause"]);
    }
    let result = invoke(
        &registry,
        "AskUserQuestion",
        json!({"question":"<&>\u{2028}\u{2029}","choices":["ok"]}),
    )
    .await;
    assert_eq!(
        text(&result),
        r#"{"question":"\u003c\u0026\u003e\u2028\u2029","choices":["ok"],"allow_freeform":true}"#
    );
}

#[tokio::test]
async fn type_errors_do_not_pause_and_cancelled_calls_do_not_execute() {
    let registry = Registry::build(&config(), []).unwrap();
    for name in registry.names() {
        let result = invoke(&registry, name, json!(42)).await;
        assert!(result.is_error);
        assert!(!result.should_pause);
        assert!(text(&result).starts_with("Invalid input:"));
    }
    let cancel = adk_runtime::CancellationToken::new();
    cancel.cancel();
    let mut ctx = context();
    ctx.operation.cancellation = Arc::new(cancel);
    let error = registry
        .get("think")
        .unwrap()
        .execute(
            &ctx,
            ToolCall {
                id: "cancel".into(),
                name: "think".into(),
                arguments: json!({"thought":"No work"}),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.info.category, ErrorCategory::Cancelled);
}

#[derive(Default)]
struct Store {
    plan: Mutex<Option<String>>,
    summary: Mutex<String>,
    fail: bool,
}
impl adk_tools::plan::ArtifactStore for Store {
    fn save<'a>(
        &'a self,
        _: &'a Context,
        session: &'a str,
        plan: &'a str,
        summary: &'a str,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            assert_eq!(session, "trusted-session");
            if self.fail {
                return Err(Error::new(ErrorCategory::Internal, "offline"));
            }
            *self.plan.lock().unwrap() = Some(plan.into());
            *self.summary.lock().unwrap() = summary.into();
            Ok(())
        })
    }
    fn get<'a>(
        &'a self,
        _: &'a Context,
        session: &'a str,
    ) -> BoxFuture<'a, Result<Option<String>, Error>> {
        Box::pin(async move {
            assert_eq!(session, "trusted-session");
            if self.fail {
                return Err(Error::new(ErrorCategory::Internal, "offline"));
            }
            Ok(self.plan.lock().unwrap().clone())
        })
    }
}

#[tokio::test]
async fn plan_artifacts_use_host_identity_byte_counts_and_source_summary_rules() {
    let store = Arc::new(Store::default());
    let registry = Registry::build(
        &config(),
        adk_tools::plan::tools(store.clone(), "trusted-session"),
    )
    .unwrap();
    assert!(registry.get("save_plan").unwrap().is_control_flow());
    assert_eq!(
        text(&invoke(&registry, "get_plan", json!("ignored")).await),
        "No plan found. Use save_plan to create one first."
    );
    assert!(invoke(&registry, "save_plan", json!({})).await.is_error);
    assert!(
        invoke(&registry, "save_plan", json!({"plan":5}))
            .await
            .is_error
    );
    let plan = "€".repeat(70);
    let result = invoke(
        &registry,
        "save_plan",
        json!({"plan":plan,"session_id":"untrusted"}),
    )
    .await;
    assert_eq!(text(&result), "Plan saved (210 bytes) to artifact store");
    assert!(!result.is_error);
    assert_eq!(
        *store.summary.lock().unwrap(),
        format!("{}��...", "€".repeat(66))
    );
    assert_eq!(text(&invoke(&registry, "get_plan", json!({})).await), plan);
    let result = invoke(
        &registry,
        "save_plan",
        json!({"plan":"new","summary":"chosen"}),
    )
    .await;
    assert!(!result.is_error);
    assert_eq!(*store.summary.lock().unwrap(), "chosen");
}

#[tokio::test]
async fn artifact_failures_are_not_success_and_finish_sink_failures_do_not_pause() {
    let registry = Registry::build(
        &config(),
        adk_tools::plan::tools(
            Arc::new(Store {
                fail: true,
                ..Default::default()
            }),
            "trusted-session",
        ),
    )
    .unwrap();
    for (name, input, prefix) in [
        ("save_plan", json!({"plan":"x"}), "Failed to persist plan:"),
        ("get_plan", json!({}), "Failed to read plan:"),
    ] {
        let result = invoke(&registry, name, input).await;
        assert!(result.is_error);
        assert!(text(&result).starts_with(prefix));
    }
    struct FailedFinish;
    impl adk_tools::signal::FinishSink for FailedFinish {
        fn finish<'a>(
            &'a self,
            _: &'a Context,
            summary: &'a str,
        ) -> BoxFuture<'a, Result<(), Error>> {
            Box::pin(async move {
                assert_eq!(summary, "done");
                Err(Error::new(ErrorCategory::Internal, "offline"))
            })
        }
    }
    let registry = Registry::build(
        &config(),
        [adk_tools::signal::finish(Arc::new(FailedFinish))],
    )
    .unwrap();
    let result = invoke(&registry, "finish", json!({"summary":"done"})).await;
    assert!(result.is_error);
    assert!(!result.should_pause);
    assert!(text(&result).starts_with("Failed to mark run as completed:"));
}

#[tokio::test]
async fn pinned_go_empty_artifact_and_summary_byte_boundaries() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../fixtures/tools/state-memory-plan-expected.json"
    ))
    .unwrap();
    for case in fixture["plans"].as_array().unwrap() {
        let store = Arc::new(Store {
            plan: Mutex::new(case["stored"].as_str().map(str::to_owned)),
            ..Default::default()
        });
        let registry = Registry::build(
            &config(),
            adk_tools::plan::tools(store.clone(), "trusted-session"),
        )
        .unwrap();
        let result = invoke(
            &registry,
            case["name"].as_str().unwrap(),
            case["input"].clone(),
        )
        .await;
        assert_eq!(text(&result), case["text"]);
        assert_eq!(result.is_error, case["is_error"]);
        assert!(!result.should_pause);
        if case["name"] == "save_plan" {
            assert_eq!(*store.summary.lock().unwrap(), case["summary"]);
            assert_eq!(
                store.plan.lock().unwrap().as_deref(),
                case["input"]["plan"].as_str()
            );
        }
    }
}

#[test]
fn plan_tools_require_explicit_host_injection() {
    let registry = Registry::build(&config(), []).unwrap();
    assert!(registry.get("save_plan").is_none());
    assert!(registry.get("get_plan").is_none());
}
