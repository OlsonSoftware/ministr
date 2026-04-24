//! The [`Storage`] trait defines the async persistence interface for ministr-core.
//!
//! All operations are async to allow implementations to use `spawn_blocking`
//! or other async-safe wrappers around synchronous backends like `SQLite`.

use std::future::Future;

use crate::error::StorageError;
use crate::session::{Session, SessionId};
use crate::types::{
    ClaimId, ClaimRelationship, ContentId, CorpusRoot, DocumentTree, RefKind, RelationType,
    SectionId, SymbolId,
};

/// Stored document metadata (without the full section tree).
///
/// # Examples
///
/// ```
/// use ministr_core::storage::DocumentRecord;
/// use ministr_core::types::ContentId;
///
/// let record = DocumentRecord {
///     id: ContentId("docs/api.md".into()),
///     title: "API Reference".into(),
///     source_path: "docs/api.md".into(),
///     summary: Some("Full API reference.".into()),
///     root_id: None,
/// };
/// assert_eq!(record.title, "API Reference");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRecord {
    /// Unique document ID.
    pub id: ContentId,
    /// Document title.
    pub title: String,
    /// Source file path relative to corpus root.
    pub source_path: String,
    /// Document-level summary.
    pub summary: Option<String>,
    /// Corpus root this document belongs to (if multi-root indexing is active).
    pub root_id: Option<String>,
}

/// Stored section with its metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionRecord {
    /// Unique section ID.
    pub id: SectionId,
    /// Parent document ID.
    pub document_id: ContentId,
    /// Heading hierarchy path.
    pub heading_path: Vec<String>,
    /// Heading depth.
    pub depth: u32,
    /// Full text content.
    pub text: String,
    /// Section summary.
    pub summary: Option<String>,
    /// Ordering position within the document.
    pub position: i64,
}

/// Stored claim with its metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimRecord {
    /// Unique claim ID.
    pub id: ClaimId,
    /// Parent section ID.
    pub section_id: SectionId,
    /// Claim text.
    pub text: String,
    /// Ordering position within the section.
    pub position: i64,
}

/// A related claim record returned by relationship queries.
#[derive(Debug, Clone, PartialEq)]
pub struct RelatedClaimRecord {
    /// The related claim's ID.
    pub claim_id: ClaimId,
    /// The related claim's text.
    pub text: String,
    /// The type of relationship.
    pub relation_type: RelationType,
    /// The section containing the related claim.
    pub section_id: SectionId,
    /// Confidence score (0.0–1.0).
    pub confidence: f32,
}

/// A section access statistics record for cross-session analytics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionAccessStat {
    /// Section ID.
    pub section_id: SectionId,
    /// Total number of times this section was accessed across all sessions.
    pub access_count: u64,
    /// Timestamp of the most recent access.
    pub last_accessed: String,
}

/// A co-access pattern record: two sections frequently accessed together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoAccessRecord {
    /// The partner section ID (the one co-accessed with the queried section).
    pub section_id: SectionId,
    /// Number of sessions in which both sections were accessed.
    pub co_count: u64,
}

/// Aggregate corpus statistics derived from cross-session analytics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusStats {
    /// Total number of section accesses across all sessions.
    pub total_accesses: u64,
    /// Number of unique sections ever accessed.
    pub unique_sections_accessed: u64,
    /// Number of co-access pairs recorded.
    pub co_access_pairs: u64,
}

/// A file hash record for incremental re-indexing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHashRecord {
    /// File path relative to corpus root.
    pub path: String,
    /// Content hash (e.g. SHA-256 hex).
    pub content_hash: String,
    /// File modification time in nanoseconds since epoch (for fast mtime pre-check).
    pub mtime_ns: Option<i64>,
    /// Version of the extractor pipeline that produced the cached refs /
    /// symbols for this file. Compared against
    /// [`crate::ingestion::EXTRACTOR_VERSION`] on re-ingest — mismatches
    /// force re-parsing even when `content_hash` is unchanged, so the
    /// index auto-heals after extractor-logic changes. Pre-versioning
    /// rows (from migration V1–V18) read back as `0`.
    pub extractor_version: i64,
}

