use adk_mcp::{
    BoxFuture, Error, Limits, Transport,
    transport::{
        HeaderProvider, HttpTransport, OAuthPolicy, OAuthToken, OAuthTokenProvider, RemoteOptions,
        StdioTransport,
    },
};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};
use url::Url;

fn limits() -> Limits {
    Limits {
        timeout: Duration::from_secs(2),
        max_message_bytes: 4096,
        ..Limits::default()
    }
}
fn options() -> RemoteOptions {
    RemoteOptions {
        tenant_id: "tenant-one".into(),
        allow_private_network: true,
        ..RemoteOptions::default()
    }
}
fn uncertain(error: Error, method: &str) {
    assert_eq!(
        error,
        Error::ReconciliationRequired {
            server: "mock".into(),
            operation: method.into()
        }
    );
}

async fn python(script: &str, env: BTreeMap<String, String>, limits: Limits) -> StdioTransport {
    StdioTransport::connect(
        "mock",
        "/usr/bin/python3",
        &["-u".into(), "-c".into(), script.into()],
        &env,
        Path::new("."),
        limits,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn stdio_scoped_env_notifications_stderr_and_reaped_shutdown() {
    let script = r#"
import sys, os, json
sys.stderr.write('\x1b[31msecret\x00' * 20000)
sys.stderr.flush()
for line in sys.stdin:
 r=json.loads(line)
 if 'id' not in r: continue
 print(json.dumps({'jsonrpc':'2.0','method':'notifications/progress','params':{}}))
 print(json.dumps({'jsonrpc':'2.0','id':r['id'],'result':{'scope':os.getenv('SCOPED'), 'path':os.getenv('PATH'), 'home':os.getenv('HOME'), 'pid':os.getpid()}}))
"#;
    let mut transport = python(
        script,
        BTreeMap::from([("SCOPED".into(), "yes".into())]),
        Limits {
            max_stderr_bytes: 16,
            ..limits()
        },
    )
    .await;
    transport
        .notify("notifications/initialized", json!({}))
        .await
        .unwrap();
    let result = transport.request("tools/list", json!({})).await.unwrap();
    assert_eq!(result["scope"], "yes");
    assert_eq!(result["path"], Value::Null);
    assert_eq!(result["home"], Value::Null);
    transport.close().await.unwrap();
    transport.close().await.unwrap();
    assert_eq!(
        transport.request("tools/list", json!({})).await,
        Err(Error::Closed)
    );
    #[cfg(unix)]
    {
        let pid = rustix::process::Pid::from_raw(result["pid"].as_i64().unwrap() as i32).unwrap();
        assert!(matches!(
            rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG),
            Err(rustix::io::Errno::CHILD)
        ));
    }
}

#[tokio::test]
async fn stdio_bounds_eof_timeout_and_cancellation_never_replay() {
    for script in [
        "import sys; sys.stdin.readline(); print('x'*5000)",
        "import sys; sys.stdin.readline(); sys.exit(0)",
        "import sys,time; sys.stdin.readline(); time.sleep(30)",
    ] {
        let mut transport = python(
            script,
            BTreeMap::new(),
            Limits {
                timeout: Duration::from_millis(200),
                ..limits()
            },
        )
        .await;
        uncertain(
            transport
                .request("tools/call", json!({}))
                .await
                .unwrap_err(),
            "tools/call",
        );
        assert_eq!(
            transport.request("tools/call", json!({})).await,
            Err(Error::Closed)
        );
        transport.close().await.unwrap();
    }
    let mut transport = python(
        "import sys,time; sys.stdin.readline(); time.sleep(30)",
        BTreeMap::new(),
        limits(),
    )
    .await;
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            transport.request("tools/call", json!({}))
        )
        .await
        .is_err()
    );
    assert_eq!(
        transport.request("tools/call", json!({})).await,
        Err(Error::Closed)
    );
    transport.close().await.unwrap();
}

