//! Host-owned, bounded MCP sessions. Repository configuration never grants authority.
pub mod client;
pub mod config;
pub mod connection;
mod names;
pub mod server;
pub mod tools;
pub mod transport;

pub use adk_core::BoxFuture;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("invalid MCP configuration: {0}")]
    Config(String),
    #[error("MCP policy denied: {0}")]
    Policy(String),
    #[error("MCP protocol error: {0}")]
    Protocol(String),
    #[error("MCP transport unavailable")]
    Transport,
    #[error("MCP bound exceeded")]
    Limit,
    #[error("MCP connection closed")]
    Closed,
    #[error("MCP config changed; explicit new snapshot required")]
    ConfigChanged,
    #[error(
        "MCP outcome unknown for {server}/{operation}; reconciliation required; request not replayed"
    )]
    ReconciliationRequired { server: String, operation: String },
    #[error("MCP remote error code {code}")]
    Remote { code: i64 },
}

#[derive(Debug, Clone)]
pub struct Limits {
    pub max_message_bytes: usize,
    pub max_pages: usize,
    pub max_items: usize,
    pub max_stderr_bytes: usize,
    pub timeout: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_message_bytes: 8 << 20,
            max_pages: 100,
            max_items: 10000,
            max_stderr_bytes: 8192,
            timeout: Duration::from_secs(30),
        }
    }
}

/// A transport owns one session; no implementation may replay a dispatched request.
/// Failures after dispatch must be ReconciliationRequired (even for read operations).
pub trait Transport: Send {
    fn request<'a>(
        &'a mut self,
        method: &'a str,
        params: Value,
    ) -> BoxFuture<'a, Result<Value, Error>>;
    fn notify<'a>(&'a mut self, method: &'a str, params: Value)
    -> BoxFuture<'a, Result<(), Error>>;
    fn close(&mut self) -> BoxFuture<'_, Result<(), Error>>;
}

pub const PROTOCOL_VERSION: &str = "2025-03-26";
