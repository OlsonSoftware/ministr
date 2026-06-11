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

    /// Optional git include URLs from `.ministr.toml`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub git_includes: Vec<GitInclude>,

    /// Optional friendly display name to override the daemon's
    /// path-derived label. Used when registering linked projects or
    /// `ministr_clone` targets so the tray UI shows the human-meaningful
    /// name (e.g. `"BurntSushi-ripgrep"` rather than the basename of the
    /// content-hashed clone dir).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
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

/// Request to update an existing corpus's path set without dropping its
/// in-memory session state.
///
/// The new path set must canonicalise to the same `corpus_id` as the
/// existing corpus — identity is derived from canonical paths, so a
/// different canonical id means the caller is changing identity, not
/// just the path expression. In that case the daemon returns 400 and
/// the client should `unregister` + `register` instead.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UpdateCorpusPathsRequest {
    /// The new path set for the corpus.
    pub paths: Vec<String>,
}

/// Summary information about a registered corpus.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CorpusInfo {
    /// Corpus identifier.
    pub id: String,
    /// Human-readable label derived from the path set's longest common
    /// ancestor (typically the directory containing `.ministr.toml`).
    /// Computed once at registration; clients display this instead of
    /// rolling their own basename heuristic.
    #[serde(default)]
    pub display_name: String,
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
    /// Unix timestamp (seconds) of last completed indexing, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_indexed: Option<i64>,
    /// Number of code symbols extracted.
    #[serde(default)]
    pub symbols_count: usize,
    /// The effective embedding model this corpus is indexed + queried with
    /// (its `.ministr.toml` `[corpus] model`, else the daemon default).
    /// Empty for a not-yet-registered (pending) corpus. (parity-gui-corpus-model-readout)
    #[serde(default)]
    pub model: String,
    /// gd6: `true` only for a placeholder synthesized for a corpus that is
    /// registered (present in the on-disk manifest) but not yet *warmed* into
    /// memory — the daemon loads corpora in the background after gd5, so the
    /// GUI shows these as "Warming up…" instead of having them pop into the
    /// list the moment their index finishes loading. Real (loaded) corpora
    /// always serialise this as `false` (the serde default).
    #[serde(default)]
    pub warming: bool,
}

/// Current indexing status of a corpus.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum IndexingStatus {
    /// Corpus is idle (fully indexed or not yet started).
    Idle,
    /// Corpus is enqueued for indexing but waiting on a scheduler permit —
    /// distinct from `Indexing` (actively running) and `Idle` (done/not-started).
    /// Set by the indexer *before* it acquires an indexing slot, so a corpus
    /// sitting in the queue no longer misreports as `Indexing` with 0 files
    /// (the misleading "finished-but-empty" display, b44874e).
    Queued,
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

/// One indexed source file with its content hash and indexed-section count.
/// Backs the desktop code browser's file tree / treemap.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FileInfo {
    /// File path relative to the corpus root.
    pub path: String,
    /// Content hash recorded at index time (e.g. SHA-256 hex).
    pub content_hash: String,
    /// File modification time in nanoseconds since epoch, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mtime_ns: Option<i64>,
    /// Number of indexed sections whose document maps to this file.
    pub section_count: usize,
}

/// Response listing the indexed files of a corpus.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ListFilesResponse {
    /// All indexed files with their section counts.
    pub files: Vec<FileInfo>,
}

/// One file's hash-verified freshness verdict (the GUI trust display).
/// `state` is one of `"current"`, `"stale"`, `"new"`, `"missing"` —
/// computed by content-hash comparison, never timestamps.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FileFreshnessInfo {
    /// File path, same key as [`FileInfo::path`].
    pub path: String,
    /// Hash-verified state.
    pub state: String,
}

/// Response for the per-corpus freshness report.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct FreshnessResponse {
    /// Every discoverable + every indexed file, with its verdict.
    pub files: Vec<FileFreshnessInfo>,
    /// True while an indexing run is in flight for this corpus —
    /// per-file states are about to change (the GUI's `updating ⟳`).
    pub indexing: bool,
}

/// One read→edit join for the GUI's trust-evidence receipts: a file a
/// session had read was observed changing (gui-rw-session-outcome).
/// The claim is the JOIN FACT, not a temporal ordering.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OutcomeEventInfo {
    /// Session that had read the file.
    pub session_id: String,
    /// Absolute path of the edited file.
    pub path: String,
    /// 1-based rank among the session's distinct reads (1 = first).
    pub read_rank: usize,
    /// True when the edited file was the session's FIRST distinct read.
    pub first_touch: bool,
    /// Distinct files the session read before this one (the wander).
    pub reads_before: usize,
    /// When the watcher observed the edit (unix ms).
    pub edited_at_ms: u64,
}

/// Per-session outcome aggregates (counts only — no synthesis).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionOutcomeInfo {
    /// Session id.
    pub session_id: String,
    /// Distinct files the session has read.
    pub distinct_reads: usize,
    /// Read→edit joins observed.
    pub joins: usize,
    /// Joins where the file was the session's first read.
    pub first_touch_hits: usize,
}

