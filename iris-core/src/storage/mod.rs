//! Storage layer for iris-core.
//!
//! Provides the [`Storage`] trait for persistence operations and a
//! [`SqliteStorage`] implementation backed by rusqlite with async-safe
//! access via `tokio::spawn_blocking`.

mod corpus;
mod schema;
mod sqlite;
pub mod traits;

pub use corpus::ensure_corpus_layout;
pub use schema::CURRENT_SCHEMA_VERSION;
pub use sqlite::SqliteStorage;
pub use traits::{ClaimRecord, DocumentRecord, FileHashRecord, SectionRecord, Storage};
