//! Pluggable query-backend seam shared by every ministr query surface.
//!
//! [`QueryBackend`] is the trait surfaces (the MCP server's tool handlers,
//! the CLI's query commands) code against. Two concrete implementations
//! cover the two deployment shapes:
//!
//! - [`LocalBackend`] (in [`local`]) — calls an in-process [`QueryService`]
//!   directly.
//! - [`DaemonBackend`] (in [`daemon`]) — forwards every call over HTTP to a
//!   running `ministr-daemon` via [`DaemonClient`].
//!
//! [`Backend`] is a concrete enum that holds one of the two impls and also
//! implements [`QueryBackend`], so a consumer can hold a single concrete
//! field without giving up the abstraction.
//!
//! Adding a third backend (mock for tests, remote-only over TLS, etc.) means
//! adding one module to this crate with `impl QueryBackend for NewBackend`
//! and one variant on [`Backend`]. No existing consumer changes — Open/Closed.
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
    CallDirection, ClaimResult, CompressedItem, DeadSymbol, DefinitionOptions, Diagnostic,
    ImpactResult, InspectOptions, InspectResult, QueryError, QueryService, RelatedClaimResult,
    SectionDetail, SolidFinding, SolidParams, SurveyResult, SymbolDefinition, SymbolRefResult,
};
use ministr_core::storage::{BridgeLinkDetail, SymbolFilter, SymbolRecord};
use ministr_core::types::{DeliveryIdentity, RefKind, RelationType, TocEntry};
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
    Client(Box<ClientError>),
    /// The requested linked project/corpus is not configured on this backend.
    ///
    /// `available` carries every route this backend would have accepted
    /// (the current project's own label first, then linked labels) so the
    /// agent-facing message can name them instead of sending the caller
    /// off to guess — or to misread the miss as corpus staleness.
    #[error("unknown project or corpus: {requested}")]
    UnknownProject {
        requested: String,
        available: Vec<String>,
    },
    /// A corpus that *is* a valid route could not be made available (cloud
    /// lazy-restore failure). Unlike [`Self::UnknownProject`] this is
    /// transient — the route is right, the corpus isn't ready — so it stays
    /// retryable.
    #[error("corpus unavailable: {0}")]
    CorpusUnavailable(String),
    /// Tenant policy rejected the requested corpus.
    #[error("permission denied for corpus: {0}")]
    PermissionDenied(String),
    /// Tool parameters or continuation cursor were invalid.
    #[error("invalid parameters: {0}")]
    InvalidParameters(String),
}

impl From<ClientError> for BackendError {
    fn from(error: ClientError) -> Self {
        Self::Client(Box::new(error))
    }
}

/// Survey payload plus transport/index state used by MCP response envelopes.
pub struct SurveyBackendResponse {
    pub results: Vec<SurveyResult>,
    pub deduplicated_count: usize,
    pub suppressed_identities: Vec<DeliveryIdentity>,
    pub metadata: ministr_api::metadata::QueryMetadata,
}

/// Backend data paired with honest index/transport metadata.
pub struct BackendResponse<T> {
    pub data: T,
    pub metadata: ministr_api::metadata::QueryMetadata,
}

/// One bounded reference page with transport-preserved continuation state.
pub struct ReferencesBackendResponse {
    pub references: Vec<SymbolRefResult>,
    pub pagination: ministr_api::metadata::Pagination,
    pub metadata: ministr_api::metadata::QueryMetadata,
}

/// One bounded collection page with transport-preserved total/continuation.
pub struct CollectionBackendResponse<T> {
    pub data: Vec<T>,
    pub pagination: ministr_api::metadata::Pagination,
    pub metadata: ministr_api::metadata::QueryMetadata,
}

/// One bounded impact page with its aggregate summary intact.
pub struct ImpactBackendResponse {
    pub impact: ImpactResult,
    pub pagination: ministr_api::metadata::Pagination,
    pub metadata: ministr_api::metadata::QueryMetadata,
}

/// One TOC page plus corpus-level aggregates from the unpaginated set.
pub struct TocBackendResponse {
    pub entries: Vec<TocEntry>,
    pub documents: usize,
    pub claims: usize,
    pub pagination: ministr_api::metadata::Pagination,
    pub metadata: ministr_api::metadata::QueryMetadata,
}

fn survey_candidate_options() -> ministr_core::service::SurveyOptions {
    ministr_core::service::SurveyOptions {
        max_total_bytes: 1_048_576,
        max_total_tokens: 262_144,
        ..ministr_core::service::SurveyOptions::default()
    }
}

impl<T> BackendResponse<T> {
    fn complete(data: T) -> Self {
        Self {
            data,
            metadata: ministr_api::metadata::QueryMetadata::default(),
        }
    }
}

impl<T> std::ops::Deref for BackendResponse<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<T> std::ops::DerefMut for BackendResponse<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

/// One compressed representation tied to the exact delivered identity whose
/// session/budget state must be updated.
#[derive(Debug, Clone)]
pub struct CompressedDelivery {
    pub identity: DeliveryIdentity,
    pub item: CompressedItem,
}

async fn compress_with_service(
    service: &QueryService,
    identities: &[DeliveryIdentity],
) -> Result<Vec<CompressedDelivery>, BackendError> {
    let content_ids: Vec<String> = identities
        .iter()
        .map(|identity| identity.content_id.clone())
        .collect();
    let items = service.compress_content(&content_ids).await?;
    let mut pending: std::collections::HashMap<
        String,
        std::collections::VecDeque<DeliveryIdentity>,
    > = std::collections::HashMap::new();
    for identity in identities {
        pending
            .entry(identity.content_id.clone())
            .or_default()
            .push_back(identity.clone());
    }
    Ok(items
        .into_iter()
        .filter_map(|item| {
            pending
                .get_mut(&item.original_id)
                .and_then(std::collections::VecDeque::pop_front)
                .map(|identity| CompressedDelivery { identity, item })
        })
        .collect())
}

fn paginate_references(
    references: Vec<SymbolRefResult>,
    metadata: ministr_api::metadata::QueryMetadata,
    requested_offset: Option<usize>,
    cursor: Option<&str>,
    requested_limit: usize,
) -> Result<ReferencesBackendResponse, BackendError> {
    let total = references.len();
    let limit = requested_limit.clamp(1, 500);
    let offset = match cursor {
        Some(value) if value.starts_with("ref:") => references
            .iter()
            .position(|reference| {
                ministr_core::service::symbol_reference_cursor(reference) == value
            })
            .map(|index| index + 1)
            .ok_or_else(|| {
                BackendError::InvalidParameters(
                    "reference cursor no longer identifies an item; restart pagination".to_string(),
                )
            })?,
        Some(value) => value
            .strip_prefix("offset:")
            .unwrap_or(value)
            .parse::<usize>()
            .map_err(|_| {
                BackendError::InvalidParameters("invalid pagination cursor".to_string())
            })?,
        None => requested_offset.unwrap_or(0),
    };
    let page: Vec<_> = references.into_iter().skip(offset).take(limit).collect();
    let consumed = offset.saturating_add(page.len()).min(total);
    let has_more = consumed < total;
    let pagination = ministr_api::metadata::Pagination {
        limit,
        offset: Some(offset),
        cursor: cursor.map(str::to_string),
        next_cursor: has_more
            .then(|| {
                page.last()
                    .map(ministr_core::service::symbol_reference_cursor)
            })
            .flatten(),
        total,
        has_more,
        omitted_count: total.saturating_sub(consumed),
    };
    Ok(ReferencesBackendResponse {
        references: page,
        pagination,
        metadata,
    })
}

