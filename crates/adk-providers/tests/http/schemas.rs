use super::*;
use adk_core::ErrorCategory;

#[tokio::test]
async fn anthropic_raw_schema_http_body_matches_pinned_go() {
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../fixtures/providers/anthropic-schemas.json"
    ))
    .unwrap();
    assert_eq!(cases.as_array().unwrap().len(), 17);
    for streaming in [false, true] {
        for case in cases.as_array().unwrap() {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let scope = Scope::new("fixture", &endpoint, None, AuthMode::ApiKey).unwrap();
            let model = Provider::new(
                "fixture",
                Protocol::Anthropic,
                Arc::new(Session::new(scope, Arc::new(StaticStore), Arc::new(NoRefresh)).unwrap()),
            )
            .unwrap();
            let mut input = request();
            input.model = "claude-opus-4.7".into();
            input
                .settings
                .insert("reasoning_effort".into(), serde_json::json!("max"));
            let cx = context();
            input.output_schema =
                Some(adk_core::JsonDocument::new(case["raw"].as_str().unwrap().into()).unwrap());
            if case["error"] == true {
                let error = if streaming {
                    model.stream(&cx, input).await.err().unwrap()
                } else {
                    model.complete(&cx, input).await.unwrap_err()
                };
                assert_eq!(error.info.category, ErrorCategory::InvalidInput);
                listener.set_nonblocking(true).unwrap();
                assert_eq!(
                    listener.accept().unwrap_err().kind(),
                    std::io::ErrorKind::WouldBlock
                );
                continue;
            }
            let server = std::thread::spawn(move || {
                let mut requests = Vec::new();
                for attempt in 0..2 {
                    let (mut socket, _) = listener.accept().unwrap();
                    requests.push(read_request(&mut socket));
                    let (status, body) = if attempt == 0 {
                        (400, r#"{"error":{"message":"effort rejected"}}"#)
                    } else {
                        (
                            200,
                            r#"{"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn"}"#,
                        )
                    };
                    write!(socket, "HTTP/1.1 {status} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                }
                requests
            });
            if streaming {
                drop(model.stream(&cx, input).await.unwrap());
            } else {
                model.complete(&cx, input).await.unwrap();
            }
            let captured = server.join().unwrap();
            assert_eq!(captured.len(), 2);
            for (attempt, captured) in captured.iter().enumerate() {
                let (_, body) = captured.split_once("\r\n\r\n").unwrap();
                let fields: std::collections::BTreeMap<String, &serde_json::value::RawValue> =
                    serde_json::from_str(body).unwrap();
                let format: std::collections::BTreeMap<String, &serde_json::value::RawValue> =
                    serde_json::from_str(fields["output_format"].get()).unwrap();
                assert_eq!(fields["stream"].get(), "true");
                let output: serde_json::Value =
                    serde_json::from_str(fields["output_config"].get()).unwrap();
                assert_eq!(output["effort"], if attempt == 0 { "max" } else { "high" });
                assert_eq!(format["type"].get(), "\"json_schema\"");
                assert_eq!(
                    format["schema"].get(),
                    case["schema_json"].as_str().unwrap()
                );
            }
        }
    }
}
