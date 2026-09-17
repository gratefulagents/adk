//! Durable project state compatible with the Go SDK's version-1 event log.
//! Lexical search is local; embedding recall is an explicit, separate capability.
mod contracts;
mod engine;
pub mod memory;
pub mod recall;
mod storage;
pub mod tools;
mod types;

pub use contracts::*;
pub use engine::*;
pub use recall::{Embedder, EmbeddingRecall, HybridConfig, LexicalRecall};
pub use storage::{
    FilesystemOptions, SQLiteOptions, StoreOptions, default_state_dir, derive_project_id,
};
pub use types::*;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("{0} not found")]
    NotFound(String),
    #[error("project-state lock timed out")]
    LockTimeout,
    #[error("store mutex poisoned")]
    Poisoned,
    #[error("policy denied: {0}")]
    Denied(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

pub(crate) fn id(prefix: &str) -> String {
    format!(
        "{prefix}_{}",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    )
}
