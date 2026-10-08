//! Safe composition of a pinned configuration and host grants before any I/O.
use crate::{
    Error, Limits, Transport,
    client::{Client, HostPolicy},
    config::{ConfigSnapshot, ConnectionConfig},
    transport::{HttpTransport, RemoteOptions, StdioTransport},
};
use std::{collections::BTreeMap, path::Path};

/// A typed connection error with separately accessed host-only diagnostics.
pub struct ConnectionFailure {
    pub error: Error,
    diagnostics: Option<String>,
}

impl ConnectionFailure {
    /// Untrusted, potentially sensitive peer text. Do not send to models or logs.
    pub fn diagnostics(&self) -> Option<&str> {
        self.diagnostics.as_deref()
    }
}

impl From<Error> for ConnectionFailure {
    fn from(error: Error) -> Self {
        Self {
            error,
            diagnostics: None,
        }
    }
}

impl std::fmt::Debug for ConnectionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionFailure")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Display for ConnectionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for ConnectionFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Establish a session from the original snapshot, never from newly read config.
/// `environment` is a host-selected per-server environment, not ambient process
/// environment. Stdio launches a local process; pass a sandbox launcher command
/// in the approved snapshot when OS containment is required.
pub async fn connect(
    snapshot: &ConfigSnapshot,
    server: &str,
    policy: HostPolicy,
    remote: Option<RemoteOptions>,
    environment: &BTreeMap<String, String>,
    working_directory: &Path,
    limits: Limits,
) -> Result<Client, Error> {
    connect_with_diagnostics(
        snapshot,
        server,
        policy,
        remote,
        environment,
        working_directory,
        limits,
    )
    .await
    .map_err(|failure| failure.error)
}

/// Like [`connect`], but retains bounded startup stderr for explicit host access.
/// Diagnostics remain untrusted and potentially sensitive, not model/log output.
pub async fn connect_with_diagnostics(
    snapshot: &ConfigSnapshot,
    server: &str,
    policy: HostPolicy,
    remote: Option<RemoteOptions>,
    environment: &BTreeMap<String, String>,
    working_directory: &Path,
    limits: Limits,
) -> Result<Client, ConnectionFailure> {
    connect_config_with_diagnostics(
        &ConnectionConfig::snapshot(snapshot.clone()),
        server,
        policy,
        remote,
        environment,
        working_directory,
        limits,
    )
    .await
}

/// Connect using an explicitly supplied, validated inline or snapshot source.
pub async fn connect_config(
    source: &ConnectionConfig,
    server: &str,
    policy: HostPolicy,
    remote: Option<RemoteOptions>,
    environment: &BTreeMap<String, String>,
    working_directory: &Path,
    limits: Limits,
) -> Result<Client, Error> {
    connect_config_with_diagnostics(
        source,
        server,
        policy,
        remote,
        environment,
        working_directory,
        limits,
    )
    .await
    .map_err(|failure| failure.error)
}

/// Like [`connect_config`], retaining bounded host-only startup diagnostics.
pub async fn connect_config_with_diagnostics(
    source: &ConnectionConfig,
    server: &str,
    policy: HostPolicy,
    remote: Option<RemoteOptions>,
    environment: &BTreeMap<String, String>,
    working_directory: &Path,
    limits: Limits,
) -> Result<Client, ConnectionFailure> {
    let config = preflight(source, server, &policy, remote.as_ref(), &limits)?.clone();
    let grant = &policy.servers[server];
    let transport: Box<dyn Transport> = if config.is_remote() {
        let remote = remote.expect("preflight remote options");
        Box::new(
            HttpTransport::connect(
                server,
                config.url(),
                config.transport_type() == "sse",
                remote,
                limits.clone(),
            )
            .await?,
        )
    } else {
        let env = config.filtered_env(environment, &grant.allow_env);
        Box::new(
            StdioTransport::connect(
                server,
                config.command(),
                config.args(),
                &env,
                working_directory,
                limits.clone(),
            )
            .await?,
        )
    };
    let client = Client::with_limits(transport, server, config, policy, limits)?;
    let mut acquired = crate::session::AcquiredClients(vec![client]);
    if let Err(error) = acquired.0[0].initialize().await {
        let closed = acquired
            .start_cleanup()
            .await
            .map_err(|_| Error::Transport)?;
        return Err(ConnectionFailure {
            error,
            diagnostics: closed[0].diagnostics(),
        });
    }
    Ok(acquired.0.pop().expect("acquired client"))
}

pub(crate) fn preflight<'a>(
    source: &'a ConnectionConfig,
    server: &str,
    policy: &HostPolicy,
    remote: Option<&RemoteOptions>,
    limits: &Limits,
) -> Result<&'a crate::config::ServerConfig, Error> {
    source.verify_unchanged()?;
    let config = source
        .config()
        .server(server)
        .ok_or_else(|| Error::Policy("server not configured".into()))?;
    let grant = policy
        .servers
        .get(server)
        .ok_or_else(|| Error::Policy("server not granted by host".into()))?;
    if !config.enabled() || !grant.enabled || policy.tenant_id.trim().is_empty() {
        return Err(Error::Policy("server disabled or tenant missing".into()));
    }
    if limits.max_pages == 0
        || limits.max_items == 0
        || limits.max_message_bytes == 0
        || limits.timeout.is_zero()
    {
        return Err(Error::Config("positive limits required".into()));
    }
    if config.is_remote() {
        if !grant.allowed_origins.contains(&config.origin()?) {
            return Err(Error::Policy("remote origin not granted by host".into()));
        }
        let remote =
            remote.ok_or_else(|| Error::Policy("remote transport requires host options".into()))?;
        if remote.tenant_id != policy.tenant_id {
            return Err(Error::Policy(
                "transport credential tenant differs from policy tenant".into(),
            ));
        }
        crate::transport::validate_remote_options(config.url(), remote, limits)?;
    }
    Ok(config)
}
