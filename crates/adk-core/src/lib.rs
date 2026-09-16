//! Runtime-neutral ADK contracts, not an agent runner or provider implementation.
//!
//! Async traits are object-safe and return boxed `Send` futures. Implementations
//! must observe the supplied cancellation/deadline context and own any background
//! work; dropping a returned future must not silently detach that work.
//!
//! Serializable types are native Rust contracts. Their Serde representation is
//! **not** the Go compatibility format; wire conversion belongs to `adk-codec`.
//! Contexts, credentials, trait objects, and runtime handles are not serializable.

mod contracts;
mod error;
mod policy;
mod types;

pub use contracts::*;
pub use error::*;
pub use policy::*;
pub use types::*;
