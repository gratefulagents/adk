use super::*;

#[tokio::test]
async fn anthropic_usage_updates_match_pinned_go_model_paths() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/providers/anthropic-usage.json"
    ))
    .unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 11);
    for (index, case) in cases.as_array().unwrap().iter().enumerate() {
        for streaming in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let body = case["events"]
                .as_array()
                .unwrap()
                .iter()
                .map(|raw| {
                    let raw = raw.as_str().unwrap();
                    let event: serde_json::Value = serde_json::from_str(raw).unwrap();
                    format!(
                        "event: {}\ndata: {raw}\n\n",
                        event["type"].as_str().unwrap()
                    )
                })
                .collect::<String>();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                read_request(&mut socket);
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            });
            let scope = Scope::new("fixture", &endpoint, None, AuthMode::ApiKey).unwrap();
            let model = Provider::new(
                "fixture",
                Protocol::Anthropic,
                Arc::new(Session::new(scope, Arc::new(StaticStore), Arc::new(NoRefresh)).unwrap()),
            )
            .unwrap();
            let cx = context();
            let response = if streaming {
                let mut stream = model.stream(&cx, request()).await.unwrap();
                let mut result = None;
                while let Some(event) = stream.next().await.unwrap() {
                    if let ModelEvent::Complete { response } = event {
                        result = Some(response);
                    }
                }
                result.unwrap()
            } else {
                model.complete(&cx, request()).await.unwrap()
            };
            server.join().unwrap();
            let expected = &case[if streaming {
                "stream_usage"
            } else {
                "complete_usage"
            }];
            let snapshot: serde_json::Value =
                serde_json::from_str(response.snapshot_raw.as_ref().unwrap().as_str()).unwrap();
            for (key, value) in [
                ("input_tokens", response.usage.input_tokens),
                ("output_tokens", response.usage.output_tokens),
                ("cache_read_input_tokens", response.usage.cache_read_tokens),
                (
                    "cache_creation_input_tokens",
                    response.usage.cache_creation_tokens,
                ),
            ] {
                let want = expected[key].as_u64().unwrap_or_default();
                assert_eq!(value, want, "case {index}, streaming={streaming}, {key}");
                assert_eq!(
                    snapshot["usage"][key].as_u64().unwrap_or_default(),
                    want,
                    "snapshot case {index}, streaming={streaming}, {key}"
                );
            }
            assert_eq!(
                response.usage.context_tokens,
                Some(
                    response.usage.input_tokens
                        + response.usage.cache_read_tokens
                        + response.usage.cache_creation_tokens
                )
            );
        }
    }
}
