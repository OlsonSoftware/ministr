//! Corpus management API types.
//!
//! Types for registering, listing, and managing indexed corpora
//! through the daemon API.

use serde::{Deserialize, Serialize};

/// Request to register a new corpus with the daemon.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RegisterCorpusRequest {
    /// Filesystem paths (directories, files, or globs) to index.
    pub paths: Vec<String>,

    /// Optional git include URLs from `.iris.toml`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub git_includes: Vec<GitInclude>,
}

/// A git repository to clone and index alongside local paths.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GitInclude {
    /// Git repository URL.
    pub repo: String,
    /// Optional sparse-checkout paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Optional branch or tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// Response after successfully registering a corpus.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct RegisterCorpusResponse {
    /// The assigned corpus ID (deterministic hash of paths).
    pub corpus_id: String,
    /// Whether indexing was started (false if corpus was already registered).
    pub indexing_started: bool,
}

/// Summary information about a registered corpus.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CorpusInfo {
    /// Corpus identifier.
    pub id: String,
    /// Source paths that make up this corpus.
    pub paths: Vec<String>,
    /// Current indexing status.
    pub status: IndexingStatus,
    /// Number of indexed files.
    pub files_indexed: usize,
    /// Number of indexed sections.
    pub sections_count: usize,
    /// Number of embeddings in the vector index.
    pub embeddings_count: usize,
}

/// Current indexing status of a corpus.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum IndexingStatus {
    /// Corpus is idle (fully indexed or not yet started).
    Idle,
    /// Indexing is in progress.
    Indexing {
        /// Number of files processed so far.
        files_done: usize,
        /// Total number of files to process.
        files_total: usize,
    },
    /// Indexing failed with an error.
    Error {
        /// Error message.
        message: String,
    },
}

/// Response listing all registered corpora.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ListCorporaResponse {
    /// All registered corpora.
    pub corpora: Vec<CorpusInfo>,
}

/// A single SSE event for ingestion progress.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct IngestionProgressEvent {
    /// Phase: `"pending"`, `"running"`, or `"complete"`.
    pub status: String,
    /// Total files discovered for ingestion.
    pub files_total: usize,
    /// Files processed so far.
    pub files_done: usize,
    /// Total embeddings to generate.
    pub embeddings_total: usize,
    /// Embeddings generated so far.
    pub embeddings_done: usize,
}
