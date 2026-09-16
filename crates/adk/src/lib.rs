//! Platform-independent agent development contracts.
//!
//! Default builds contain only native contracts. `compat` enables explicit Go
//! codecs; `runtime` enables the standalone runner, streaming and owned cancellation.
#[cfg(feature = "compat")]
pub use adk_codec as codec;
pub use adk_core as core;
#[cfg(feature = "runtime")]
pub use adk_runtime as runtime;
