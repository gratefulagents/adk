//! Revocable runtime access with one retained shutdown attempt per owned manager.
use crate::{
    BoxFuture, Error,
    client::{Capabilities, CatalogEntry, ClientManager},
    tools::ToolManager,
};
use adk_core::ToolDefinition;
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{oneshot, watch};

pub(crate) struct AcquiredClients(pub Vec<crate::client::Client>);

impl AcquiredClients {
    pub fn start_cleanup(&mut self) -> tokio::task::JoinHandle<Vec<crate::client::Client>> {
        let mut clients = std::mem::take(&mut self.0);
        tokio::spawn(async move {
            for client in &mut clients {
                let _ = client.close().await;
            }
            clients
        })
    }
}

impl Drop for AcquiredClients {
    fn drop(&mut self) {
        if !self.0.is_empty() && tokio::runtime::Handle::try_current().is_ok() {
            self.start_cleanup();
        }
    }
}

/// Must be closed before the Tokio executor shuts down. Dropping requests cleanup;
/// it cannot guarantee remote acknowledgement after executor shutdown.
pub struct OwnedMcpSession {
    handle: McpHandle,
    shutdown: watch::Sender<bool>,
    completion: watch::Receiver<Option<Result<(), Error>>>,
}

#[derive(Clone)]
pub struct McpHandle {
    manager: Arc<ClientManager>,
    shutdown: watch::Receiver<bool>,
}

impl OwnedMcpSession {
    /// Takes exclusive lifecycle authority over an already assembled manager.
    /// Connection and discovery failures before this call remain caller-owned.
    pub fn new(manager: ClientManager) -> Self {
        let manager = Arc::new(manager);
        let (shutdown, mut requested) = watch::channel(false);
        let (completed, completion) = watch::channel(None);
        let handle = McpHandle {
            manager: manager.clone(),
            shutdown: requested.clone(),
        };
        tokio::spawn(async move {
            let _ = requested.wait_for(|closed| *closed).await;
            let result = manager.close().await;
            completed.send_replace(Some(result));
        });
        Self {
            handle,
            shutdown,
            completion,
        }
    }

    pub fn handle(&self) -> McpHandle {
        self.handle.clone()
    }

    pub fn catalog(&self) -> Vec<CatalogEntry> {
        self.handle.manager.catalog()
    }

    pub fn connected_servers(&self) -> &BTreeMap<String, Capabilities> {
        self.handle.manager.connected_servers()
    }

    /// Revokes all handles before waiting. Cancellation of this waiter does not
    /// cancel cleanup; subsequent waiters observe the same completion result.
    pub async fn close(&self) -> Result<(), Error> {
        self.shutdown.send_replace(true);
        let mut completion = self.completion.clone();
        completion
            .wait_for(Option::is_some)
            .await
            .map_err(|_| Error::Closed)?
            .as_ref()
            .expect("completed shutdown")
            .clone()
    }
}

impl Drop for OwnedMcpSession {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
    }
}

impl McpHandle {
    async fn active(
        &self,
        operation: BoxFuture<'static, Result<Value, Error>>,
    ) -> Result<Value, Error> {
        let mut shutdown = self.shutdown.clone();
        if *shutdown.borrow() {
            return Err(Error::Closed);
        }
        let (mut completed, result) = oneshot::channel();
        // A retained, unpolled caller future must not hold a client lock across shutdown.
        tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                _ = shutdown.wait_for(|closed| *closed) => Err(Error::Closed),
                _ = completed.closed() => return,
                result = operation => result,
            };
            let _ = completed.send(result);
        });
        let result = result.await.map_err(|_| Error::Closed)?;
        if *self.shutdown.borrow() {
            Err(Error::Closed)
        } else {
            result
        }
    }
}

impl ToolManager for McpHandle {
    fn definitions(&self) -> Vec<ToolDefinition> {
        if *self.shutdown.borrow() {
            vec![]
        } else {
            self.manager.definitions()
        }
    }

    fn has_resources(&self) -> bool {
        !*self.shutdown.borrow() && self.manager.has_resources()
    }

    fn call<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Result<Value, Error>> {
        let manager = self.manager.clone();
        let name = name.to_owned();
        Box::pin(self.active(Box::pin(
            async move { manager.call(&name, arguments).await },
        )))
    }

    fn list_resources<'a>(
        &'a self,
        server: Option<&'a str>,
    ) -> BoxFuture<'a, Result<Value, Error>> {
        let manager = self.manager.clone();
        let server = server.map(str::to_owned);
        Box::pin(self.active(Box::pin(async move {
            manager.list_resources(server.as_deref()).await
        })))
    }

    fn read_resource<'a>(
        &'a self,
        server: &'a str,
        uri: &'a str,
    ) -> BoxFuture<'a, Result<Value, Error>> {
        let manager = self.manager.clone();
        let server = server.to_owned();
        let uri = uri.to_owned();
        Box::pin(self.active(Box::pin(async move {
            manager.read_resource(&server, &uri).await
        })))
    }
}
