use super::*;
use adk::runtime::{CompactionModelResolver, compaction::LocalCompactionPolicy};
use sha2::{Digest, Sha256};

mod metadata;

struct Resolver {
    value: Option<(u64, u64)>,
    models: Mutex<Vec<String>>,
}
impl CompactionModelResolver for Resolver {
    fn thresholds<'a>(
        &'a self,
        _: &'a Context,
        model: &'a str,
    ) -> BoxFuture<'a, Result<Option<(u64, u64)>, Error>> {
        Box::pin(async move {
            self.models.lock().unwrap().push(model.into());
            Ok(self.value)
        })
    }
}
struct ProbeModel {
    requests: Mutex<Vec<ModelRequest>>,
    continuing: bool,
}
impl ProbeModel {
    fn response(&self, request: ModelRequest) -> ModelResponse {
        let mut requests = self.requests.lock().unwrap();
        let mut out = response();
        out.items = vec![RunItem::Message {
            message: Message {
                role: Role::Assistant,
                content: vec![Content::Text {
                    text: "done".into(),
                }],
            },
        }];
        out.end_turn = Some(!self.continuing || !requests.is_empty());
        requests.push(request);
        out
    }
}
impl Model for ProbeModel {
    fn provider(&self) -> &str {
        "offline"
    }
    fn complete<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<ModelResponse, Error>> {
        Box::pin(async move { Ok(self.response(request)) })
    }
}
impl StreamingModel for ProbeModel {
    fn stream<'a>(
        &'a self,
        _: &'a Context,
        request: ModelRequest,
    ) -> BoxFuture<'a, Result<Box<dyn ModelStream + 'a>, Error>> {
        Box::pin(async move {
            Ok(Box::new(Events(VecDeque::from([ModelEvent::Complete {
                response: self.response(request),
            }]))) as Box<dyn ModelStream>)
        })
    }
}

#[tokio::test]
async fn resolver_requests_and_call_frequency_match_pinned_sdk() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/run-instructions/observations.json"
    ))
    .unwrap();
    for case in fixture["resolver_cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case.get("retry").is_none())
    {
        let thresholds = &case["thresholds"];
        let resolver = Arc::new(Resolver {
            value: (thresholds[2] == 1).then(|| {
                (
                    thresholds[0].as_u64().unwrap(),
                    thresholds[1].as_u64().unwrap(),
                )
            }),
            models: Mutex::new(vec![]),
        });
        let model = Arc::new(ProbeModel {
            requests: Mutex::new(vec![]),
            continuing: case["continuing"].as_bool().unwrap(),
        });
        let mut bundle = Builder::new(Config {
            model: "gpt-5-mini".into(),
            instructions: "base".into(),
            work_dir: "".into(),
            local_compaction: Some(LocalCompactionPolicy {
                use_llm_summary: false,
                ..Default::default()
            }),
            features: Some(Features {
                compaction: case["feature"].as_bool().unwrap(),
                ..Default::default()
            }),
            ..Default::default()
        })
        .model("openai", Kind::OpenAi, model.clone())
        .unwrap()
        .runner_config(RunnerConfig {
            compaction_model_resolver: Some(resolver.clone()),
            ..Default::default()
        })
        .build(&context())
        .await
        .unwrap();
        let input = (0..100)
            .map(|i| RunItem::Message {
                message: Message {
                    role: Role::User,
                    content: vec![Content::Text {
                        text: format!(
                            "message {i:03}: {}",
                            "old conversation ".repeat(if i >= 80 { 2 } else { 400 })
                        ),
                    }],
                },
            })
            .collect();
        if case["streaming"] == true {
            bundle
                .stream(context(), input, Arc::new(TestHost))
                .finish()
                .await
                .unwrap();
        } else {
            bundle
                .run(context(), input, Arc::new(TestHost))
                .await
                .unwrap();
        }
        assert_eq!(
            json!(*resolver.models.lock().unwrap()),
            case["models"],
            "{case}"
        );
        let digests = model
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| {
                let mut hash = Sha256::new();
                for item in &request.input {
                    let RunItem::Message { message } = item else {
                        panic!("unexpected nonmessage")
                    };
                    let [Content::Text { text }] = message.content.as_slice() else {
                        panic!("unexpected content")
                    };
                    hash.update(format!("{}:", text.len()).as_bytes());
                    hash.update(text.as_bytes());
                }
                format!("{hash:x}", hash = hash.finalize())
            })
            .collect::<Vec<_>>();
        assert_eq!(json!(digests), case["text_digests"], "{case}");
        bundle.close().await.unwrap();
    }
}
