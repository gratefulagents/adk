use adk_codec::{
    approval::{ApprovalMarker, ApprovalMarkerBoundary, ApprovalPhase},
    dto::AgentRef,
};
use adk_core::*;
use adk_runtime::{CancellationToken, runner};
use serde_json::{Value, json};
use std::sync::Arc;

use adk_runtime::compaction::*;
use runner::Compactor;

fn fixture() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/compaction.json")).unwrap()
}
fn inputs() -> Vec<Value> {
    serde_json::from_str(include_str!("../../../fixtures/compaction_inputs.json")).unwrap()
}
fn history(case: &Value) -> Vec<RunItem> {
    fixture_history(&case["history"]).0
}
fn fixture_history(items: &Value) -> (Vec<RunItem>, Vec<ApprovalMarkerBoundary>) {
    let mut items = items.clone();
    for item in items.as_array_mut().unwrap() {
        let content = if item["type"] == "message" {
            item["message"]["content"].as_array_mut()
        } else {
            item["output"]["content"].as_array_mut()
        };
        if let Some(content) = content {
            for part in content {
                if let Some(repeat) = part.as_object_mut().unwrap().remove("repeat") {
                    part["text"] = Value::String(
                        part["text"]
                            .as_str()
                            .unwrap()
                            .repeat(repeat.as_u64().unwrap() as usize),
                    );
                }
            }
        }
    }
    let mut history = vec![];
    let mut markers = vec![];
    for item in items.as_array().unwrap() {
        if item["type"] == "approval" {
            let call: ToolCall = serde_json::from_value(item["call"].clone()).unwrap();
            let phase = match item["phase"].as_str().unwrap() {
                "pending" => ApprovalPhase::Pending,
                "approved" => ApprovalPhase::Approved,
                "denied" => ApprovalPhase::Denied,
                _ => panic!("invalid fixture phase"),
            };
            markers.push(ApprovalMarkerBoundary {
                before_item: history.len(),
                marker: ApprovalMarker::from_call(
                    &call,
                    phase,
                    item["agent"]
                        .as_str()
                        .map(|name| AgentRef { name: name.into() }),
                ),
            });
        } else {
            history.push(serde_json::from_value(item.clone()).unwrap());
        }
    }
    (history, markers)
}
fn policy(case: &Value) -> LocalCompactionPolicy {
    let default = LocalCompactionPolicy::default();
    let get = |key, default| case["policy"][key].as_u64().unwrap_or(default);
    LocalCompactionPolicy {
        enabled: !case["disabled"].as_bool().unwrap(),
        trigger_tokens: get("trigger_tokens", default.trigger_tokens),
        target_tokens: get("target_tokens", default.target_tokens),
        preserve_recent_items: get(
            "preserve_recent_items",
            default.preserve_recent_items as u64,
        ) as usize,
        preserve_initial_user_messages: get(
            "preserve_initial_user_messages",
            default.preserve_initial_user_messages as u64,
        ) as usize,
        summary_bullet_limit: get("summary_bullet_limit", default.summary_bullet_limit as u64)
            as usize,
    }
}
fn assert_policy(actual: LocalCompactionPolicy, expected: &Value) {
    assert_eq!(actual.enabled, expected["Enabled"]);
    assert_eq!(actual.trigger_tokens, expected["TriggerTokens"]);
    assert_eq!(actual.target_tokens, expected["TargetTokens"]);
    assert_eq!(
        actual.preserve_recent_items,
        expected["PreserveRecentItems"]
    );
    assert_eq!(
        actual.preserve_initial_user_messages,
        expected["PreserveInitialUserMessages"]
    );
    assert_eq!(actual.summary_bullet_limit, expected["SummaryBulletLimit"]);
}
#[test]
fn pinned_go_local_compaction_differential() {
    let reference = fixture();
    assert_eq!(
        reference["sdk_revision"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert_policy(
        LocalCompactionPolicy::default(),
        &reference["default_policy"],
    );
    assert_eq!(reference["default_policy"]["UseLLMSummary"], true);
    let cases = inputs();
    assert_eq!(reference["cases"].as_object().unwrap().len(), cases.len());
    for case in cases {
        let name = case["name"].as_str().unwrap();
        let expected = &reference["cases"][name];
        let (items, markers) = fixture_history(&case["history"]);
        let original = items.clone();
        let original_markers = markers.clone();
        let policy = policy(&case);
        assert_policy(policy.normalized(), &expected["normalized_policy"]);
        assert_eq!(
            estimate_history_tokens_with_approvals(&items, &markers),
            expected["history_estimate"],
            "{name} estimate"
        );
        let actual =
            compact_with_approvals(&items, &markers, policy, case["overhead"].as_i64().unwrap());
        if markers.is_empty() {
            assert_eq!(
                actual,
                compact_for_request(&items, policy, case["overhead"].as_i64().unwrap()),
                "{name} marker-free delegation"
            );
        }
        assert_eq!(markers, original_markers, "{name} input markers changed");
        assert_eq!(items, original, "{name} input changed");
        assert_eq!(actual.changed, expected["changed"], "{name} changed");
        assert_eq!(actual.reason, expected["reason"], "{name} reason");
        assert_eq!(
            actual.before_tokens, expected["before_tokens"],
            "{name} before"
        );
        assert_eq!(
            actual.after_tokens, expected["after_tokens"],
            "{name} after"
        );
        assert_eq!(
            extract_summary(&actual.history),
            expected["summary"],
            "{name} summary"
        );
        let (expected_history, expected_markers) = fixture_history(&expected["history"]);
        assert_eq!(actual.history, expected_history, "{name} history");
        assert_eq!(actual.markers, expected_markers, "{name} markers");
        let finalized = if actual.changed {
            finalize_local_history_with_approvals(
                &actual.history,
                &actual.markers,
                &items,
                &markers,
            )
        } else {
            (actual.history.clone(), actual.markers.clone())
        };
        let expected_final = fixture_history(&expected["final_history"]);
        assert_eq!(
            finalized, expected_final,
            "{name} final history and markers"
        );
        if actual.changed {
            assert!(
                actual.after_tokens < actual.before_tokens,
                "{name} must shrink"
            );
        }
    }
}
#[test]
fn identical_wire_markers_retain_distinct_phases_and_missing_input() {
    let (items, mut markers) = fixture_history(&json!([
        {"type":"message", "message":{"role":"user", "content":[{"type":"text", "text":"task"}]}},
        {"type":"message", "message":{"role":"assistant", "content":[{"type":"text", "text":"old ", "repeat":2000}]}},
        {"type":"approval", "phase":"pending", "call":{"id":"same", "name":"deploy", "arguments":null}},
        {"type":"approval", "phase":"denied", "call":{"id":"same", "name":"deploy", "arguments":null}}
    ]));
    assert_eq!(
        markers[0].marker.to_wire().unwrap(),
        markers[1].marker.to_wire().unwrap()
    );
    let with_null = estimate_history_tokens_with_approvals(&items, &markers);
    for marker in &mut markers {
        marker.marker.data.input = adk_codec::dto::RawJson::Missing;
    }
    assert_eq!(
        with_null - estimate_history_tokens_with_approvals(&items, &markers),
        4
    );
    let policy = LocalCompactionPolicy {
        trigger_tokens: 1000,
        target_tokens: 800,
        preserve_recent_items: 2,
        ..Default::default()
    };
    let actual = compact_with_approvals(&items, &markers, policy, 0);
    assert!(actual.changed);
    assert_eq!(actual.markers, markers);
    let (finalized, final_markers) =
        finalize_local_history_with_approvals(&actual.history, &actual.markers, &items, &markers);
    assert_eq!(finalized, actual.history);
    assert_eq!(final_markers, markers);
}

fn model_request() -> ModelRequest {
    ModelRequest {
        input_provenance: Vec::new(),
        model: "scripted".into(),
        instructions: "instructions".into(),
        input: vec![],
        tools: vec![ToolDefinition {
            name: "read".into(),
            description: "Read a file".into(),
            input_schema: serde_json::from_value(json!({"type":"object"})).unwrap(),
            read_only: true,
            requires_approval: false,
        }],
        output_schema: None,
        output_schema_name: String::new(),
        output_schema_strict: false,
        settings: Default::default(),
    }
}
#[test]
fn pinned_go_token_estimates_and_request_reserve() {
    let reference = fixture();
    for case in reference["strings"].as_array().unwrap() {
        assert_eq!(
            estimate_string_tokens(case["text"].as_str().unwrap()),
            case["tokens"]
        );
    }
    for case in reference["overhead"].as_array().unwrap() {
        let mut request = model_request();
        request
            .settings
            .insert("max_tokens".into(), case["max_tokens"].clone());
        request
            .settings
            .insert("thinking_budget".into(), case["thinking_budget"].clone());
        assert_eq!(output_reserve_tokens(&request), case["reserve"]);
        assert_eq!(estimate_request_overhead_tokens(&request), case["overhead"]);
    }
}
#[test]
fn calibration_excludes_reserve_and_uses_normalized_prompt_usage() {
    let mut calibration = EstimateCalibration::default();
    let request = model_request();
    let prompt = estimate_request_overhead_tokens(&request)
        - output_reserve_tokens(&request)
        - REQUEST_SAFETY_BUFFER;
    calibration.observe(prompt * 2, &[], &request);
    assert_eq!(calibration.0, 1.5);
    assert_eq!(
        calibration
            .apply(LocalCompactionPolicy::default())
            .trigger_tokens,
        120_000
    );
    calibration.observe_estimate(100000, 1);
    assert_eq!(calibration.0, 2.5);
    for _ in 0..10 {
        calibration.observe_estimate(1, 100000);
    }
    assert_eq!(calibration.0, 0.5);
    calibration.observe_estimate(0, 100);
    assert_eq!(calibration.0, 0.5);
    assert_eq!(
        calibration
            .apply(LocalCompactionPolicy::default())
            .target_tokens,
        200_000
    );
}
#[tokio::test]
async fn compactor_adapter_is_local_zero_usage_and_cost() {
    let case = inputs()
        .into_iter()
        .find(|c| c["name"] == "overhead_crossing")
        .unwrap();
    let history = history(&case);
    let policy = policy(&case);
    let config = policy.config();
    assert_eq!(config.trigger_tokens, policy.trigger_tokens);
    assert_eq!(config.target_tokens, policy.target_tokens);
    let context = Context {
        run_id: "local-compaction".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    };
    let request = runner::CompactionRequest {
        history_provenance: Vec::new(),
        agent: "assistant".into(),
        model: "never-called".into(),
        context_tokens: estimate_history_tokens(&history) + 500,
        target_tokens: policy.target_tokens,
        history: history.clone(),
    };
    let actual = config.compactor.compact(&context, request).await.unwrap();
    let expected = compact_for_request(&history, policy, 500);
    assert_eq!(actual.history, expected.history);
    assert_eq!(actual.context_tokens, expected.after_tokens);
    assert_eq!(actual.usage, Usage::default());
    assert_eq!(actual.cost, 0.0);
    // Default construction is usable through the existing trait as well.
    let default = LocalCompactor::default();
    let unchanged = default
        .compact(
            &context,
            runner::CompactionRequest {
                history_provenance: Vec::new(),
                agent: "assistant".into(),
                model: "never-called".into(),
                history: history.clone(),
                context_tokens: estimate_history_tokens(&history),
                target_tokens: 100000,
            },
        )
        .await
        .unwrap();
    assert_eq!(unchanged.history, history);
}
#[test]
fn rust_only_media_system_and_handoff_context_is_preserved() {
    let case = inputs()
        .into_iter()
        .find(|c| c["name"] == "minimal_summary")
        .unwrap();
    let mut items = history(&case);
    let image = RunItem::Message {
        message: Message {
            role: Role::User,
            content: vec![Content::Image {
                uri: "file:test.png".into(),
                media_type: "image/png".into(),
            }],
        },
    };
    let system = RunItem::Message {
        message: Message {
            role: Role::System,
            content: vec![Content::Text {
                text: "system instructions".into(),
            }],
        },
    };
    let handoff = RunItem::Handoff {
        call_id: "handoff".into(),
        agent: "target".into(),
    };
    items.splice(1..1, [image.clone(), system.clone(), handoff.clone()]);
    let result = compact_for_request(&items, policy(&case), 0);
    assert!(result.changed);
    for item in [image, system, handoff] {
        assert!(result.history.contains(&item));
    }
}
#[test]
fn compaction_does_not_modify_request_cache_identity() {
    use sha2::{Digest, Sha256};
    let wire = format!("{:x}", Sha256::digest(b"namespace\0logical"));
    assert_eq!(wire, fixture()["cache_wire_key"]);
    let mut request = model_request();
    request
        .settings
        .insert("prompt_cache_key".into(), json!(wire));
    let settings = request.settings.clone();
    let instructions = request.instructions.clone();
    let case = inputs()
        .into_iter()
        .find(|c| c["name"] == "minimal_summary")
        .unwrap();
    let compacted = compact_for_request(&history(&case), policy(&case), 0);
    assert!(compacted.changed);
    request.input = compacted.history;
    assert_eq!(request.settings, settings);
    assert_eq!(request.instructions, instructions);
}

#[derive(Default)]
struct RecordingModel {
    requests: std::sync::Mutex<Vec<ModelRequest>>,
    responses: std::sync::Mutex<std::collections::VecDeque<ModelResponse>>,
}
impl Model for RecordingModel {
    fn provider(&self) -> &str {
        "scripted"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(request);
            Ok(self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("script exhausted"))
        })
    }
}
#[derive(Default)]
struct RecordingHost;
impl Host for RecordingHost {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Approve) })
    }
}
#[derive(Default)]
struct CompactionObservations(std::sync::Mutex<Vec<runner::Observation>>);
impl runner::RunHooks for CompactionObservations {
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        observation: runner::Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            self.0.lock().unwrap().push(observation);
            Ok(())
        })
    }
}
fn msg(role: Role, text: impl Into<String>) -> RunItem {
    RunItem::Message {
        message: Message {
            role,
            content: vec![Content::Text { text: text.into() }],
        },
    }
}
fn reply(items: Vec<RunItem>, end: bool, context_tokens: Option<u64>) -> ModelResponse {
    ModelResponse {
        raw: None,
        items,
        usage: Usage {
            input_tokens: 100,
            output_tokens: 2,
            context_tokens,
            ..Usage::default()
        },
        end_turn: Some(end),
        response_id: None,
        metadata: Default::default(),
    }
}
fn run_request(input: Vec<RunItem>) -> RunRequest {
    RunRequest {
        input_provenance: Vec::new(),
        input,
        policy: RunPolicy {
            max_turns: std::num::NonZeroU32::new(3).unwrap(),
            tools: ToolPolicy::default(),
            tool_use: ToolUseBehavior::Continue,
        },
    }
}
fn run_context(token: Arc<CancellationToken>) -> Context {
    Context {
        run_id: "compaction-integration".into(),
        cancellation: token,
        deadline: None,
    }
}
#[tokio::test]
async fn default_runner_compacts_first_request_and_preserves_cache_transients_and_append_only_items()
 {
    let case = inputs()
        .into_iter()
        .find(|c| c["name"] == "default_triggered")
        .unwrap();
    let new = vec![
        msg(Role::Assistant, "generated old ".repeat(70000)),
        msg(Role::Assistant, "latest response"),
    ];
    let final_item = msg(Role::Assistant, "done");
    let model = Arc::new(RecordingModel {
        responses: std::sync::Mutex::new(
            vec![
                reply(new.clone(), false, None),
                reply(vec![final_item.clone()], true, None),
            ]
            .into(),
        ),
        ..Default::default()
    });
    let observations = Arc::new(CompactionObservations::default());
    let transient = msg(Role::User, "transient private hint");
    let mut agent = runner::AgentConfig::new(
        "assistant",
        runner::ModelBinding::complete("local", model.clone()),
    );
    agent.instructions = "stable instructions".into();
    let config = runner::RunnerConfig {
        cache_prefix: "stable prefix".into(),
        prompt_cache_key: Some("logical".into()),
        prompt_cache_namespace: Some("namespace".into()),
        transient_context: vec![transient.clone()],
        hooks: Some(observations.clone()),
        ..Default::default()
    };
    let runner = runner::Runner::new(agent, config).unwrap();
    let outcome = runner
        .run(
            run_context(Arc::new(CancellationToken::new())),
            run_request(history(&case)),
            Arc::new(RecordingHost),
        )
        .await
        .unwrap();
    let requests = model.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests.iter() {
        assert!(!extract_summary(&request.input).is_empty());
        assert_eq!(request.input.last(), Some(&transient));
        assert_eq!(request.instructions, "stable prefix\nstable instructions");
        assert_eq!(
            request.settings["prompt_cache_key"],
            fixture()["cache_wire_key"]
        );
    }
    assert_eq!(requests[0].settings, requests[1].settings);
    assert!(!outcome.result.history.contains(&transient));
    assert!(!extract_summary(&outcome.result.history).contains("transient private hint"));
    let mut expected_new = new;
    expected_new.push(final_item);
    assert_eq!(outcome.result.new_items, expected_new);
    assert_eq!(outcome.result.usage.input_tokens, 200);
    assert_eq!(outcome.result.usage.output_tokens, 4);
    assert!(
        observations
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|o| matches!(o, runner::Observation::Compacted { .. }))
            .count()
            >= 2
    );
}
struct FailingCompactor {
    category: ErrorCategory,
    cancel: Option<Arc<CancellationToken>>,
    pending: bool,
}
impl runner::Compactor for FailingCompactor {
    fn compact<'a>(
        &'a self,
        _: &'a Context,
        _: runner::CompactionRequest,
    ) -> BoxFuture<'a, Result<runner::CompactedHistory, Error>> {
        Box::pin(async move {
            if let Some(token) = &self.cancel {
                token.cancel();
            }
            if self.pending {
                return std::future::pending().await;
            }
            Err(Error::new(
                self.category,
                "scripted custom compactor failure",
            ))
        })
    }
}
#[tokio::test(start_paused = true)]
async fn recoverable_custom_compactor_failures_fall_back_to_local_or_noop() {
    for (category, pending) in [
        (ErrorCategory::Provider, false),
        (ErrorCategory::Unsupported, false),
        (ErrorCategory::DeadlineExceeded, true),
    ] {
        for oversized in [false, true] {
            let first_items = vec![
                msg(
                    Role::Assistant,
                    if oversized {
                        "old ".repeat(190000)
                    } else {
                        "old".into()
                    },
                ),
                msg(Role::Assistant, "latest"),
            ];
            let model = Arc::new(RecordingModel {
                responses: std::sync::Mutex::new(
                    vec![
                        reply(first_items, false, Some(100)),
                        reply(vec![msg(Role::Assistant, "done")], true, None),
                    ]
                    .into(),
                ),
                ..Default::default()
            });
            let observations = Arc::new(CompactionObservations::default());
            let config = runner::RunnerConfig {
                compaction: Some(runner::CompactionConfig {
                    trigger_tokens: 2,
                    target_tokens: 1,
                    compactor: Arc::new(FailingCompactor {
                        category,
                        cancel: None,
                        pending,
                    }),
                }),
                model_idle_timeout: Some(std::time::Duration::from_secs(1)),
                hooks: Some(observations.clone()),
                ..Default::default()
            };
            let runner = runner::Runner::new(
                runner::AgentConfig::new(
                    "assistant",
                    runner::ModelBinding::complete("local", model.clone()),
                ),
                config,
            )
            .unwrap();
            let outcome = runner
                .run(
                    run_context(Arc::new(CancellationToken::new())),
                    run_request(vec![msg(Role::User, "task")]),
                    Arc::new(RecordingHost),
                )
                .await
                .unwrap();
            assert_eq!(model.requests.lock().unwrap().len(), 2);
            assert_eq!(
                !extract_summary(&outcome.result.history).is_empty(),
                oversized
            );
            let events = observations.0.lock().unwrap();
            assert!(
                events
                    .iter()
                    .any(|o| matches!(o, runner::Observation::CompactionFailed { .. }))
            );
            assert_eq!(
                events
                    .iter()
                    .any(|o| matches!(o, runner::Observation::Compacted { .. })),
                oversized
            );
            assert_eq!(outcome.result.usage.input_tokens, 200);
        }
    }
}
#[tokio::test]
async fn parent_cancellation_during_custom_compaction_is_fatal_not_local_fallback() {
    let token = Arc::new(CancellationToken::new());
    let model = Arc::new(RecordingModel {
        responses: std::sync::Mutex::new(
            vec![reply(
                vec![
                    msg(Role::Assistant, "old ".repeat(190000)),
                    msg(Role::Assistant, "latest"),
                ],
                false,
                Some(100),
            )]
            .into(),
        ),
        ..Default::default()
    });
    let observations = Arc::new(CompactionObservations::default());
    let config = runner::RunnerConfig {
        compaction: Some(runner::CompactionConfig {
            trigger_tokens: 2,
            target_tokens: 1,
            compactor: Arc::new(FailingCompactor {
                category: ErrorCategory::Provider,
                cancel: Some(token.clone()),
                pending: false,
            }),
        }),
        hooks: Some(observations.clone()),
        ..Default::default()
    };
    let runner = runner::Runner::new(
        runner::AgentConfig::new(
            "assistant",
            runner::ModelBinding::complete("local", model.clone()),
        ),
        config,
    )
    .unwrap();
    let failure = runner
        .run(
            run_context(token),
            run_request(vec![msg(Role::User, "task")]),
            Arc::new(RecordingHost),
        )
        .await
        .err()
        .expect("cancelled run");
    assert_eq!(failure.error.info.category, ErrorCategory::Cancelled);
    assert_eq!(model.requests.lock().unwrap().len(), 1);
    assert!(
        !observations
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|o| matches!(o, runner::Observation::Compacted { .. }))
    );
}

