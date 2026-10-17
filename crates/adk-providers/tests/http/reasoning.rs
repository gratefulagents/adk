use super::*;
use adk_core::{ModelActivitySink, ModelReasoningSink, RunItem};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

#[tokio::test]
async fn completion_does_not_replay_buffered_json_reasoning_as_live_deltas() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        read_request(&mut socket);
        let body = r#"{"choices":[{"message":{"content":"ok","reasoning_content":"already complete"},"finish_reason":"stop"}]}"#;
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let sink: ModelReasoningSink = Arc::new(|_| panic!("buffered reasoning is not a live delta"));
    let response = provider(&endpoint)
        .complete_with_observers(&context(), request(), None, Some(sink))
        .await
        .unwrap();
    server.join().unwrap();
    assert!(response.items.iter().any(|item| matches!(item, RunItem::Reasoning { reasoning } if reasoning.text == "already complete")));
}

#[tokio::test]
async fn completion_reasoning_survives_sse_and_buffered_protocol_paths() {
    for protocol in [Protocol::Responses, Protocol::Anthropic] {
        for content_type in ["text/event-stream", "application/octet-stream"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
            let events = match protocol {
                Protocol::Responses => vec![
                    serde_json::json!({"type":"response.reasoning_summary_text.delta","delta":""}),
                    serde_json::json!({"type":"response.reasoning_summary_text.delta","delta":"think "}),
                    serde_json::json!({"type":"response.reasoning_summary_text.delta","delta":"deeply"}),
                    serde_json::json!({"type":"response.completed","response":{"output":[]}}),
                ],
                Protocol::Anthropic => vec![
                    serde_json::json!({"type":"message_start","message":{"content":[],"usage":{}}}),
                    serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}),
                    serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}),
                    serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"think "}}),
                    serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"deeply"}}),
                    serde_json::json!({"type":"content_block_stop","index":0}),
                    serde_json::json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{}}),
                    serde_json::json!({"type":"message_stop"}),
                ],
                Protocol::Chat => unreachable!(),
            };
            let body = events
                .iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect::<String>();
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                read_request(&mut socket);
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            });
            let scope = Scope::new("fixture", &endpoint, None, AuthMode::ApiKey).unwrap();
            let session =
                Arc::new(Session::new(scope, Arc::new(StaticStore), Arc::new(NoRefresh)).unwrap());
            let provider = Provider::new("fixture", protocol, session).unwrap();
            let chunks = Arc::new(Mutex::new(Vec::new()));
            let observed = chunks.clone();
            let sink: ModelReasoningSink =
                Arc::new(move |text| observed.lock().unwrap().push(text.to_owned()));
            let response = provider
                .complete_with_observers(&context(), request(), None, Some(sink))
                .await
                .unwrap();
            server.join().unwrap();
            assert_eq!(*chunks.lock().unwrap(), ["think ", "deeply"]);
            assert!(response.items.iter().any(|item| matches!(item, RunItem::Reasoning { reasoning } if reasoning.text == "think deeply")));
        }
    }
}

#[tokio::test]
async fn completion_reasoning_is_live_ordered_and_forwarded() {
    for wrapper in ["direct", "routes", "copilot"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let (ack, received) = mpsc::channel();
        let activity_count = Arc::new(AtomicUsize::new(0));
        let activity_observed = activity_count.clone();
        let activity: ModelActivitySink = Arc::new(move || {
            activity_observed.fetch_add(1, Ordering::SeqCst);
        });
        let chunks = Arc::new(Mutex::new(Vec::new()));
        let observed = chunks.clone();
        let reasoning: ModelReasoningSink = Arc::new(move |text| {
            observed.lock().unwrap().push(text.to_owned());
            ack.send(()).unwrap();
        });
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            read_request(&mut socket);
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
            for text in ["think ", "deeply"] {
                let chunk = format!(
                    "data: {{\"choices\":[{{\"delta\":{{\"reasoning_content\":\"{text}\"}}}}]}}\n\n"
                );
                write!(socket, "{:x}\r\n{chunk}\r\n", chunk.len()).unwrap();
                socket.flush().unwrap();
                received.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            let chunk = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
            write!(socket, "{:x}\r\n{chunk}\r\n0\r\n\r\n", chunk.len()).unwrap();
        });
        let model: Arc<dyn Model> = match wrapper {
            "routes" => {
                let mut routes = adk_providers::routing::Routes::new("fixture");
                routes
                    .register("fixture", Arc::new(provider(&endpoint)))
                    .unwrap();
                Arc::new(routes)
            }
            "copilot" => {
                let scope = Scope::new("fixture", &endpoint, None, AuthMode::CopilotOAuth).unwrap();
                let session = Arc::new(
                    Session::new(scope, Arc::new(StaticStore), Arc::new(NoRefresh)).unwrap(),
                );
                Arc::new(adk_providers::copilot::Copilot::new("fixture", session).unwrap())
            }
            _ => Arc::new(provider(&endpoint)),
        };
        let response = model
            .complete_with_observers(&context(), request(), Some(activity), Some(reasoning))
            .await
            .unwrap();
        server.join().unwrap();
        assert!(activity_count.load(Ordering::SeqCst) > 0, "{wrapper}");
        assert_eq!(*chunks.lock().unwrap(), ["think ", "deeply"], "{wrapper}");
        assert!(response.items.iter().any(|item| matches!(item, RunItem::Reasoning { reasoning } if reasoning.text == "think deeply")), "{wrapper}: {:?}", response.items);
    }
}