#[tokio::test]
async fn stdio_preflight_bound_is_not_dispatched_and_remote_error_is_definitive() {
    let mut transport = python("import sys,json\nfor l in sys.stdin:\n r=json.loads(l); print(json.dumps({'jsonrpc':'2.0','id':r['id'],'error':{'code':-32601,'message':'secret'}}))", BTreeMap::new(), limits()).await;
    assert_eq!(
        transport
            .request("tools/call", json!({"large":"x".repeat(5000)}))
            .await,
        Err(Error::Limit)
    );
    for _ in 0..2 {
        assert_eq!(
            transport.request("missing", json!({})).await,
            Err(Error::Remote { code: -32601 })
        );
    }
    transport.close().await.unwrap();
}

async fn request(stream: &mut TcpStream) -> (String, Value) {
    let mut bytes = Vec::new();
    let end;
    loop {
        let byte = stream.read_u8().await.unwrap();
        bytes.push(byte);
        assert!(bytes.len() < 32768);
        if bytes.ends_with(b"\r\n\r\n") {
            end = bytes.len();
            break;
        }
    }
    let headers = String::from_utf8(bytes).unwrap();
    let length = headers
        .lines()
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|s| s.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await.unwrap();
    assert_eq!(end, headers.len());
    (
        headers,
        if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        },
    )
}

async fn respond(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
    extra: &str,
) {
    stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n{extra}\r\n{body}", body.len()).as_bytes()).await.unwrap();
}

async fn one_response(
    status: &'static str,
    content_type: &'static str,
    body: String,
    extra: String,
) -> (String, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let (headers, _) = request(&mut stream).await;
        respond(&mut stream, status, content_type, &body, &extra).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "unexpected replay"
        );
        headers
    });
    (url, task)
}

#[tokio::test]
async fn http_json_and_sse_replies() {
    for (kind, body) in [
        (
            "application/json; charset=utf-8",
            r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#,
        ),
        (
            "text/event-stream",
            ":heartbeat\r\nevent: message\r\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\r\n\r\ndata: {\"jsonrpc\":\"2.0\",\r\ndata: \"id\":1,\"result\":{\"ok\":true}}\r\n\r\n",
        ),
    ] {
        let (url, server) = one_response("200 OK", kind, body.into(), String::new()).await;
        let mut transport = HttpTransport::connect("mock", &url, false, options(), limits())
            .await
            .unwrap();
        assert_eq!(
            transport.request("tools/list", json!({})).await.unwrap(),
            json!({"ok":true})
        );
        transport.close().await.unwrap();
        let headers = server.await.unwrap().to_lowercase();
        assert!(headers.starts_with("post /mcp "));
        assert!(headers.contains("mcp-protocol-version: 2025-03-26"));
    }
}

#[tokio::test]
async fn http_ambiguous_status_disconnect_oversize_wrong_id_no_replay() {
    for (status, body) in [
        ("500 Internal Server Error", "secret".into()),
        ("200 OK", "x".repeat(5000)),
        ("200 OK", r#"{"jsonrpc":"2.0","id":2,"result":{}}"#.into()),
        ("200 OK", "not-json".into()),
    ] {
        let (url, server) = one_response(status, "application/json", body, String::new()).await;
        let mut transport = HttpTransport::connect("mock", &url, false, options(), limits())
            .await
            .unwrap();
        uncertain(
            transport
                .request("tools/call", json!({}))
                .await
                .unwrap_err(),
            "tools/call",
        );
        assert_eq!(
            transport.request("tools/call", json!({})).await,
            Err(Error::Closed)
        );
        server.await.unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        request(&mut stream).await;
        drop(stream);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let mut transport = HttpTransport::connect("mock", &url, false, options(), limits())
        .await
        .unwrap();
    uncertain(
        transport
            .request("tools/call", json!({}))
            .await
            .unwrap_err(),
        "tools/call",
    );
    server.await.unwrap();
}

#[tokio::test]
async fn http_timeout_and_cancel_poison_session() {
    for cancel in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            request(&mut stream).await;
            tokio::time::sleep(Duration::from_millis(300)).await;
        });
        let mut transport = HttpTransport::connect(
            "mock",
            &url,
            false,
            options(),
            Limits {
                timeout: Duration::from_millis(200),
                ..limits()
            },
        )
        .await
        .unwrap();
        if cancel {
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(100),
                    transport.request("tools/call", json!({}))
                )
                .await
                .is_err()
            );
        } else {
            uncertain(
                transport
                    .request("tools/call", json!({}))
                    .await
                    .unwrap_err(),
                "tools/call",
            );
        }
        assert_eq!(
            transport.request("tools/call", json!({})).await,
            Err(Error::Closed)
        );
        transport.close().await.unwrap();
        server.await.unwrap();
    }
}