#[test]
fn compaction_preserves_phased_multimodal_items_tool_pairs_and_provider_origins() {
    let case = inputs()
        .into_iter()
        .find(|c| c["name"] == "minimal_summary")
        .unwrap();
    let mut items = history(&case);
    let attachments = vec![
        Content::Text {
            text: "inspect".into(),
        },
        Content::Attachment {
            media_type: "application/pdf".into(),
            data: "cGRm".into(),
            detail: String::new(),
        },
        Content::Attachment {
            media_type: "image/png".into(),
            data: "cG5n".into(),
            detail: "high".into(),
        },
    ];
    let mut protected = vec![
        RunItem::PhasedMessage {
            message: Message {
                role: Role::Assistant,
                content: attachments.clone(),
            },
            phase: "commentary".into(),
        },
        RunItem::ToolCall {
            call: ToolCall {
                id: "images".into(),
                name: "inspect".into(),
                arguments: json!({}),
            },
        },
        RunItem::ToolResult {
            call_id: "images".into(),
            output: ToolOutput {
                content: attachments,
                is_error: false,
                should_pause: false,
            },
        },
    ];
    for origin in ["openai", "anthropic", ""] {
        protected.push(RunItem::Compaction {
            compaction: Compaction {
                id: format!("cmp_{origin}"),
                content: "summary".into(),
                encrypted_content: "opaque".into(),
                created_by: origin.into(),
            },
        });
    }
    items.splice(1..1, protected.clone());
    let recent = RunItem::PhasedMessage {
        message: Message {
            role: Role::Assistant,
            content: vec![Content::Text {
                text: "recent answer".into(),
            }],
        },
        phase: "final_answer".into(),
    };
    items.push(recent.clone());
    let before = items.clone();
    let actual = compact_for_request(&items, policy(&case), 0);
    assert!(actual.changed);
    assert_eq!(items, before);
    assert_eq!(actual, compact_for_request(&items, policy(&case), 0));
    let finalized = finalize_local_history(&actual.history, &items);
    for item in protected.iter().chain(std::iter::once(&recent)) {
        assert!(finalized.contains(item), "lost {item:?}");
    }
    let agents: Vec<_> = finalized
        .iter()
        .map(|item| match item {
            RunItem::Message { message } | RunItem::PhasedMessage { message, .. }
                if message.role == Role::Assistant =>
            {
                Some(AgentRef {
                    name: "worker".into(),
                })
            }
            _ => None,
        })
        .collect();
    let wire = adk_codec::approval::encode_history(&finalized, &agents, &[]).unwrap();
    assert_eq!(
        adk_codec::approval::decode_history(&wire, &[])
            .unwrap()
            .items,
        finalized
    );
}

