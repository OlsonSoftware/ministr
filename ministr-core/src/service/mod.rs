//! Query service layer for ministr-core.
//!
//! [`QueryService`] composes the storage, embedding, and vector index
//! subsystems into a high-level API for searching, reading, and extracting
//! content from the corpus. This is the primary interface consumed by
//! transport adapters (e.g. the MCP server in `ministr-mcp`).

mod code;
mod compress;
mod diagnostics;
mod query;
mod solid;

use std::sync::Arc;

use serde::Serialize;
use tracing::instrument;

use crate::embedding::{DualEmbedder, Embedder, Reranker, SparseEmbedder};
use crate::error::{IndexError, StorageError};
use crate::index::{SparseIndex, VectorIndex};
use crate::storage::{SqliteStorage, Storage};
use crate::token::count_tokens;
use crate::types::{ContentId, CorpusRoot, TocEntry};

// Re-export the language-agnostic diagnostics types (defined alongside the
// toolchain registry in `crate::code::diagnostics`) so transport crates import
// them from `ministr_core::service`, like the other analysis-family types.
pub use crate::code::diagnostics::{Diagnostic, DiagnosticSeverity};

/// A ranked result from a corpus survey search.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SurveyResult {
    /// The content ID from the vector index.
    pub content_id: String,
    /// Resolution level of this result.
    pub resolution: String,
    /// Relevance score (higher is better, 0.0–1.0).
    pub score: f32,
    /// Content text — summary for summary-level, section text for section-level,
    /// claim text for claim-level.
    pub text: String,
    /// Heading path for section-level results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_path: Option<Vec<String>>,
    /// F6.3-a — corpus that produced this hit. `None` for single-corpus
    /// queries (the existing path; the agent already knows which
    /// corpus it queried). `Some(corpus_id)` for cross-corpus
    /// `corpus_ids` fan-out so the agent can disambiguate hits with
    /// colliding `content_id` strings across corpora.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_corpus: Option<String>,
}

/// Detailed section content returned by `read_section`.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SectionDetail {
    /// Section identifier.
    pub section_id: String,
    /// Heading hierarchy path.
    pub heading_path: Vec<String>,
    /// Full section text.
    pub text: String,
    /// Section summary, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Number of claims available for extraction.
    pub claims_available: usize,
}

/// A claim result from extraction.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ClaimResult {
    /// Claim identifier.
    pub claim_id: String,
    /// Claim text.
    pub text: String,
    /// Relevance score when filtered by query (0.0–1.0). `None` if no query.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relevance: Option<f32>,
}

/// A compressed summary of a content item, used for eviction.
///
/// When an agent wants to free budget, it can compress sections into shorter
/// summaries that preserve the gist while reducing token count.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct CompressedItem {
    /// The original content ID that was compressed.
    pub original_id: String,
    /// The compressed summary text.
    pub summary: String,
    /// Token count of the original content.
    pub original_tokens: usize,
    /// Token count of the compressed summary.
    pub compressed_tokens: usize,
    /// Compression method used: `"extractive"` or `"abstractive"`.
    pub method: String,
}

/// A related claim returned by `related_claims`.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct RelatedClaimResult {
    /// The related claim's ID.
    pub claim_id: String,
    /// The related claim's text.
    pub text: String,
    /// The type of relationship.
    pub relation_type: String,
    /// The section containing the related claim.
    pub source_section: String,
    /// Confidence score (0.0–1.0).
    pub confidence: f32,
}

/// A symbol definition with source context and module hierarchy.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SymbolDefinition {
    /// The symbol record from storage.
    pub id: String,
    /// Symbol name.
    pub name: String,
    /// Symbol kind (e.g. "function", "struct").
    pub kind: String,
    /// Visibility (e.g. "pub", "pub(crate)").
    pub visibility: String,
    /// Declaration signature (without body).
    pub signature: String,
    /// Doc comment text, if present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_comment: Option<String>,
    /// Source file path relative to corpus root.
    pub file_path: String,
    /// Start line (1-based).
    pub line_start: u32,
    /// End line (1-based, inclusive).
    pub line_end: u32,
    /// Module hierarchy path (e.g. `["config", "MinistrConfig"]`).
    pub heading_path: Vec<String>,
    /// Source code of the symbol with 3 lines of surrounding context.
    pub source_context: String,
}

/// A symbol reference result from cross-reference queries.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SymbolRefResult {
    /// The symbol that holds the reference.
    pub from_symbol_id: String,
    /// Name of the referencing symbol.
    pub from_name: String,
    /// File containing the referencing symbol.
    pub from_file: String,
    /// Line of the referencing symbol.
    pub from_line: u32,
    /// The symbol being referenced.
    pub to_symbol_id: String,
    /// Name of the referenced symbol.
    pub to_name: String,
    /// File containing the referenced symbol.
    pub to_file: String,
    /// Line of the referenced symbol.
    pub to_line: u32,
    /// The kind of reference.
    pub ref_kind: String,
}

/// Risk level for an impact analysis result.
#[derive(Debug, Clone, Copy, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ImpactRisk {
    Low,
    Medium,
    High,
}

/// One transitive caller in an impact analysis.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ImpactCaller {
    /// Symbol ID of the caller.
    pub symbol_id: String,
    /// Caller name.
    pub name: String,
    /// Symbol kind (e.g. "function", "method").
    pub kind: String,
    /// File containing the caller.
    pub file: String,
    /// Line where the caller is defined.
    pub line: u32,
    /// Depth in the call graph (1 = direct caller).
    pub depth: u32,
}

/// Direction to walk the call graph for [`QueryService::compute_impact`].
///
/// Both directions reuse the same depth-bounded, cycle-safe BFS over `Calls`
/// edges; only which endpoint of each edge is followed differs. This is the
/// LSP call-hierarchy distinction (incoming = "who calls this", outgoing =
/// "what does this call").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CallDirection {
    /// Transitive callers — who reaches this symbol (the blast radius). Default.
    #[default]
    Incoming,
    /// Transitive callees — what this symbol reaches (its fan-out).
    Outgoing,
}

impl CallDirection {
    /// Lowercase wire string (`"incoming"` / `"outgoing"`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Incoming => "incoming",
            Self::Outgoing => "outgoing",
        }
    }

    /// Parse a wire string; unrecognized / absent input is `None` so callers
    /// can fall back to the default (incoming).
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "incoming" | "callers" | "in" => Some(Self::Incoming),
            "outgoing" | "callees" | "out" => Some(Self::Outgoing),
            _ => None,
        }
    }
}

