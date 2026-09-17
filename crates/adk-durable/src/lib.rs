//! Durable Go-compatible records, transactional stores, and conservative effect recovery.
mod codec;
mod filesystem;
#[cfg(feature = "postgres")]
mod pg;
mod store;
mod types;

pub use codec::{decode_document, encode_document};
pub use filesystem::FilesystemStore;
#[cfg(feature = "postgres")]
pub use pg::PostgresStore;
pub use store::*;
pub use types::*;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("durable: run not found")]
    NotFound,
    #[error("durable: run already exists")]
    AlreadyExists,
    #[error("durable: revision conflict")]
    Conflict,
    #[error("durable: lease held")]
    LeaseHeld,
    #[error("durable: lease lost")]
    LeaseLost,
    #[error("durable: unsupported schema version {0}")]
    UnsupportedSchema(i32),
    #[error("durable: {0}")]
    Invalid(String),
    #[error("durable: persistence transformation: {0}")]
    Protection(String),
    #[error("durable: IO: {0}")]
    Io(#[from] std::io::Error),
    #[error("durable: JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[cfg(feature = "postgres")]
    #[error("durable: PostgreSQL: {0}")]
    Postgres(#[from] postgres::Error),
}
