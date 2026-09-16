//! Platform integration boundary. No Kubernetes client, deployment, or worker
//! implementation is provided by this foundation.
//!
//! Implement the reusable [`Host`] contract here, never by importing platform
//! types into `adk-core`. Platform-specific configuration stays in this crate.
pub use adk_core::Host;
pub mod codec;

/// Identity supplied explicitly by a platform integration, not inferred by core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunIdentity {
    pub namespace: String,
    pub name: String,
}
