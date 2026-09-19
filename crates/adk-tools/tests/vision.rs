#![cfg(any(target_os = "linux", target_os = "macos"))]

use adk_tools::{capabilities, vision};

use adk_core::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

fn context(root: &Path) -> ToolContext {
    ToolContext {
        operation: Context {
            run_id: "vision".into(),
            cancellation: Arc::new(adk_runtime::CancellationToken::new()),
            deadline: None,
        },
        work_dir: root.into(),
        policy: Default::default(),
        idempotency_key: None,
    }
}
fn call(arguments: Value) -> ToolCall {
    ToolCall {
        id: "call".into(),
        name: "AnalyzeImage".into(),
        arguments,
    }
}
async fn invoke(config: vision::Config, root: &Path, arguments: Value) -> ToolOutput {
    vision::tool(config)
        .execute(&context(root), call(arguments))
        .await
        .unwrap()
}
fn text(output: &ToolOutput) -> &str {
    match output.content.first().unwrap() {
        Content::Text { text } => text,
        _ => panic!("text"),
    }
}
fn image(output: &ToolOutput) -> (Vec<u8>, &str, &str) {
    assert!(!output.is_error, "{output:?}");
    assert!(!output.should_pause);
    assert_eq!(output.content.len(), 2);
    match &output.content[1] {
        Content::Attachment {
            data,
            media_type,
            detail,
        } => (STANDARD.decode(data).unwrap(), media_type, detail),
        _ => panic!("native attachment"),
    }
}
struct UnusedAnalyzer;
impl vision::Analyzer for UnusedAnalyzer {
    fn analyze<'a>(
        &'a self,
        _: &'a Context,
        _: &'a [u8],
        _: &'a str,
        _: &'a str,
        _: &'a str,
    ) -> BoxFuture<'a, Result<String, Error>> {
        panic!("pinned SDK never invokes a separate analyzer")
    }
}

#[tokio::test]
async fn pinned_fixture_results_with_and_without_analyzer() {
    let fixtures: Vec<Value> =
        serde_json::from_str(include_str!("../../../fixtures/tools/vision.json")).unwrap();
    for configured in [false, true] {
        for fixture in &fixtures {
            let root = tempfile::tempdir().unwrap();
            let data: Vec<u8> = fixture["data"]
                .as_array()
                .map(|bytes| bytes.iter().map(|n| n.as_u64().unwrap() as u8).collect())
                .unwrap_or_default();
            if let Some(path) = fixture["path"].as_str() {
                std::fs::write(root.path().join(path), &data).unwrap();
            }
            let config = vision::Config {
                analyzer: configured.then(|| Arc::new(UnusedAnalyzer) as Arc<dyn vision::Analyzer>),
                ..Default::default()
            };
            let output = invoke(config, root.path(), fixture["arguments"].clone()).await;
            if let Some(error) = fixture["error"].as_str() {
                assert!(output.is_error, "{fixture}");
                assert_eq!(text(&output), error);
            } else if let Some(prefix) = fixture["error_prefix"].as_str() {
                assert!(output.is_error, "{fixture}");
                assert!(text(&output).starts_with(prefix));
            } else {
                assert_eq!(
                    text(&output),
                    fixture["arguments"]["prompt"].as_str().unwrap()
                );
                let (bytes, mime, detail) = image(&output);
                assert_eq!(bytes, data, "{fixture}");
                assert_eq!(mime, fixture["mime"].as_str().unwrap(), "{fixture}");
                assert_eq!(detail, fixture["detail"].as_str().unwrap());
            }
        }
    }
}

#[test]
fn registry_contract_and_public_only_vision_are_not_browser_gated() {
    let capability = capabilities()
        .iter()
        .find(|c| c.name == "AnalyzeImage")
        .unwrap();
    let tool = vision::tool(Default::default());
    assert_eq!(Some(tool.definition()), capability.definition.as_ref());
    assert!(!tool.is_control_flow());
    assert!(tool.timeout().is_none());
    let registry = adk_tools::Registry::build(
        &adk_tools::Config {
            features: adk_tools::Features::Strict(["Vision".into(), "Browser".into()].into()),
            access: AccessMode::ReadOnly,
            ..Default::default()
        },
        [tool],
    )
    .unwrap();
    assert_eq!(registry.names().collect::<Vec<_>>(), vec!["AnalyzeImage"]);
}

