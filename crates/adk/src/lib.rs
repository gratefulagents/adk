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
/// Project tasks, memory and independent embedding storage.
#[cfg(feature = "project-state")]
pub use adk_project_state as project_state;
#[cfg(feature = "providers")]
pub use adk_providers as providers;
#[cfg(feature = "runtime")]
pub use adk_runtime as runtime;
#[cfg(feature = "execution")]
pub mod execution;
#[cfg(feature = "execution")]
pub use adk_sandbox as sandbox;
#[cfg(feature = "execution")]
pub use adk_security as security;
