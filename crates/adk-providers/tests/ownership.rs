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
                id_token: None,
                email: None,
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
        input_provenance: Vec::new(),
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

struct DropNotice(Arc<std::sync::atomic::AtomicUsize>);
impl Drop for DropNotice {
    fn drop(&mut self) {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}
struct BlockingStore {
    block_replace: bool,
    entered: tokio::sync::Notify,
    dropped: Arc<std::sync::atomic::AtomicUsize>,
}
impl CredentialStore for BlockingStore {
    fn load<'a>(
        &'a self,
        ctx: &'a Context,
        scope: &'a Scope,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async move {
            if !self.block_replace {
                let _notice = DropNotice(self.dropped.clone());
                self.entered.notify_one();
                std::future::pending::<()>().await;
            }
            let mut value = Credentials.load(ctx, scope).await?;
            value.access_token = Secret::new("");
            value.refresh_token = Some(Secret::new("fixture-refresh"));
            Ok(value)
        })
    }
    fn replace<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        _: u64,
        _: Material,
    ) -> BoxFuture<'a, Result<bool, Error>> {
        Box::pin(async move {
            assert!(self.block_replace);
            let _notice = DropNotice(self.dropped.clone());
            self.entered.notify_one();
            std::future::pending().await
        })
    }
}
impl Refresh for BlockingStore {
    fn refresh<'a>(
        &'a self,
        _: &'a Context,
        _: &'a Scope,
        mut value: Material,
    ) -> BoxFuture<'a, Result<Material, Error>> {
        Box::pin(async move {
            value.access_token = Secret::new("refreshed-fixture");
            Ok(value)
        })
    }
}
#[tokio::test]
async fn dropping_auth_load_or_cas_drops_host_future_and_releases_scope_lock() {
    for block_replace in [false, true] {
        let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let store = Arc::new(BlockingStore {
            block_replace,
            entered: tokio::sync::Notify::new(),
            dropped: dropped.clone(),
        });
        let scope = Scope::new(
            "owned-auth",
            "https://fixture.example.test",
            None,
            AuthMode::AnthropicOAuth,
        )
        .unwrap();
        let session = Session::new(scope, store.clone(), store.clone()).unwrap();
        let ctx = Context {
            run_id: "auth-owner".into(),
            cancellation: Arc::new(CancellationToken::new()),
            deadline: None,
        };
        for expected_drops in 1..=2 {
            let mut operation = Box::pin(session.material(&ctx));
            tokio::select! {
                _ = store.entered.notified() => {},
                _ = &mut operation => panic!("auth operation must remain blocked"),
            }
            drop(operation);
            assert_eq!(
                dropped.load(std::sync::atomic::Ordering::SeqCst),
                expected_drops
            );
        }
    }
}

#[tokio::test]
async fn cancellation_interrupts_pending_host_cas_without_detaching_it() {
    let dropped = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let store = Arc::new(BlockingStore {
        block_replace: true,
        entered: tokio::sync::Notify::new(),
        dropped: dropped.clone(),
    });
    let scope = Scope::new(
        "owned-auth",
        "https://fixture.example.test",
        None,
        AuthMode::AnthropicOAuth,
    )
    .unwrap();
    let session = Session::new(scope, store.clone(), store.clone()).unwrap();
    let cancel = Arc::new(CancellationToken::new());
    let ctx = Context {
        run_id: "auth-owner".into(),
        cancellation: cancel.clone(),
        deadline: None,
    };
    let cancellation = async {
        store.entered.notified().await;
        cancel.cancel();
    };
    let (result, ()) = tokio::join!(session.material(&ctx), cancellation);
    assert_eq!(result.unwrap_err().info.category, ErrorCategory::Cancelled);
    assert_eq!(dropped.load(std::sync::atomic::Ordering::SeqCst), 1);
}
