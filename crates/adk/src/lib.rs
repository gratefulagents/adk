//! Platform-independent agent development contracts.
//!
//! Default builds contain only native contracts. `compat` enables explicit Go
//! codecs; `runtime` enables Tokio cancellation/task ownership, not an agent loop.
#[cfg(feature = "compat")]
pub use adk_codec as codec;
pub use adk_core as core;
#[cfg(feature = "runtime")]
pub use adk_runtime as runtime;
