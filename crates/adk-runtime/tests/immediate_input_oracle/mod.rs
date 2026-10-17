//! Bounded projection of actual pinned SDK runner observations, not fabricated expectations.
use super::*;

mod signal;

struct InputQueue {
    name: String,
    polls: AtomicUsize,
    finalizers: AtomicUsize,
}
impl InputQueue {
    fn batch(texts: &[&str]) -> ImmediateInputBatch {
        ImmediateInputBatch {
            items: texts.iter().map(|text| message(Role::User, text)).collect(),
            provenance: vec![ItemProvenance::Unattributed; texts.len()],
        }
    }
}
impl ImmediateInputPoller for InputQueue {
    fn poll<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<ImmediateInputBatch, Error>> {
        Box::pin(async move {
            let count = self.polls.fetch_add(1, Ordering::SeqCst) + 1;
            if self.name == "poll_error_best_effort" && count == 1 {
                // Rust Result cannot carry an errored batch; no items are admitted.
                return Err(Error::new(ErrorCategory::Host, "oracle poll failure"));
            }
            let poll_at = match self.name.as_str() {
                "poll_before_first_request" => 1,
                "poll_after_tool_response" => 2,
                _ => 0,
            };
            Ok(if count == poll_at {
                Self::batch(&["polled-1", "polled-2"])
            } else {
                Self::batch(&[])
            })
        })
    }
}
impl ImmediateInputFinalizer for InputQueue {
    fn finalize<'a>(&'a self, _: &'a Context) -> BoxFuture<'a, Result<ImmediateInputBatch, Error>> {
        Box::pin(async move {
            let count = self.finalizers.fetch_add(1, Ordering::SeqCst) + 1;
            if self.name == "finalizer_error_propagated" {
                return Err(Error::new(ErrorCategory::Host, "oracle finalizer failure"));
            }
            Ok(
                if count == 1
                    && matches!(
                        self.name.as_str(),
                        "finalizer_late_input" | "max_turns_finalizer_extends"
                    )
                {
                    Self::batch(&["late"])
                } else {
                    Self::batch(&[])
                },
            )
        })
    }
}
fn content_text(content: &[Content]) -> String {
    content
        .iter()
        .map(|content| match content {
            Content::Text { text } => text.as_str(),
            _ => panic!("oracle projection does not cover media"),
        })
        .collect()
}
fn project(items: &[RunItem], provenance: &[ItemProvenance]) -> Value {
    assert_eq!(items.len(), provenance.len());
    Value::Array(items.iter().zip(provenance).map(|(item, provenance)| {
        let agent = match provenance {
            ItemProvenance::Agent { name } => json!(name),
            ItemProvenance::Unattributed => Value::Null,
            ItemProvenance::Unknown => panic!("fixture authorship must remain known"),
        };
        match item {
            RunItem::Message { message } => json!({"type":"message", "agent":agent, "message":content_text(&message.content)}),
            RunItem::ToolCall { call } => json!({"type":"tool_call", "agent":agent, "tool_call":{"id":call.id,"name":call.name,"input":call.arguments}}),
            RunItem::ToolResult { call_id, output } => {
                assert!(!output.is_error && !output.should_pause);
                json!({"type":"tool_output", "agent":agent, "tool_output":{"call_id":call_id,"content":content_text(&output.content)}})
            }
            _ => panic!("uncovered oracle item: {item:?}"),
        }
    }).collect())
}

#[tokio::test]
async fn boundary_polling_and_finalization_match_pinned_sdk_in_run_and_stream() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/handoff/sdk-immediate-input.json"
    ))
    .unwrap();
    assert_eq!(
        fixture["sdk_revision"],
        "1dc92b73900fac74dc357a938e4b5eee6392b418"
    );
    assert_eq!(fixture["schema_version"], 1);
    let cases = fixture["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 14);
    for expected in cases {
        let name = expected["name"].as_str().unwrap();
        let streaming = expected["streaming"].as_bool().unwrap();
        let first_tool = matches!(
            name,
            "poll_after_tool_response" | "max_turns_finalizer_extends"
        );
        let mut replies = vec![if first_tool {
            response(vec![call("call-1", "inspect")], Some(false))
        } else {
            answer("answer-1")
        }];
        if first_tool || name == "finalizer_late_input" {
            replies.push(answer("answer-2"));
        }
        let model = TestModel::with(replies.iter().cloned().map(Ok).collect());
        *model.streams.lock().unwrap() = replies
            .into_iter()
            .map(|response| vec![StreamStep::Event(ModelEvent::Complete { response })])
            .collect();
        let mut tool = TestTool::new("inspect", false, false);
        Arc::get_mut(&mut tool).unwrap().output.content = vec![Content::Text {
            text: "tool result".into(),
        }];
        let mut agent = agent(model.clone());
        agent.name = "oracle".into();
        agent.tools.push(tool.clone());
        let queue = Arc::new(InputQueue {
            name: name.into(),
            polls: AtomicUsize::new(0),
            finalizers: AtomicUsize::new(0),
        });
        let runner = Runner::new(
            agent,
            RunnerConfig {
                immediate_input_poller: Some(queue.clone()),
                immediate_input_finalizer: Some(queue.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let input = RunRequest {
            input: vec![message(Role::User, "initial")],
            input_provenance: vec![ItemProvenance::Unattributed],
            policy: policy(expected["max_turns"].as_u64().unwrap() as u32),
        };
        let result = if streaming {
            runner
                .stream(context(), input, Arc::new(TestHost::default()))
                .finish()
                .await
        } else {
            runner
                .run(context(), input, Arc::new(TestHost::default()))
                .await
        };
        let (result, error) = match result {
            Ok(outcome) => (outcome.result, "none"),
            Err(error) => {
                assert_eq!(error.error.info.category, ErrorCategory::Host, "{name}");
                assert_eq!(
                    error.error.info.message, "oracle finalizer failure",
                    "{name}"
                );
                (*error.partial.unwrap(), "finalizer_error")
            }
        };
        let requests: Vec<Value> = model
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| json!({"input":project(&request.input, &request.input_provenance)}))
            .collect();
        let actual = json!({
            "name":name, "streaming":streaming, "max_turns":expected["max_turns"],
            "requests":requests, "poll_calls":queue.polls.load(Ordering::SeqCst),
            "finalizer_calls":queue.finalizers.load(Ordering::SeqCst),
            "tool_calls":tool.calls.load(Ordering::SeqCst), "output":result.final_output,
            "error":error, "new_items":project(&result.new_items,&result.new_items_provenance),
            "history":project(&result.history,&result.history_provenance)
        });
        assert_eq!(&actual, expected, "{name} streaming={streaming}");
    }
}
