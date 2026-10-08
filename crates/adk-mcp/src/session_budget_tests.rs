use super::*;
use crate::{
    Transport,
    client::{Client, HostPolicy, ServerPolicy},
};
use serde_json::json;
use std::{collections::VecDeque, sync::atomic::Ordering};

#[test]
fn reservations_release_capacity_and_overflow_never_wraps() {
    let budget = CatalogBudget::new(5);
    let first = budget.reserve(3).unwrap();
    let second = budget.reserve(2).unwrap();
    assert!(matches!(budget.reserve(1), Err(Error::Limit)));
    assert!(matches!(budget.reserve(usize::MAX), Err(Error::Limit)));
    assert_eq!(budget.used.load(Ordering::Acquire), 5);
    drop(first);
    assert_eq!(budget.used.load(Ordering::Acquire), 2);
    let third = budget.reserve(3).unwrap();
    drop((second, third));
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}

struct Peer(VecDeque<Value>);
impl Transport for Peer {
    fn request<'a>(&'a mut self, _: &'a str, _: Value) -> BoxFuture<'a, Result<Value, Error>> {
        Box::pin(async move { Ok(self.0.pop_front().expect("unexpected request")) })
    }
    fn notify<'a>(&'a mut self, _: &'a str, _: Value) -> BoxFuture<'a, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
    fn close(&mut self) -> BoxFuture<'_, Result<(), Error>> {
        Box::pin(async { Ok(()) })
    }
}
fn hello() -> Value {
    json!({"protocolVersion":crate::PROTOCOL_VERSION,"capabilities":{"tools":{},"resources":{},"prompts":{}}})
}
fn tools() -> Value {
    json!({"tools":[{"name":"tool","inputSchema":{"type":"object"}}]})
}
fn resources() -> Value {
    json!({"resources":[{"name":"resource","uri":"test://resource"}]})
}
fn prompts() -> Value {
    json!({"prompts":[{"name":"prompt"}]})
}

#[tokio::test]
async fn cached_catalog_reservations_release_on_invalidation_refresh_reconnect_and_drop() {
    let peer = Peer(VecDeque::from([
        hello(),
        tools(),
        resources(),
        prompts(),
        prompts(),
        resources(),
        resources(),
    ]));
    let mut client = Client::new(
        Box::new(peer),
        "test",
        serde_json::from_value(json!({"command":"mock"})).unwrap(),
        HostPolicy {
            tenant_id: "tenant".into(),
            servers: BTreeMap::from([(
                "test".into(),
                ServerPolicy {
                    enabled: true,
                    ..Default::default()
                },
            )]),
            ..Default::default()
        },
    )
    .unwrap();
    let budget = CatalogBudget::new(2);
    client.set_catalog_budget(budget.clone());
    client.initialize().await.unwrap();
    client.list_tools().await.unwrap();
    assert_eq!(budget.used.load(Ordering::Acquire), 1);
    client.list_resources().await.unwrap();
    assert_eq!(budget.used.load(Ordering::Acquire), 2);
    assert!(matches!(client.list_prompts().await, Err(Error::Limit)));
    assert_eq!(budget.used.load(Ordering::Acquire), 2);
    client.invalidate_discovery();
    assert_eq!(budget.used.load(Ordering::Acquire), 1);
    client.list_prompts().await.unwrap();
    assert_eq!(budget.used.load(Ordering::Acquire), 2);
    client.set_discovery_cache_ttl(Duration::ZERO);
    assert_eq!(budget.used.load(Ordering::Acquire), 1);
    client.list_resources().await.unwrap();
    client.list_resources().await.unwrap();
    assert_eq!(budget.used.load(Ordering::Acquire), 2);
    client
        .reconnect(Box::new(Peer(VecDeque::from([hello(), tools()]))))
        .await
        .unwrap();
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
    client.list_tools().await.unwrap();
    assert_eq!(budget.used.load(Ordering::Acquire), 1);
    drop(client);
    assert_eq!(budget.used.load(Ordering::Acquire), 0);
}