fn paginate_collection<T>(
    items: Vec<T>,
    metadata: ministr_api::metadata::QueryMetadata,
    offset: usize,
    requested_limit: usize,
) -> CollectionBackendResponse<T> {
    let total = items.len();
    let limit = requested_limit.clamp(1, 500);
    let page: Vec<_> = items.into_iter().skip(offset).take(limit).collect();
    let consumed = offset.saturating_add(page.len()).min(total);
    CollectionBackendResponse {
        data: page,
        pagination: ministr_api::metadata::Pagination {
            limit,
            offset: Some(offset),
            cursor: None,
            next_cursor: (consumed < total).then(|| format!("offset:{consumed}")),
            total,
            has_more: consumed < total,
            omitted_count: total.saturating_sub(consumed),
        },
        metadata,
    }
}

fn paginate_related(
    items: Vec<RelatedClaimResult>,
    metadata: ministr_api::metadata::QueryMetadata,
    requested_offset: Option<usize>,
    cursor: Option<&str>,
    requested_limit: usize,
) -> Result<CollectionBackendResponse<RelatedClaimResult>, BackendError> {
    let offset = match cursor {
        Some(value) if value.starts_with("related:") => items
            .iter()
            .position(|item| ministr_core::service::related_claim_cursor(item) == value)
            .map(|index| index + 1)
            .ok_or_else(|| {
                BackendError::InvalidParameters(
                    "related cursor no longer identifies an edge; restart pagination".to_string(),
                )
            })?,
        Some(value) => value
            .strip_prefix("offset:")
            .unwrap_or(value)
            .parse::<usize>()
            .map_err(|_| BackendError::InvalidParameters("invalid pagination cursor".into()))?,
        None => requested_offset.unwrap_or(0),
    };
    let mut response = paginate_collection(items, metadata, offset, requested_limit);
    response.pagination.cursor = cursor.map(str::to_string);
    if response.pagination.has_more {
        response.pagination.next_cursor = response
            .data
            .last()
            .map(ministr_core::service::related_claim_cursor);
    }
    Ok(response)
}

fn paginate_impact(
    mut impact: ImpactResult,
    metadata: ministr_api::metadata::QueryMetadata,
    requested_offset: Option<usize>,
    cursor: Option<&str>,
    requested_limit: usize,
) -> Result<ImpactBackendResponse, BackendError> {
    let total = impact.callers.len();
    let offset = match cursor {
        Some(value) if value.starts_with("impact:") => impact
            .callers
            .iter()
            .position(|caller| ministr_core::service::impact_caller_cursor(caller) == value)
            .map(|index| index + 1)
            .ok_or_else(|| {
                BackendError::InvalidParameters(
                    "impact cursor no longer identifies an item; restart pagination".to_string(),
                )
            })?,
        Some(value) => value
            .strip_prefix("offset:")
            .unwrap_or(value)
            .parse::<usize>()
            .map_err(|_| BackendError::InvalidParameters("invalid pagination cursor".into()))?,
        None => requested_offset.unwrap_or(0),
    };
    let limit = requested_limit.clamp(1, 500);
    impact.callers = impact
        .callers
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect();
    let consumed = offset.saturating_add(impact.callers.len()).min(total);
    Ok(ImpactBackendResponse {
        pagination: ministr_api::metadata::Pagination {
            limit,
            offset: Some(offset),
            cursor: cursor.map(str::to_string),
            next_cursor: (consumed < total)
                .then(|| {
                    impact
                        .callers
                        .last()
                        .map(ministr_core::service::impact_caller_cursor)
                })
                .flatten(),
            total,
            has_more: consumed < total,
            omitted_count: total.saturating_sub(consumed),
        },
        impact,
        metadata,
    })
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
    /// Every backend consumes the exact exclusion set. In daemon-forward mode
    /// the MCP proxy is the single delivery/dedup authority; direct daemon API
    /// sessions remain daemon-owned and are a separate transport contract.
    fn survey_with_exclude(
        &self,
        query: &str,
        top_k: usize,
        exclude_ids: &std::collections::HashSet<DeliveryIdentity>,
    ) -> impl Future<Output = Result<SurveyBackendResponse, BackendError>> + Send;

    /// Read a section by ID.
    fn read_section(
        &self,
        section_id: &str,
    ) -> impl Future<Output = Result<BackendResponse<SectionDetail>, BackendError>> + Send;

    /// Pull atomic claims from a section, optionally query-filtered.
    fn extract_claims(
        &self,
        section_id: &str,
        query: Option<&str>,
    ) -> impl Future<Output = Result<BackendResponse<Vec<ClaimResult>>, BackendError>> + Send;

    /// Search the symbol index with optional filters.
    fn search_symbols(
        &self,
        filter: SymbolFilter,
    ) -> impl Future<Output = Result<BackendResponse<Vec<SymbolRecord>>, BackendError>> + Send;

    /// Full definition of a symbol by ID.
    fn definition(
        &self,
        symbol_id: &str,
        options: DefinitionOptions,
    ) -> impl Future<Output = Result<BackendResponse<SymbolDefinition>, BackendError>> + Send;

    /// Bounded compound symbol navigation.
    fn inspect_symbol(
        &self,
        symbol_id: &str,
        options: InspectOptions,
    ) -> impl Future<Output = Result<BackendResponse<InspectResult>, BackendError>> + Send;

    /// Position-addressed bounded compound symbol navigation.
    fn inspect_at_position(
        &self,
        file_path: &str,
        line: u32,
        col: u32,
        options: InspectOptions,
    ) -> impl Future<Output = Result<BackendResponse<InspectResult>, BackendError>> + Send;

    /// Callers, implementors, importers, and bridge links for a symbol.
    fn references(
        &self,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
        through_implementors: bool,
    ) -> impl Future<Output = Result<BackendResponse<Vec<SymbolRefResult>>, BackendError>> + Send;

    /// Transitive call hierarchy of a symbol in one direction (incoming =
    /// callers / blast radius, outgoing = callees). `tests_only` restricts the
    /// result to nodes in test files (FL6 test↔code mapping).
    fn impact(
        &self,
        symbol_id: &str,
        max_depth: u32,
        direction: CallDirection,
        tests_only: bool,
    ) -> impl Future<Output = Result<BackendResponse<ImpactResult>, BackendError>> + Send;

    /// Zero-reference symbol candidates.
    fn dead_code(
        &self,
        kind: Option<&str>,
        module: Option<&str>,
        min_lines: u32,
        limit: usize,
    ) -> impl Future<Output = Result<BackendResponse<Vec<DeadSymbol>>, BackendError>> + Send;

    /// Structured compiler/linter diagnostics from the project's own
    /// toolchain(s) (FL5 — the "verify" stage). `languages` optionally
    /// restricts which toolchains run; `None` = every detected toolchain.
    fn diagnostics(
        &self,
        languages: Option<&[String]>,
        limit: usize,
    ) -> impl Future<Output = Result<BackendResponse<Vec<Diagnostic>>, BackendError>> + Send;

    /// Deterministic SOLID-violation candidates.
    fn solid(
        &self,
        params: &SolidParams,
    ) -> impl Future<Output = Result<BackendResponse<Vec<SolidFinding>>, BackendError>> + Send;

    /// Follow claim-relationship edges.
    fn related_claims(
        &self,
        claim_id: &str,
        relation_types: Option<&[RelationType]>,
    ) -> impl Future<Output = Result<BackendResponse<Vec<RelatedClaimResult>>, BackendError>> + Send;

    /// Extractive TF-IDF summarisation for a batch of content IDs.
    fn compress(
        &self,
        identities: &[DeliveryIdentity],
    ) -> impl Future<Output = Result<Vec<CompressedDelivery>, BackendError>> + Send;

    /// Structural TOC entries for the corpus or a specific document.
    ///
    /// As of the toc-schema-convergence work the daemon backend is at parity
    /// with the local backend: `api::TocEntry` carries `heading_path`,
    /// `claims_available`, and `token_count`, and `document_id` rides on
    /// `source_path`, so daemon-mode TOC entries are no longer lossy.
    fn toc(
        &self,
        document_id: Option<&str>,
    ) -> impl Future<Output = Result<BackendResponse<Vec<TocEntry>>, BackendError>> + Send;

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
    ) -> impl Future<Output = Result<BackendResponse<Vec<BridgeLinkDetail>>, BackendError>> + Send;

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

