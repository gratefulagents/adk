//! Platform-independent agent development contracts.
//!
//! Default builds contain only native contracts. `compat` enables explicit Go
//! codecs; `runtime` enables the standalone runner, streaming and owned cancellation.
//! `providers` enables explicit provider adapters; `providers-runtime` adds their
//! runner cost and compaction integration. Credentials remain host-owned.
#[cfg(feature = "compat")]
pub use adk_codec as codec;
pub use adk_core as core;
#[cfg(feature = "providers")]
pub use adk_providers as providers;
#[cfg(feature = "runtime")]
pub use adk_runtime as runtime;