#[tokio::test]
async fn filesystem_confinement_and_managed_absolute_paths() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let managed = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    for dir in [root.path(), managed.path(), outside.path()] {
        std::fs::write(dir.join("image.png"), b"png").unwrap();
    }
    let config = vision::Config {
        allowed_image_dirs: vec!["  ".into(), managed.path().into()],
        ..Default::default()
    };
    for path in [
        root.path().join("image.png"),
        managed.path().join("image.png"),
    ] {
        assert_eq!(
            image(
                &invoke(
                    config.clone(),
                    root.path(),
                    json!({"image_path":path,"prompt":"inspect"})
                )
                .await
            )
            .0,
            b"png"
        );
    }
    std::fs::write(managed.path().join("only-managed.png"), b"x").unwrap();
    symlink(outside.path(), root.path().join("escape")).unwrap();
    symlink(
        root.path().join("image.png"),
        root.path().join("inside-link.png"),
    )
    .unwrap();
    symlink(
        outside.path().join("image.png"),
        managed.path().join("link.png"),
    )
    .unwrap();
    std::fs::hard_link(
        outside.path().join("image.png"),
        root.path().join("hard.png"),
    )
    .unwrap();
    for path in [
        outside.path().join("image.png"),
        Path::new("../image.png").into(),
        Path::new("only-managed.png").into(),
        Path::new("escape/image.png").into(),
        Path::new("inside-link.png").into(),
        managed.path().join("link.png"),
        Path::new("hard.png").into(),
        Path::new(".").into(),
        Path::new("missing.png").into(),
    ] {
        let output = invoke(
            config.clone(),
            root.path(),
            json!({"image_path":path,"prompt":"inspect","allowed_image_dirs":[outside.path()]}),
        )
        .await;
        assert!(output.is_error, "{path:?}: {output:?}");
        assert!(text(&output).starts_with("Failed to load image:"));
    }
    let fifo = root.path().join("fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let output = invoke(
        config,
        root.path(),
        json!({"image_path":"fifo","prompt":"inspect"}),
    )
    .await;
    assert!(output.is_error);
    assert!(text(&output).contains("not a regular file"));
}

#[tokio::test]
async fn exact_file_byte_limit_and_oversize_preflight() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("large.png");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(20 * 1024 * 1024).unwrap();
    let output = invoke(
        Default::default(),
        root.path(),
        json!({"image_path":"large.png","prompt":"inspect"}),
    )
    .await;
    assert_eq!(image(&output).0.len(), 20 * 1024 * 1024);
    file.set_len(20 * 1024 * 1024 + 1).unwrap();
    let output = invoke(
        Default::default(),
        root.path(),
        json!({"image_path":"large.png","prompt":"inspect"}),
    )
    .await;
    assert!(output.is_error);
    assert_eq!(
        text(&output),
        "Failed to load image: image too large (20971521 bytes, max 20971520)"
    );
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
async fn url_invoke(url: &str) -> ToolOutput {
    invoke(
        vision::Config {
            allow_private_network_urls: true,
            ..Default::default()
        },
        Path::new(""),
        json!({"url":url,"prompt":"inspect"}),
    )
    .await
}

#[tokio::test]
async fn url_headers_mime_precedence_redirects_and_http_errors() {
    for (extra, body, expected) in [
        ("", b"\x89PNG".as_slice(), "image/png"),
        (
            "Content-Type: text/plain; charset=UTF-8\r\n",
            b"\x89PNG".as_slice(),
            "text/plain; charset=UTF-8",
        ),
        (
            "Content-Encoding: gzip\r\n",
            b"raw-wire".as_slice(),
            "application/octet-stream",
        ),
    ] {
        let (url, task) = server("200 OK", extra, body).await;
        let output = url_invoke(&format!("{url}/image.jpg")).await;
        let (data, mime, detail) = image(&output);
        assert_eq!(data, body);
        assert_eq!(mime, expected);
        assert_eq!(detail, "high");
        let request = task.await.unwrap().to_lowercase();
        assert!(request.contains("user-agent: gratefulagents-bot/1.0"));
        assert!(!request.contains("accept-encoding:"));
    }
    for status in [
        "400 Bad Request",
        "404 Not Found",
        "500 Internal Server Error",
    ] {
        let (url, task) = server(status, "", b"error").await;
        let output = url_invoke(&url).await;
        assert!(output.is_error);
        assert_eq!(
            text(&output),
            format!("Failed to load image: HTTP {} fetching image", &status[..3])
        );
        task.await.unwrap();
    }
    for status in [
        "301 Moved Permanently",
        "302 Found",
        "303 See Other",
        "307 Temporary Redirect",
        "308 Permanent Redirect",
    ] {
        let (destination, end) = server("200 OK", "", b"GIF8").await;
        let (url, start) = server(status, &format!("Location: {destination}\r\n"), b"").await;
        assert_eq!(image(&url_invoke(&url).await).0, b"GIF8");
        start.await.unwrap();
        assert!(end.await.unwrap().contains(&format!("referer: {url}/\r\n")));
    }
    let (url, task) = server("302 Found", "", b"body").await;
    assert_eq!(image(&url_invoke(&url).await).0, b"body");
    task.await.unwrap();
}