/// Set-once holder for the current project's human label — the name of its
/// root directory (`kadodi` for `~/Code/kadodi`), which is exactly the
/// label a *sibling* project would get by linking it.
///
/// Without this, the only routes a session accepts are linked labels and
/// opaque corpus-id hashes, so `project: "kadodi"` inside kadodi — the
/// most natural thing an agent can pass — fails as an unknown corpus even
/// though that corpus is the one being served. Populated once at boot from
/// the corpus paths (see `MinistrServer::prune_tools`); a session whose
/// label cannot be resolved keeps the old strict behaviour.
#[derive(Debug, Default)]
pub struct PrimaryLabel(std::sync::OnceLock<String>);

impl PrimaryLabel {
    /// Record the label. Later calls are ignored — the first boot-time
    /// value wins, so a route can never be redefined mid-session.
    pub fn set(&self, label: impl Into<String>) {
        let label = label.into();
        if !label.trim().is_empty() {
            let _ = self.0.set(label.trim().to_string());
        }
    }

    #[must_use]
    pub fn get(&self) -> Option<&str> {
        self.0.get().map(String::as_str)
    }

    /// Whether `candidate` names the current project. Case-insensitive:
    /// directory casing is not something an agent can be expected to
    /// reproduce, and there is nothing else in a single-corpus session for
    /// a case variant to collide with.
    #[must_use]
    pub fn matches(&self, candidate: &str) -> bool {
        self.get()
            .is_some_and(|label| label.eq_ignore_ascii_case(candidate.trim()))
    }
}

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
    /// Cloud mode. `default_service` answers calls with no `project`
    /// argument (compatibility with single-corpus tools); `registry`
    /// resolves a `project = corpus_id` argument through the shared
    /// daemon registry, including the lazy blob-restore path wired by
    /// `cmd_serve_http`. Restoring a corpus on demand means every `/mcp`
    /// tool call observes the same source of truth the REST surface does
    /// — without this variant the MCP layer routes every call through
    /// `default_service`, which is bound to an empty placeholder corpus
    /// on a fresh pod.
    ///
    /// `tenant_filter`, when wired, gates the `project → corpus_id`
    /// lookup. When the caller threads a `tenant_subject` and the filter
    /// denies, the resolver returns `Err(default_service)` (same shape
    /// as a typo) so the cross-tenant probe does not leak corpus
    /// existence. `None` filter ⇒ legacy permissive behaviour
    /// (self-hosted / single-tenant serve).
    Registry {
        default_service: Arc<QueryService>,
        registry: Arc<ministr_daemon::registry::CorpusRegistry>,
        tenant_filter: Option<Arc<dyn TenantCorpusFilter>>,
    },
}

#[allow(clippy::missing_errors_doc, clippy::too_many_arguments)] // dispatch methods share one explicit routing contract
impl Backend {
    /// Canonical corpus id for a routed call when it is known without I/O.
    ///
    /// # Errors
    ///
    /// Returns [`BackendError::UnknownProject`] when the route is not registered.
    pub fn routed_corpus_id(&self, project: Option<&str>) -> Result<String, BackendError> {
        self.validate_project_route(project)?;
        match self {
            Self::Local(_) => Ok(ministr_core::types::PRIMARY_CORPUS_ID.to_string()),
            Self::Daemon(backend) => Ok(backend.corpus_id().to_string()),
            Self::DaemonMulti(backends) => {
                Ok(backends.for_project(project)?.corpus_id().to_string())
            }
            Self::Registry { .. } => Ok(project
                .unwrap_or(ministr_core::types::PRIMARY_CORPUS_ID)
                .to_string()),
        }
    }

    /// Record the current project's label so a `project` argument naming
    /// the session's own project resolves to it.
    ///
    /// Called once at boot with the corpus paths' root directory name. A
    /// no-op for the cloud `Registry` variant, where routes are corpus ids
    /// resolved per call against the shared registry.
    pub fn set_primary_label(&self, label: &str) {
        match self {
            Self::Local(b) => b.primary_label().set(label),
            Self::Daemon(b) => b.primary_label().set(label),
            Self::DaemonMulti(m) => m.primary_label().set(label),
            Self::Registry { .. } => {}
        }
    }

    /// The current project's label, when it was resolved at boot.
    #[must_use]
    pub fn primary_label(&self) -> Option<&str> {
        match self {
            Self::Local(b) => b.primary_label().get(),
            Self::Daemon(b) => b.primary_label().get(),
            Self::DaemonMulti(m) => m.primary_label().get(),
            Self::Registry { .. } => None,
        }
    }

    /// Whether `project` names the corpus this session is primarily
    /// serving — by label, by corpus id, or by the primary sentinel.
    #[must_use]
    fn route_is_primary(&self, project: &str) -> bool {
        let project = project.trim();
        match self {
            Self::Local(b) => {
                b.primary_label().matches(project)
                    || project == ministr_core::types::PRIMARY_CORPUS_ID
            }
            Self::Daemon(b) => b.primary_label().matches(project) || b.corpus_id() == project,
            Self::DaemonMulti(m) => m.route_is_primary(project),
            Self::Registry { .. } => false,
        }
    }

    /// Every route this backend accepts: the current project's label (when
    /// known) followed by linked labels. Feeds the unknown-route message.
    #[must_use]
    pub fn available_routes(&self) -> Vec<String> {
        let mut routes: Vec<String> = self
            .primary_label()
            .map(|label| vec![label.to_string()])
            .unwrap_or_default();
        routes.extend(self.linked_labels());
        routes
    }

