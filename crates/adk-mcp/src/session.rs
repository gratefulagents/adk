//! Revocable runtime access with one retained shutdown attempt per owned manager.
use crate::{
    BoxFuture, Error,
    client::{Capabilities, CatalogEntry, ClientManager},
    tools::ToolManager,
};
use adk_core::ToolDefinition;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Arc,
    time::Duration,
};
use tokio::sync::{oneshot, watch};

#[cfg(test)]
#[path = "session_budget_tests.rs"]
mod budget_tests;

pub(crate) struct CatalogBudget {
    limit: usize,
    used: std::sync::atomic::AtomicUsize,
}

pub(crate) struct CatalogReservation {
    budget: Arc<CatalogBudget>,
    count: usize,
}

impl CatalogBudget {
    fn new(limit: usize) -> Arc<Self> {
        Arc::new(Self {
            limit,
            used: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    pub(crate) fn reserve(self: &Arc<Self>, count: usize) -> Result<CatalogReservation, Error> {
        use std::sync::atomic::Ordering;
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(count).filter(|total| *total <= self.limit)
            })
            .map_err(|_| Error::Limit)?;
        Ok(CatalogReservation {
            budget: self.clone(),
            count,
        })
    }
}

impl Drop for CatalogReservation {
    fn drop(&mut self) {
        self.budget
            .used
            .fetch_sub(self.count, std::sync::atomic::Ordering::Release);
    }
}

#[derive(Clone, Default)]
pub struct ServerOptions {
    pub remote: Option<crate::transport::RemoteOptions>,
    pub environment: BTreeMap<String, String>,
}

pub struct ConnectionSet {
    pub config: crate::config::ConnectionConfig,
    pub host_policy: crate::client::HostPolicy,
    pub server_options: BTreeMap<String, ServerOptions>,
    pub limits: crate::Limits,
    pub max_servers: usize,
    pub max_catalog_items: usize,
    pub build_timeout: Duration,
}

impl ConnectionSet {
    pub fn new(
        config: crate::config::ConnectionConfig,
        host_policy: crate::client::HostPolicy,
    ) -> Self {
        Self {
            config,
            host_policy,
            server_options: BTreeMap::new(),
            limits: crate::Limits::default(),
            max_servers: 32,
            max_catalog_items: 10_000,
            build_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Clone, Default)]
pub struct ToolSelection {
    pub allow_all: bool,
    pub allowed: BTreeSet<String>,
    pub resources: bool,
}

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
    /// Preflights every selected grant before connecting, then owns acquisition,
    /// discovery and rollback as well as the published manager's lifecycle.
    pub async fn connect(
        mut input: ConnectionSet,
        servers: BTreeSet<String>,
        tools: ToolSelection,
        work_dir: &Path,
    ) -> Result<Self, Error> {
        if input.max_servers == 0
            || input.max_catalog_items == 0
            || input.build_timeout.is_zero()
            || input.limits.max_pages == 0
            || input.limits.max_items == 0
            || input.limits.max_message_bytes == 0
            || input.limits.timeout.is_zero()
        {
            return Err(Error::Config("positive session bounds required".into()));
        }
        if servers.len() > input.max_servers {
            return Err(Error::Limit);
        }
        for server in &servers {
            let options = input.server_options.get(server);
            crate::connection::preflight(
                &input.config,
                server,
                &input.host_policy,
                options.and_then(|options| options.remote.as_ref()),
                &input.limits,
            )?;
        }
        tokio::time::timeout(input.build_timeout, async move {
            let mut acquired = AcquiredClients(Vec::new());
            let budget = CatalogBudget::new(input.max_catalog_items);
            for server in servers {
                let options = input.server_options.remove(&server).unwrap_or_default();
                let mut client = crate::connection::connect_config(
                    &input.config,
                    &server,
                    input.host_policy.clone(),
                    options.remote,
                    &options.environment,
                    work_dir,
                    input.limits.clone(),
                )
                .await?;
                client.set_catalog_budget(budget.clone());
                acquired.0.push(client);
                acquired
                    .0
                    .last_mut()
                    .expect("acquired client")
                    .list_tools()
                    .await?;
            }
            let manager = ClientManager::new(std::mem::take(&mut acquired.0))
                .await?
                .select_tools(tools.allow_all, &tools.allowed, tools.resources);
            Ok(Self::new(manager))
        })
        .await
        .map_err(|_| Error::Transport)?
    }

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
        self.begin_close();
        let mut completion = self.completion.clone();
        completion
            .wait_for(Option::is_some)
            .await
            .map_err(|_| Error::Closed)?
            .as_ref()
            .expect("completed shutdown")
            .clone()
    }

    pub fn begin_close(&self) {
        self.shutdown.send_replace(true);
    }
}

impl Drop for OwnedMcpSession {
    fn drop(&mut self) {
        self.begin_close();
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
