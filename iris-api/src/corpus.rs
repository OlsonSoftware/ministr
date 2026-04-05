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
    /// Number of active MCP sessions using this corpus.
    #[serde(default)]
    pub active_sessions: usize,
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
    /// Current ingestion phase: `"idle"`, `"discovering"`, `"parsing"`,
    /// `"embedding"`, or `"finalizing"`.
    pub phase: String,
    /// Total files discovered for ingestion.
    pub files_total: usize,
    /// Files processed so far.
    pub files_done: usize,
    /// Sections created so far.
    pub sections_done: usize,
    /// Total embeddings to generate.
    pub embeddings_total: usize,
    /// Embeddings generated so far.
    pub embeddings_done: usize,
    /// Relative path of the file currently being processed (empty if idle).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub current_file: String,
}

/// Compact bundle manifest for API responses.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct BundleManifestApi {
    /// Bundle format version.
    pub format_version: u32,
    /// Embedding model name.
    pub model_name: String,
    /// Embedding vector dimension.
    pub dimension: usize,
    /// Number of vectors in the index.
    pub vector_count: usize,
    /// Number of documents.
    pub document_count: usize,
    /// Number of code symbols.
    pub symbol_count: usize,
    /// Content-addressable version hash for staleness checking.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_version: Option<String>,
}

/// Response after exporting a corpus to a bundle.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ExportBundleResponse {
    /// Filesystem path to the generated bundle file.
    pub bundle_path: String,
    /// Bundle manifest metadata.
    pub manifest: BundleManifestApi,
}

/// Request to import a bundle into the daemon.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ImportBundleRequest {
    /// Filesystem path to the `.iris-index` bundle file.
    pub bundle_path: String,
}

/// Response after importing a bundle.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ImportBundleResponse {
    /// Corpus ID assigned to the imported bundle.
    pub corpus_id: String,
    /// Bundle manifest metadata.
    pub manifest: BundleManifestApi,
}
