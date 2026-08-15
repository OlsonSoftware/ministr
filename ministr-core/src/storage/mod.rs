//! Storage layer for ministr-core.
//!
//! Provides the [`Storage`] trait for persistence operations and a
//! [`SqliteStorage`] implementation backed by rusqlite with async-safe
//! access via `tokio::spawn_blocking`.

mod corpus;
mod lease;
mod schema;
mod sqlite;
pub mod traits;

pub use corpus::{ensure_corpus_layout, ensure_corpus_sidecar};
pub use lease::{IndexLease, LEASE_FILE_NAME, LeaseError};
pub use schema::CURRENT_SCHEMA_VERSION;
pub use sqlite::SqliteStorage;
pub use traits::{
    BridgeEndpointRecord, BridgeLinkDetail, BridgeLinkRecord, ClaimRecord, CoAccessRecord,
    CorpusStats, DocumentRecord, FileHashRecord, GitCacheRecord, RelatedClaimRecord,
    SectionAccessStat, SectionRecord, Storage, SymbolFilter, SymbolRecord, SymbolRefRecord,
    WebCacheRecord,
};