#[tokio::test]
async fn http_ssrf_url_policy_and_redirects() {
    for url in [
        "http://example.com/mcp",
        "https://user:password@example.com/mcp",
        "https://example.com/mcp?secret=x",
        "https://example.com/mcp#secret",
        "https://127.0.0.1/mcp",
        "https://localhost/mcp",
        "https://[::1]/mcp",
        "https://[::ffff:127.0.0.1]/mcp",
        "https://169.254.169.254/mcp",
        "https://100.64.0.1/mcp",
        "https://192.0.2.1/mcp",
    ] {
        let result = HttpTransport::connect(
            "mock",
            url,
            false,
            RemoteOptions {
                allow_private_network: false,
                ..options()
            },
            limits(),
        )
        .await;
        assert!(
            matches!(result, Err(Error::Policy(_))),
            "unexpected acceptance of {url}"
        );
    }
    let target = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (url, server) = one_response(
        "307 Temporary Redirect",
        "text/plain",
        String::new(),
        format!(
            "Location: http://{}/stolen\r\n",
            target.local_addr().unwrap()
        ),
    )
    .await;
    let mut transport = HttpTransport::connect("mock", &url, false, options(), limits())
        .await
        .unwrap();
    uncertain(
        transport
            .request("tools/call", json!({}))
            .await
            .unwrap_err(),
        "tools/call",
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), target.accept())
            .await
            .is_err()
    );
    server.await.unwrap();
}

#[tokio::test]
async fn sse_stream_byte_and_event_limits_are_enforced() {
    for body in [
        format!(":{}\n\n", "x".repeat(5000)),
        "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n".repeat(3),
    ] {
        let (url, server) = one_response("200 OK", "text/event-stream", body, String::new()).await;
        let mut transport = HttpTransport::connect(
            "mock",
            &url,
            false,
            options(),
            Limits {
                max_items: 2,
                ..limits()
            },
        )
        .await
        .unwrap();
        uncertain(
            transport
                .request("tools/list", json!({}))
                .await
                .unwrap_err(),
            "tools/list",
        );
        assert_eq!(
            transport.request("tools/list", json!({})).await,
            Err(Error::Closed)
        );
        server.await.unwrap();
    }
}

struct Tokens {
    calls: AtomicUsize,
    mode: AtomicUsize,
    seen: Mutex<Vec<(String, String)>>,
}
impl OAuthTokenProvider for Tokens {
    fn token<'a>(
        &'a self,
        tenant: &'a str,
        server: &'a str,
    ) -> BoxFuture<'a, Result<OAuthToken, Error>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen
                .lock()
                .unwrap()
                .push((tenant.into(), server.into()));
            let mode = self.mode.load(Ordering::SeqCst);
            Ok(OAuthToken {
                access_token: "secret-token".into(),
                audience: if mode == 1 { "wrong" } else { "mcp-audience" }.into(),
                scopes: if mode == 2 {
                    vec![]
                } else {
                    vec!["tools".into()]
                },
                expiry: if mode == 3 {
                    SystemTime::UNIX_EPOCH
                } else {
                    SystemTime::now() + Duration::from_secs(60)
                },
            })
        })
    }
}
fn token_options(provider: Arc<Tokens>) -> RemoteOptions {
    RemoteOptions {
        oauth: Some(OAuthPolicy {
            provider,
            audience: "mcp-audience".into(),
            required_scopes: vec!["tools".into()],
        }),
        ..options()
    }
}

