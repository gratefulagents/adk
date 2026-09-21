//! Platform-independent agent development contracts.
//!
//! Default builds contain only native contracts. `compat` enables explicit Go
//! codecs; `runtime` enables the standalone runner, streaming and owned cancellation.
//! `providers` enables explicit provider adapters; `providers-runtime` adds their
//! runner cost and compaction integration. Credentials remain host-owned.
//! `execution` adds opt-in policy/guardrails and enforcing subprocess backends;
//! it does not register tools or enable production execution.
#[cfg(feature = "compat")]
pub use adk_codec as codec;
pub use adk_core as core;
/// Compatible durable documents, stores and effect recovery.
#[cfg(feature = "durable")]
pub use adk_durable as durable;
/// Host-owned MCP sessions, discovery and policy-gated server mode.
#[cfg(feature = "mcp")]
pub use adk_mcp as mcp;
/// Project tasks, memory and independent embedding storage.
#[cfg(feature = "project-state")]
pub use adk_project_state as project_state;
#[cfg(feature = "providers")]
pub use adk_providers as providers;
#[cfg(feature = "runtime")]
pub use adk_runtime as runtime;
/// Source-backed tool registry composition; not yet complete SDK tool parity.
#[cfg(feature = "tools")]
pub use adk_tools as tools;
#[cfg(feature = "execution")]
pub mod execution;
#[cfg(feature = "execution")]
pub use adk_sandbox as sandbox;
#[cfg(feature = "execution")]
pub use adk_security as security;

/// Lifecycle-owned prepared tool integration with the standalone runner.
#[cfg(all(feature = "tools", feature = "runtime"))]
pub mod tool_runtime;

/// Ordered native events, explicit capture policy and host-owned telemetry.
#[cfg(feature = "observability")]
pub mod observability;

/// Compatible SDK trace documents and Linux confined category storage.
#[cfg(feature = "observability")]
pub mod tracestore;

/// SDK schema-2 trace writer and host-published span records.
#[cfg(feature = "observability")]
pub mod tracewriter;

/// Explicitly owned stdout/OTLP exporters and SDK endpoint defaults.
#[cfg(feature = "otel")]
pub mod telemetry;

/// Host configuration, provider/tool composition and explicit bundle/session ownership.
#[cfg(feature = "builder")]
pub mod builder;
