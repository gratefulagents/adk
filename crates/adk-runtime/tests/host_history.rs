use adk_codec::{approval::*, dto};
use adk_core::*;
use adk_runtime::{tracing::*, *};
use serde_json::json;
use std::sync::{Arc, Mutex};

fn context() -> Context {
    Context {
        run_id: "history".into(),
        cancellation: Arc::new(CancellationToken::new()),
        deadline: None,
    }
}
fn message() -> RunItem {
    RunItem::Message {
        message: Message {
            role: Role::Assistant,
            content: vec![Content::Text {
                text: "answer".into(),
            }],
        },
    }
}
fn history() -> (RunRequest, Vec<ApprovalMarkerBoundary>) {
    let call = ToolCall {
        raw_arguments: None,
        id: "historic".into(),
        name: "write".into(),
        arguments: json!({"x": 1}),
    };
    let markers = [ApprovalPhase::Pending, ApprovalPhase::Denied]
        .into_iter()
        .map(|phase| ApprovalMarkerBoundary {
            before_item: 1,
            marker: ApprovalMarker::from_call(
                &call,
                phase,
                Some(dto::AgentRef {
                    name: "old-agent".into(),
                }),
            )
            .unwrap(),
        })
        .collect();
    (
        RunRequest {
            input: vec![
                RunItem::ToolCall { call },
                RunItem::ToolResult {
                    call_id: "historic".into(),
                    output: ToolOutput {
                        content: vec![Content::Text {
                            text: "denied".into(),
                        }],
                        is_error: true,
                        should_pause: false,
                    },
                },
            ],
            input_provenance: vec![
                ItemProvenance::Agent {
                    name: "old-agent".into()
                };
                2
            ],
            policy: RunPolicy::default(),
        },
        markers,
    )
}
#[derive(Default)]
struct Probe {
    requests: Mutex<Vec<ModelRequest>>,
    generations: Mutex<Vec<GenerationRecord>>,
    commits: Mutex<Vec<(Vec<RunItem>, Vec<ApprovalMarkerBoundary>)>>,
}
impl Model for Probe {
    fn provider(&self) -> &str {
        "test"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        self.requests.lock().unwrap().push(request);
        Box::pin(async {
            Ok(ModelResponse {
                items: vec![message()],
                end_turn: Some(true),
                usage: Usage::default(),
                response_id: None,
                metadata: Default::default(),
                raw: None,
                snapshot_raw: None,
                snapshot_projection: None,
            })
        })
    }
}
impl Host for Probe {
    fn emit<'a>(&'a self, _: &'a Context, _: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        panic!("historic calls must not request approval")
    }
}
impl GenerationObserver for Probe {
    fn start(&self, _: &Context, record: &GenerationRecord) {
        self.generations.lock().unwrap().push(record.clone());
    }
    fn end(&self, _: &Context, _: &GenerationRecord) {}
}
impl RunHooks for Probe {
    fn durable_observer(&self) -> bool {
        true
    }
    fn observe<'a>(
        &'a self,
        _: &'a Context,
        observation: Observation,
    ) -> BoxFuture<'a, Result<(), Error>> {
        if let Observation::CommittedItems { items, markers, .. } = observation {
            self.commits.lock().unwrap().push((items, markers));
        }
        Box::pin(async { Ok(()) })
    }
}
fn runner(probe: &Arc<Probe>) -> Runner {
    Runner::new(
        AgentConfig::new("agent", ModelBinding::complete("test", probe.clone())),
        RunnerConfig {
            hooks: Some(probe.clone()),
            generation_observer: Some(probe.clone()),
            ..Default::default()
        },
    )
    .unwrap()
}

#[tokio::test]
async fn seeded_history_preserves_snapshot_order_and_provenance_without_replaying_or_appending() {
    let probe = Arc::new(Probe::default());
    let (request, markers) = history();
    let input = request.input.clone();
    let outcome = runner(&probe)
        .run_with_approval_history(context(), request, probe.clone(), markers.clone())
        .await
        .unwrap();
    assert_eq!(outcome.result.new_items, vec![message()]);
    assert_eq!(&outcome.result.history[..2], &input);
    assert!(outcome.result.pending_approvals.is_empty());
    assert_eq!(
        *probe.commits.lock().unwrap(),
        vec![(vec![message()], vec![])]
    );
    let requests = probe.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].input, input);
    assert_eq!(requests[0].input_provenance, history().0.input_provenance);
    let generations = probe.generations.lock().unwrap();
    let record = &generations[0];
    assert_eq!(
        record.request_snapshot.as_ref().unwrap(),
        &adk_codec::snapshots::RequestSnapshot::from_native_with_approvals(
            "agent",
            &record.request,
            &[],
            &markers
        )
        .unwrap()
    );
    assert_eq!(
        record.request_snapshot.as_ref().unwrap().input_items.len(),
        4
    );
}