    /// Build the unknown-route error with this backend's accepted routes
    /// attached, so the caller never has to guess what would have worked.
    fn unknown_project(&self, requested: &str) -> BackendError {
        BackendError::UnknownProject {
            requested: requested.to_string(),
            available: self.available_routes(),
        }
    }

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

    /// Construct a cloud-mode backend with a tenant-isolation filter.
    /// Dispatch calls that pass a `tenant_subject` will be rejected via
    /// the typo-tolerance fallback when the filter denies access.
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
            Self::DaemonMulti(m) => m.for_project(project).ok(),
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

    /// Fetch daemon-owned prefetch measurements for the selected route.
    ///
    /// # Errors
    ///
    /// Returns a backend error when the route is invalid or the daemon request fails.
    pub async fn daemon_prefetch_metrics(
        &self,
        project: Option<&str>,
    ) -> Result<Option<ministr_api::session::PrefetchMetricsResponse>, BackendError> {
        let Some(backend) = self.daemon_for_project(project) else {
            return Ok(None);
        };
        Ok(Some(
            backend
                .client()
                .prefetch_metrics(backend.corpus_id())
                .await?,
        ))
    }

    /// Fetch daemon-owned prefetch measurements for every routed corpus.
    pub async fn all_daemon_prefetch_metrics(
        &self,
    ) -> Vec<ministr_api::session::PrefetchMetricsResponse> {
        let backends: Vec<&Arc<DaemonBackend>> = match self {
            Self::Daemon(backend) => vec![backend],
            Self::DaemonMulti(backends) => backends.all_backends(),
            Self::Local(_) | Self::Registry { .. } => return Vec::new(),
        };
        let mut metrics = Vec::with_capacity(backends.len());
        for backend in backends {
            if let Ok(value) = backend.client().prefetch_metrics(backend.corpus_id()).await {
                metrics.push(value);
            }
        }
        metrics
    }

    /// Remove exact deliveries from whichever daemon corpus owns them.
    ///
    /// Local and registry backends deduplicate from the MCP exclusion set, so
    /// their session shadow is updated by the caller and needs no second write.
    pub async fn drop_deliveries(
        &self,
        identities: &[DeliveryIdentity],
        content_ids: &[String],
    ) -> Result<(), BackendError> {
        match self {
            Self::Local(_) | Self::Registry { .. } => Ok(()),
            Self::Daemon(backend) => {
                let owned: Vec<_> = identities
                    .iter()
                    .filter(|identity| identity.corpus_id == backend.corpus_id())
                    .cloned()
                    .collect();
                backend.drop_deliveries(&owned, content_ids).await
            }
            Self::DaemonMulti(backends) => {
                let primary_corpus = backends.default_backend().corpus_id();
                for backend in backends.all_backends() {
                    let owned: Vec<_> = identities
                        .iter()
                        .filter(|identity| identity.corpus_id == backend.corpus_id())
                        .cloned()
                        .collect();
                    let legacy_ids = if backend.corpus_id() == primary_corpus {
                        content_ids
                    } else {
                        &[]
                    };
                    backend.drop_deliveries(&owned, legacy_ids).await?;
                }
                Ok(())
            }
        }
    }