#[tokio::test]
async fn oauth_claims_checked_for_every_request_and_tenant_server() {
    for mode in [1, 2, 3] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let provider = Arc::new(Tokens {
            calls: AtomicUsize::new(0),
            mode: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        });
        let mut transport = HttpTransport::connect(
            "mock",
            &url,
            false,
            token_options(provider.clone()),
            limits(),
        )
        .await
        .unwrap();
        provider.mode.store(mode, Ordering::SeqCst);
        assert!(matches!(
            transport.request("tools/list", json!({})).await,
            Err(Error::Policy(_))
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
        assert!(
            provider
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|pair| pair == &("tenant-one".into(), "mock".into()))
        );
    }
}

#[tokio::test]
async fn oauth_credential_reflection_raw_and_json_escaped_rejected() {
    for body in [
        r#"{"jsonrpc":"2.0","id":1,"result":{"name":"secret-token"}}"#,
        r#"{"jsonrpc":"2.0","id":1,"result":{"name":"\u0073ecret-token"}}"#,
    ] {
        let (url, server) =
            one_response("200 OK", "application/json", body.into(), String::new()).await;
        let provider = Arc::new(Tokens {
            calls: AtomicUsize::new(0),
            mode: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        });
        let mut transport =
            HttpTransport::connect("mock", &url, false, token_options(provider), limits())
                .await
                .unwrap();
        uncertain(
            transport
                .request("tools/list", json!({}))
                .await
                .unwrap_err(),
            "tools/list",
        );
        assert!(server.await.unwrap().contains("Bearer secret-token"));
    }
}

struct Headers {
    name: &'static str,
}
impl HeaderProvider for Headers {
    fn headers<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        endpoint: &'a Url,
    ) -> BoxFuture<'a, Result<HeaderMap, Error>> {
        Box::pin(async move {
            assert!(endpoint.query().is_none());
            let mut headers = HeaderMap::new();
            headers.insert(self.name, HeaderValue::from_static("host-secret"));
            Ok(headers)
        })
    }
}