/// Result of computing the transitive impact of changing a symbol.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct ImpactResult {
    /// The target symbol ID that was analyzed.
    pub target_symbol_id: String,
    /// Direction walked: `incoming` (transitive callers) or `outgoing`
    /// (transitive callees). Tells the caller what `callers` actually holds.
    pub direction: CallDirection,
    /// Maximum BFS depth that was walked.
    pub depth: u32,
    /// Distinct transitive node count (callers for incoming, callees for outgoing).
    pub symbols: usize,
    /// Distinct files touched by the reached nodes.
    pub files: usize,
    /// Distinct test files among the touched files.
    pub tests: usize,
    /// Risk level (low / medium / high). Most meaningful for `incoming`.
    pub risk: ImpactRisk,
    /// Reached nodes (callers for incoming, callees for outgoing), ordered by
    /// depth then file then name.
    pub callers: Vec<ImpactCaller>,
}

/// A dead-code candidate: a non-public symbol with zero references.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct DeadSymbol {
    /// Symbol ID.
    pub symbol_id: String,
    /// Symbol name.
    pub name: String,
    /// Symbol kind (e.g. "function", "struct").
    pub kind: String,
    /// Visibility.
    pub visibility: String,
    /// File path.
    pub file: String,
    /// Line where the symbol is defined.
    pub line: u32,
    /// Line count of the symbol body.
    pub lines: u32,
}

/// Which SOLID concern a [`SolidFinding`] surfaces.
///
/// Used both as a filter on the input ([`SolidParams::principles`]) and as a
/// discriminator on the output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SolidPrinciple {
    /// DRY + OCP — near-duplicate symbols that probably want an extracted
    /// abstraction. Surfaces as a clustered [`SolidFinding::Redundancy`].
    DryOcp,
    /// Single-Responsibility — a container (impl / class / module) whose
    /// methods split into multiple cohesion clusters with little overlap.
    /// Surfaces as [`SolidFinding::LowCohesion`].
    Srp,
    /// Interface-Segregation — a trait / interface with many methods that
    /// most implementors only partially use. Surfaces as
    /// [`SolidFinding::FatInterface`].
    Isp,
    /// Dependency-Inversion — a consumer depending directly on a concrete
    /// symbol in another package when a trait abstraction is available.
    /// Surfaces as [`SolidFinding::ConcreteDependency`].
    Dip,
    /// Fowler's Shotgun Surgery — the same logical change must be made in
    /// many parallel sites (same `(name, kind)` symbol appears across N
    /// files with mostly disjoint callee sets). Surfaces as
    /// [`SolidFinding::ShotgunSurgery`]. Adjacent to OCP — the abstraction
    /// to introduce is usually a trait or strategy interface.
    ShotgunSurgery,
    /// Architectural cyclic dependency — a strongly-connected component
    /// of size ≥ 2 in the package-level import graph. Surfaces as
    /// [`SolidFinding::CyclicDependency`]. Adjacent to DIP — a layered
    /// abstraction is missing somewhere on the cycle.
    CyclicDependency,
}

/// Parameters for [`QueryService::detect_solid_violations`].
///
/// All fields have working defaults via [`SolidParams::default`]; callers
/// override only the knobs they care about.
#[derive(Debug, Clone)]
pub struct SolidParams {
    /// Limit candidate symbols to this kind (e.g. `"function"`). `None` = all.
    pub kind: Option<String>,
    /// Limit candidate symbols to this module path prefix. `None` = all.
    pub module: Option<String>,
    /// Which SOLID principles to evaluate. Empty = run all four.
    pub principles: Vec<SolidPrinciple>,
    /// Symbol kinds treated as "containers" for the SRP pseudo-LCOM4 sweep
    /// (Rust impl/struct, Python/TS class, Go file-as-package, etc.).
    pub container_kinds: Vec<String>,
    /// Symbol kinds treated as "interfaces" for the ISP fat-trait sweep
    /// (Rust trait, TS/Java interface, Python protocol, Go interface).
    pub interface_kinds: Vec<String>,
    /// Cosine threshold for the DRY/OCP clone detector (default 0.86).
    pub similarity_threshold: f32,
    /// Jaccard threshold over callee-sets for DRY/OCP (default 0.4).
    pub jaccard_threshold: f32,
    /// Cosine threshold for the SRP within-container edge graph (default 0.7).
    pub srp_cohesion_threshold: f32,
    /// Minimum method count for ISP to fire (default 6).
    pub isp_min_methods: usize,
    /// Implementor counts as "under-using" when it overlaps with at most this
    /// fraction of the trait's methods (default 0.33).
    pub isp_max_overlap_fraction: f32,
    /// Skip candidate symbols shorter than this many lines (default 5).
    pub min_lines: u32,
    /// Total findings cap across all principles (default 50, hard max 500).
    pub limit: usize,
    /// Hard cap on pairwise comparisons inside any single DRY/OCP bucket.
    /// Buckets larger than this are downsampled with a warning (default 100k).
    pub max_pairs: usize,
    /// Maximum representative members included per finding component list.
    /// When a component / member array exceeds this, it's truncated to the
    /// first N entries and the remainder is reported separately as
    /// `omitted` so the response stays well below MCP token budgets
    /// (default 5).
    pub representative_count: usize,
    /// Minimum number of files a `(name, kind)` group must span before
    /// [`SolidFinding::ShotgunSurgery`] fires (default 3).
    pub shotgun_min_sites: usize,
    /// Maximum callee-set Jaccard between two members of a Shotgun-Surgery
    /// group. Above this the group is dropped — at that point the pattern
    /// is a true DRY/OCP clone, not a Shotgun-Surgery dispatch family
    /// (default 0.5).
    pub shotgun_max_jaccard: f32,
    /// Sites of a Shotgun-Surgery group must span at least this many
    /// distinct package prefixes (see `package_prefix`). Single-package
    /// fan-out is usually intentional polymorphism inside one crate
    /// (e.g. per-language trait impls) — not a cross-layer smell
    /// (default 2).
    pub shotgun_min_packages: usize,
    /// When `true`, drop Shotgun-Surgery candidates whose `name` matches a
    /// built-in list of conventional method names (`new`, `default`,
    /// `fmt`, `clone`, `as_str`, `parse`, `main`, etc.). These almost
    /// always represent Rust trait conformance or universal language
    /// idioms, not real fan-out smells. Set to `false` to opt back in
    /// (default `true`).
    pub shotgun_skip_conventional_names: bool,
    /// CyclicDependency: minimum number of distinct cross-package edges
    /// required *in each direction* before two packages are considered
    /// mutually dependent. Single-edge "cycles" are typically phantom
    /// edges from ambiguous symbol-name resolution (e.g. a common
    /// method name colliding across crates) — real architectural
    /// coupling shows up as multiple touch points (default 2).
    pub cyclic_min_edges_per_direction: usize,
    /// CyclicDependency: when `true`, exclude edges whose source or
    /// target lives in a test / fixture / snapshot path. Sample data is
    /// not part of the workspace's actual dependency graph (default
    /// `true`).
    pub cyclic_skip_test_paths: bool,
}

