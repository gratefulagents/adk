//! Signal ordering compared with the independent public SDK runner.
use super::*;
use std::sync::atomic::AtomicBool;
use tokio::sync::Notify;

#[derive(Default)]
struct SignalQueue {
    pending: Mutex<Vec<&'static str>>,
    callbacks: Mutex<Vec<Value>>,
    notification: Notify,
    released: Notify,
    triggered: AtomicBool,
}
impl SignalQueue {
    fn steer(&self) {
        assert!(!self.triggered.swap(true, Ordering::SeqCst));
        *self.pending.lock().unwrap() = vec!["steer-1", "steer-2"];
        self.notification.notify_one();
    }
    fn drain(&self, name: &str) -> ImmediateInputBatch {
        let values = std::mem::take(&mut *self.pending.lock().unwrap());
        let batch = InputQueue::batch(&values);
        self.callbacks.lock().unwrap().push(json!({
            "name":name, "items":project(&batch.items,&batch.provenance)
        }));
        batch
    }
}
impl ImmediateInputPoller for SignalQueue {
    fn poll<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<ImmediateInputBatch, Error>> {
        Box::pin(async move { Ok(self.drain("poll")) })
    }
}
impl ImmediateInputFinalizer for SignalQueue {
    fn finalize<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<ImmediateInputBatch, Error>> {
        Box::pin(async move { Ok(self.drain("finalizer")) })
    }
}
impl ImmediateInputSignal for SignalQueue {
    fn wait<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.notification.notified().await;
            self.released.notify_one();
        })
    }
}
struct Aborted<'a>(&'a AtomicBool);
impl Drop for Aborted<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
struct SignalModel {
    queue: Arc<SignalQueue>,
    requests: Mutex<Vec<ModelRequest>>,
    cancelled: AtomicBool,
    visible: bool,
    reasoning: bool,
    tool: bool,
}
impl SignalModel {
    async fn begin(&self, request: ModelRequest) -> bool {
        let first = {
            let mut requests = self.requests.lock().unwrap();
            requests.push(request);
            assert!(requests.len() <= 2);
            requests.len() == 1
        };
        if first && !self.visible {
            let _drop = Aborted(&self.cancelled);
            self.queue.steer();
            std::future::pending::<()>().await;
        }
        first
    }
    fn reply(&self, first: bool) -> ModelResponse {
        if first && self.tool {
            response(vec![call("call-1", "inspect")], Some(false))
        } else {
            answer(if first { "answer-1" } else { "answer-2" })
        }
    }
}
impl Model for SignalModel {
    fn provider(&self) -> &str {
        "offline"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move { Ok(self.reply(self.begin(request).await)) })
    }
}
struct SignalStream {
    queue: Arc<SignalQueue>,
    delta: Option<ModelEvent>,
    response: Option<ModelResponse>,
    gated: bool,
}
impl ModelStream for SignalStream {
    fn next(&mut self) -> BoxFuture<'_, Result<Option<ModelEvent>, Error>> {
        Box::pin(async move {
            if let Some(delta) = self.delta.take() {
                return Ok(Some(delta));
            }
            if self.gated {
                self.queue.released.notified().await;
                self.gated = false;
            }
            Ok(self
                .response
                .take()
                .map(|response| ModelEvent::Complete { response }))
        })
    }
}
impl StreamingModel for SignalModel {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            let first = self.begin(request).await;
            let delta = (first && self.visible).then(|| {
                if self.reasoning {
                    ModelEvent::ReasoningDelta {
                        delta: "visible-first-attempt".into(),
                    }
                } else {
                    ModelEvent::TextDelta {
                        delta: "visible-first-attempt".into(),
                    }
                }
            });
            Ok(Box::new(SignalStream {
                queue: self.queue.clone(),
                gated: delta.is_some(),
                delta,
                response: Some(self.reply(first)),
            }) as Box<dyn ModelStream>)
        })
    }
}
struct SignalHost {
    queue: Arc<SignalQueue>,
    deltas: Mutex<Vec<Value>>,
}
impl Host for SignalHost {
    fn emit<'a>(&'a self, _: &'a Context, event: RunEvent) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async move {
            let delta = match event {
                RunEvent::Model {
                    event: ModelEvent::TextDelta { delta },
                } => Some(("model.delta", delta)),
                RunEvent::Model {
                    event: ModelEvent::ReasoningDelta { delta },
                } => Some(("model.reasoning_delta", delta)),
                _ => None,
            };
            if let Some((name, text)) = delta {
                self.deltas
                    .lock()
                    .unwrap()
                    .push(json!({"name":name,"text":text}));
                self.queue.steer();
            }
            Ok(())
        })
    }
    fn approve<'a>(
        &'a self,
        _: &'a Context,
        _: ApprovalRequest,
    ) -> BoxFuture<'a, Result<ApprovalDecision, Error>> {
        Box::pin(async { Ok(ApprovalDecision::Approve) })
    }
}
#[tokio::test]
async fn signal_replacement_and_committed_streams_match_pinned_sdk() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/handoff/sdk-immediate-signal.json"
    ))
    .unwrap();
    assert_eq!(fixture["schema_version"], 1);
    assert_eq!(
        fixture["sdk_revision"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 8);
    for expected in cases {
        let name = expected["name"].as_str().unwrap();
        let streaming = expected["streaming"].as_bool().unwrap();
        let queue = Arc::new(SignalQueue::default());
        let model = Arc::new(SignalModel {
            queue: queue.clone(),
            requests: Mutex::new(vec![]),
            cancelled: AtomicBool::new(false),
            visible: name.starts_with("after_visible"),
            reasoning: name.contains("reasoning"),
            tool: name.ends_with("tool_boundary"),
        });
        let mut tool = TestTool::new("inspect", false, false);
        Arc::get_mut(&mut tool).unwrap().output.content = vec![Content::Text {
            text: "tool result".into(),
        }];
        let mut agent =
            AgentConfig::new("oracle", ModelBinding::streaming("offline", model.clone()));
        agent.tools.push(tool.clone());
        let runner = Runner::new(
            agent,
            RunnerConfig {
                immediate_input_poller: Some(queue.clone()),
                immediate_input_finalizer: Some(queue.clone()),
                immediate_input_signal: Some(queue.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let request = RunRequest {
            input: vec![message(Role::User, "initial")],
            input_provenance: vec![ItemProvenance::Unattributed],
            policy: policy(expected["max_turns"].as_u64().unwrap() as u32),
        };
        let host = Arc::new(SignalHost {
            queue: queue.clone(),
            deltas: Mutex::new(vec![]),
        });
        let run = async {
            if streaming {
                runner
                    .stream(context(), request, host.clone())
                    .finish()
                    .await
            } else {
                runner.run(context(), request, host.clone()).await
            }
        };
        let result = tokio::time::timeout(Duration::from_secs(3), run)
            .await
            .expect(name)
            .unwrap()
            .result;
        let requests = model.requests.lock().unwrap();
        let callbacks = queue.callbacks.lock().unwrap();
        assert_eq!(
            requests.len(),
            expected["request_count"].as_u64().unwrap() as usize,
            "{name}"
        );
        assert_eq!(
            json!(
                requests
                    .iter()
                    .map(|r| json!({"input":project(&r.input,&r.input_provenance)}))
                    .collect::<Vec<_>>()
            ),
            expected["requests"],
            "{name}"
        );
        assert_eq!(json!(&*callbacks), expected["callbacks"], "{name}");
        assert_eq!(
            json!(callbacks.iter().filter(|c| c["name"] == "poll").count()),
            expected["poll_calls"],
            "{name}"
        );
        assert_eq!(
            json!(
                callbacks
                    .iter()
                    .filter(|c| c["name"] == "finalizer")
                    .count()
            ),
            expected["finalizer_calls"],
            "{name}"
        );
        assert_eq!(
            json!(model.cancelled.load(Ordering::SeqCst)),
            expected["first_attempt_canceled"],
            "{name}"
        );
        assert_eq!(
            json!(tool.calls.load(Ordering::SeqCst)),
            expected["tool_calls"],
            "{name}"
        );
        assert_eq!(json!(result.final_output), expected["output"], "{name}");
        assert_eq!(
            project(&result.history, &result.history_provenance),
            expected["history"],
            "{name}"
        );
        assert_eq!(
            project(&result.new_items, &result.new_items_provenance),
            expected["new_items"],
            "{name}"
        );
        assert_eq!(
            json!(&*host.deltas.lock().unwrap()),
            expected["consumed_stream_deltas"],
            "{name}"
        );
        assert_eq!(
            result.responses.len(),
            expected["accepted_responses"].as_array().unwrap().len(),
            "{name}"
        );
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("hidden-superseded-answer")
        );
    }
}