#[test]
fn phased_text_has_the_same_local_compaction_semantics_as_plain_text() {
    let case = inputs()
        .into_iter()
        .find(|c| c["name"] == "minimal_summary")
        .unwrap();
    let plain = history(&case);
    let phased: Vec<_> = plain
        .iter()
        .cloned()
        .map(|item| match item {
            RunItem::Message { message } if message.role == Role::Assistant => {
                RunItem::PhasedMessage {
                    message,
                    phase: "commentary".into(),
                }
            }
            other => other,
        })
        .collect();
    assert_eq!(
        estimate_history_tokens(&plain),
        estimate_history_tokens(&phased)
    );
    let expected = compact_for_request(&plain, policy(&case), 0);
    let actual = compact_for_request(&phased, policy(&case), 0);
    assert!(actual.changed);
    assert_eq!(actual.before_tokens, expected.before_tokens);
    assert_eq!(actual.after_tokens, expected.after_tokens);
    assert_eq!(
        extract_summary(&actual.history),
        extract_summary(&expected.history)
    );
    let unphased: Vec<_> = actual
        .history
        .into_iter()
        .map(|item| match item {
            RunItem::PhasedMessage { message, .. } => RunItem::Message { message },
            other => other,
        })
        .collect();
    assert_eq!(unphased, expected.history);
}

