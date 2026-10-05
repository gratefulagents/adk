use adk_mcp::{
    BoxFuture, Error, PROTOCOL_VERSION, Transport,
    client::{Client, ClientManager, HostPolicy, ServerPolicy},
    session::OwnedMcpSession,
    tools::ToolManager,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Notify, Semaphore};

struct Control {
    calls: AtomicUsize,
    closes: AtomicUsize,
    dispatched: Notify,
    closing: Notify,
    release: Semaphore,
    fail_close: bool,
}
impl Default for Control {
    fn default() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            closes: AtomicUsize::new(0),
            dispatched: Notify::new(),
            closing: Notify::new(),
            release: Semaphore::new(0),
            fail_close: false,
        }
    }
}
struct Peer(Arc<Control>);
impl Transport for Peer {
    fn request<'a>(&'a mut self, method: &'a str, _: Value) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async move {
            match method {
                "initialize" => Ok(
                    json!({"protocolVersion":PROTOCOL_VERSION,"capabilities":{"tools":{},"resources":{}}}),
                ),
                "tools/list" => Ok(
                    json!({"tools":[{"name":"read","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}]}),
                ),
                "tools/call" => {
                    self.0.calls.fetch_add(1, Ordering::SeqCst);
                    self.0.dispatched.notify_one();
                    std::future::pending().await
                }
                _ => panic!("unexpected request {method}"),
            }
        })
    }
    fn notify<'a>(&'a mut self, _: &'a str, _: Value) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn close(&mut self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async move {
            self.0.closes.fetch_add(1, Ordering::SeqCst);
            self.0.closing.notify_one();
            self.0.release.acquire().await.unwrap().forget();
            if self.0.fail_close {
                Err(Error::Transport)
            } else {
                Ok(())
            }
        })
    }
}
async fn session(controls: &[Arc<Control>]) -> OwnedMcpSession {
    let mut clients = vec![];
    for (index, control) in controls.iter().enumerate() {
        let name = format!("server{index}");
        clients.push(
            Client::new(
                Box::new(Peer(control.clone())),
                &name,
                serde_json::from_value(json!({"command":"mock","trustReadOnlyHint":true})).unwrap(),
                HostPolicy {
                    tenant_id: "tenant".into(),
                    servers: BTreeMap::from([(
                        name.clone(),
                        ServerPolicy {
                            enabled: true,
                            ..Default::default()
                        },
                    )]),
                    ..Default::default()
                },
            )
            .unwrap(),
        );
    }
    OwnedMcpSession::new(ClientManager::new(clients).await.unwrap())
}
async fn notified(notify: &Notify) {
    tokio::time::timeout(Duration::from_secs(2), notify.notified())
        .await
        .unwrap();
}

#[tokio::test]
async fn cancelled_close_waiter_keeps_cleanup_and_revokes_retained_handles() {
    let control = Arc::new(Control::default());
    let session = Arc::new(session(&[control.clone()]).await);
    let handle = session.handle();
    assert_eq!(session.catalog()[0].tool_name, "read");
    assert_eq!(session.connected_servers().len(), 1);
    let call = {
        let handle = handle.clone();
        tokio::spawn(async move { handle.call("mcp__server0__read", json!({})).await })
    };
    notified(&control.dispatched).await;
    let closing = {
        let session = session.clone();
        tokio::spawn(async move { session.close().await })
    };
    notified(&control.closing).await;
    closing.abort();
    assert!(closing.await.unwrap_err().is_cancelled());
    assert_eq!(call.await.unwrap(), Err(Error::Closed));
    assert!(handle.definitions().is_empty());
    assert!(!handle.has_resources());
    assert_eq!(
        handle.call("mcp__server0__read", json!({})).await,
        Err(Error::Closed)
    );
    assert_eq!(handle.list_resources(None).await, Err(Error::Closed));
    assert_eq!(
        handle.read_resource("server0", "file:///x").await,
        Err(Error::Closed)
    );
    control.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(2), session.close())
        .await
        .unwrap()
        .unwrap();
    session.close().await.unwrap();
    drop(session);
    assert_eq!(control.closes.load(Ordering::SeqCst), 1);
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn drop_revokes_without_polling_and_cleanup_failure_still_closes_other_servers() {
    let first = Arc::new(Control {
        fail_close: true,
        ..Default::default()
    });
    let second = Arc::new(Control::default());
    let session = session(&[first.clone(), second.clone()]).await;
    let handle = session.handle();
    drop(session);
    assert_eq!(
        handle.call("mcp__server0__read", json!({})).await,
        Err(Error::Closed)
    );
    notified(&first.closing).await;
    first.release.add_permits(1);
    notified(&second.closing).await;
    second.release.add_permits(1);
    assert_eq!(first.closes.load(Ordering::SeqCst), 1);
    assert_eq!(second.closes.load(Ordering::SeqCst), 1);
    assert_eq!(first.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn retained_unpolled_call_does_not_block_owner_shutdown() {
    let control = Arc::new(Control::default());
    control.release.add_permits(1);
    let session = session(&[control.clone()]).await;
    let handle = session.handle();
    let mut call = handle.call("mcp__server0__read", json!({}));
    std::future::poll_fn(|cx| {
        assert!(call.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    notified(&control.dispatched).await;
    tokio::time::timeout(Duration::from_secs(2), session.close())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(call.await, Err(Error::Closed));
    assert_eq!(control.closes.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn dropped_dispatched_call_poisoning_is_preserved_without_replay() {
    let control = Arc::new(Control::default());
    control.release.add_permits(1);
    let session = session(&[control.clone()]).await;
    let handle = session.handle();
    let mut call = handle.call("mcp__server0__read", json!({}));
    std::future::poll_fn(|cx| {
        assert!(call.as_mut().poll(cx).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    notified(&control.dispatched).await;
    drop(call);
    assert_eq!(
        tokio::time::timeout(
            Duration::from_secs(2),
            handle.call("mcp__server0__read", json!({}))
        )
        .await
        .unwrap(),
        Err(Error::Closed)
    );
    assert_eq!(control.calls.load(Ordering::SeqCst), 1);
    session.close().await.unwrap();
}

#[tokio::test]
async fn repeated_close_returns_same_failure_after_all_servers_are_closed() {
    let first = Arc::new(Control {
        fail_close: true,
        ..Default::default()
    });
    let second = Arc::new(Control::default());
    first.release.add_permits(1);
    second.release.add_permits(1);
    let session = session(&[first.clone(), second.clone()]).await;
    for _ in 0..2 {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), session.close())
                .await
                .unwrap(),
            Err(Error::Transport)
        );
    }
    assert_eq!(first.closes.load(Ordering::SeqCst), 1);
    assert_eq!(second.closes.load(Ordering::SeqCst), 1);
}