/// A web cache record tracking fetch metadata for staleness detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebCacheRecord {
    /// The source URL (primary key).
    pub source_url: String,
    /// ISO 8601 timestamp of the last fetch.
    pub fetch_timestamp: String,
    /// HTTP `ETag` header from the last response.
    pub etag: Option<String>,
    /// HTTP `Last-Modified` header from the last response.
    pub last_modified: Option<String>,
    /// SHA-256 hex digest of the fetched content.
    pub content_hash: String,
    /// Content-Type from the HTTP response.
    pub content_type: Option<String>,
}

/// A git cache record tracking clone metadata for staleness detection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCacheRecord {
    /// The remote repository URL (primary key).
    pub repo_url: String,
    /// The branch that was cloned (None for default branch).
    pub branch: Option<String>,
    /// The commit SHA at clone time.
    pub commit_sha: String,
    /// Epoch seconds timestamp of the clone.
    pub clone_timestamp: String,
    /// Path to the clone directory on disk.
    pub clone_dir: String,
    /// Paths that were checked out via sparse checkout (empty = full checkout).
    pub checked_out_paths: Vec<String>,
}

/// A stored code symbol with its metadata.
///
/// # Examples
///
/// ```
/// use ministr_core::storage::SymbolRecord;
/// use ministr_core::types::SymbolId;
///
/// let record = SymbolRecord {
///     id: SymbolId("sym-config::MinistrConfig".into()),
///     file_path: "src/config.rs".into(),
///     name: "MinistrConfig".into(),
///     kind: "struct".into(),
///     visibility: "pub".into(),
///     signature: "pub struct MinistrConfig".into(),
///     doc_comment: Some("Configuration for ministr.".into()),
///     module_path: "config".into(),
///     line_start: 10,
///     line_end: 25,
///     cyclomatic_complexity: None,
/// };
/// assert_eq!(record.name, "MinistrConfig");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRecord {
    /// Unique symbol ID.
    pub id: SymbolId,
    /// Source file path relative to corpus root.
    pub file_path: String,
    /// Symbol name.
    pub name: String,
    /// Symbol kind (e.g. "function", "struct", "trait").
    pub kind: String,
    /// Visibility (e.g. "pub", "pub(crate)", "").
    pub visibility: String,
    /// Declaration signature (without body).
    pub signature: String,
    /// Doc comment text, if present.
    pub doc_comment: Option<String>,
    /// Module path (e.g. `config::sub` for nested modules).
    pub module_path: String,
    /// Start line number (1-based).
    pub line_start: u32,
    /// End line number (1-based, inclusive).
    pub line_end: u32,
    /// Cyclomatic complexity (only set for function symbols).
    pub cyclomatic_complexity: Option<u32>,
}

/// A stored cross-reference between two symbols.
///
/// # Examples
///
/// ```
/// use ministr_core::storage::SymbolRefRecord;
/// use ministr_core::types::{SymbolId, RefKind};
///
/// let record = SymbolRefRecord {
///     from_symbol_id: SymbolId("sym-main::run".into()),
///     to_symbol_id: SymbolId("sym-config::MinistrConfig".into()),
///     ref_kind: RefKind::Uses,
/// };
/// assert_eq!(record.ref_kind, RefKind::Uses);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolRefRecord {
    /// The symbol that references another.
    pub from_symbol_id: SymbolId,
    /// The symbol being referenced.
    pub to_symbol_id: SymbolId,
    /// The kind of reference.
    pub ref_kind: RefKind,
}