impl Default for SolidParams {
    fn default() -> Self {
        Self {
            kind: None,
            module: None,
            principles: Vec::new(),
            container_kinds: vec!["impl".into(), "struct".into(), "class".into(), "mod".into()],
            interface_kinds: vec!["trait".into(), "interface".into(), "protocol".into()],
            similarity_threshold: 0.86,
            jaccard_threshold: 0.4,
            srp_cohesion_threshold: 0.7,
            isp_min_methods: 6,
            isp_max_overlap_fraction: 0.33,
            min_lines: 5,
            limit: 50,
            max_pairs: 100_000,
            representative_count: 5,
            shotgun_min_sites: 3,
            shotgun_max_jaccard: 0.5,
            shotgun_min_packages: 2,
            shotgun_skip_conventional_names: true,
            cyclic_min_edges_per_direction: 2,
            cyclic_skip_test_paths: true,
        }
    }
}

/// A minimal symbol summary embedded in [`SolidFinding`] payloads.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SolidSymbolRef {
    /// Symbol ID.
    pub symbol_id: String,
    /// Symbol name.
    pub name: String,
    /// Symbol kind (e.g. `"function"`, `"struct"`).
    pub kind: String,
    /// File path.
    pub file: String,
    /// Line where the symbol is defined (1-based).
    pub line: u32,
}

/// A single SOLID-violation finding.
///
/// Each variant corresponds to a [`SolidPrinciple`]. The `principle` field is
/// duplicated on the wire so clients can branch on a single string without
/// destructuring.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SolidFinding {
    /// Near-duplicate symbols clustered by embedding + callee-set overlap
    /// (DRY/OCP).
    Redundancy {
        /// Always [`SolidPrinciple::DryOcp`].
        principle: SolidPrinciple,
        /// Truncated to `representative_count` — see `members_omitted`.
        members: Vec<SolidSymbolRef>,
        /// How many additional members exist beyond `members`.
        #[serde(skip_serializing_if = "is_zero_usize")]
        members_omitted: usize,
        /// Total members in the cluster (members.len() + members_omitted).
        members_total: usize,
        /// The longest member, suggested as the canonical implementation.
        canonical: SolidSymbolRef,
        /// Average pairwise cosine similarity across the cluster.
        avg_cosine: f32,
        /// Average pairwise Jaccard over callee-sets across the cluster.
        avg_jaccard: f32,
        /// True when members span more than one file (stronger OCP signal).
        cross_module: bool,
    },
    /// A container whose methods split into multiple weakly-connected
    /// cohesion components (SRP).
    LowCohesion {
        /// Always [`SolidPrinciple::Srp`].
        principle: SolidPrinciple,
        /// The container symbol (impl / class / module owner).
        container: SolidSymbolRef,
        /// Per-component summary: members truncated, sizes preserved.
        components: Vec<SolidComponent>,
        /// Total method count in the container (sum across components).
        method_count: usize,
    },
    /// A trait / interface with many methods that most implementors only
    /// partially use (ISP).
    FatInterface {
        /// Always [`SolidPrinciple::Isp`].
        principle: SolidPrinciple,
        /// The interface symbol.
        interface: SolidSymbolRef,
        /// Number of methods on the interface.
        method_count: usize,
        /// Method names no implementor in the corpus appears to override.
        unused_methods: Vec<String>,
        /// How many unused methods exist beyond `unused_methods`.
        #[serde(skip_serializing_if = "is_zero_usize")]
        unused_methods_omitted: usize,
        /// Implementors whose overlap with the interface is ≤ the
        /// configured fraction. Truncated to `representative_count`.
        under_using_implementors: Vec<SolidSymbolRef>,
        /// How many under-using implementors exist beyond the truncated list.
        #[serde(skip_serializing_if = "is_zero_usize")]
        under_using_implementors_omitted: usize,
    },
    /// A high-level consumer depending directly on a concrete symbol in
    /// another package when an abstraction is available (DIP).
    ConcreteDependency {
        /// Always [`SolidPrinciple::Dip`].
        principle: SolidPrinciple,
        /// The consuming symbol.
        consumer: SolidSymbolRef,
        /// The concrete target it depends on.
        concrete_target: SolidSymbolRef,
        /// A trait/interface that could replace the concrete edge, if any.
        suggested_abstraction: Option<SolidSymbolRef>,
    },
    /// Fowler's Shotgun Surgery — the same `(name, kind)` symbol appears
    /// across multiple files with mostly disjoint callee sets, signalling
    /// a parallel-dispatch fan-out where a single abstraction would do.
    /// This is the cousin of Type-4 clones: similar surface, deliberately
    /// disjoint internals.
    ShotgunSurgery {
        /// Always [`SolidPrinciple::ShotgunSurgery`].
        principle: SolidPrinciple,
        /// The shared symbol name (e.g. `dead_code`).
        name: String,
        /// The shared kind (e.g. `function`).
        kind: String,
        /// Representative call-sites (one per file), truncated.
        sites: Vec<SolidSymbolRef>,
        /// Sites omitted from the truncated list.
        #[serde(skip_serializing_if = "is_zero_usize")]
        sites_omitted: usize,
        /// Total file count across the dispatch family.
        sites_total: usize,
        /// Average pairwise callee-set Jaccard. Low values strengthen the
        /// "disjoint internals" interpretation.
        avg_jaccard: f32,
    },
    /// An architectural strongly-connected component in the package-level
    /// import graph. Cycles of size ≥ 2 mean every package in the cycle
    /// depends on every other — typically a sign of a missing layered
    /// abstraction.
    CyclicDependency {
        /// Always [`SolidPrinciple::CyclicDependency`].
        principle: SolidPrinciple,
        /// Packages in the cycle, ordered for stable display.
        packages: Vec<String>,
        /// Total edges inside the SCC (cycle "weight").
        edge_count: usize,
        /// One example cross-package edge per pair, capped at
        /// `representative_count`.
        example_edges: Vec<SolidEdge>,
        /// Example edges omitted beyond `example_edges`.
        #[serde(skip_serializing_if = "is_zero_usize")]
        example_edges_omitted: usize,
    },
}

/// One cohesion component inside an SRP finding.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SolidComponent {
    /// Total methods in this component.
    pub size: usize,
    /// Truncated method list (≤ `representative_count`).
    pub members: Vec<SolidSymbolRef>,
    /// How many members were omitted.
    #[serde(skip_serializing_if = "is_zero_usize")]
    pub members_omitted: usize,
}

/// One package-to-package edge surfaced inside a `CyclicDependency` finding.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct SolidEdge {
    /// Source package (e.g. `ministr-core`).
    pub from: String,
    /// Target package.
    pub to: String,
    /// A representative symbol-pair witnessing the edge.
    pub example_from: SolidSymbolRef,
    /// The target symbol of the example edge.
    pub example_to: SolidSymbolRef,
}

// Serde's `skip_serializing_if` requires `&T`, so the by-ref signature is
// forced — silence the pedantic by-value suggestion.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_usize(n: &usize) -> bool {
    *n == 0
}