/// Response for the per-corpus outcomes report.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OutcomesResponse {
    /// Join events, newest edit first.
    pub events: Vec<OutcomeEventInfo>,
    /// Per-session aggregates for sessions with at least one read.
    pub stats: Vec<SessionOutcomeInfo>,
}

/// One stored section as the retrieval layer serves it (the drill-in's
/// "as your AI sees it" view; gui-rw-file-drillin).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct IndexedSectionInfo {
    /// Human heading path (joined), empty for root sections.
    pub heading: String,
    /// The stored section text — exactly what retrieval serves.
    pub text: String,
}

/// The indexed view of one file.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct IndexedFileResponse {
    /// Stored sections in document order. Empty when the file isn't indexed.
    pub sections: Vec<IndexedSectionInfo>,
    /// False when no indexed document matches the path.
    pub found: bool,
}

/// A single SSE event for ingestion progress.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct IngestionProgressEvent {
    /// Phase: `"pending"`, `"running"`, `"complete"`, or `"failed"`.
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
    /// Terminal error message — set on the final event when `status == "failed"`.
    /// Lets clients surface the cause without re-querying.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Per-corpus ingestion-progress snapshot — the point-in-time form of the
/// live counters the daemon's per-corpus progress SSE streams. Unlike
/// [`IngestionProgressEvent`] (string `status`/`phase` for the SSE wire), this
/// mirrors the in-process `IngestionProgress` shape the desktop app consumes:
/// numeric `status` (0 = pending, 1 = running, 2 = complete). Returned in bulk
/// by `GET /api/v1/progress` so a GUI that no longer hosts its own indexer can
/// poll every corpus's progress over UDS (gd2b).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct IngestionProgressInfo {
    /// Corpus identifier this snapshot belongs to.
    pub corpus_id: String,
    /// Status: `0` = pending, `1` = running, `2` = complete.
    pub status: u8,
    /// Current ingestion phase (`"idle"`, `"discovering"`, `"parsing"`,
    /// `"embedding"`, `"finalizing"`).
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

/// Response for `GET /api/v1/progress` — one [`IngestionProgressInfo`] per
/// registered corpus (gd2b).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ProgressSnapshotResponse {
    /// Progress snapshot for every registered corpus.
    pub corpora: Vec<IngestionProgressInfo>,
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
    /// Filesystem path to the `.ministr-index` bundle file.
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

/// Request body for the daemon's clone-and-link endpoint
/// (`POST /api/v1/corpora/{parent_id}/clone`).
///
/// The daemon clones the repo into a managed directory under
/// `~/.ministr/clones/`, registers it as a new corpus, and appends a
/// `[[linked]]` entry to the parent corpus's `.ministr.toml` so future
/// MCP sessions can target the new corpus by label.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CloneRepoRequest {
    /// Remote git repository URL (HTTPS or SSH).
    pub repo: String,
    /// Optional sparse-checkout paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Optional branch or tag.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Optional human-readable label for the new linked project. When
    /// omitted, the daemon derives one from the repo URL (typically
    /// `owner-repo`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// F2.1 — when set AND an
    /// [`InstallationTokenMinter`](crate::github_app::InstallationTokenMinter)
    /// is wired into the daemon's `AppState`, the daemon mints a short-lived
    /// installation access token and splices it into the clone URL as
    /// `https://x-access-token:<token>@…`. Lets a Pro/Team customer
    /// clone a private repo without ever handing the daemon a PAT — the
    /// token is minted on demand per indexing job and discarded.
    ///
    /// When unset, the daemon clones `repo` verbatim (PAT-in-URL flow
    /// for self-hosted serve).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub github_installation_id: Option<String>,
}

/// Response from the daemon's clone-and-link endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CloneRepoResponse {
    /// Corpus ID assigned to the newly cloned project.
    pub corpus_id: String,
    /// Absolute filesystem path of the cloned working tree (this is the
    /// path written into the parent's `[[linked]]` entry).
    pub clone_dir: String,
    /// Label used in the parent's `[[linked]] label = "…"` entry. Use
    /// this value in future tool calls as `project: "<label>"`.
    pub label: String,
    /// Resolved commit SHA at clone time.
    pub commit_sha: String,
    /// Resolved branch name.
    pub branch: String,
    /// Whether the parent's `.ministr.toml` was updated (false if the
    /// link already existed and was a no-op).
    pub linked_toml_updated: bool,
    /// Whether indexing of the new corpus has started.
    pub indexing_started: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    // cq-status: the `Queued` variant must round-trip on the wire as
    // `{"state":"queued"}`, distinct from `idle`/`indexing`, so the GUI can
    // render a queued corpus separately from an actively-indexing one.
    #[test]
    fn indexing_status_queued_serializes_distinctly() {
        let json = serde_json::to_string(&IndexingStatus::Queued).unwrap();
        assert_eq!(json, r#"{"state":"queued"}"#);

        let back: IndexingStatus = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, IndexingStatus::Queued));

        // And it is NOT confused with idle or indexing.
        assert_ne!(
            serde_json::to_string(&IndexingStatus::Queued).unwrap(),
            serde_json::to_string(&IndexingStatus::Idle).unwrap()
        );
    }
}