#[tokio::test]
async fn url_security_is_host_owned_and_revalidated_on_redirects() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    for url in [
        format!("http://{}", listener.local_addr().unwrap()),
        "http://localhost/".into(),
        "http://[::1]/".into(),
        "http://169.254.169.254/".into(),
    ] {
        let output = invoke(
            Default::default(),
            Path::new(""),
            json!({"url":url,"prompt":"inspect","allow_private_network_urls":true}),
        )
        .await;
        assert!(output.is_error);
        assert!(text(&output).contains("private or local"));
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
    for invalid in [
        "file:///etc/passwd",
        "ftp://example.com",
        "http://user:pass@127.0.0.1/",
        "http://@127.0.0.1/",
    ] {
        assert!(url_invoke(invalid).await.is_error);
        let (url, task) = server("302 Found", &format!("Location: {invalid}\r\n"), b"").await;
        let output = url_invoke(&url).await;
        assert!(output.is_error);
        assert!(text(&output).starts_with("Failed to load image:"));
        task.await.unwrap();
    }
}

#[tokio::test]
async fn url_byte_limit_is_enforced_without_relying_on_content_length() {
    let (url, task) = server("200 OK", "", &vec![0; 20 * 1024 * 1024]).await;
    assert_eq!(image(&url_invoke(&url).await).0.len(), 20 * 1024 * 1024);
    task.await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        headers(&mut socket).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1400001\r\n")
            .await
            .unwrap();
        let _ = socket.write_all(&vec![0; 20 * 1024 * 1024 + 1]).await;
        let _ = socket.write_all(b"\r\n0\r\n\r\n").await;
    });
    let output = url_invoke(&url).await;
    assert!(output.is_error);
    assert_eq!(
        text(&output),
        "Failed to load image: image too large (> 20971520 bytes)"
    );
    task.await.unwrap();
}

#[tokio::test]
async fn cancellation_and_deadline_interrupt_headers_and_body() {
    for body_started in [false, true] {
        for cancel in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let (ready, received) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                headers(&mut socket).await;
                if body_started {
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
                        .await
                        .unwrap();
                }
                let _ = ready.send(());
                let mut bytes = [0; 1];
                socket.read(&mut bytes).await.unwrap_or(0)
            });
            let token = Arc::new(adk_runtime::CancellationToken::new());
            let mut ctx = context(Path::new(""));
            ctx.operation.cancellation = token.clone();
            if !cancel {
                ctx.operation.deadline = Some(Instant::now() + Duration::from_millis(300));
            }
            let operation = tokio::spawn(async move {
                vision::tool(vision::Config {
                    allow_private_network_urls: true,
                    ..Default::default()
                })
                .execute(&ctx, call(json!({"url":url,"prompt":"inspect"})))
                .await
            });
            tokio::time::timeout(Duration::from_secs(2), received)
                .await
                .unwrap()
                .unwrap();
            if cancel {
                token.cancel();
            }
            let error = operation.await.unwrap().unwrap_err();
            assert_eq!(
                error.info.category,
                if cancel {
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
    let token = Arc::new(adk_runtime::CancellationToken::new());
    token.cancel();
    let mut ctx = context(Path::new(""));
    ctx.operation.cancellation = token;
    assert_eq!(
        vision::tool(Default::default())
            .execute(&ctx, call(Value::Null))
            .await
            .unwrap_err()
            .info
            .category,
        ErrorCategory::Cancelled
    );
}

#[tokio::test]
async fn url_read_failure_and_fifteen_second_request_timeout() {
    for stall in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            headers(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
                .await
                .unwrap();
            if stall {
                let mut bytes = [0; 1];
                assert_eq!(socket.read(&mut bytes).await.unwrap_or(0), 0);
            }
        });
        let output = tokio::time::timeout(Duration::from_secs(18), url_invoke(&url))
            .await
            .unwrap();
        assert!(output.is_error);
        if stall {
            assert_eq!(
                text(&output),
                "Failed to load image: fetching image: request timed out"
            );
        } else {
            assert!(text(&output).starts_with("Failed to load image: reading response:"));
        }
        task.await.unwrap();
    }
}
