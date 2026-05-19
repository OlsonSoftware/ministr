//! Pluggable backend abstraction for MCP tool handlers.
//!
//! [`QueryBackend`] is the trait MCP tool handlers code against. Two concrete
//! implementations cover the two deployment shapes:
//!
//! - [`LocalBackend`] (in [`local`]) — calls an in-process [`QueryService`]
//!   directly.
//! - [`DaemonBackend`] (in [`daemon`]) — forwards every call over HTTP to a
//!   running `ministr-daemon` via [`DaemonClient`].
//!
//! [`Backend`] is a concrete enum that holds one of the two impls and also
//! implements [`QueryBackend`], so the MCP server can hold a single concrete
//! field without giving up the abstraction.
//!
//! Adding a third backend (mock for tests, remote-only over TLS, etc.) means
//! adding one module under `backend/` with `impl QueryBackend for NewBackend`
//! and one variant on [`Backend`]. No existing handler changes — Open/Closed.
//!
//! ## Out of scope
//!
//! `bridges` and `toc` are deliberately **not** in this trait yet — the
//! `ministr-api` wire types for those endpoints are leaner than the
//! service-layer types (no `binding_key` on `BridgeLink`; flattened
//! `TocEntry` `kind`/`source_path`). Promoting them would require richer
//! wire types (tracked alongside the `toc-schema-convergence` TODO in
//! `proxy.rs`). Those handlers still live separately in `MinistrServer` and
//! `ProxyServer` until the schemas converge.

// `manual_async_fn` is intentionally allowed: returning `impl Future`
// matches the project's existing `Storage` trait convention and avoids
// the async-fn-in-trait dyn-compatibility friction in current stable Rust.
#![allow(clippy::manual_async_fn)]

use std::future::Future;
use std::sync::Arc;

use ministr_api::client::{ClientError, DaemonClient};
use ministr_core::service::{
    ClaimResult, CompressedItem, DeadSymbol, ImpactResult, QueryError, QueryService,
    RelatedClaimResult, SectionDetail, SolidFinding, SolidParams, SurveyResult, SymbolDefinition,
    SymbolRefResult,
};
use ministr_core::storage::{BridgeLinkDetail, SymbolFilter, SymbolRecord};
use ministr_core::types::{RefKind, RelationType, TocEntry};
use thiserror::Error;

mod convert;
mod daemon;
mod daemon_multi;
mod local;

pub use daemon::DaemonBackend;
pub use daemon_multi::DaemonMultiBackend;
pub use local::LocalBackend;

