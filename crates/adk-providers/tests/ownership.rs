use adk_core::{BoxFuture, Context, Error, ErrorCategory, Model, ModelRequest, StreamingModel};
use adk_providers::{auth::*, client::Provider, wire::Protocol};
use adk_runtime::CancellationToken;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Arc,
    time::Duration,
};
struct Credentials;
impl CredentialStore for Credentials {
    fn load<'a>(&'a self, _: &'a Context, _: &'a Scope) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async {
            Ok(Material {
                access_token: Secret::new("fixture"),
                refresh_token: None,
                account: None,
                expires_at: None,
                last_refresh: None,
                revision: 0,
            })
        })
    }
    fn replace<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        _: u64,
        _: Material,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async { panic!("unexpected refresh") })
    }
}
impl Refresh for Credentials {
    fn refresh<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        _: Material,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async { panic!("unexpected refresh") })
    }
}
fn request() -> ModelRequest {
    ModelRequest {
        model: "test".into(),
        instructions: String::new(),
        input: vec![],
        tools: vec![],
        output_schema: None,
        output_schema_name: String::new(),
        output_schema_strict: false,
        settings: Default::default(),
    }
}
fn read_request(socket: &mut TcpStream) {
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0];
        socket.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let headers = String::from_utf8(bytes).unwrap().to_lowercase();
    let length: usize = headers
        .lines()
        .find_map(|l| l.strip_prefix("content-length: "))
        .unwrap()
        .parse()
        .unwrap();
    socket.read_exact(&mut vec![0; length]).unwrap();
}
fn setup() -> (TcpListener, Provider, Context, Arc<CancellationToken>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let scope = Scope::new(
        "test",
        &format!("http://{}", listener.local_addr().unwrap()),
        None,
        AuthMode::ApiKey,
    )
    .unwrap();
    let session =
        Arc::new(Session::new(scope, Arc::new(Credentials), Arc::new(Credentials)).unwrap());
    let provider = Provider::new("test", Protocol::Chat, session).unwrap();
    let cancel = Arc::new(CancellationToken::new());
    let context = Context {
        run_id: "test".into(),
        cancellation: cancel.clone(),
        deadline: Some(std::time::Instant::now() + Duration::from_secs(5)),
    };
    (listener, provider, context, cancel)
}
#[tokio::test]
async fn cancellation_closes_an_inflight_http_request() {
    let (listener, provider, context, cancel) = setup();
    let (sent, received) = tokio::sync::oneshot::channel();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        read_request(&mut socket);
        sent.send(()).unwrap();
        let mut byte = [0];
        assert_eq!(socket.read(&mut byte).unwrap(), 0);
    });
    let cancellation = async {
        received.await.unwrap();
        cancel.cancel();
    };
    let (result, ()) = tokio::join!(provider.complete(&context, request()), cancellation);
    assert_eq!(result.unwrap_err().info.category, ErrorCategory::Cancelled);
    // Allow the owned HTTP connection driver to observe body/future drop.
    tokio::time::sleep(Duration::from_millis(10)).await;
    server.join().unwrap();
}
#[tokio::test]
async fn consumer_drop_closes_http_stream_without_detached_reader() {
    let (listener, provider, context, _) = setup();
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        read_request(&mut socket);
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap();
        let mut byte = [0];
        assert_eq!(socket.read(&mut byte).unwrap(), 0);
    });
    let stream = provider.stream(&context, request()).await.unwrap();
    drop(stream);
    tokio::time::sleep(Duration::from_millis(10)).await;
    server.join().unwrap();
}
