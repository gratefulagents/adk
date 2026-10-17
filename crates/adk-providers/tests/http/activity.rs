use super::*;
use adk_core::ModelActivitySink;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

#[tokio::test]
async fn every_body_read_reports_activity_even_without_a_model_event() {
    for transport in ["json", "collected-sse", "stream"] {
        for wrapper in ["direct", "routes", "copilot"] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
            let (ack, received) = mpsc::channel();
            let count = Arc::new(AtomicUsize::new(0));
            let observed = count.clone();
            let activity: ModelActivitySink = Arc::new(move || {
                observed.fetch_add(1, Ordering::SeqCst);
                ack.send(()).unwrap();
            });
            let server = std::thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                read_request(&mut socket);
                let content_type = if transport == "json" {
                    "application/json"
                } else {
                    "text/event-stream"
                };
                write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
                let chunks = if transport == "json" {
                    [
                        " ",
                        "\n",
                        r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}"#,
                    ]
                } else {
                    [
                        ": heartbeat one\n\n",
                        ": heartbeat two\n\n",
                        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n",
                    ]
                };
                for chunk in chunks {
                    write!(socket, "{:x}\r\n{chunk}\r\n", chunk.len()).unwrap();
                    socket.flush().unwrap();
                    received.recv_timeout(Duration::from_secs(3)).unwrap();
                }
                let _ = socket.write_all(b"0\r\n\r\n");
            });
            let model: Arc<dyn StreamingModel> = match wrapper {
                "routes" => {
                    let mut routes = adk_providers::routing::Routes::new("fixture");
                    routes
                        .register("fixture", Arc::new(provider(&endpoint)))
                        .unwrap();
                    Arc::new(routes)
                }
                "copilot" => {
                    let scope =
                        Scope::new("fixture", &endpoint, None, AuthMode::CopilotOAuth).unwrap();
                    let session = Arc::new(
                        Session::new(scope, Arc::new(StaticStore), Arc::new(NoRefresh)).unwrap(),
                    );
                    Arc::new(adk_providers::copilot::Copilot::new("fixture", session).unwrap())
                }
                _ => Arc::new(provider(&endpoint)),
            };
            let ctx = context();
            if transport == "stream" {
                let mut stream = model
                    .stream_with_activity(&ctx, request(), Some(activity))
                    .await
                    .unwrap();
                let mut complete = 0;
                while let Some(event) = stream.next().await.unwrap() {
                    if matches!(event, ModelEvent::Complete { .. }) {
                        complete += 1;
                    }
                }
                assert_eq!(complete, 1);
            } else {
                let response = model
                    .complete_with_activity(&ctx, request(), Some(activity))
                    .await
                    .unwrap();
                assert!(!response.items.is_empty());
            }
            server.join().unwrap();
            assert!(count.load(Ordering::SeqCst) >= 3, "{transport} {wrapper}");
        }
    }
}