/// Errors any backend can surface to MCP handlers.
#[derive(Debug, Error)]
pub enum BackendError {
    /// In-process query service failed.
    #[error(transparent)]
    Query(#[from] QueryError),
    /// HTTP forwarder failed.
    #[error(transparent)]
    Client(#[from] ClientError),
}

/// The abstract contract MCP tool handlers code against.
///
/// All methods return `impl Future` rather than `async fn` so the trait is
/// usable with generic dispatch (`B: QueryBackend`) while matching the
/// project's existing async-trait convention (see `Storage` in
/// `ministr-core/src/storage/traits.rs`).
pub trait QueryBackend: Send + Sync {
    /// Semantic search across the corpus.
    fn survey(
        &self,
        query: &str,
        top_k: usize,
    ) -> impl Future<Output = Result<Vec<SurveyResult>, BackendError>> + Send;

    /// Semantic search excluding content IDs already delivered in this
    /// session. Returns the result set plus a count of deduplicated IDs.
    ///
    /// The daemon backend ignores `exclude_ids` — it dedupes server-side
    /// using the `session_id` captured at construction. The local backend
    /// needs the exclude set explicitly.
    fn survey_with_exclude(
        &self,
        query: &str,
        top_k: usize,
        exclude_ids: &std::collections::HashSet<String>,
    ) -> impl Future<Output = Result<(Vec<SurveyResult>, usize), BackendError>> + Send;

    /// Read a section by ID.
    fn read_section(
        &self,
        section_id: &str,
    ) -> impl Future<Output = Result<SectionDetail, BackendError>> + Send;

    /// Pull atomic claims from a section, optionally query-filtered.
    fn extract_claims(
        &self,
        section_id: &str,
        query: Option<&str>,
    ) -> impl Future<Output = Result<Vec<ClaimResult>, BackendError>> + Send;

    /// Search the symbol index with optional filters.
    fn search_symbols(
        &self,
        filter: SymbolFilter,
    ) -> impl Future<Output = Result<Vec<SymbolRecord>, BackendError>> + Send;

    /// Full definition of a symbol by ID.
    fn definition(
        &self,
        symbol_id: &str,
    ) -> impl Future<Output = Result<SymbolDefinition, BackendError>> + Send;

    /// Callers, implementors, importers, and bridge links for a symbol.
    fn references(
        &self,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
    ) -> impl Future<Output = Result<Vec<SymbolRefResult>, BackendError>> + Send;

    /// Transitive blast radius of changing a symbol.
    fn impact(
        &self,
        symbol_id: &str,
        max_depth: u32,
    ) -> impl Future<Output = Result<ImpactResult, BackendError>> + Send;

    /// Zero-reference symbol candidates.
    fn dead_code(
        &self,
        kind: Option<&str>,
        module: Option<&str>,
        min_lines: u32,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<DeadSymbol>, BackendError>> + Send;

    /// Deterministic SOLID-violation candidates.
    fn solid(
        &self,
        params: &SolidParams,
    ) -> impl Future<Output = Result<Vec<SolidFinding>, BackendError>> + Send;

    /// Follow claim-relationship edges.
    fn related_claims(
        &self,
        claim_id: &str,
        relation_types: Option<&[RelationType]>,
    ) -> impl Future<Output = Result<Vec<RelatedClaimResult>, BackendError>> + Send;

    /// Extractive TF-IDF summarisation for a batch of content IDs.
    fn compress(
        &self,
        content_ids: &[String],
    ) -> impl Future<Output = Result<Vec<CompressedItem>, BackendError>> + Send;

    /// Structural TOC entries for the corpus or a specific document.
    ///
    /// The daemon backend currently returns lossy entries (`api::TocEntry`
    /// flattens away `document_id`, `section_id`, `claims_available`,
    /// `token_count`); daemon-mode handlers tolerate the missing fields
    /// with defaults. Tracked alongside `toc-schema-convergence` in
    /// `proxy.rs`.
    fn toc(
        &self,
        document_id: Option<&str>,
    ) -> impl Future<Output = Result<Vec<TocEntry>, BackendError>> + Send;

    /// Cross-language bridge links with optional filters.
    ///
    /// Daemon backend is similarly lossy here (`api::BridgeLink` drops
    /// `binding_key`, per-endpoint file/line). Same TODO as `toc`.
    fn bridges(
        &self,
        query: Option<&str>,
        kind: Option<&str>,
        language: Option<&str>,
        file_path: Option<&str>,
    ) -> impl Future<Output = Result<Vec<BridgeLinkDetail>, BackendError>> + Send;
}

// ---------------------------------------------------------------------------
// Backend — concrete enum dispatch.
// ---------------------------------------------------------------------------

/// Concrete dispatching wrapper holding one of the backend impls.
///
/// `MinistrServer` holds this directly rather than `Arc<dyn QueryBackend>`
/// so the rmcp `#[tool_router]` macro can work with a concrete receiver.
/// `Backend` implements [`QueryBackend`] for the single-corpus path
/// (project = None implicit) and adds a parallel set of project-aware
/// inherent methods used by tool handlers that carry a `project`
/// argument; those resolve labels via [`DaemonMultiBackend`] when this
/// is the multi-corpus variant, or ignore the label otherwise.
#[derive(Clone)]
pub enum Backend {
    Local(Arc<LocalBackend>),
    Daemon(Arc<DaemonBackend>),
    DaemonMulti(Arc<DaemonMultiBackend>),
}

impl Backend {
    /// Construct a local backend from an existing [`QueryService`].
    #[must_use]
    pub fn local(service: Arc<QueryService>) -> Self {
        Self::Local(Arc::new(LocalBackend::new(service)))
    }

    /// Construct a daemon-forwarding backend bound to a corpus + session.
    #[must_use]
    pub fn daemon(
        client: Arc<DaemonClient>,
        corpus_id: String,
        session_id: Option<String>,
    ) -> Self {
        Self::Daemon(Arc::new(DaemonBackend::new(client, corpus_id, session_id)))
    }

    /// Construct a multi-corpus daemon-forwarding backend.
    #[must_use]
    pub fn daemon_multi(multi: DaemonMultiBackend) -> Self {
        Self::DaemonMulti(Arc::new(multi))
    }

    /// Return the underlying [`QueryService`] if this is a local backend.
    /// Escape hatch for handlers not yet migrated to the trait.
    #[must_use]
    pub fn as_local(&self) -> Option<&Arc<QueryService>> {
        match self {
            Self::Local(b) => Some(b.service()),
            Self::Daemon(_) | Self::DaemonMulti(_) => None,
        }
    }

    /// Resolve a project label to the concrete daemon backend that should
    /// answer the call. Returns `None` for non-daemon variants.
    ///
    /// `project = None` always returns the default / session-primary
    /// daemon backend. An unknown label falls back to the default (see
    /// [`DaemonMultiBackend::for_project`]).
    #[must_use]
    pub fn daemon_for_project(&self, project: Option<&str>) -> Option<&Arc<DaemonBackend>> {
        match self {
            Self::Local(_) => None,
            Self::Daemon(b) => Some(b),
            Self::DaemonMulti(m) => Some(m.for_project(project)),
        }
    }

    /// List the linked-project labels available on this backend.
    /// Empty when this is a single-corpus backend.
    #[must_use]
    pub fn linked_labels(&self) -> Vec<String> {
        match self {
            Self::DaemonMulti(m) => m.labels(),
            Self::Local(_) | Self::Daemon(_) => Vec::new(),
        }
    }
}

/// Inherent project-aware dispatch methods.
///
/// Handlers call these instead of the `QueryBackend` trait directly so
/// they can route to a linked project by label. For [`Self::Local`] and
/// [`Self::Daemon`] (single-corpus variants) the `project` argument is
/// ignored — there's only one corpus to dispatch to. For
/// [`Self::DaemonMulti`] the label is resolved via
/// [`DaemonMultiBackend::for_project`].
///
/// Every method returns [`BackendError`] (transparently wrapping the
/// underlying [`QueryError`] or [`ClientError`]); per-method `# Errors`
/// blocks are omitted here because the failure mode is the same shape
/// across the entire surface.
#[allow(clippy::missing_errors_doc)]
impl Backend {
    pub async fn survey(
        &self,
        project: Option<&str>,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SurveyResult>, BackendError> {
        match self {
            Self::Local(b) => b.survey(query, top_k).await,
            Self::Daemon(b) => b.survey(query, top_k).await,
            Self::DaemonMulti(m) => m.for_project(project).survey(query, top_k).await,
        }
    }

    pub async fn survey_with_exclude(
        &self,
        project: Option<&str>,
        query: &str,
        top_k: usize,
        exclude_ids: &std::collections::HashSet<String>,
    ) -> Result<(Vec<SurveyResult>, usize), BackendError> {
        match self {
            Self::Local(b) => b.survey_with_exclude(query, top_k, exclude_ids).await,
            Self::Daemon(b) => b.survey_with_exclude(query, top_k, exclude_ids).await,
            Self::DaemonMulti(m) => {
                m.for_project(project)
                    .survey_with_exclude(query, top_k, exclude_ids)
                    .await
            }
        }
    }

    pub async fn read_section(
        &self,
        project: Option<&str>,
        section_id: &str,
    ) -> Result<SectionDetail, BackendError> {
        match self {
            Self::Local(b) => b.read_section(section_id).await,
            Self::Daemon(b) => b.read_section(section_id).await,
            Self::DaemonMulti(m) => m.for_project(project).read_section(section_id).await,
        }
    }

    pub async fn extract_claims(
        &self,
        project: Option<&str>,
        section_id: &str,
        query: Option<&str>,
    ) -> Result<Vec<ClaimResult>, BackendError> {
        match self {
            Self::Local(b) => b.extract_claims(section_id, query).await,
            Self::Daemon(b) => b.extract_claims(section_id, query).await,
            Self::DaemonMulti(m) => {
                m.for_project(project)
                    .extract_claims(section_id, query)
                    .await
            }
        }
    }

    pub async fn search_symbols(
        &self,
        project: Option<&str>,
        filter: SymbolFilter,
    ) -> Result<Vec<SymbolRecord>, BackendError> {
        match self {
            Self::Local(b) => b.search_symbols(filter).await,
            Self::Daemon(b) => b.search_symbols(filter).await,
            Self::DaemonMulti(m) => m.for_project(project).search_symbols(filter).await,
        }
    }

    pub async fn definition(
        &self,
        project: Option<&str>,
        symbol_id: &str,
    ) -> Result<SymbolDefinition, BackendError> {
        match self {
            Self::Local(b) => b.definition(symbol_id).await,
            Self::Daemon(b) => b.definition(symbol_id).await,
            Self::DaemonMulti(m) => m.for_project(project).definition(symbol_id).await,
        }
    }

    pub async fn references(
        &self,
        project: Option<&str>,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
    ) -> Result<Vec<SymbolRefResult>, BackendError> {
        match self {
            Self::Local(b) => b.references(symbol_id, ref_kind).await,
            Self::Daemon(b) => b.references(symbol_id, ref_kind).await,
            Self::DaemonMulti(m) => m.for_project(project).references(symbol_id, ref_kind).await,
        }
    }

    pub async fn impact(
        &self,
        project: Option<&str>,
        symbol_id: &str,
        max_depth: u32,
    ) -> Result<ImpactResult, BackendError> {
        match self {
            Self::Local(b) => b.impact(symbol_id, max_depth).await,
            Self::Daemon(b) => b.impact(symbol_id, max_depth).await,
            Self::DaemonMulti(m) => m.for_project(project).impact(symbol_id, max_depth).await,
        }
    }

    pub async fn dead_code(
        &self,
        project: Option<&str>,
        kind: Option<&str>,
        module: Option<&str>,
        min_lines: u32,
        limit: usize,
    ) -> Result<Vec<DeadSymbol>, BackendError> {
        match self {
            Self::Local(b) => b.dead_code(kind, module, min_lines, limit).await,
            Self::Daemon(b) => b.dead_code(kind, module, min_lines, limit).await,
            Self::DaemonMulti(m) => {
                m.for_project(project)
                    .dead_code(kind, module, min_lines, limit)
                    .await
            }
        }
    }

    pub async fn solid(
        &self,
        project: Option<&str>,
        params: &SolidParams,
    ) -> Result<Vec<SolidFinding>, BackendError> {
        match self {
            Self::Local(b) => b.solid(params).await,
            Self::Daemon(b) => b.solid(params).await,
            Self::DaemonMulti(m) => m.for_project(project).solid(params).await,
        }
    }

    pub async fn related_claims(
        &self,
        project: Option<&str>,
        claim_id: &str,
        relation_types: Option<&[RelationType]>,
    ) -> Result<Vec<RelatedClaimResult>, BackendError> {
        match self {
            Self::Local(b) => b.related_claims(claim_id, relation_types).await,
            Self::Daemon(b) => b.related_claims(claim_id, relation_types).await,
            Self::DaemonMulti(m) => {
                m.for_project(project)
                    .related_claims(claim_id, relation_types)
                    .await
            }
        }
    }

    pub async fn compress(
        &self,
        project: Option<&str>,
        content_ids: &[String],
    ) -> Result<Vec<CompressedItem>, BackendError> {
        match self {
            Self::Local(b) => b.compress(content_ids).await,
            Self::Daemon(b) => b.compress(content_ids).await,
            Self::DaemonMulti(m) => m.for_project(project).compress(content_ids).await,
        }
    }

    pub async fn toc(
        &self,
        project: Option<&str>,
        document_id: Option<&str>,
    ) -> Result<Vec<TocEntry>, BackendError> {
        match self {
            Self::Local(b) => b.toc(document_id).await,
            Self::Daemon(b) => b.toc(document_id).await,
            Self::DaemonMulti(m) => m.for_project(project).toc(document_id).await,
        }
    }

    pub async fn bridges(
        &self,
        project: Option<&str>,
        query: Option<&str>,
        kind: Option<&str>,
        language: Option<&str>,
        file_path: Option<&str>,
    ) -> Result<Vec<BridgeLinkDetail>, BackendError> {
        match self {
            Self::Local(b) => b.bridges(query, kind, language, file_path).await,
            Self::Daemon(b) => b.bridges(query, kind, language, file_path).await,
            Self::DaemonMulti(m) => {
                m.for_project(project)
                    .bridges(query, kind, language, file_path)
                    .await
            }
        }
    }
}
