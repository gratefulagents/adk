use adk_core::*;
use adk_tools::{Config, Features, Registry};
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};
fn tool(private: bool) -> Arc<dyn Tool> {
    Registry::build(
        &Config {
            features: Features::Strict(["WebFetch".into()].into()),
            allow_private_network_urls: private,
            ..Default::default()
        },
        [],
    )
    .unwrap()
    .get("WebFetch")
    .unwrap()
    .clone()
}
fn context() -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "web".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: Default::default(),
        policy: Default::default(),
        idempotency_key: None,
    }
}
async fn invoke(private: bool, args: Value) -> ToolOutput {
    tool(private)
        .execute(
            &context(),
            ToolCall {
                id: "call".into(),
                name: "WebFetch".into(),
                arguments: args,
            },
        )
        .await
        .unwrap()
}
fn text(output: &ToolOutput) -> &str {
    match &output.content[..] {
        [Content::Text { text }] => text,
        _ => panic!("text"),
    }
}
async fn headers(socket: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0; 1024];
    while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
        let count = socket.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 16384);
    }
    String::from_utf8(bytes).unwrap()
}
async fn server(
    status: &str,
    extra: &str,
    body: &[u8],
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let mut response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(body);
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = headers(&mut socket).await;
        let _ = socket.write_all(&response).await;
        request
    });
    (url, task)
}
#[tokio::test]
async fn public_policy_cannot_be_overridden_by_arguments() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let output=invoke(false,json!({"url":format!("http://{}",listener.local_addr().unwrap()),"allow_private_network_urls":true})).await;
    assert!(output.is_error);
    assert!(text(&output).contains("private or local"));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
    for url in [
        "file:///etc/passwd",
        "http://user:pass@example.com",
        "http://@example.com",
    ] {
        assert!(invoke(true, json!({"url":url})).await.is_error);
    }
    for arguments in [Value::Null, json!({"url":3}), json!({"url":""}), json!([])] {
        assert!(invoke(false, arguments).await.is_error);
    }
}
#[tokio::test]
async fn html_pagination_status_and_request_headers() {
    let (url, task) = server(
        "200 OK",
        "Content-Type: text/html\r\n",
        b"<h1>Title</h1><script>hidden</script><p>Hello world</p>",
    )
    .await;
    let result = invoke(true, json!({"url":url,"max_length":7})).await;
    assert!(!result.is_error, "{result:?}");
    assert_eq!(
        text(&result),
        "# Title\n\n--- Content truncated. Use start_index=7 to continue reading. ---"
    );
    let request = task.await.unwrap().to_lowercase();
    assert!(request.contains("user-agent: gratefulagents-bot/1.0"));
    assert!(!request.contains("accept-encoding:"));
    let (url, task) = server("404 Not Found", "", b"error").await;
    let result = invoke(true, json!({"url":url})).await;
    assert!(result.is_error);
    assert_eq!(text(&result), "HTTP 404: 404 Not Found");
    task.await.unwrap();
    let (url, task) = server("200 OK", "", "€abcdef".as_bytes()).await;
    let result = invoke(true, json!({"url":url,"start_index":1,"max_length":1})).await;
    assert_eq!(
        text(&result),
        "�\n\n--- Content truncated. Use start_index=2 to continue reading. ---"
    );
    task.await.unwrap();
}
#[tokio::test]
async fn redirects_are_revalidated_and_compression_is_not_decoded() {
    let (destination, end) = server("200 OK", "", b"redirected").await;
    let (url, start) = server("302 Found", &format!("Location: {destination}\r\n"), b"").await;
    let url = format!("{url}/#fragment");
    assert_eq!(text(&invoke(true, json!({"url":url})).await), "redirected");
    start.await.unwrap();
    assert!(end.await.unwrap().contains(&format!("referer: {url}\r\n")));
    for location in ["file:///etc/passwd", "http://user:pass@127.0.0.1/"] {
        let (url, task) = server("302 Found", &format!("Location: {location}\r\n"), b"").await;
        assert!(invoke(true, json!({"url":url})).await.is_error);
        task.await.unwrap();
    }
    let (url, task) = server("200 OK", "Content-Encoding: gzip\r\n", b"raw-wire-not-gzip").await;
    assert_eq!(
        text(&invoke(true, json!({"url":url})).await),
        "raw-wire-not-gzip"
    );
    task.await.unwrap();
    let (url, task) = server("200 OK", "", &vec![b'x'; 2 * 1024 * 1024 + 1024]).await;
    let result = invoke(true, json!({"url":url,"max_length":3000000})).await;
    assert!(!result.is_error);
    assert_eq!(text(&result).len(), 2 * 1024 * 1024);
    task.await.unwrap();
}
#[tokio::test]
async fn cancellation_and_deadline_close_pending_http_connections() {
    for cancelled in [true, false] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (ready, received) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            headers(&mut socket).await;
            let _ = ready.send(());
            let mut bytes = [0; 1];
            socket.read(&mut bytes).await.unwrap_or(0)
        });
        let token = Arc::new(adk_runtime::CancellationToken::new());
        let mut ctx = context();
        ctx.operation.cancellation = token.clone();
        if !cancelled {
            ctx.operation.deadline = Some(Instant::now() + Duration::from_millis(300));
        }
        let operation = tokio::spawn(async move {
            tool(true)
                .execute(
                    &ctx,
                    ToolCall {
                        id: "call".into(),
                        name: "WebFetch".into(),
                        arguments: json!({"url":url}),
                    },
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), received)
            .await
            .unwrap()
            .unwrap();
        if cancelled {
            token.cancel();
        }
        let error = operation.await.unwrap().unwrap_err();
        assert_eq!(
            error.info.category,
            if cancelled {
                ErrorCategory::Cancelled
            } else {
                ErrorCategory::DeadlineExceeded
            }
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), server)
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }
}