/// Errors from the query service layer.
#[derive(Debug, thiserror::Error)]
pub enum QueryError {
    /// A storage operation failed.
    #[error("storage error: {0}")]
    Storage(#[from] StorageError),

    /// An index or embedding operation failed.
    #[error("index error: {0}")]
    Index(#[from] IndexError),

    /// The requested section was not found.
    #[error("section not found: {id}")]
    SectionNotFound { id: String },

    /// The requested claim was not found.
    #[error("claim not found: {id}")]
    ClaimNotFound { id: String },

    /// The requested symbol was not found.
    #[error("symbol not found: {id}")]
    SymbolNotFound { id: String },

    /// A source file could not be read from disk.
    #[error("source file unavailable: {path}: {source}")]
    FileUnavailable {
        /// The stored file path that failed to resolve or read.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

/// High-level query service that composes storage, embedding, and vector index.
///
/// This is the main service interface consumed by transport layers. It provides
/// three operations corresponding to the ministr MCP tools:
/// - [`survey`](Self::survey) — multi-resolution search
/// - [`read_section`](Self::read_section) — full section retrieval
/// - [`extract_claims`](Self::extract_claims) — claim-level extraction
pub struct QueryService {
    storage: SqliteStorage,
    embedder: Arc<dyn Embedder>,
    index: Arc<dyn VectorIndex>,
    sparse_embedder: Option<Arc<dyn SparseEmbedder>>,
    sparse_index: Option<Arc<dyn SparseIndex>>,
    reranker: Option<Arc<dyn Reranker>>,
    /// Optional dual embedder for two-stage Matryoshka reranking at query time.
    dual_embedder: Option<Arc<dyn DualEmbedder>>,
    /// Number of coarse candidates to rescore with full-dim vectors.
    matryoshka_rerank_depth: usize,
}

impl QueryService {
    /// Create a new query service with the given dependencies.
    #[must_use]
    pub fn new(
        storage: SqliteStorage,
        embedder: Arc<dyn Embedder>,
        index: Arc<dyn VectorIndex>,
    ) -> Self {
        Self {
            storage,
            embedder,
            index,
            sparse_embedder: None,
            sparse_index: None,
            reranker: None,
            dual_embedder: None,
            matryoshka_rerank_depth: 100,
        }
    }

    /// Add sparse search components for hybrid retrieval.
    #[must_use]
    pub fn with_sparse(
        mut self,
        sparse_embedder: Arc<dyn SparseEmbedder>,
        sparse_index: Arc<dyn SparseIndex>,
    ) -> Self {
        self.sparse_embedder = Some(sparse_embedder);
        self.sparse_index = Some(sparse_index);
        self
    }

    /// Enable two-stage Matryoshka reranking at query time.
    ///
    /// When set, survey results are rescored using full-dimension cosine
    /// similarity before content resolution and cross-encoder reranking.
    #[must_use]
    pub fn with_matryoshka_rerank(
        mut self,
        dual_embedder: Arc<dyn DualEmbedder>,
        rerank_depth: usize,
    ) -> Self {
        self.dual_embedder = Some(dual_embedder);
        self.matryoshka_rerank_depth = rerank_depth;
        self
    }

    /// Add a cross-encoder reranker for improved relevance scoring.
    ///
    /// When configured, survey results are reranked by the cross-encoder
    /// before truncation to `top_k`. The reranker processes the top
    /// candidates from vector search to produce higher-quality rankings.
    #[must_use]
    pub fn with_reranker(mut self, reranker: Arc<dyn Reranker>) -> Self {
        self.reranker = Some(reranker);
        self
    }

    /// Borrow the configured cross-encoder reranker, if any.
    ///
    /// Exposed so adjacent pipelines (e.g. the daemon's `ask`
    /// verification stage) can re-use the same loaded model as a
    /// premise-vs-hypothesis entailment scorer without paying for a
    /// second model load.
    #[must_use]
    pub fn reranker(&self) -> Option<&Arc<dyn Reranker>> {
        self.reranker.as_ref()
    }

    /// Access the embedder for external use (e.g. topical prefetch).
    #[must_use]
    pub fn embedder(&self) -> &dyn Embedder {
        self.embedder.as_ref()
    }

    /// Access the vector index for external use (e.g. topical prefetch).
    #[must_use]
    pub fn index(&self) -> &dyn VectorIndex {
        self.index.as_ref()
    }

    /// Access the storage layer for external use (e.g. MCP resource listing).
    #[must_use]
    pub fn storage(&self) -> &SqliteStorage {
        &self.storage
    }

    /// Return a table of contents for the corpus.
    ///
    /// Lists all documents and their sections as metadata-only entries.
    /// When `document_id` is provided, returns only sections from that document.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] if a database operation fails.
    #[instrument(skip(self))]
    pub async fn toc(&self, document_id: Option<&str>) -> Result<Vec<TocEntry>, QueryError> {
        let docs = match document_id {
            Some(id) => {
                let cid = ContentId(id.to_string());
                match self.storage.get_document(&cid).await? {
                    Some(doc) => vec![doc],
                    None => vec![],
                }
            }
            None => self.storage.list_documents().await?,
        };

        let mut entries = Vec::new();
        for doc in &docs {
            let sections = self.storage.list_sections(&doc.id).await?;
            for section in sections {
                let claims = self.storage.list_claims(&section.id).await?;
                entries.push(TocEntry {
                    document_id: doc.id.clone(),
                    section_id: section.id,
                    heading_path: section.heading_path,
                    depth: section.depth,
                    claims_available: claims.len(),
                    token_count: count_tokens(&section.text),
                });
            }
        }

        Ok(entries)
    }

    /// List all registered corpus roots with their metadata and language stats.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Storage`] if the database operation fails.
    pub async fn list_corpus_roots(&self) -> Result<Vec<CorpusRoot>, QueryError> {
        Ok(self.storage.list_corpus_roots().await?)
    }
}

/// Check if resolved text is an unresolved placeholder from indexing.
///
/// During indexing, `resolve_content` returns bracket-delimited placeholders
/// like `[claim not found: ...]` or `[symbol not found: ...]` when the
/// underlying content hasn't been indexed yet. These should be filtered
/// out of survey results rather than surfaced to the agent.
fn is_unresolved_placeholder(text: &str) -> bool {
    text.starts_with('[') && (text.contains("not found:") || text.contains("unavailable:"))
}

/// Compute cosine similarity between two vectors.
pub(super) fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::extraction::AbstractiveCompressor;
    use crate::index::HnswIndex;
    use crate::storage::{SqliteStorage, SymbolFilter, SymbolRecord};
    use crate::types::{
        Claim, ClaimId, ClaimRelationship, ContentId, DocumentTree, RelationType, Section,
        SectionId, SymbolId,
    };

    /// Deterministic mock embedder for testing.
    struct MockEmbedder {
        dim: usize,
    }

    impl Embedder for MockEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0f32; self.dim];
                    for (i, b) in t.bytes().enumerate() {
                        v[i % self.dim] += f32::from(b) / 255.0;
                    }
                    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                    if norm > 0.0 {
                        for x in &mut v {
                            *x /= norm;
                        }
                    }
                    v
                })
                .collect())
        }

        fn dimension(&self) -> usize {
            self.dim
        }
    }

    fn make_test_doc() -> DocumentTree {
        let claims = vec![
            Claim {
                id: ClaimId("c1".into()),
                text: "JWT tokens use RS256 signing algorithm.".into(),
                section_id: SectionId("docs/auth.md#tokens".into()),
            },
            Claim {
                id: ClaimId("c2".into()),
                text: "Tokens expire after 24 hours by default.".into(),
                section_id: SectionId("docs/auth.md#tokens".into()),
            },
        ];

        let section = Section {
            id: SectionId("docs/auth.md#tokens".into()),
            heading_path: vec!["Authentication".into(), "Tokens".into()],
            depth: 2,
            text: "JWT tokens use RS256 signing. Tokens expire after 24 hours.".into(),
            structural_nodes: vec![],
            children: vec![],
            claims,
            summary: Some("Token authentication details.".into()),
        };

        DocumentTree {
            id: ContentId("docs/auth.md".into()),
            title: "Authentication Guide".into(),
            source_path: "docs/auth.md".into(),
            sections: vec![section],
            summary: Some("Complete authentication reference.".into()),
        }
    }

    async fn setup_service() -> QueryService {
        let dim = 8;
        let embedder = Arc::new(MockEmbedder { dim });
        let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();

        let doc = make_test_doc();
        storage.insert_document(&doc).await.unwrap();

        // Insert vectors into the index for each content piece
        let texts_and_ids = [
            (
                "doc-summary::docs/auth.md",
                "Complete authentication reference.",
            ),
            (
                "sec-summary::docs/auth.md#tokens",
                "Token authentication details.",
            ),
            (
                "section::docs/auth.md#tokens",
                "JWT tokens use RS256 signing. Tokens expire after 24 hours.",
            ),
            ("claim::c1", "JWT tokens use RS256 signing algorithm."),
            ("claim::c2", "Tokens expire after 24 hours by default."),
        ];

        for (id, text) in &texts_and_ids {
            let vecs = embedder.embed(&[*text]).unwrap();
            index.insert(id, &vecs[0]).unwrap();
        }

        QueryService::new(storage, embedder, index)
    }

    // --- survey tests ---

    #[tokio::test]
    async fn survey_returns_results_for_relevant_query() {
        let service = setup_service().await;
        let results = service
            .survey("JWT authentication tokens", 5)
            .await
            .unwrap();

        assert!(!results.is_empty(), "survey should return results");
        for r in &results {
            assert!(r.score > 0.0);
            assert!(!r.text.is_empty());
            assert!(!r.content_id.is_empty());
        }
    }

    #[tokio::test]
    async fn survey_results_sorted_by_score() {
        let service = setup_service().await;
        let results = service.survey("token signing RS256", 10).await.unwrap();

        for window in results.windows(2) {
            assert!(window[0].score >= window[1].score);
        }
    }

    #[tokio::test]
    async fn survey_respects_top_k() {
        let service = setup_service().await;
        let results = service.survey("tokens", 2).await.unwrap();

        assert!(results.len() <= 2);
    }

    #[tokio::test]
    async fn survey_enriches_section_results_with_heading_path() {
        let service = setup_service().await;
        let results = service.survey("JWT tokens signing", 10).await.unwrap();

        let section_result = results.iter().find(|r| r.resolution == "section");
        if let Some(sr) = section_result {
            assert!(
                sr.heading_path.is_some(),
                "section results should have heading_path"
            );
        }
    }

    // --- read_section tests ---

    #[tokio::test]
    async fn read_section_returns_existing_section() {
        let service = setup_service().await;
        let detail = service.read_section("docs/auth.md#tokens").await.unwrap();

        assert_eq!(detail.section_id, "docs/auth.md#tokens");
        assert_eq!(
            detail.heading_path,
            vec!["Authentication".to_string(), "Tokens".to_string()]
        );
        assert!(detail.text.contains("JWT tokens"));
        assert_eq!(detail.claims_available, 2);
        assert_eq!(
            detail.summary.as_deref(),
            Some("Token authentication details.")
        );
    }

    #[tokio::test]
    async fn read_section_not_found() {
        let service = setup_service().await;
        let result = service.read_section("nonexistent#section").await;

        assert!(matches!(result, Err(QueryError::SectionNotFound { .. })));
    }

    // --- extract_claims tests ---

    #[tokio::test]
    async fn extract_claims_returns_all_claims_without_query() {
        let service = setup_service().await;
        let claims = service
            .extract_claims("docs/auth.md#tokens", None)
            .await
            .unwrap();

        assert_eq!(claims.len(), 2);
        assert!(claims[0].relevance.is_none(), "no relevance without query");
        assert!(claims.iter().any(|c| c.text.contains("RS256")));
        assert!(claims.iter().any(|c| c.text.contains("24 hours")));
    }

    #[tokio::test]
    async fn extract_claims_with_query_returns_scored_results() {
        let service = setup_service().await;
        let claims = service
            .extract_claims("docs/auth.md#tokens", Some("signing algorithm"))
            .await
            .unwrap();

        assert_eq!(claims.len(), 2);
        for c in &claims {
            assert!(c.relevance.is_some(), "should have relevance with query");
        }
        // Results should be sorted by relevance descending
        assert!(claims[0].relevance.unwrap() >= claims[1].relevance.unwrap());
    }

    #[tokio::test]
    async fn extract_claims_section_not_found() {
        let service = setup_service().await;
        let result = service.extract_claims("nonexistent#section", None).await;

        assert!(matches!(result, Err(QueryError::SectionNotFound { .. })));
    }

    #[tokio::test]
    async fn extract_claims_empty_section() {
        let service = setup_service().await;

        // Insert a section with no claims
        let doc = DocumentTree {
            id: ContentId("empty-doc".into()),
            title: "Empty".into(),
            source_path: "empty.md".into(),
            sections: vec![Section {
                id: SectionId("empty.md#intro".into()),
                heading_path: vec!["Intro".into()],
                depth: 1,
                text: "Just some text.".into(),
                structural_nodes: vec![],
                children: vec![],
                claims: vec![],
                summary: None,
            }],
            summary: None,
        };
        service.storage.insert_document(&doc).await.unwrap();

        let claims = service
            .extract_claims("empty.md#intro", None)
            .await
            .unwrap();
        assert!(claims.is_empty());
    }

    // --- compress_content tests ---

    #[tokio::test]
    async fn compress_known_section_returns_summary() {
        let service = setup_service().await;
        let results = service
            .compress_content(&["docs/auth.md#tokens".to_string()])
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_id, "docs/auth.md#tokens");
        assert!(!results[0].summary.is_empty());
        assert!(results[0].original_tokens > 0);
        assert!(results[0].compressed_tokens <= results[0].original_tokens);
    }

    #[tokio::test]
    async fn compress_unknown_section_is_skipped() {
        let service = setup_service().await;
        let results = service
            .compress_content(&["nonexistent#section".to_string()])
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn compress_empty_list_returns_empty() {
        let service = setup_service().await;
        let results = service.compress_content(&[]).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn compress_uses_existing_summary_when_available() {
        let service = setup_service().await;
        let results = service
            .compress_content(&["docs/auth.md#tokens".to_string()])
            .await
            .unwrap();

        // The test section has a pre-generated summary "Token authentication details."
        assert_eq!(results[0].summary, "Token authentication details.");
    }

    #[tokio::test]
    async fn compress_skips_small_sections_without_summary() {
        // A section with only 1-2 sentences and no pre-existing summary
        // cannot be compressed — the extractive summarizer returns identity.
        // compress_content should skip such sections rather than returning
        // a 0%-compression result.
        let dim = 8;
        let embedder = Arc::new(MockEmbedder { dim });
        let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();

        let small_section = Section {
            id: SectionId("docs/tiny.md#intro".into()),
            heading_path: vec!["Tiny".into(), "Intro".into()],
            depth: 2,
            text: "This is a short section.".into(),
            structural_nodes: vec![],
            children: vec![],
            claims: vec![],
            summary: None, // No pre-existing summary
        };

        let doc = DocumentTree {
            id: ContentId("docs/tiny.md".into()),
            title: "Tiny Doc".into(),
            source_path: "docs/tiny.md".into(),
            sections: vec![small_section],
            summary: None,
        };

        storage.insert_document(&doc).await.unwrap();
        let service = QueryService::new(storage, embedder, index);

        let results = service
            .compress_content(&["docs/tiny.md#intro".to_string()])
            .await
            .unwrap();

        // Small section should be skipped — no point returning identity compression
        assert!(
            results.is_empty(),
            "expected small section to be skipped, got {} results with compressed_tokens={:?} vs original_tokens={:?}",
            results.len(),
            results.first().map(|r| r.compressed_tokens),
            results.first().map(|r| r.original_tokens),
        );
    }

    // --- compress_content_abstractive tests ---

    /// A mock abstractive compressor that returns a canned short summary.
    struct MockAbstractiveCompressor {
        response: String,
    }

    impl MockAbstractiveCompressor {
        fn succeeding(response: &str) -> Self {
            Self {
                response: response.to_string(),
            }
        }
    }

    impl AbstractiveCompressor for MockAbstractiveCompressor {
        async fn compress(
            &self,
            _text: &str,
            _context_hint: &str,
        ) -> Result<String, crate::extraction::abstractive::CompressError> {
            Ok(self.response.clone())
        }
    }

    /// A mock compressor that always fails, triggering extractive fallback.
    struct FailingAbstractiveCompressor;

    impl AbstractiveCompressor for FailingAbstractiveCompressor {
        async fn compress(
            &self,
            _text: &str,
            _context_hint: &str,
        ) -> Result<String, crate::extraction::abstractive::CompressError> {
            Err(crate::extraction::abstractive::CompressError::Unavailable(
                "test: no peer".into(),
            ))
        }
    }

    #[tokio::test]
    async fn abstractive_compress_uses_compressor_when_available() {
        let service = setup_service().await;
        let compressor = MockAbstractiveCompressor::succeeding("Dense summary.");
        let results = service
            .compress_content_abstractive(&["docs/auth.md#tokens".to_string()], &compressor)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].summary, "Dense summary.");
        assert_eq!(results[0].method, "abstractive");
        assert!(results[0].compressed_tokens < results[0].original_tokens);
    }

    #[tokio::test]
    async fn abstractive_compress_falls_back_on_failure() {
        let service = setup_service().await;
        let compressor = FailingAbstractiveCompressor;
        let results = service
            .compress_content_abstractive(&["docs/auth.md#tokens".to_string()], &compressor)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].method, "extractive");
        // Should use the pre-existing summary as extractive fallback
        assert_eq!(results[0].summary, "Token authentication details.");
    }

    #[tokio::test]
    async fn abstractive_compress_falls_back_on_empty_response() {
        let service = setup_service().await;
        let compressor = MockAbstractiveCompressor::succeeding("  "); // whitespace-only
        let results = service
            .compress_content_abstractive(&["docs/auth.md#tokens".to_string()], &compressor)
            .await
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].method, "extractive");
    }

    #[tokio::test]
    async fn abstractive_compress_skips_unknown_sections() {
        let service = setup_service().await;
        let compressor = MockAbstractiveCompressor::succeeding("Dense.");
        let results = service
            .compress_content_abstractive(&["nonexistent#section".to_string()], &compressor)
            .await
            .unwrap();

        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn abstractive_compress_reports_method_field() {
        let service = setup_service().await;
        // Extractive path
        let results = service
            .compress_content(&["docs/auth.md#tokens".to_string()])
            .await
            .unwrap();
        assert_eq!(results[0].method, "extractive");
    }

    // --- symbol compress/extract tests ---

    async fn setup_service_with_symbol() -> QueryService {
        let dim = 8;
        let embedder = Arc::new(MockEmbedder { dim });
        let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();

        // Create a test source file
        let test_dir = tempfile::tempdir().unwrap();
        let src_file = test_dir.path().join("config.rs");
        std::fs::write(
            &src_file,
            "/// The main config struct.\n/// It provides 3 configurable fields for runtime tuning.\npub struct Config {\n    pub max_items: usize,\n}\n",
        )
        .unwrap();

        // Store the corpus root and symbol
        let symbol = SymbolRecord {
            id: SymbolId("sym-config.rs::Config".into()),
            file_path: src_file.to_str().unwrap().to_string(),
            name: "Config".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            signature: "pub struct Config".into(),
            doc_comment: Some(
                "The main config struct. It provides 3 configurable fields for runtime tuning."
                    .into(),
            ),
            module_path: String::new(),
            line_start: 3,
            line_end: 5,
            cyclomatic_complexity: None,
        };
        storage.insert_symbols(&[symbol]).await.unwrap();

        QueryService::new(storage, embedder, index)
    }

    #[tokio::test]
    async fn compress_symbol_returns_stub_summary() {
        let service = setup_service_with_symbol().await;
        let results = service
            .compress_content(&["sym-config.rs::Config".to_string()])
            .await
            .unwrap();

        assert_eq!(results.len(), 1, "should return 1 compressed symbol");
        assert_eq!(results[0].original_id, "sym-config.rs::Config");
        assert_eq!(results[0].method, "symbol_stub");
        assert!(
            results[0].summary.contains("pub struct Config"),
            "summary should contain signature: {:?}",
            results[0].summary
        );
    }

    #[tokio::test]
    async fn extract_claims_from_symbol_doc_comment() {
        let service = setup_service_with_symbol().await;
        let claims = service
            .extract_claims("sym-config.rs::Config", None)
            .await
            .unwrap();

        // The doc comment has assertive sentences → should produce claims
        assert!(!claims.is_empty(), "should extract claims from doc comment");
    }

    #[tokio::test]
    async fn extract_claims_symbol_without_doc_returns_empty() {
        let dim = 8;
        let embedder = Arc::new(MockEmbedder { dim });
        let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();

        let symbol = SymbolRecord {
            id: SymbolId("sym-bare.rs::Bare".into()),
            file_path: "bare.rs".into(),
            name: "Bare".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            signature: "pub struct Bare".into(),
            doc_comment: None,
            module_path: String::new(),
            line_start: 1,
            line_end: 1,
            cyclomatic_complexity: None,
        };
        storage.insert_symbols(&[symbol]).await.unwrap();
        let service = QueryService::new(storage, embedder, index);

        let claims = service
            .extract_claims("sym-bare.rs::Bare", None)
            .await
            .unwrap();
        assert!(claims.is_empty(), "no doc comment → no claims");
    }

    // --- name_exact filter tests ---

    #[tokio::test]
    async fn list_symbols_name_exact_matches_only_exact() {
        let storage = SqliteStorage::open_in_memory().unwrap();

        let symbols = vec![
            SymbolRecord {
                id: SymbolId("sym-a.rs::Default".into()),
                file_path: "a.rs".into(),
                name: "Default".into(),
                kind: "trait".into(),
                visibility: "pub".into(),
                signature: "pub trait Default".into(),
                doc_comment: None,
                module_path: String::new(),
                line_start: 1,
                line_end: 1,
                cyclomatic_complexity: None,
            },
            SymbolRecord {
                id: SymbolId("sym-b.rs::DEFAULT_TOP_LIMIT".into()),
                file_path: "b.rs".into(),
                name: "DEFAULT_TOP_LIMIT".into(),
                kind: "const".into(),
                visibility: "pub".into(),
                signature: "pub const DEFAULT_TOP_LIMIT: usize = 10".into(),
                doc_comment: None,
                module_path: String::new(),
                line_start: 1,
                line_end: 1,
                cyclomatic_complexity: None,
            },
        ];
        storage.insert_symbols(&symbols).await.unwrap();

        // Fuzzy name search matches both
        let fuzzy = storage
            .list_symbols(&SymbolFilter {
                name: Some("Default".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(fuzzy.len(), 2, "fuzzy should match both symbols");

        // Exact name search matches only "Default"
        let exact = storage
            .list_symbols(&SymbolFilter {
                name_exact: Some("Default".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(exact.len(), 1, "exact should match only one");
        assert_eq!(exact[0].name, "Default");
    }

    // --- cosine_similarity tests ---

    #[test]
    fn cosine_similarity_identical_vectors() {
        let v = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-5);
    }

    #[test]
    fn cosine_similarity_zero_vector() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![0.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < f32::EPSILON);
    }

    // --- related_claims tests ---

    #[tokio::test]
    async fn related_claims_returns_related() {
        let service = setup_service().await;

        // Insert relationships
        let relationships = vec![ClaimRelationship {
            source_claim_id: ClaimId("c1".into()),
            target_claim_id: ClaimId("c2".into()),
            relation_type: RelationType::References,
            confidence: 0.8,
        }];
        service
            .storage
            .insert_claim_relationships(&relationships)
            .await
            .unwrap();

        let related = service.related_claims("c1", None).await.unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].claim_id, "c2");
        assert_eq!(related[0].relation_type, "references");
        assert_eq!(related[0].source_section, "docs/auth.md#tokens");
    }

    #[tokio::test]
    async fn related_claims_filters_by_type() {
        let service = setup_service().await;

        let relationships = vec![
            ClaimRelationship {
                source_claim_id: ClaimId("c1".into()),
                target_claim_id: ClaimId("c2".into()),
                relation_type: RelationType::References,
                confidence: 0.8,
            },
            ClaimRelationship {
                source_claim_id: ClaimId("c1".into()),
                target_claim_id: ClaimId("c2".into()),
                relation_type: RelationType::Updates,
                confidence: 0.6,
            },
        ];
        service
            .storage
            .insert_claim_relationships(&relationships)
            .await
            .unwrap();

        let related = service
            .related_claims("c1", Some(&[RelationType::Updates]))
            .await
            .unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].relation_type, "updates");
    }

    #[tokio::test]
    async fn related_claims_not_found() {
        let service = setup_service().await;
        let result = service.related_claims("nonexistent", None).await;
        assert!(matches!(result, Err(QueryError::ClaimNotFound { .. })));
    }

    // --- toc tests ---

    /// Build a multi-doc corpus with nested headings for toc testing.
    async fn setup_multi_doc_service() -> QueryService {
        let dim = 8;
        let embedder = Arc::new(MockEmbedder { dim });
        let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();

        let docs = vec![
            DocumentTree {
                id: ContentId("docs/auth.md".into()),
                title: "Authentication Guide".into(),
                source_path: "docs/auth.md".into(),
                sections: vec![
                    Section {
                        id: SectionId("docs/auth.md#tokens".into()),
                        heading_path: vec!["Authentication".into(), "Tokens".into()],
                        depth: 2,
                        text: "JWT tokens use RS256 signing.".into(),
                        structural_nodes: vec![],
                        children: vec![],
                        claims: vec![
                            Claim {
                                id: ClaimId("auth-c1".into()),
                                text: "JWT tokens use RS256.".into(),
                                section_id: SectionId("docs/auth.md#tokens".into()),
                            },
                            Claim {
                                id: ClaimId("auth-c2".into()),
                                text: "Tokens expire after 24h.".into(),
                                section_id: SectionId("docs/auth.md#tokens".into()),
                            },
                        ],
                        summary: None,
                    },
                    Section {
                        id: SectionId("docs/auth.md#oauth".into()),
                        heading_path: vec!["Authentication".into(), "OAuth".into()],
                        depth: 2,
                        text: "OAuth 2.0 with PKCE.".into(),
                        structural_nodes: vec![],
                        children: vec![],
                        claims: vec![Claim {
                            id: ClaimId("auth-c3".into()),
                            text: "OAuth 2.0 is supported.".into(),
                            section_id: SectionId("docs/auth.md#oauth".into()),
                        }],
                        summary: None,
                    },
                ],
                summary: Some("Auth reference.".into()),
            },
            DocumentTree {
                id: ContentId("docs/api.md".into()),
                title: "API Reference".into(),
                source_path: "docs/api.md".into(),
                sections: vec![Section {
                    id: SectionId("docs/api.md#rate-limits".into()),
                    heading_path: vec!["API Reference".into(), "Rate Limits".into()],
                    depth: 2,
                    text: "100 requests per minute.".into(),
                    structural_nodes: vec![],
                    children: vec![],
                    claims: vec![Claim {
                        id: ClaimId("api-c1".into()),
                        text: "Rate limit is 100/min.".into(),
                        section_id: SectionId("docs/api.md#rate-limits".into()),
                    }],
                    summary: None,
                }],
                summary: Some("API docs.".into()),
            },
        ];

        for doc in &docs {
            storage.insert_document(doc).await.unwrap();
        }

        QueryService::new(storage, embedder, index)
    }

    #[tokio::test]
    async fn toc_returns_correct_tree_for_multi_doc_corpus() {
        let service = setup_multi_doc_service().await;
        let entries = service.toc(None).await.unwrap();

        // Should have 3 sections total across 2 documents
        assert_eq!(entries.len(), 3, "expected 3 sections total");

        // Verify auth doc sections
        let auth_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.document_id.as_ref() == "docs/auth.md")
            .collect();
        assert_eq!(auth_entries.len(), 2, "auth doc should have 2 sections");

        let tokens_entry = auth_entries
            .iter()
            .find(|e| e.section_id.as_ref() == "docs/auth.md#tokens")
            .expect("should find tokens section");
        assert_eq!(tokens_entry.heading_path, vec!["Authentication", "Tokens"]);
        assert_eq!(tokens_entry.depth, 2);
        assert_eq!(tokens_entry.claims_available, 2);
        assert!(tokens_entry.token_count > 0);

        // Verify api doc section
        let api_entries: Vec<_> = entries
            .iter()
            .filter(|e| e.document_id.as_ref() == "docs/api.md")
            .collect();
        assert_eq!(api_entries.len(), 1, "api doc should have 1 section");
        assert_eq!(api_entries[0].claims_available, 1);
    }

    #[tokio::test]
    async fn toc_filters_by_document_id() {
        let service = setup_multi_doc_service().await;
        let entries = service.toc(Some("docs/api.md")).await.unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].document_id.as_ref(), "docs/api.md");
        assert_eq!(entries[0].section_id.as_ref(), "docs/api.md#rate-limits");
    }

    #[tokio::test]
    async fn toc_returns_empty_for_unknown_document() {
        let service = setup_multi_doc_service().await;
        let entries = service.toc(Some("nonexistent.md")).await.unwrap();

        assert!(entries.is_empty());
    }

    // --- reranker tests ---

    /// Mock reranker that scores by document text length (longer = higher).
    struct LengthReranker;

    impl Reranker for LengthReranker {
        #[allow(clippy::cast_precision_loss)]
        fn rerank(
            &self,
            _query: &str,
            documents: &[&str],
        ) -> Result<Vec<crate::embedding::RerankScore>, IndexError> {
            let mut scores: Vec<crate::embedding::RerankScore> = documents
                .iter()
                .enumerate()
                .map(|(i, doc)| crate::embedding::RerankScore {
                    index: i,
                    score: doc.len() as f32,
                })
                .collect();
            scores.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            Ok(scores)
        }
    }

    async fn setup_service_with_reranker(reranker: Arc<dyn Reranker>) -> QueryService {
        let dim = 8;
        let embedder = Arc::new(MockEmbedder { dim });
        let index = Arc::new(HnswIndex::new(dim, 1000).unwrap());
        let storage = SqliteStorage::open_in_memory().unwrap();

        let doc = make_test_doc();
        storage.insert_document(&doc).await.unwrap();

        let texts_and_ids = [
            (
                "doc-summary::docs/auth.md",
                "Complete authentication reference.",
            ),
            (
                "sec-summary::docs/auth.md#tokens",
                "Token authentication details.",
            ),
            (
                "section::docs/auth.md#tokens",
                "JWT tokens use RS256 signing. Tokens expire after 24 hours.",
            ),
            ("claim::c1", "JWT tokens use RS256 signing algorithm."),
            ("claim::c2", "Tokens expire after 24 hours by default."),
        ];

        for (id, text) in &texts_and_ids {
            let vecs = embedder.embed(&[*text]).unwrap();
            index.insert(id, &vecs[0]).unwrap();
        }

        QueryService::new(storage, embedder, index).with_reranker(reranker)
    }

    #[tokio::test]
    async fn survey_with_reranker_uses_reranked_scores() {
        let service = setup_service_with_reranker(Arc::new(LengthReranker)).await;
        let results = service.survey("JWT tokens", 5).await.unwrap();

        assert!(!results.is_empty());
        // LengthReranker scores by text length, so results should be sorted
        // by text length descending
        for window in results.windows(2) {
            assert!(
                window[0].score >= window[1].score,
                "results should be sorted by reranked score: {} >= {}",
                window[0].score,
                window[1].score,
            );
        }
    }

    #[tokio::test]
    async fn survey_with_reranker_respects_top_k() {
        let service = setup_service_with_reranker(Arc::new(LengthReranker)).await;
        let results = service.survey("JWT tokens", 2).await.unwrap();

        assert!(
            results.len() <= 2,
            "reranked results should respect top_k=2, got {}",
            results.len()
        );
    }

    #[tokio::test]
    async fn survey_excluding_with_reranker() {
        let service = setup_service_with_reranker(Arc::new(LengthReranker)).await;
        // Exclude using the content_id format (after vector_id.content_id() extraction)
        let exclude: HashSet<String> = ["docs/auth.md".to_string()].into_iter().collect();

        let (results, dedup_count) = service
            .survey_excluding("JWT tokens", 5, &exclude)
            .await
            .unwrap();

        // The excluded content_id should have been filtered out
        for r in &results {
            assert_ne!(r.content_id, "docs/auth.md");
        }
        assert!(dedup_count > 0 || results.is_empty());
    }

    #[tokio::test]
    async fn survey_without_reranker_unchanged() {
        // Verify that without a reranker, survey works as before
        let service = setup_service().await;
        let results = service.survey("JWT authentication", 5).await.unwrap();

        assert!(!results.is_empty());
        for window in results.windows(2) {
            assert!(window[0].score >= window[1].score);
        }
    }

    #[test]
    fn unresolved_placeholder_detection() {
        assert!(is_unresolved_placeholder("[claim not found: foo:c0]"));
        assert!(is_unresolved_placeholder("[symbol not found: sym-bar]"));
        assert!(is_unresolved_placeholder("[section not found: baz#qux]"));
        assert!(is_unresolved_placeholder(
            "[content unavailable: something]"
        ));
        // Normal content should not match
        assert!(!is_unresolved_placeholder("JWT tokens use RS256 signing."));
        assert!(!is_unresolved_placeholder(""));
        // Bracketed text that doesn't contain the marker
        assert!(!is_unresolved_placeholder("[some other bracket text]"));
    }
}