    /// Return one reference page without truncating daemon results before the
    /// MCP pagination layer sees their total or continuation.
    pub async fn references_page(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
        through_implementors: bool,
        offset: Option<usize>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ReferencesBackendResponse, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Daemon(backend) => {
                backend
                    .references_page(
                        symbol_id,
                        ref_kind,
                        through_implementors,
                        offset,
                        cursor,
                        limit,
                    )
                    .await
            }
            Self::DaemonMulti(backends) => {
                backends
                    .for_project(project)?
                    .references_page(
                        symbol_id,
                        ref_kind,
                        through_implementors,
                        offset,
                        cursor,
                        limit,
                    )
                    .await
            }
            Self::Local(_) | Self::Registry { .. } => {
                let response = self
                    .references(
                        tenant_subject,
                        project,
                        symbol_id,
                        ref_kind,
                        through_implementors,
                    )
                    .await?;
                paginate_references(response.data, response.metadata, offset, cursor, limit)
            }
        }
    }

    /// Return one extracted-claim page while preserving daemon totals.
    pub async fn extract_claims_page(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        section_id: &str,
        query: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<CollectionBackendResponse<ClaimResult>, BackendError> {
        match self {
            Self::Daemon(backend) => {
                backend
                    .extract_claims_page(section_id, query, offset, limit)
                    .await
            }
            Self::DaemonMulti(backends) => {
                backends
                    .for_project(project)?
                    .extract_claims_page(section_id, query, offset, limit)
                    .await
            }
            Self::Local(_) | Self::Registry { .. } => {
                let response = self
                    .extract_claims(tenant_subject, project, section_id, query)
                    .await?;
                Ok(paginate_collection(
                    response.data,
                    response.metadata,
                    offset,
                    limit,
                ))
            }
        }
    }

    /// Return one symbol page while preserving daemon totals.
    pub async fn search_symbols_page(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        filter: SymbolFilter,
        offset: usize,
        limit: usize,
    ) -> Result<CollectionBackendResponse<SymbolRecord>, BackendError> {
        match self {
            Self::Daemon(backend) => backend.search_symbols_page(filter, offset, limit).await,
            Self::DaemonMulti(backends) => {
                backends
                    .for_project(project)?
                    .search_symbols_page(filter, offset, limit)
                    .await
            }
            Self::Local(_) | Self::Registry { .. } => {
                let response = self.search_symbols(tenant_subject, project, filter).await?;
                Ok(paginate_collection(
                    response.data,
                    response.metadata,
                    offset,
                    limit,
                ))
            }
        }
    }

    /// Return one impact page using a stable item cursor.
    #[allow(clippy::too_many_arguments)]
    pub async fn impact_page(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        symbol_id: &str,
        max_depth: u32,
        direction: CallDirection,
        tests_only: bool,
        offset: Option<usize>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ImpactBackendResponse, BackendError> {
        match self {
            Self::Daemon(backend) => {
                backend
                    .impact_page(
                        symbol_id, max_depth, direction, tests_only, offset, cursor, limit,
                    )
                    .await
            }
            Self::DaemonMulti(backends) => {
                backends
                    .for_project(project)?
                    .impact_page(
                        symbol_id, max_depth, direction, tests_only, offset, cursor, limit,
                    )
                    .await
            }
            Self::Local(_) | Self::Registry { .. } => {
                let response = self
                    .impact(
                        tenant_subject,
                        project,
                        symbol_id,
                        max_depth,
                        direction,
                        tests_only,
                    )
                    .await?;
                paginate_impact(response.data, response.metadata, offset, cursor, limit)
            }
        }
    }

    /// Return one dead-code page while preserving daemon totals.
    #[allow(clippy::too_many_arguments)]
    pub async fn dead_code_page(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        kind: Option<&str>,
        module: Option<&str>,
        min_lines: u32,
        offset: usize,
        limit: usize,
    ) -> Result<CollectionBackendResponse<DeadSymbol>, BackendError> {
        match self {
            Self::Daemon(backend) => {
                backend
                    .dead_code_page(kind, module, min_lines, offset, limit)
                    .await
            }
            Self::DaemonMulti(backends) => {
                backends
                    .for_project(project)?
                    .dead_code_page(kind, module, min_lines, offset, limit)
                    .await
            }
            Self::Local(_) | Self::Registry { .. } => {
                let response = self
                    .dead_code(tenant_subject, project, kind, module, min_lines, 500)
                    .await?;
                Ok(paginate_collection(
                    response.data,
                    response.metadata,
                    offset,
                    limit,
                ))
            }
        }
    }

    /// Return one diagnostics page while preserving daemon totals.
    pub async fn diagnostics_page(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        languages: Option<&[String]>,
        offset: usize,
        limit: usize,
    ) -> Result<CollectionBackendResponse<Diagnostic>, BackendError> {
        match self {
            Self::Daemon(backend) => backend.diagnostics_page(languages, offset, limit).await,
            Self::DaemonMulti(backends) => {
                backends
                    .for_project(project)?
                    .diagnostics_page(languages, offset, limit)
                    .await
            }
            Self::Local(_) | Self::Registry { .. } => {
                let response = self
                    .diagnostics(tenant_subject, project, languages, 500)
                    .await?;
                Ok(paginate_collection(
                    response.data,
                    response.metadata,
                    offset,
                    limit,
                ))
            }
        }
    }

    /// Return one SOLID finding page while preserving daemon totals.
    pub async fn solid_page(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        params: &SolidParams,
        offset: usize,
        limit: usize,
    ) -> Result<CollectionBackendResponse<SolidFinding>, BackendError> {
        match self {
            Self::Daemon(backend) => backend.solid_page(params, offset, limit).await,
            Self::DaemonMulti(backends) => {
                backends
                    .for_project(project)?
                    .solid_page(params, offset, limit)
                    .await
            }
            Self::Local(_) | Self::Registry { .. } => {
                let mut full_params = params.clone();
                full_params.limit = 500;
                let response = self.solid(tenant_subject, project, &full_params).await?;
                Ok(paginate_collection(
                    response.data,
                    response.metadata,
                    offset,
                    limit,
                ))
            }
        }
    }

    /// Return one related-claim page using a stable edge cursor.
    #[allow(clippy::too_many_arguments)]
    pub async fn related_claims_page(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        claim_id: &str,
        relation_types: Option<&[RelationType]>,
        offset: Option<usize>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<CollectionBackendResponse<RelatedClaimResult>, BackendError> {
        match self {
            Self::Daemon(backend) => {
                backend
                    .related_claims_page(claim_id, relation_types, offset, cursor, limit)
                    .await
            }
            Self::DaemonMulti(backends) => {
                backends
                    .for_project(project)?
                    .related_claims_page(claim_id, relation_types, offset, cursor, limit)
                    .await
            }
            Self::Local(_) | Self::Registry { .. } => {
                let response = self
                    .related_claims(tenant_subject, project, claim_id, relation_types)
                    .await?;
                paginate_related(response.data, response.metadata, offset, cursor, limit)
            }
        }
    }

    /// Return one TOC page while preserving daemon totals.
    pub async fn toc_page(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        document_id: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<TocBackendResponse, BackendError> {
        match self {
            Self::Daemon(backend) => backend.toc_page(document_id, offset, limit).await,
            Self::DaemonMulti(backends) => {
                backends
                    .for_project(project)?
                    .toc_page(document_id, offset, limit)
                    .await
            }
            Self::Local(_) | Self::Registry { .. } => {
                let response = self.toc(tenant_subject, project, document_id).await?;
                let claims = response
                    .data
                    .iter()
                    .map(|entry| entry.claims_available)
                    .sum();
                let documents = response
                    .data
                    .iter()
                    .map(|entry| entry.document_id.as_ref())
                    .collect::<std::collections::HashSet<_>>()
                    .len();
                let page = paginate_collection(response.data, response.metadata, offset, limit);
                Ok(TocBackendResponse {
                    entries: page.data,
                    documents,
                    claims,
                    pagination: page.pagination,
                    metadata: page.metadata,
                })
            }
        }
    }

    /// Return one bridge page while preserving daemon totals.
    #[allow(clippy::too_many_arguments)]
    pub async fn bridges_page(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        query: Option<&str>,
        kind: Option<&str>,
        language: Option<&str>,
        file_path: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<CollectionBackendResponse<BridgeLinkDetail>, BackendError> {
        match self {
            Self::Daemon(backend) => {
                backend
                    .bridges_page(query, kind, language, file_path, offset, limit)
                    .await
            }
            Self::DaemonMulti(backends) => {
                backends
                    .for_project(project)?
                    .bridges_page(query, kind, language, file_path, offset, limit)
                    .await
            }
            Self::Local(_) | Self::Registry { .. } => {
                let response = self
                    .bridges(tenant_subject, project, query, kind, language, file_path)
                    .await?;
                Ok(paginate_collection(
                    response.data,
                    response.metadata,
                    offset,
                    limit,
                ))
            }
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
    /// When `project = None` AND the caller threaded a `tenant_subject`
    /// AND a `tenant_filter` is wired, ask the filter for the tenant's
    /// default corpus (currently: most-recently-created). If found,
    /// `ensure_present` that `corpus_id` and dispatch through its
    /// `QueryService`. If the filter returns `None` (or the lookup
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
        // None project, tenant in scope: ask the filter for the tenant's
        // default corpus. Allocate a String so the rest of the resolver
        // works against a borrowed `&str` uniformly, without forcing the
        // trait method to hand out a borrowed Cow.
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
        // Gate the lookup behind the tenant filter when one is wired AND
        // the caller threaded its tenant identity. A missing
        // tenant_subject in cloud mode is itself a deny: handlers that
        // accept a `project` argument MUST extract the Tenant from
        // RequestContext and pass its subject. A `None` arrives only on
        // self-hosted serve (where tenant_filter is None too). The
        // default-resolution branch above already used the filter to pick
        // the corpus_id, but we re-check `allowed` here for uniform
        // treatment — the same filter implementation will obviously
        // approve its own choice.
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
#[allow(clippy::missing_errors_doc, clippy::too_many_arguments)]
impl Backend {
    /// Gate a `project` route before dispatch.
    ///
    /// Single-corpus backends accept only a route that names the corpus
    /// they serve — its label, its corpus id, or the primary sentinel.
    /// Anything else is rejected rather than silently answered from the
    /// primary, because an agent that believes it queried another repo and
    /// gets this one back is worse off than one that gets an error naming
    /// the routes that exist. Multi-corpus and registry variants resolve
    /// the label per call ([`DaemonMultiBackend::for_project`] /
    /// [`Self::resolve_registry_handle`]).
    fn validate_project_route(&self, project: Option<&str>) -> Result<(), BackendError> {
        let Some(project) = project else {
            return Ok(());
        };
        match self {
            Self::Local(_) | Self::Daemon(_) if !self.route_is_primary(project) => {
                Err(self.unknown_project(project))
            }
            Self::Local(_) | Self::Daemon(_) | Self::DaemonMulti(_) | Self::Registry { .. } => {
                Ok(())
            }
        }
    }

    async fn validate_registry_project(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
    ) -> Result<(), BackendError> {
        let (
            Self::Registry {
                registry,
                tenant_filter,
                ..
            },
            Some(corpus_id),
        ) = (self, project)
        else {
            return Ok(());
        };

        if let Some(filter) = tenant_filter {
            let Some(subject) = tenant_subject else {
                return Err(BackendError::PermissionDenied(corpus_id.to_string()));
            };
            match filter.allowed(subject, corpus_id).await {
                Ok(true) => {}
                Ok(false) | Err(_) => {
                    return Err(BackendError::PermissionDenied(corpus_id.to_string()));
                }
            }
        }
        registry
            .ensure_present(corpus_id)
            .await
            .map(|_| ())
            .map_err(|_| BackendError::CorpusUnavailable(corpus_id.to_string()))
    }

    pub async fn survey(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SurveyResult>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.survey(query, top_k).await,
            Self::Daemon(b) => b.survey(query, top_k).await,
            Self::DaemonMulti(m) => m.for_project(project)?.survey(query, top_k).await,
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

    /// Ranked survey candidates without session exclusion, for continuation
    /// paging where offset must be applied before deduplication.
    pub async fn survey_window(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        query: &str,
        top_k: usize,
    ) -> Result<SurveyBackendResponse, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        let empty = std::collections::HashSet::new();
        match self {
            Self::Local(backend) => backend.survey_with_exclude(query, top_k, &empty).await,
            Self::Daemon(backend) => backend.survey_window(query, top_k).await,
            Self::DaemonMulti(backends) => {
                backends
                    .for_project(project)?
                    .survey_window(query, top_k)
                    .await
            }
            Self::Registry {
                default_service,
                registry,
                tenant_filter,
            } => {
                let (results, suppressed_identities) = match Self::resolve_registry_handle(
                    default_service,
                    registry,
                    tenant_filter.as_ref(),
                    tenant_subject,
                    project,
                )
                .await
                {
                    Ok(handle) => {
                        let corpus_id = handle.current_info().await.id;
                        handle
                            .service
                            .survey_excluding_identities_detailed_with_options(
                                query,
                                top_k,
                                &corpus_id,
                                &empty,
                                survey_candidate_options(),
                            )
                            .await?
                    }
                    Err(default) => {
                        default
                            .survey_excluding_identities_detailed_with_options(
                                query,
                                top_k,
                                ministr_core::types::PRIMARY_CORPUS_ID,
                                &empty,
                                survey_candidate_options(),
                            )
                            .await?
                    }
                };
                Ok(SurveyBackendResponse {
                    results,
                    deduplicated_count: suppressed_identities.len(),
                    suppressed_identities,
                    metadata: ministr_api::metadata::QueryMetadata::default(),
                })
            }
        }
    }

    pub async fn survey_with_exclude(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        query: &str,
        top_k: usize,
        exclude_ids: &std::collections::HashSet<DeliveryIdentity>,
    ) -> Result<SurveyBackendResponse, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.survey_with_exclude(query, top_k, exclude_ids).await,
            Self::Daemon(b) => b.survey_with_exclude(query, top_k, exclude_ids).await,
            Self::DaemonMulti(m) => {
                m.for_project(project)?
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
                Ok(handle) => {
                    let corpus_id = handle.current_info().await.id;
                    let (results, suppressed_identities) = handle
                        .service
                        .survey_excluding_identities_detailed_with_options(
                            query,
                            top_k,
                            &corpus_id,
                            exclude_ids,
                            survey_candidate_options(),
                        )
                        .await?;
                    let deduplicated_count = suppressed_identities.len();
                    Ok(SurveyBackendResponse {
                        results,
                        deduplicated_count,
                        suppressed_identities,
                        metadata: ministr_api::metadata::QueryMetadata::default(),
                    })
                }
                Err(default) => {
                    let (results, suppressed_identities) = default
                        .survey_excluding_identities_detailed_with_options(
                            query,
                            top_k,
                            ministr_core::types::PRIMARY_CORPUS_ID,
                            exclude_ids,
                            survey_candidate_options(),
                        )
                        .await?;
                    let deduplicated_count = suppressed_identities.len();
                    Ok(SurveyBackendResponse {
                        results,
                        deduplicated_count,
                        suppressed_identities,
                        metadata: ministr_api::metadata::QueryMetadata::default(),
                    })
                }
            },
        }
    }

    pub async fn read_section(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        section_id: &str,
    ) -> Result<BackendResponse<SectionDetail>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.read_section(section_id).await,
            Self::Daemon(b) => b.read_section(section_id).await,
            Self::DaemonMulti(m) => m.for_project(project)?.read_section(section_id).await,
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
                Ok(handle) => Ok(BackendResponse::complete(
                    handle.service.read_section(section_id).await?,
                )),
                Err(default) => Ok(BackendResponse::complete(
                    default.read_section(section_id).await?,
                )),
            },
        }
    }

    pub async fn extract_claims(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        section_id: &str,
        query: Option<&str>,
    ) -> Result<BackendResponse<Vec<ClaimResult>>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.extract_claims(section_id, query).await,
            Self::Daemon(b) => b.extract_claims(section_id, query).await,
            Self::DaemonMulti(m) => {
                m.for_project(project)?
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
                Ok(handle) => Ok(BackendResponse::complete(
                    handle.service.extract_claims(section_id, query).await?,
                )),
                Err(default) => Ok(BackendResponse::complete(
                    default.extract_claims(section_id, query).await?,
                )),
            },
        }
    }

    pub async fn search_symbols(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        filter: SymbolFilter,
    ) -> Result<BackendResponse<Vec<SymbolRecord>>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.search_symbols(filter).await,
            Self::Daemon(b) => b.search_symbols(filter).await,
            Self::DaemonMulti(m) => m.for_project(project)?.search_symbols(filter).await,
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
                Ok(handle) => Ok(BackendResponse::complete(
                    handle.service.search_symbols(&filter).await?,
                )),
                Err(default) => Ok(BackendResponse::complete(
                    default.search_symbols(&filter).await?,
                )),
            },
        }
    }

    pub async fn definition(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        symbol_id: &str,
        options: DefinitionOptions,
    ) -> Result<BackendResponse<SymbolDefinition>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.definition(symbol_id, options).await,
            Self::Daemon(b) => b.definition(symbol_id, options).await,
            Self::DaemonMulti(m) => m.for_project(project)?.definition(symbol_id, options).await,
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
                Ok(handle) => Ok(BackendResponse::complete(
                    handle
                        .service
                        .get_symbol_definition_with_options(symbol_id, options)
                        .await?,
                )),
                Err(default) => Ok(BackendResponse::complete(
                    default
                        .get_symbol_definition_with_options(symbol_id, options)
                        .await?,
                )),
            },
        }
    }

    pub async fn inspect_symbol(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        symbol_id: &str,
        options: InspectOptions,
    ) -> Result<BackendResponse<InspectResult>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(backend) => backend.inspect_symbol(symbol_id, options).await,
            Self::Daemon(backend) => backend.inspect_symbol(symbol_id, options).await,
            Self::DaemonMulti(backends) => {
                backends
                    .for_project(project)?
                    .inspect_symbol(symbol_id, options)
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
                Ok(handle) => Ok(BackendResponse::complete(
                    handle.service.inspect_symbol(symbol_id, &options).await?,
                )),
                Err(default) => Ok(BackendResponse::complete(
                    default.inspect_symbol(symbol_id, &options).await?,
                )),
            },
        }
    }

    pub async fn inspect_at_position(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        file_path: &str,
        line: u32,
        col: u32,
        options: InspectOptions,
    ) -> Result<BackendResponse<InspectResult>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(backend) => {
                backend
                    .inspect_at_position(file_path, line, col, options)
                    .await
            }
            Self::Daemon(backend) => {
                backend
                    .inspect_at_position(file_path, line, col, options)
                    .await
            }
            Self::DaemonMulti(backends) => {
                backends
                    .for_project(project)?
                    .inspect_at_position(file_path, line, col, options)
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
                Ok(handle) => Ok(BackendResponse::complete(
                    handle
                        .service
                        .inspect_at_position(file_path, line, col, &options)
                        .await?,
                )),
                Err(default) => Ok(BackendResponse::complete(
                    default
                        .inspect_at_position(file_path, line, col, &options)
                        .await?,
                )),
            },
        }
    }

    pub async fn references(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        symbol_id: &str,
        ref_kind: Option<RefKind>,
        through_implementors: bool,
    ) -> Result<BackendResponse<Vec<SymbolRefResult>>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => {
                b.references(symbol_id, ref_kind, through_implementors)
                    .await
            }
            Self::Daemon(b) => {
                b.references(symbol_id, ref_kind, through_implementors)
                    .await
            }
            Self::DaemonMulti(m) => {
                m.for_project(project)?
                    .references(symbol_id, ref_kind, through_implementors)
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
                Ok(handle) if through_implementors => Ok(BackendResponse::complete(
                    handle
                        .service
                        .get_symbol_references_through_implementors(symbol_id, ref_kind, 500)
                        .await?,
                )),
                Ok(handle) => Ok(BackendResponse::complete(
                    handle
                        .service
                        .get_symbol_references(symbol_id, ref_kind)
                        .await?,
                )),
                Err(default) if through_implementors => Ok(BackendResponse::complete(
                    default
                        .get_symbol_references_through_implementors(symbol_id, ref_kind, 500)
                        .await?,
                )),
                Err(default) => Ok(BackendResponse::complete(
                    default.get_symbol_references(symbol_id, ref_kind).await?,
                )),
            },
        }
    }

    pub async fn impact(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        symbol_id: &str,
        max_depth: u32,
        direction: CallDirection,
        tests_only: bool,
    ) -> Result<BackendResponse<ImpactResult>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.impact(symbol_id, max_depth, direction, tests_only).await,
            Self::Daemon(b) => b.impact(symbol_id, max_depth, direction, tests_only).await,
            Self::DaemonMulti(m) => {
                m.for_project(project)?
                    .impact(symbol_id, max_depth, direction, tests_only)
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
                Ok(handle) => Ok(BackendResponse::complete(
                    handle
                        .service
                        .compute_impact(symbol_id, max_depth, direction, tests_only)
                        .await?,
                )),
                Err(default) => Ok(BackendResponse::complete(
                    default
                        .compute_impact(symbol_id, max_depth, direction, tests_only)
                        .await?,
                )),
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
    ) -> Result<BackendResponse<Vec<DeadSymbol>>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.dead_code(kind, module, min_lines, limit).await,
            Self::Daemon(b) => b.dead_code(kind, module, min_lines, limit).await,
            Self::DaemonMulti(m) => {
                m.for_project(project)?
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
                Ok(handle) => Ok(BackendResponse::complete(
                    handle
                        .service
                        .find_dead_code(kind, module, min_lines, limit)
                        .await?,
                )),
                Err(default) => Ok(BackendResponse::complete(
                    default
                        .find_dead_code(kind, module, min_lines, limit)
                        .await?,
                )),
            },
        }
    }

    pub async fn diagnostics(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        languages: Option<&[String]>,
        limit: usize,
    ) -> Result<BackendResponse<Vec<Diagnostic>>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.diagnostics(languages, limit).await,
            Self::Daemon(b) => b.diagnostics(languages, limit).await,
            Self::DaemonMulti(m) => m.for_project(project)?.diagnostics(languages, limit).await,
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
                Ok(handle) => Ok(BackendResponse::complete(
                    handle.service.diagnostics(languages, limit).await?,
                )),
                Err(default) => Ok(BackendResponse::complete(
                    default.diagnostics(languages, limit).await?,
                )),
            },
        }
    }

    pub async fn solid(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        params: &SolidParams,
    ) -> Result<BackendResponse<Vec<SolidFinding>>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.solid(params).await,
            Self::Daemon(b) => b.solid(params).await,
            Self::DaemonMulti(m) => m.for_project(project)?.solid(params).await,
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
                Ok(handle) => Ok(BackendResponse::complete(
                    handle.service.detect_solid_violations(params).await?,
                )),
                Err(default) => Ok(BackendResponse::complete(
                    default.detect_solid_violations(params).await?,
                )),
            },
        }
    }

    pub async fn related_claims(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        claim_id: &str,
        relation_types: Option<&[RelationType]>,
    ) -> Result<BackendResponse<Vec<RelatedClaimResult>>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.related_claims(claim_id, relation_types).await,
            Self::Daemon(b) => b.related_claims(claim_id, relation_types).await,
            Self::DaemonMulti(m) => {
                m.for_project(project)?
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
                Ok(handle) => Ok(BackendResponse::complete(
                    handle
                        .service
                        .related_claims(claim_id, relation_types)
                        .await?,
                )),
                Err(default) => Ok(BackendResponse::complete(
                    default.related_claims(claim_id, relation_types).await?,
                )),
            },
        }
    }

    /// Corpus local directory roots (abs path + root id) for reconstructing a
    /// stored index key from a changed file's absolute path in diff-impact
    /// (ingest-key-locator-decouple). Daemon-forward backends return empty —
    /// diff-impact there falls back to absolute-key matching; the daemon
    /// resolves relative corpora via its own in-process Registry backend.
    pub async fn corpus_roots(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
    ) -> Vec<(std::path::PathBuf, String)> {
        match self {
            Self::Local(b) => b.local_dir_roots().await,
            Self::Daemon(_) | Self::DaemonMulti(_) => Vec::new(),
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
                Ok(handle) => handle.service.local_dir_roots().await,
                Err(default) => default.local_dir_roots().await,
            },
        }
    }

    pub async fn compress(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        identities: &[DeliveryIdentity],
    ) -> Result<Vec<CompressedDelivery>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.compress(identities).await,
            Self::Daemon(b) => b.compress(identities).await,
            Self::DaemonMulti(m) => m.for_project(project)?.compress(identities).await,
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
                Ok(handle) => compress_with_service(&handle.service, identities).await,
                Err(default) => compress_with_service(default, identities).await,
            },
        }
    }

    pub async fn toc(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        document_id: Option<&str>,
    ) -> Result<BackendResponse<Vec<TocEntry>>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.toc(document_id).await,
            Self::Daemon(b) => b.toc(document_id).await,
            Self::DaemonMulti(m) => m.for_project(project)?.toc(document_id).await,
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
                Ok(handle) => Ok(BackendResponse::complete(
                    handle.service.toc(document_id).await?,
                )),
                Err(default) => Ok(BackendResponse::complete(default.toc(document_id).await?)),
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
    ) -> Result<BackendResponse<Vec<BridgeLinkDetail>>, BackendError> {
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.bridges(query, kind, language, file_path).await,
            Self::Daemon(b) => b.bridges(query, kind, language, file_path).await,
            Self::DaemonMulti(m) => {
                m.for_project(project)?
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
                Ok(handle) => Ok(BackendResponse::complete(
                    handle
                        .service
                        .query_bridges(query, kind, language, file_path)
                        .await?,
                )),
                Err(default) => Ok(BackendResponse::complete(
                    default
                        .query_bridges(query, kind, language, file_path)
                        .await?,
                )),
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
        self.validate_project_route(project)?;
        self.validate_registry_project(tenant_subject, project)
            .await?;
        match self {
            Self::Local(b) => b.symbol_at_position(file_path, line, col).await,
            Self::Daemon(b) => b.symbol_at_position(file_path, line, col).await,
            Self::DaemonMulti(m) => {
                m.for_project(project)?
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
    //! Tenant-filter behaviour tests for `Backend::Registry`.
    //!
    //! These exercise `resolve_registry_handle` in isolation so they don't
    //! need a live `CorpusRegistry` fixture (that's covered by the
    //! daemon's `tests/common`). The tests focus on what the resolver
    //! decides given the filter alone:
    //! - `tenant_filter = Some` + `tenant_subject = None` denies.
    //! - `tenant_filter = Some` + filter returns `Ok(false)` denies.
    //! - `tenant_filter = Some` + filter returns `Err` denies (fail closed).
    //! - `project = None` consults `default_corpus_for_tenant`; falls
    //!   back to `default_service` when that returns `None` or `Err`.

    use super::*;

    fn daemon_backend(corpus_id: &str) -> Backend {
        Backend::daemon(
            Arc::new(ministr_api::client::DaemonClient::new()),
            corpus_id.to_string(),
            None,
        )
    }

    /// The kadodi bug: a single-corpus session rejected `project: "<its own
    /// repo>"` as an unavailable corpus, while every call that omitted
    /// `project` worked — so the miss read as staleness, not a misroute.
    #[test]
    fn single_corpus_backend_accepts_its_own_project_label() {
        let backend = daemon_backend("multi-d6edc116");

        assert!(matches!(
            backend.validate_project_route(Some("kadodi")),
            Err(BackendError::UnknownProject { .. })
        ));

        backend.set_primary_label("kadodi");

        assert!(backend.validate_project_route(Some("kadodi")).is_ok());
        assert!(backend.validate_project_route(Some("KADODI")).is_ok());
        // The corpus id and "no route at all" keep working.
        assert!(
            backend
                .validate_project_route(Some("multi-d6edc116"))
                .is_ok()
        );
        assert!(backend.validate_project_route(None).is_ok());
    }

    #[test]
    fn single_corpus_backend_still_rejects_another_projects_label() {
        let backend = daemon_backend("multi-d6edc116");
        backend.set_primary_label("kadodi");

        // Answering this from kadodi's corpus would silently hand the agent
        // the wrong repo's code — worse than an error.
        let Err(BackendError::UnknownProject {
            requested,
            available,
        }) = backend.validate_project_route(Some("ministr-private"))
        else {
            panic!("expected an unknown-route error");
        };
        assert_eq!(requested, "ministr-private");
        assert_eq!(available, ["kadodi"]);
    }

    #[test]
    fn routed_corpus_id_resolves_the_primary_label() {
        let backend = daemon_backend("multi-d6edc116");
        backend.set_primary_label("kadodi");
        assert_eq!(
            backend.routed_corpus_id(Some("kadodi")).unwrap(),
            "multi-d6edc116"
        );
    }

    #[test]
    fn primary_label_is_set_once() {
        let backend = daemon_backend("corpus-1");
        backend.set_primary_label("kadodi");
        backend.set_primary_label("something-else");
        assert_eq!(backend.primary_label(), Some("kadodi"));
        assert!(
            backend
                .validate_project_route(Some("something-else"))
                .is_err()
        );
    }

    #[test]
    fn blank_labels_are_ignored() {
        let backend = daemon_backend("corpus-1");
        backend.set_primary_label("   ");
        assert_eq!(backend.primary_label(), None);
        assert!(backend.available_routes().is_empty());
    }

    fn related(id: &str) -> RelatedClaimResult {
        RelatedClaimResult {
            claim_id: id.to_string(),
            text: format!("claim {id}"),
            relation_type: "references".to_string(),
            source_section: "docs/test.md#claims".to_string(),
            confidence: 1.0,
        }
    }

    #[test]
    fn related_cursor_remains_stable_when_an_edge_is_inserted_before_it() {
        let first = paginate_related(
            vec![related("b"), related("c"), related("d")],
            ministr_api::metadata::QueryMetadata::default(),
            None,
            None,
            2,
        )
        .unwrap();
        let cursor = first.pagination.next_cursor.unwrap();
        assert_eq!(first.data[1].claim_id, "c");

        let second = paginate_related(
            vec![related("a"), related("b"), related("c"), related("d")],
            ministr_api::metadata::QueryMetadata::default(),
            None,
            Some(&cursor),
            2,
        )
        .unwrap();
        assert_eq!(second.data.len(), 1);
        assert_eq!(second.data[0].claim_id, "d");
    }
    use ministr_api::tenant_filter::{
        DefaultCorpusFuture, TenantCorpusFilter, TenantFilterError, TenantFilterFuture,
    };
    use std::sync::Mutex;

    #[derive(Debug)]
    struct MockFilter {
        decision: Mutex<Result<bool, &'static str>>,
        /// Configurable response for `default_corpus_for_tenant`.
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

    /// `project = None` consults `default_corpus_for_tenant`. With no
    /// override set, the trait's default impl returns `None`, so the
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

    /// `project = None` + no `tenant_subject` → fall back without calling
    /// either filter method.
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

    /// `project = None` + no filter → fall back without any filter call
    /// (preserves self-hosted / single-tenant behaviour).
    #[tokio::test]
    async fn project_none_no_filter_falls_back() {
        let default = dummy_default_service();
        let registry = dummy_registry();
        let outcome =
            Backend::resolve_registry_handle(&default, &registry, None, Some("alice"), None).await;
        assert!(outcome.is_err());
    }

    /// When `default_corpus_for_tenant` returns `Some(id)`, the resolver
    /// re-checks `allowed` for the chosen corpus, then proceeds to
    /// `ensure_present`. In tests `ensure_present` errors (empty
    /// registry), so the outcome is Err — but BOTH filter methods were
    /// exercised.
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

    /// Storage error on `default_corpus_for_tenant` → fall back to
    /// `default_service` (don't crash, don't leak the error).
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
            embedder,
            "mock-model:test".to_string(),
            config,
        ))
    }
}