#[tokio::test]
async fn malformed_markers_fail_before_model_or_commit() {
    for case in 0..3 {
        let probe = Arc::new(Probe::default());
        let (request, mut markers) = history();
        match case {
            0 => markers[1].before_item = 3,
            1 => markers[1].before_item = 0,
            _ => markers[0].marker.data.approved = true,
        }
        let error = runner(&probe)
            .run_with_approval_history(context(), request, probe.clone(), markers)
            .await
            .err()
            .unwrap();
        assert_eq!(error.error.info.category, ErrorCategory::InvalidInput);
        assert!(probe.requests.lock().unwrap().is_empty());
        assert!(probe.generations.lock().unwrap().is_empty());
        assert!(probe.commits.lock().unwrap().is_empty());
    }
}

#[derive(Default)]
struct Store(Mutex<Vec<RunnerCheckpoint>>);
impl CheckpointStore for Store {
    fn persist<'a>(
        &'a self,
        _: &'a Context,
        checkpoint: &'a RunnerCheckpoint,
    ) -> BoxFuture<'a, Result<(), Error>> {
        let decoded = RunnerCheckpoint::decode(&serde_json::to_vec(checkpoint).unwrap()).unwrap();
        self.0.lock().unwrap().push(decoded);
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn durable_restore_retains_historical_only_markers_without_republishing() {
    let probe = Arc::new(Probe::default());
    let runner = runner(&probe);
    let store = Arc::new(Store::default());
    let (request, markers) = history();
    runner
        .run_durable(
            context(),
            request,
            probe.clone(),
            DurableRun::new(store.clone()),
        )
        .await
        .unwrap();
    let mut checkpoint = serde_json::to_value(
        store
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.execution_boundary() == "run_started")
            .unwrap(),
    )
    .unwrap();
    checkpoint["runtime"]["approval_journal_present"] = json!(true);
    checkpoint["runtime"]["approval_journal"] = json!(
        markers
            .iter()
            .map(|boundary| adk_runtime::compat::ApprovalJournalEntry {
                argument_text: Some(boundary.marker.data.input.text().into_owned()),
                marker: boundary.marker.clone(),
                new_items_before: 0,
                history_before: Some(boundary.before_item),
                historical_only: true,
                reason: None,
            })
            .collect::<Vec<_>>()
    );
    let checkpoint = RunnerCheckpoint::decode(&serde_json::to_vec(&checkpoint).unwrap()).unwrap();
    probe.commits.lock().unwrap().clear();
    probe.generations.lock().unwrap().clear();
    let resumed = Arc::new(Store::default());
    runner
        .run_durable(
            context(),
            RunRequest {
                input: vec![],
                input_provenance: vec![],
                policy: RunPolicy::default(),
            },
            probe.clone(),
            DurableRun {
                resume: Some(checkpoint),
                ..DurableRun::new(resumed.clone())
            },
        )
        .await
        .unwrap();
    assert_eq!(
        *probe.commits.lock().unwrap(),
        vec![(vec![message()], vec![])]
    );
    let checkpoints = resumed.0.lock().unwrap();
    let saved = checkpoints.last().unwrap();
    let value = serde_json::to_value(saved).unwrap();
    let entries = value["runtime"]["approval_journal"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry["historical_only"] == true));
    assert_eq!(saved.history.len(), 5);
    assert_eq!(
        probe.generations.lock().unwrap()[0]
            .request_snapshot
            .as_ref()
            .unwrap()
            .input_items
            .len(),
        4
    );
}

#[tokio::test]
async fn missing_marker_input_has_explicit_provenance_and_is_not_fabricated_as_null() {
    let probe = Arc::new(Probe::default());
    let (mut request, mut markers) = history();
    if let RunItem::ToolCall { call } = &mut request.input[0] {
        call.arguments = json!(null);
        call.raw_arguments = Some(String::new());
    }
    for marker in &mut markers {
        marker.marker.data.input = dto::RawJson::Missing;
    }
    runner(&probe)
        .run_with_approval_history(context(), request, probe.clone(), markers)
        .await
        .unwrap();
    let requests = probe.requests.lock().unwrap();
    let RunItem::ToolCall { call } = &requests[0].input[0] else {
        panic!("expected historical call")
    };
    assert_eq!(call.raw_arguments.as_deref(), Some(""));
    assert_eq!(call.argument_text(), "");
}