/// A stored bridge endpoint record.
/// A reference that could not be resolved during ingestion, persisted for
/// deferred resolution on subsequent warm restarts.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingRefRecord {
    /// Symbol ID of the referencing side.
    pub from_symbol_id: String,
    /// Name of the target symbol that was not found.
    pub target_name: String,
    /// Kind of reference (`implements`, `imports`, etc.).
    pub kind: String,
    /// Source file path.
    pub file_path: String,
    /// Optional target crate hint for cross-crate disambiguation.
    pub target_crate: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeEndpointRecord {
    /// Auto-generated row ID (set after insert).
    pub id: Option<i64>,
    /// Source file path relative to corpus root.
    pub file_path: String,
    /// Canonical binding key for cross-language joining.
    pub binding_key: String,
    /// Bridge mechanism kind (e.g. `"tauri_command"`).
    pub kind: String,
    /// Endpoint role: `"export"` or `"import"`.
    pub role: String,
    /// Source language (e.g. `"rust"`, `"typescript"`).
    pub language: String,
    /// Source line number.
    pub line: u32,
    /// Symbol name as it appears in source.
    pub symbol_name: String,
    /// Confidence score in the range `0.0..=1.0`.
    pub confidence: f32,
}

/// A stored bridge link joining an export and import endpoint.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeLinkRecord {
    /// Export endpoint row ID.
    pub export_ep_id: i64,
    /// Import endpoint row ID.
    pub import_ep_id: i64,
    /// Bridge mechanism kind.
    pub kind: String,
    /// Combined confidence: `min(export, import)`.
    pub confidence: f32,
}

/// A detailed bridge link result with endpoint information inlined.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeLinkDetail {
    /// Bridge mechanism kind.
    pub kind: String,
    /// Combined confidence.
    pub confidence: f32,
    /// Export endpoint file path.
    pub export_file: String,
    /// Export binding key.
    pub export_binding_key: String,
    /// Export symbol name.
    pub export_symbol: String,
    /// Export language.
    pub export_language: String,
    /// Export line number.
    pub export_line: u32,
    /// Import endpoint file path.
    pub import_file: String,
    /// Import binding key.
    pub import_binding_key: String,
    /// Import symbol name.
    pub import_symbol: String,
    /// Import language.
    pub import_language: String,
    /// Import line number.
    pub import_line: u32,
}

/// A cached answer from the `answer_cache` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedAnswer {
    /// SHA-256 hash of the normalized query string.
    pub query_hash: String,
    /// Original query text (for debugging and display).
    pub query_text: String,
    /// The synthesized answer.
    pub answer: String,
    /// Model used for synthesis (e.g. "haiku").
    pub model: String,
    /// Token count of the answer.
    pub token_count: usize,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
}

/// A source section that contributed to a cached answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedAnswerSource {
    /// Section ID that was retrieved for this answer.
    pub section_id: String,
    /// SHA-256 hash of the section's text at synthesis time.
    pub section_hash: String,
}

/// Filter criteria for querying the symbol index.
///
/// All fields are optional — `None` means "no filter" for that field.
#[derive(Debug, Clone, Default)]
pub struct SymbolFilter {
    /// Fuzzy name match (case-insensitive substring).
    pub name: Option<String>,
    /// Exact name match (case-sensitive). Takes precedence over `name` when set.
    pub name_exact: Option<String>,
    /// Exact kind match (e.g. "function", "struct").
    pub kind: Option<String>,
    /// Exact visibility match (e.g. "pub").
    pub visibility: Option<String>,
    /// Module path prefix match (e.g. "config" matches `config::sub`).
    pub module: Option<String>,
    /// File path match.
    pub file_path: Option<String>,
}

/// Async storage interface for the ministr content database.
///
/// Implementations must be `Send + Sync` to work across async tasks.
pub trait Storage: Send + Sync {
    // -- Documents --