#[tokio::test]
async fn reserved_headers_fail_closed() {
    for name in [
        "host",
        "cookie",
        "x-forwarded-for",
        "idempotency-key",
        "mcp-session-id",
        "content-length",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let opts = RemoteOptions {
            headers: Some(Arc::new(Headers { name })),
            ..options()
        };
        match HttpTransport::connect("mock", &url, false, opts, limits()).await {
            Err(Error::Policy(_)) => {}
            Ok(mut transport) => assert!(matches!(
                transport.request("tools/list", json!({})).await,
                Err(Error::Policy(_))
            )),
            _ => panic!("expected reserved-header rejection"),
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(25), listener.accept())
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn http_session_initialize_notification_and_delete() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for index in 0..3 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (headers, body) = request(&mut stream).await;
            if index == 0 {
                assert_eq!(body["method"], "initialize");
                respond(
                    &mut stream,
                    "200 OK",
                    "application/json",
                    r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
                    "Mcp-Session-Id: session-secret\r\n",
                )
                .await;
            } else {
                assert!(
                    headers
                        .to_lowercase()
                        .contains("mcp-session-id: session-secret")
                );
                if index == 1 {
                    assert_eq!(body["method"], "notifications/initialized");
                    assert!(body.get("id").is_none());
                } else {
                    assert!(headers.starts_with("DELETE "));
                }
                respond(&mut stream, "202 Accepted", "application/json", "", "").await;
            }
        }
    });
    let mut transport = HttpTransport::connect("mock", &url, false, options(), limits())
        .await
        .unwrap();
    transport.request("initialize", json!({})).await.unwrap();
    transport
        .notify("notifications/initialized", json!({}))
        .await
        .unwrap();
    transport.close().await.unwrap();
    transport.close().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn legacy_sse_endpoint_query_posts_and_correlates() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/sse", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut sse, _) = listener.accept().await.unwrap();
        let (headers, _) = request(&mut sse).await;
        assert!(headers.starts_with("GET /sse "));
        sse.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\nevent: endpoint\ndata: /messages?sessionId=opaque\n\n").await.unwrap();
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (headers, body) = request(&mut stream).await;
            assert!(headers.starts_with("POST /messages?sessionId=opaque "));
            assert!(headers.contains("host-secret"));
            respond(&mut stream, "202 Accepted", "application/json", "", "").await;
            sse.write_all(
                format!(
                    "event: message\ndata: {}\n\n",
                    json!({"jsonrpc":"2.0","id":body["id"],"result":{"ok":true}})
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        }
    });
    let mut transport = HttpTransport::connect(
        "mock",
        &url,
        true,
        RemoteOptions {
            headers: Some(Arc::new(Headers { name: "x-api-key" })),
            ..options()
        },
        limits(),
    )
    .await
    .unwrap();
    for _ in 0..2 {
        assert_eq!(
            transport.request("tools/list", json!({})).await.unwrap(),
            json!({"ok":true})
        );
    }
    transport.close().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn legacy_sse_cross_origin_endpoint_rejected() {
    let (url, server) = one_response(
        "200 OK",
        "text/event-stream",
        "event: endpoint\ndata: http://localhost:9/messages?sessionId=secret\n\n".into(),
        String::new(),
    )
    .await;
    assert!(matches!(
        HttpTransport::connect("mock", &url, true, options(), limits()).await,
        Err(Error::Policy(_))
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn cancellation_kills_stdio_group_and_reaps_without_dropping_transport() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let script = format!(
        r#"
import sys, json, socket, subprocess, os, time
s=socket.create_connection(('127.0.0.1', {port}))
s.set_inheritable(True)
subprocess.Popen(['/bin/sleep','30'], close_fds=False)
r=json.loads(sys.stdin.readline())
print(json.dumps({{'jsonrpc':'2.0','id':r['id'],'result':os.getpid()}}))
sys.stdin.readline()
time.sleep(30)
"#
    );
    let mut transport = python(&script, BTreeMap::new(), limits()).await;
    let pid = transport.request("initialize", json!({})).await.unwrap();
    let (mut socket, _) = listener.accept().await.unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            transport.request("tools/call", json!({}))
        )
        .await
        .is_err()
    );
    let mut byte = [0];
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert_eq!(
        transport.request("tools/call", json!({})).await,
        Err(Error::Closed)
    );
    #[cfg(unix)]
    {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let pid = rustix::process::Pid::from_raw(pid.as_i64().unwrap() as i32).unwrap();
        assert!(matches!(
            rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::NOHANG),
            Err(rustix::io::Errno::CHILD)
        ));
    }
    transport.close().await.unwrap();
}

#[tokio::test]
async fn cancellation_releases_legacy_sse_without_dropping_transport() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/sse", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut sse, _) = listener.accept().await.unwrap();
        request(&mut sse).await;
        sse.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\nevent: endpoint\ndata: /messages?sessionId=opaque\n\n").await.unwrap();
        let (mut stream, _) = listener.accept().await.unwrap();
        request(&mut stream).await;
        respond(&mut stream, "202 Accepted", "application/json", "", "").await;
        let mut byte = [0];
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), sse.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    });
    let mut transport = HttpTransport::connect("mock", &url, true, options(), limits())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            transport.request("tools/call", json!({}))
        )
        .await
        .is_err()
    );
    assert_eq!(
        transport.request("tools/call", json!({})).await,
        Err(Error::Closed)
    );
    server.await.unwrap();
    transport.close().await.unwrap();
}
