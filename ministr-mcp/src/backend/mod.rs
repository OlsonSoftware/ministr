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
//! `bridges` and `toc` are part of this trait. Their `ministr-api` wire types
//! (`BridgeLink`, `TocEntry`) were once leaner than the service-layer types,
//! but the schema-convergence work enriched them — `TocEntry` carries
//! `heading_path`/`claims_available`/`token_count` and `BridgeLink` carries the
//! per-endpoint binding key/symbol/file/line — so the `backend::convert`
//! converters are lossless for the agent-facing fields and daemon mode is at
//! parity with local mode.

// `manual_async_fn` is intentionally allowed: returning `impl Future`
// matches the project's existing `Storage` trait convention and avoids
// the async-fn-in-trait dyn-compatibility friction in current stable Rust.
#![allow(clippy::manual_async_fn)]

use std::future::Future;
use std::sync::Arc;

use ministr_api::TenantCorpusFilter;
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
    /// As of the toc-schema-convergence work the daemon backend is at parity
    /// with the local backend: `api::TocEntry` carries `heading_path`,
    /// `claims_available`, and `token_count`, and `document_id` rides on
    /// `source_path`, so daemon-mode TOC entries are no longer lossy.
    fn toc(
        &self,
        document_id: Option<&str>,
    ) -> impl Future<Output = Result<Vec<TocEntry>, BackendError>> + Send;

    /// Cross-language bridge links with optional filters.
    ///
    /// As of the schema-convergence work the daemon backend is at parity with
    /// the local backend: `api::BridgeLink` carries the per-endpoint binding
    /// key, symbol, file, and line, and `api::BridgeRequest` carries
    /// `file_path`, so neither the result fields nor the `file_path` filter
    /// are dropped in daemon mode.
    fn bridges(
        &self,
        query: Option<&str>,
        kind: Option<&str>,
        language: Option<&str>,
        file_path: Option<&str>,
    ) -> impl Future<Output = Result<Vec<BridgeLinkDetail>, BackendError>> + Send;

    /// Resolve a file position (1-based `line`, 0-based byte `col`) to the
    /// symbol id of the identifier under the cursor, or `None` when the
    /// position covers no occurrence. The position→symbol bridge (FL2) that
    /// makes [`Self::definition`]/[`Self::references`] position-addressable.
    fn symbol_at_position(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> impl Future<Output = Result<Option<String>, BackendError>> + Send;
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
    /// F2.x-a — cloud mode. `default_service` answers calls with no
    /// `project` argument (compatibility with single-corpus tools);
    /// `registry` resolves a `project = corpus_id` argument through the
    /// shared daemon registry, including the lazy blob-restore path
    /// wired by `cmd_serve_http`. Restoring a corpus on demand means
    /// every `/mcp` tool call observes the same source of truth the
    /// REST surface does — without this variant the MCP layer routes
    /// every call through `default_service`, which is bound to an
    /// empty placeholder corpus on a fresh pod.
    ///
    /// F2.x-b — `tenant_filter`, when wired, gates the `project →
    /// corpus_id` lookup. When the caller threads a `tenant_subject` and
    /// the filter denies, the resolver returns `Err(default_service)`
    /// (same shape as a typo) so the cross-tenant probe does not leak
    /// corpus existence. `None` filter ⇒ legacy permissive behaviour
    /// (self-hosted / single-tenant serve).
    Registry {
        default_service: Arc<QueryService>,
        registry: Arc<ministr_daemon::registry::CorpusRegistry>,
        tenant_filter: Option<Arc<dyn TenantCorpusFilter>>,
    },
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

    /// Construct a cloud-mode backend that dispatches per-call through
    /// a shared [`CorpusRegistry`](ministr_daemon::registry::CorpusRegistry).
    #[must_use]
    pub fn registry(
        default_service: Arc<QueryService>,
        registry: Arc<ministr_daemon::registry::CorpusRegistry>,
    ) -> Self {
        Self::Registry {
            default_service,
            registry,
            tenant_filter: None,
        }
    }

    /// Construct a cloud-mode backend with a tenant-isolation filter
    /// (F2.x-b). Dispatch calls that pass a `tenant_subject` will be
    /// rejected via the typo-tolerance fallback when the filter denies
    /// access.
    #[must_use]
    pub fn registry_with_filter(
        default_service: Arc<QueryService>,
        registry: Arc<ministr_daemon::registry::CorpusRegistry>,
        tenant_filter: Arc<dyn TenantCorpusFilter>,
    ) -> Self {
        Self::Registry {
            default_service,
            registry,
            tenant_filter: Some(tenant_filter),
        }
    }

    /// Return the underlying [`QueryService`] if this is a local backend.
    /// Escape hatch for handlers not yet migrated to the trait.
    #[must_use]
    pub fn as_local(&self) -> Option<&Arc<QueryService>> {
        match self {
            Self::Local(b) => Some(b.service()),
            Self::Registry {
                default_service, ..
            } => Some(default_service),
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
            Self::Local(_) | Self::Registry { .. } => None,
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
            Self::Local(_) | Self::Daemon(_) | Self::Registry { .. } => Vec::new(),
        }
    }

    /// Resolve `project` (a `corpus_id` in registry mode) to a handle
    /// whose owned `QueryService` should answer this call. Returns
    /// `Err(default_service)` when `project` is `None` (and no tenant
    /// default is available), the registry can't produce a handle
    /// (unknown id / blob restore failure), or the tenant filter
    /// denies access — all collapse to the same typo-tolerance shape
    /// so a cross-tenant probe leaks no more information than a typo
    /// would.
    ///
    /// F2.x-c — when `project = None` AND the caller threaded a
    /// `tenant_subject` AND a `tenant_filter` is wired, ask the filter
    /// for the tenant's default corpus (currently: most-recently-created).
    /// If found, `ensure_present` that `corpus_id` and dispatch through
    /// its `QueryService`. If the filter returns `None` (or the lookup
    /// errors), continue the existing fallback to `default_service`.
    ///
    /// The returned `Ok` arm carries the `Arc<CorpusHandle>` so the
    /// caller keeps the handle alive across its `.await` on
    /// `handle.service.<method>(…)`.
    async fn resolve_registry_handle<'a>(
        default_service: &'a Arc<QueryService>,
        registry: &Arc<ministr_daemon::registry::CorpusRegistry>,
        tenant_filter: Option<&Arc<dyn TenantCorpusFilter>>,
        tenant_subject: Option<&str>,
        project: Option<&str>,
    ) -> Result<Arc<ministr_daemon::registry::CorpusHandle>, &'a Arc<QueryService>> {
        // F2.x-c — None project, tenant in scope: ask the filter for
        // the tenant's default corpus. Allocate a String so the rest of
        // the resolver works against a borrowed `&str` uniformly,
        // without forcing the trait method to hand out a borrowed Cow.
        let resolved_owned: Option<String>;
        let corpus_id: &str = if let Some(id) = project {
            id
        } else {
            let Some(filter) = tenant_filter else {
                return Err(default_service);
            };
            let Some(subject) = tenant_subject else {
                return Err(default_service);
            };
            match filter.default_corpus_for_tenant(subject).await {
                Ok(Some(id)) => {
                    tracing::debug!(
                        subject,
                        corpus_id = %id,
                        "tenant default corpus resolved"
                    );
                    resolved_owned = Some(id);
                    resolved_owned.as_deref().unwrap()
                }
                Ok(None) => return Err(default_service),
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        subject,
                        "tenant default-corpus lookup failed — falling back"
                    );
                    return Err(default_service);
                }
            }
        };
        // F2.x-b — gate the lookup behind the tenant filter when one is
        // wired AND the caller threaded its tenant identity. A missing
        // tenant_subject in cloud mode is itself a deny: handlers that
        // accept a `project` argument MUST extract the Tenant from
        // RequestContext and pass its subject. A `None` arrives only on
        // self-hosted serve (where tenant_filter is None too). The
        // F2.x-c default-resolution branch above already used the
        // filter to pick the corpus_id, but we re-check `allowed` here
        // for uniform treatment — the same filter implementation
        // will obviously approve its own choice.
        if let Some(filter) = tenant_filter {
            let Some(subject) = tenant_subject else {
                tracing::warn!(
                    corpus_id,
                    "tenant filter wired but caller passed no tenant_subject — denying"
                );
                return Err(default_service);
            };
            match filter.allowed(subject, corpus_id).await {
                Ok(true) => {}
                Ok(false) => {
                    tracing::debug!(
                        subject,
                        corpus_id,
                        "tenant filter denied — falling back to default service"
                    );
                    return Err(default_service);
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        subject,
                        corpus_id,
                        "tenant filter storage error — denying (fail closed)"
                    );
                    return Err(default_service);
                }
            }
        }
        match registry.ensure_present(corpus_id).await {
            Ok(handle) => Ok(handle),
            Err(_) => Err(default_service),
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
        tenant_subject: Option<&str>,
        project: Option<&str>,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SurveyResult>, BackendError> {
        match self {
            Self::Local(b) => b.survey(query, top_k).await,
            Self::Daemon(b) => b.survey(query, top_k).await,
            Self::DaemonMulti(m) => m.for_project(project).survey(query, top_k).await,
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle.service.survey(query, top_k).await?),
                Err(default) => Ok(default.survey(query, top_k).await?),
            },
        }
    }

    pub async fn survey_with_exclude(
        &self,
        tenant_subject: Option<&str>,
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
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle
                    .service
                    .survey_excluding(query, top_k, exclude_ids)
                    .await?),
                Err(default) => Ok(default.survey_excluding(query, top_k, exclude_ids).await?),
            },
        }
    }

    pub async fn read_section(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        section_id: &str,
    ) -> Result<SectionDetail, BackendError> {
        match self {
            Self::Local(b) => b.read_section(section_id).await,
            Self::Daemon(b) => b.read_section(section_id).await,
            Self::DaemonMulti(m) => m.for_project(project).read_section(section_id).await,
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle.service.read_section(section_id).await?),
                Err(default) => Ok(default.read_section(section_id).await?),
            },
        }
    }

    pub async fn extract_claims(
        &self,
        tenant_subject: Option<&str>,
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
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle.service.extract_claims(section_id, query).await?),
                Err(default) => Ok(default.extract_claims(section_id, query).await?),
            },
        }
    }

    pub async fn search_symbols(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        filter: SymbolFilter,
    ) -> Result<Vec<SymbolRecord>, BackendError> {
        match self {
            Self::Local(b) => b.search_symbols(filter).await,
            Self::Daemon(b) => b.search_symbols(filter).await,
            Self::DaemonMulti(m) => m.for_project(project).search_symbols(filter).await,
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle.service.search_symbols(&filter).await?),
                Err(default) => Ok(default.search_symbols(&filter).await?),
            },
        }
    }

    pub async fn definition(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        symbol_id: &str,
    ) -> Result<SymbolDefinition, BackendError> {
        match self {
            Self::Local(b) => b.definition(symbol_id).await,
            Self::Daemon(b) => b.definition(symbol_id).await,
            Self::DaemonMulti(m) => m.for_project(project).definition(symbol_id).await,
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle.service.get_symbol_definition(symbol_id).await?),
                Err(default) => Ok(default.get_symbol_definition(symbol_id).await?),
            },
        }
    }

    pub async fn references(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
    ) -> Result<Vec<SymbolRefResult>, BackendError> {
        match self {
            Self::Local(b) => b.references(symbol_id, ref_kind).await,
            Self::Daemon(b) => b.references(symbol_id, ref_kind).await,
            Self::DaemonMulti(m) => m.for_project(project).references(symbol_id, ref_kind).await,
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle
                    .service
                    .get_symbol_references(symbol_id, ref_kind)
                    .await?),
                Err(default) => Ok(default.get_symbol_references(symbol_id, ref_kind).await?),
            },
        }
    }

    pub async fn impact(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        symbol_id: &str,
        max_depth: u32,
    ) -> Result<ImpactResult, BackendError> {
        match self {
            Self::Local(b) => b.impact(symbol_id, max_depth).await,
            Self::Daemon(b) => b.impact(symbol_id, max_depth).await,
            Self::DaemonMulti(m) => m.for_project(project).impact(symbol_id, max_depth).await,
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle.service.compute_impact(symbol_id, max_depth).await?),
                Err(default) => Ok(default.compute_impact(symbol_id, max_depth).await?),
            },
        }
    }

    pub async fn dead_code(
        &self,
        tenant_subject: Option<&str>,
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
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle
                    .service
                    .find_dead_code(kind, module, min_lines, limit)
                    .await?),
                Err(default) => Ok(default
                    .find_dead_code(kind, module, min_lines, limit)
                    .await?),
            },
        }
    }

    pub async fn solid(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        params: &SolidParams,
    ) -> Result<Vec<SolidFinding>, BackendError> {
        match self {
            Self::Local(b) => b.solid(params).await,
            Self::Daemon(b) => b.solid(params).await,
            Self::DaemonMulti(m) => m.for_project(project).solid(params).await,
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle.service.detect_solid_violations(params).await?),
                Err(default) => Ok(default.detect_solid_violations(params).await?),
            },
        }
    }

    pub async fn related_claims(
        &self,
        tenant_subject: Option<&str>,
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
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle
                    .service
                    .related_claims(claim_id, relation_types)
                    .await?),
                Err(default) => Ok(default.related_claims(claim_id, relation_types).await?),
            },
        }
    }

    pub async fn compress(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        content_ids: &[String],
    ) -> Result<Vec<CompressedItem>, BackendError> {
        match self {
            Self::Local(b) => b.compress(content_ids).await,
            Self::Daemon(b) => b.compress(content_ids).await,
            Self::DaemonMulti(m) => m.for_project(project).compress(content_ids).await,
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle.service.compress_content(content_ids).await?),
                Err(default) => Ok(default.compress_content(content_ids).await?),
            },
        }
    }

    pub async fn toc(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        document_id: Option<&str>,
    ) -> Result<Vec<TocEntry>, BackendError> {
        match self {
            Self::Local(b) => b.toc(document_id).await,
            Self::Daemon(b) => b.toc(document_id).await,
            Self::DaemonMulti(m) => m.for_project(project).toc(document_id).await,
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle.service.toc(document_id).await?),
                Err(default) => Ok(default.toc(document_id).await?),
            },
        }
    }

    pub async fn bridges(
        &self,
        tenant_subject: Option<&str>,
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
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle
                    .service
                    .query_bridges(query, kind, language, file_path)
                    .await?),
                Err(default) => Ok(default
                    .query_bridges(query, kind, language, file_path)
                    .await?),
            },
        }
    }

    pub async fn symbol_at_position(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        file_path: &str,
        line: u32,
        col: u32,
    ) -> Result<Option<String>, BackendError> {
        match self {
            Self::Local(b) => b.symbol_at_position(file_path, line, col).await,
            Self::Daemon(b) => b.symbol_at_position(file_path, line, col).await,
            Self::DaemonMulti(m) => {
                m.for_project(project)
                    .symbol_at_position(file_path, line, col)
                    .await
            }
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => match Self::resolve_registry_handle(
                default_service,
                registry,
                tenant_filter.as_ref(),
                tenant_subject,
                project,
            )
            .await
            {
                Ok(handle) => Ok(handle
                    .service
                    .symbol_at_position(file_path, line, col)
                    .await?),
                Err(default) => Ok(default.symbol_at_position(file_path, line, col).await?),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    //! F2.x-b/c — tenant-filter behaviour tests for `Backend::Registry`.
    //!
    //! These exercise `resolve_registry_handle` in isolation so they don't
    //! need a live `CorpusRegistry` fixture (that's covered by the
    //! daemon's `tests/common`). The tests focus on what the resolver
    //! decides given the filter alone:
    //! - F2.x-b: `tenant_filter = Some` + `tenant_subject = None` denies.
    //! - F2.x-b: `tenant_filter = Some` + filter returns `Ok(false)` denies.
    //! - F2.x-b: `tenant_filter = Some` + filter returns `Err` denies (fail closed).
    //! - F2.x-c: `project = None` consults `default_corpus_for_tenant`; falls
    //!   back to `default_service` when that returns `None` or `Err`.

    use super::*;
    use ministr_api::tenant_filter::{
        DefaultCorpusFuture, TenantCorpusFilter, TenantFilterError, TenantFilterFuture,
    };
    use std::sync::Mutex;

    #[derive(Debug)]
    struct MockFilter {
        decision: Mutex<Result<bool, &'static str>>,
        /// F2.x-c — configurable response for `default_corpus_for_tenant`.
        /// `None` (the default) preserves the trait's default impl
        /// behaviour. `Some(Ok(...))` returns a `corpus_id`, `Some(Err(_))`
        /// simulates a storage failure.
        default_corpus: Mutex<Option<Result<Option<String>, &'static str>>>,
        calls: Mutex<Vec<(String, String)>>,
        default_calls: Mutex<Vec<String>>,
    }

    impl MockFilter {
        fn allow() -> Self {
            Self {
                decision: Mutex::new(Ok(true)),
                default_corpus: Mutex::new(None),
                calls: Mutex::new(Vec::new()),
                default_calls: Mutex::new(Vec::new()),
            }
        }
        fn deny() -> Self {
            Self {
                decision: Mutex::new(Ok(false)),
                default_corpus: Mutex::new(None),
                calls: Mutex::new(Vec::new()),
                default_calls: Mutex::new(Vec::new()),
            }
        }
        fn err() -> Self {
            Self {
                decision: Mutex::new(Err("simulated storage failure")),
                default_corpus: Mutex::new(None),
                calls: Mutex::new(Vec::new()),
                default_calls: Mutex::new(Vec::new()),
            }
        }
        fn with_default_corpus(self, value: Result<Option<String>, &'static str>) -> Self {
            *self.default_corpus.lock().unwrap() = Some(value);
            self
        }
        fn calls(&self) -> Vec<(String, String)> {
            self.calls.lock().unwrap().clone()
        }
        fn default_calls(&self) -> Vec<String> {
            self.default_calls.lock().unwrap().clone()
        }
    }

    impl TenantCorpusFilter for MockFilter {
        fn allowed<'a>(
            &'a self,
            tenant_subject: &'a str,
            corpus_id: &'a str,
        ) -> TenantFilterFuture<'a> {
            self.calls
                .lock()
                .unwrap()
                .push((tenant_subject.to_string(), corpus_id.to_string()));
            let decision = *self.decision.lock().unwrap();
            Box::pin(async move { decision.map_err(|m| TenantFilterError::Storage(m.into())) })
        }

        fn default_corpus_for_tenant<'a>(
            &'a self,
            tenant_subject: &'a str,
        ) -> DefaultCorpusFuture<'a> {
            self.default_calls
                .lock()
                .unwrap()
                .push(tenant_subject.to_string());
            let configured = self.default_corpus.lock().unwrap().clone();
            Box::pin(async move {
                match configured {
                    None => Ok(None),
                    Some(Ok(id)) => Ok(id),
                    Some(Err(m)) => Err(TenantFilterError::Storage(m.into())),
                }
            })
        }
    }

    /// F2.x-c: `project = None` consults `default_corpus_for_tenant`. With
    /// no override set, the trait's default impl returns `None`, so the
    /// resolver falls back to `default_service`. `allowed` is never
    /// called because there's no `corpus_id` to gate against.
    #[tokio::test]
    async fn project_none_consults_default_corpus_then_falls_back() {
        let concrete: Arc<MockFilter> = Arc::new(MockFilter::deny());
        let filter: Arc<dyn TenantCorpusFilter> = concrete.clone();
        let default = dummy_default_service();
        let registry = dummy_registry();
        let outcome = Backend::resolve_registry_handle(
            &default,
            &registry,
            Some(&filter),
            Some("alice"),
            None,
        )
        .await;
        assert!(outcome.is_err(), "None project + None default falls back");
        assert_eq!(
            concrete.default_calls(),
            vec!["alice".to_string()],
            "default_corpus_for_tenant must be consulted"
        );
        assert!(
            concrete.calls().is_empty(),
            "allowed must not be called when default returns None"
        );
    }

    /// F2.x-c: `project = None` + no `tenant_subject` → fall back without
    /// calling either filter method.
    #[tokio::test]
    async fn project_none_no_tenant_skips_filter_entirely() {
        let concrete: Arc<MockFilter> = Arc::new(MockFilter::allow());
        let filter: Arc<dyn TenantCorpusFilter> = concrete.clone();
        let default = dummy_default_service();
        let registry = dummy_registry();
        let outcome =
            Backend::resolve_registry_handle(&default, &registry, Some(&filter), None, None).await;
        assert!(outcome.is_err());
        assert!(concrete.default_calls().is_empty());
        assert!(concrete.calls().is_empty());
    }

    /// F2.x-c: `project = None` + no filter → fall back without any
    /// filter call (preserves self-hosted / single-tenant behaviour).
    #[tokio::test]
    async fn project_none_no_filter_falls_back() {
        let default = dummy_default_service();
        let registry = dummy_registry();
        let outcome =
            Backend::resolve_registry_handle(&default, &registry, None, Some("alice"), None).await;
        assert!(outcome.is_err());
    }

    /// F2.x-c: when `default_corpus_for_tenant` returns `Some(id)`,
    /// the resolver re-checks `allowed` for the chosen corpus, then
    /// proceeds to `ensure_present`. In tests `ensure_present` errors
    /// (empty registry), so the outcome is Err — but BOTH filter
    /// methods were exercised.
    #[tokio::test]
    async fn project_none_default_corpus_drives_lookup() {
        let concrete: Arc<MockFilter> =
            Arc::new(MockFilter::allow().with_default_corpus(Ok(Some("alice-corpus-1".into()))));
        let filter: Arc<dyn TenantCorpusFilter> = concrete.clone();
        let default = dummy_default_service();
        let registry = dummy_registry();
        let outcome = Backend::resolve_registry_handle(
            &default,
            &registry,
            Some(&filter),
            Some("alice"),
            None,
        )
        .await;
        assert!(outcome.is_err(), "registry lookup misses in this fixture");
        assert_eq!(concrete.default_calls(), vec!["alice".to_string()]);
        assert_eq!(
            concrete.calls(),
            vec![("alice".to_string(), "alice-corpus-1".to_string())],
            "allowed must re-check the chosen corpus_id"
        );
    }

    /// F2.x-c: storage error on `default_corpus_for_tenant` → fall
    /// back to `default_service` (don't crash, don't leak the error).
    #[tokio::test]
    async fn project_none_default_corpus_error_falls_back() {
        let concrete: Arc<MockFilter> = Arc::new(
            MockFilter::allow().with_default_corpus(Err("simulated default-lookup failure")),
        );
        let filter: Arc<dyn TenantCorpusFilter> = concrete.clone();
        let default = dummy_default_service();
        let registry = dummy_registry();
        let outcome = Backend::resolve_registry_handle(
            &default,
            &registry,
            Some(&filter),
            Some("alice"),
            None,
        )
        .await;
        assert!(outcome.is_err());
        assert_eq!(concrete.default_calls(), vec!["alice".to_string()]);
        assert!(concrete.calls().is_empty(), "allowed not reached on error");
    }

    /// Filter wired + `tenant_subject` = None → deny (fail closed).
    #[tokio::test]
    async fn no_tenant_subject_denies_when_filter_is_wired() {
        let filter: Arc<dyn TenantCorpusFilter> = Arc::new(MockFilter::allow());
        let default = dummy_default_service();
        let registry = dummy_registry();
        let outcome = Backend::resolve_registry_handle(
            &default,
            &registry,
            Some(&filter),
            None,
            Some("any-corpus"),
        )
        .await;
        assert!(outcome.is_err(), "missing tenant_subject must deny");
    }

    /// Filter returns Ok(false) → deny.
    #[tokio::test]
    async fn filter_deny_returns_default_service_fallback() {
        let filter: Arc<dyn TenantCorpusFilter> = Arc::new(MockFilter::deny());
        let default = dummy_default_service();
        let registry = dummy_registry();
        let outcome = Backend::resolve_registry_handle(
            &default,
            &registry,
            Some(&filter),
            Some("alice"),
            Some("bob-corpus"),
        )
        .await;
        assert!(outcome.is_err(), "explicit deny falls back to default");
    }

    /// Filter returns Err → deny (fail closed).
    #[tokio::test]
    async fn filter_storage_error_fails_closed() {
        let filter: Arc<dyn TenantCorpusFilter> = Arc::new(MockFilter::err());
        let default = dummy_default_service();
        let registry = dummy_registry();
        let outcome = Backend::resolve_registry_handle(
            &default,
            &registry,
            Some(&filter),
            Some("alice"),
            Some("any-corpus"),
        )
        .await;
        assert!(
            outcome.is_err(),
            "storage error must fail closed, not bypass"
        );
    }

    /// Helpers that build the bare minimum of the cross-crate types so
    /// the resolver can be exercised in isolation. `default_service`
    /// and `registry` are never dereferenced by the resolver on the
    /// paths these tests cover (project=None / filter-deny / filter-
    /// error all return before `ensure_present` runs), so we ship them
    /// as `Arc::new(unsafe_uninit)` style placeholders.
    fn dummy_default_service() -> Arc<QueryService> {
        // Cheap construction: in-memory SQLite + zero-dim mock embedder
        // + bare HnswIndex. Resolver paths under test never call any
        // method on this Arc; it just needs to type-check.
        use ministr_core::embedding::Embedder;
        use ministr_core::error::IndexError;
        use ministr_core::index::{HnswIndex, VectorIndex};
        use ministr_core::storage::SqliteStorage;

        struct ZeroEmbedder;
        impl Embedder for ZeroEmbedder {
            fn embed(&self, _: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
                Ok(vec![vec![0.0; 4]])
            }
            fn dimension(&self) -> usize {
                4
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        let storage = SqliteStorage::open(tmp.path().join("test.db")).unwrap();
        let embedder: Arc<dyn Embedder> = Arc::new(ZeroEmbedder);
        let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(4, 16).unwrap());
        // Leak the tempdir to keep the SQLite file alive for the test;
        // the test process exits anyway.
        std::mem::forget(tmp);
        Arc::new(QueryService::new(storage, embedder, index))
    }

    fn dummy_registry() -> Arc<ministr_daemon::registry::CorpusRegistry> {
        use ministr_core::embedding::Embedder;
        use ministr_core::error::IndexError;

        struct ZeroEmbedder;
        impl Embedder for ZeroEmbedder {
            fn embed(&self, _: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
                Ok(vec![vec![0.0; 4]])
            }
            fn dimension(&self) -> usize {
                4
            }
        }

        let tmp = tempfile::tempdir().unwrap();
        let config = ministr_core::config::MinistrConfig {
            data_dir: tmp.path().to_path_buf(),
            ..ministr_core::config::MinistrConfig::default()
        };
        std::mem::forget(tmp);
        let embedder: Arc<dyn Embedder> = Arc::new(ZeroEmbedder);
        Arc::new(ministr_daemon::registry::CorpusRegistry::new(
            embedder, config,
        ))
    }
}