    /// Insert a full document tree (document + sections + claims).
    fn insert_document(
        &self,
        doc: &DocumentTree,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Get a document by ID (metadata only, no sections).
    fn get_document(
        &self,
        id: &ContentId,
    ) -> impl Future<Output = Result<Option<DocumentRecord>, StorageError>> + Send;

    /// Count documents in the corpus (lighter than listing all).
    fn document_count(&self) -> impl Future<Output = Result<usize, StorageError>> + Send;

    /// Count sections in the corpus (lighter than listing all).
    fn section_count(&self) -> impl Future<Output = Result<usize, StorageError>> + Send;

    /// Count symbols in the corpus (lighter than listing all).
    fn symbol_count(&self) -> impl Future<Output = Result<usize, StorageError>> + Send;

    /// List all documents in the corpus.
    fn list_documents(
        &self,
    ) -> impl Future<Output = Result<Vec<DocumentRecord>, StorageError>> + Send;

    /// List documents belonging to a specific corpus root.
    fn list_documents_by_root(
        &self,
        root_id: &str,
    ) -> impl Future<Output = Result<Vec<DocumentRecord>, StorageError>> + Send;

    /// Delete a document and all its sections/claims (cascading).
    fn delete_document(
        &self,
        id: &ContentId,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    // -- Sections --

    /// Get a section by ID.
    fn get_section(
        &self,
        id: &SectionId,
    ) -> impl Future<Output = Result<Option<SectionRecord>, StorageError>> + Send;

    /// List all sections for a document.
    fn list_sections(
        &self,
        document_id: &ContentId,
    ) -> impl Future<Output = Result<Vec<SectionRecord>, StorageError>> + Send;

    /// Get the next section after the given section within the same document.
    ///
    /// Returns the section with the next higher position value, or `None`
    /// if this is the last section in the document.
    fn get_next_section(
        &self,
        section_id: &SectionId,
    ) -> impl Future<Output = Result<Option<SectionRecord>, StorageError>> + Send;

    /// Get the parent document for a given section.
    ///
    /// Returns the document record that contains the given section, or `None`
    /// if the section does not exist.
    fn get_document_for_section(
        &self,
        section_id: &SectionId,
    ) -> impl Future<Output = Result<Option<DocumentRecord>, StorageError>> + Send;

    // -- Claims --

    /// Get a claim by ID.
    fn get_claim(
        &self,
        id: &ClaimId,
    ) -> impl Future<Output = Result<Option<ClaimRecord>, StorageError>> + Send;

    /// List all claims for a section.
    fn list_claims(
        &self,
        section_id: &SectionId,
    ) -> impl Future<Output = Result<Vec<ClaimRecord>, StorageError>> + Send;

    // -- Claim relationships --

    /// Insert a batch of claim relationships.
    fn insert_claim_relationships(
        &self,
        relationships: &[ClaimRelationship],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Get claims related to the given claim, optionally filtered by relation type.
    fn get_related_claims(
        &self,
        claim_id: &ClaimId,
        relation_types: Option<&[RelationType]>,
    ) -> impl Future<Output = Result<Vec<RelatedClaimRecord>, StorageError>> + Send;

    /// Delete all relationships involving claims in the given section.
    ///
    /// Used during re-indexing to clean up stale relationships.
    fn delete_relationships_for_section(
        &self,
        section_id: &SectionId,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    // -- File hashes --

    /// Upsert a file hash record (insert or update on conflict).
    fn upsert_file_hash(
        &self,
        record: &FileHashRecord,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Get the stored hash for a file path.
    fn get_file_hash(
        &self,
        path: &str,
    ) -> impl Future<Output = Result<Option<FileHashRecord>, StorageError>> + Send;

    /// Delete a file hash record.
    fn delete_file_hash(
        &self,
        path: &str,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    /// List all file hash records (for manifest-level mtime fast skip).
    fn list_file_hashes(
        &self,
    ) -> impl Future<Output = Result<Vec<FileHashRecord>, StorageError>> + Send;

    // -- Sessions --

    /// Save a session to persistent storage for crash recovery.
    ///
    /// Uses upsert semantics — creates the session if it doesn't exist,
    /// or replaces all delivered items if it does.
    fn save_session(
        &self,
        session: &Session,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Load a previously persisted session by ID.
    ///
    /// Returns `None` if no session with the given ID exists.
    fn load_session(
        &self,
        id: &SessionId,
    ) -> impl Future<Output = Result<Option<Session>, StorageError>> + Send;

    /// Delete a persisted session and all its delivery records.
    fn delete_session(
        &self,
        id: &SessionId,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    // -- Cross-session analytics --

    /// Record a section access, incrementing its access count.
    fn record_section_access(
        &self,
        section_id: &SectionId,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Record co-access patterns for sections accessed in the same session.
    ///
    /// For each unique pair in the provided list, increments the co-access count.
    fn record_co_accesses(
        &self,
        section_ids: &[SectionId],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Record a pre-computed set of co-access pairs.
    ///
    /// Unlike [`Storage::record_co_accesses`], which derives all pairs
    /// from a single trajectory, this method accepts pairs directly. It
    /// is the primitive used by `Analytics::record_co_access_incremental`
    /// to record only the NEW pairs that arose since the last flush,
    /// without double-counting pairs already recorded.
    ///
    /// Self-pairs (where both sides are equal) are skipped defensively.
    fn record_co_access_pairs(
        &self,
        pairs: &[(SectionId, SectionId)],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Get the most frequently accessed sections.
    fn get_top_sections(
        &self,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<SectionAccessStat>, StorageError>> + Send;

    /// Get sections most frequently co-accessed with the given section.
    fn get_co_accessed(
        &self,
        section_id: &SectionId,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<CoAccessRecord>, StorageError>> + Send;

    /// Get aggregate corpus analytics statistics.
    fn get_corpus_stats(&self) -> impl Future<Output = Result<CorpusStats, StorageError>> + Send;

    // -- Web cache --

    /// Upsert a web cache record (insert or update on conflict).
    fn upsert_web_cache(
        &self,
        record: &WebCacheRecord,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Get the cached metadata for a URL.
    fn get_web_cache(
        &self,
        url: &str,
    ) -> impl Future<Output = Result<Option<WebCacheRecord>, StorageError>> + Send;

    /// List all cached web URLs.
    fn list_web_cache(
        &self,
    ) -> impl Future<Output = Result<Vec<WebCacheRecord>, StorageError>> + Send;

    /// Delete a web cache record.
    fn delete_web_cache(
        &self,
        url: &str,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    // -- Git cache --

    /// Upsert a git cache record (insert or update on conflict).
    fn upsert_git_cache(
        &self,
        record: &GitCacheRecord,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Get the cached metadata for a repository URL.
    fn get_git_cache(
        &self,
        repo_url: &str,
    ) -> impl Future<Output = Result<Option<GitCacheRecord>, StorageError>> + Send;

    /// List all cached git clones.
    fn list_git_cache(
        &self,
    ) -> impl Future<Output = Result<Vec<GitCacheRecord>, StorageError>> + Send;

    /// Delete a git cache record.
    fn delete_git_cache(
        &self,
        repo_url: &str,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    // -- Symbols --

    /// Insert a batch of symbols (upsert: replaces on ID conflict).
    fn insert_symbols(
        &self,
        symbols: &[SymbolRecord],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// List symbols matching the given filter criteria.
    fn list_symbols(
        &self,
        filter: &SymbolFilter,
    ) -> impl Future<Output = Result<Vec<SymbolRecord>, StorageError>> + Send;

    /// Get a single symbol by ID.
    fn get_symbol(
        &self,
        id: &SymbolId,
    ) -> impl Future<Output = Result<Option<SymbolRecord>, StorageError>> + Send;

    /// Delete all symbols belonging to a given file path.
    ///
    /// Used during re-indexing to clean up stale symbols.
    fn delete_symbols_for_file(
        &self,
        file_path: &str,
    ) -> impl Future<Output = Result<u64, StorageError>> + Send;

    // -- Symbol references --

    /// Insert a batch of symbol cross-references.
    fn insert_symbol_refs(
        &self,
        refs: &[SymbolRefRecord],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Query references for a symbol, optionally filtered by reference kind.
    fn query_refs(
        &self,
        symbol_id: &SymbolId,
        ref_kind: Option<RefKind>,
    ) -> impl Future<Output = Result<Vec<SymbolRefRecord>, StorageError>> + Send;

    /// Delete all references involving symbols in the given file.
    fn delete_refs_for_file(
        &self,
        file_path: &str,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Compute transitive caller counts for a batch of symbols.
    ///
    /// Returns a map from symbol ID to the number of unique symbols that
    /// transitively call into it (following `Calls` `ref_kind` edges).
    fn transitive_caller_counts(
        &self,
        symbol_ids: &[SymbolId],
    ) -> impl Future<Output = Result<std::collections::HashMap<SymbolId, u32>, StorageError>> + Send;

    // -- Bridge endpoints & links --

    /// Insert a batch of bridge endpoints, returning their auto-generated row IDs.
    fn insert_bridge_endpoints(
        &self,
        endpoints: &[BridgeEndpointRecord],
    ) -> impl Future<Output = Result<Vec<i64>, StorageError>> + Send;

    /// Insert a batch of bridge links between previously inserted endpoints.
    fn insert_bridge_links(
        &self,
        links: &[BridgeLinkRecord],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Query bridge links with optional filters.
    ///
    /// When `file_path` is provided, returns links where either the export
    /// or import endpoint is in that file. When `kind` is provided, filters
    /// to that bridge mechanism.
    fn query_bridge_links(
        &self,
        file_path: Option<&str>,
        kind: Option<&str>,
    ) -> impl Future<Output = Result<Vec<BridgeLinkDetail>, StorageError>> + Send;

    /// Delete all bridge endpoints and links for a given file path.
    ///
    /// Used during re-indexing to clean up stale bridge data.
    fn delete_bridge_data_for_file(
        &self,
        file_path: &str,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Truncate every bridge endpoint and link in this corpus.
    ///
    /// Bridge data is a global view derived from current extractor logic.
    /// Per-file content hashes do NOT capture extractor-rule changes, so
    /// a full ingest pass always clears and rebuilds bridge tables from
    /// scratch to keep them in sync with the current extractors.
    fn clear_bridge_data(&self) -> impl Future<Output = Result<(), StorageError>> + Send;

    // -- Pending refs (deferred resolution queue) --

    /// Insert or replace pending refs that could not be resolved in the current pass.
    fn upsert_pending_refs(
        &self,
        refs: &[PendingRefRecord],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Load all pending refs for deferred resolution.
    fn list_pending_refs(
        &self,
    ) -> impl Future<Output = Result<Vec<PendingRefRecord>, StorageError>> + Send;

    /// Delete pending refs that have been successfully resolved.
    fn delete_pending_refs(
        &self,
        refs: &[PendingRefRecord],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    // -- Corpus roots --

    /// Upsert a corpus root (insert or update on conflict).
    fn upsert_corpus_root(
        &self,
        root: &CorpusRoot,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Get a corpus root by ID.
    fn get_corpus_root(
        &self,
        id: &str,
    ) -> impl Future<Output = Result<Option<CorpusRoot>, StorageError>> + Send;

    /// List all registered corpus roots.
    fn list_corpus_roots(
        &self,
    ) -> impl Future<Output = Result<Vec<CorpusRoot>, StorageError>> + Send;

    /// Delete a corpus root by ID.
    fn delete_corpus_root(
        &self,
        id: &str,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    /// Tag a document with its corpus root.
    fn set_document_root(
        &self,
        doc_id: &ContentId,
        root_id: &str,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;
}