#[tokio::test]
async fn identical_messages_retain_source_positions_and_summaries_are_unattributed() {
    let message = |role, text: String| RunItem::Message {
        message: Message {
            role,
            content: vec![Content::Text { text }],
        },
    };
    let mut history = vec![message(Role::User, "task".into())];
    for _ in 0..16 {
        history.push(message(Role::Assistant, "old detail ".repeat(100)));
    }
    // The first duplicate is removed and the second retained. Content lookup would choose A.
    history.push(message(Role::Assistant, "identical".into()));
    for _ in 0..8 {
        history.push(message(Role::Assistant, "other detail ".repeat(100)));
    }
    history.push(message(Role::Assistant, "identical".into()));
    let mut provenance = vec![ItemProvenance::Agent { name: "A".into() }; history.len()];
    *provenance.last_mut().unwrap() = ItemProvenance::Agent { name: "B".into() };
    let compactor = LocalCompactor {
        policy: LocalCompactionPolicy {
            trigger_tokens: 1000,
            target_tokens: 700,
            preserve_recent_items: 1,
            preserve_initial_user_messages: 1,
            ..Default::default()
        },
    };
    let output = compactor
        .compact(
            &Context {
                run_id: "identity".into(),
                cancellation: Arc::new(CancellationToken::new()),
                deadline: None,
            },
            runner::CompactionRequest {
                agent: "current".into(),
                model: "model".into(),
                context_tokens: estimate_history_tokens(&history),
                target_tokens: 700,
                history: history.clone(),
                history_provenance: provenance,
            },
        )
        .await
        .unwrap();
    assert!(output.history.len() < history.len());
    assert_eq!(output.history.len(), output.history_provenance.len());
    assert_eq!(output.history.last(), history.last());
    assert_eq!(
        output.history_provenance.last(),
        Some(&ItemProvenance::Agent { name: "B".into() })
    );
    let summary = output.history.iter().position(|item| matches!(item, RunItem::Message { message } if matches!(&message.content[0], Content::Text { text } if text.starts_with(SUMMARY_MARKER)))).unwrap();
    assert_eq!(
        output.history_provenance[summary],
        ItemProvenance::Unattributed
    );
}
