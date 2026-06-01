//! Shared multi-language e2e graph-test harness (FE1).
//!
//! This is the foundation the FE2–FE5 matrices build on. It ingests a
//! multi-file fixture project end-to-end through the real
//! [`IngestionPipeline`] into in-memory storage, then exposes the resulting
//! **symbol + reference graph** for cheap, declarative assertions.
//!
//! # Design (single-responsibility split)
//!
//! - [`MockEmbedder`] — deterministic, hash-based embeddings so ingestion is
//!   reproducible and offline. It is the *only* embedding concern here.
//! - [`IngestedProject`] — owns the temp dir + the in-memory storage and is
//!   the sole *graph access* surface (symbols, refs, id→file lookup). It knows
//!   nothing about assertions.
//! - The `assert_*` free functions are the *assertion* layer. They consume an
//!   `&IngestedProject` and never reach into ingestion internals, so the graph
//!   surface and the assertion vocabulary evolve independently.
//!
//! # Fixture convention (open for extension)
//!
//! Adding a language/case never edits this harness — you either:
//!
//! 1. pass an in-memory file map to [`IngestedProject::from_files`] (best for
//!    self-contained, edge-case-focused cases), or
//! 2. drop a directory under `tests/fixtures/langgraph/<lang>/<case>/` and load
//!    it with [`IngestedProject::from_fixture_dir`] (best for larger, realistic
//!    projects shared across tests).
//!
//! One directory per language/case keeps fixtures discoverable and lets the
//! FE6 coverage guard cross-check the fixture tree against the supported
//! language set.

// Different FE test crates (fe_cpp, fe2_extraction, fe3_refs, …) each compile
// this module and exercise a different subset of the surface, so unused-symbol
// warnings here are expected and benign — standard for a shared tests/ helper.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_core::index::HnswIndex;
use ministr_core::ingestion::IngestionPipeline;
use ministr_core::storage::{SqliteStorage, Storage, SymbolFilter, SymbolRecord};
use ministr_core::types::{RefKind, SymbolId};

/// Deterministic, hash-based mock embedder.
///
/// Produces L2-normalised vectors purely from byte content, so ingestion is
/// reproducible and needs no model. Embedding quality is irrelevant to the
/// graph suite — we only assert on the symbol/reference graph, never on
/// retrieval ranking.
pub struct MockEmbedder {
    dim: usize,
}

impl MockEmbedder {
    /// A mock embedder of the given dimensionality.
    #[must_use]
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl Default for MockEmbedder {
    fn default() -> Self {
        Self { dim: 8 }
    }
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

/// An ingested multi-file fixture project with its symbol + reference graph
/// available for assertions.
///
/// Holds the [`tempfile::TempDir`] alive for the project's lifetime (so file
/// paths stay valid) alongside the in-memory storage the pipeline populated.
pub struct IngestedProject {
    // Kept alive so the on-disk fixture outlives every query; `None` when the
    // project was loaded from a committed fixture directory (nothing temp to
    // clean up).
    _tmp: Option<tempfile::TempDir>,
    storage: SqliteStorage,
    root: PathBuf,
}

impl IngestedProject {
    /// Ingest an in-memory file map — each entry is `(relative path, contents)`.
    ///
    /// Relative paths may contain subdirectories (e.g. `"include/shape.h"`);
    /// intermediate directories are created automatically.
    pub async fn from_files(files: &[(&str, &str)]) -> Self {
        let tmp = tempfile::tempdir().expect("create tempdir");
        let root = tmp.path().join("project");
        std::fs::create_dir_all(&root).expect("create project root");

        for (rel, contents) in files {
            let path = root.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create fixture parent dir");
            }
            std::fs::write(&path, contents).expect("write fixture file");
        }

        let storage = ingest_into_storage(&root).await;
        Self {
            _tmp: Some(tmp),
            storage,
            root,
        }
    }

    /// Ingest a committed fixture directory, given a path relative to
    /// `tests/fixtures/` (e.g. `"langgraph/cpp/basic"`).
    pub async fn from_fixture_dir(rel: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(rel);
        assert!(
            root.is_dir(),
            "fixture directory does not exist: {}",
            root.display()
        );
        let storage = ingest_into_storage(&root).await;
        Self {
            _tmp: None,
            storage,
            root,
        }
    }

    /// The project root directory on disk.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Raw storage handle, for the rare assertion that needs the full trait.
    #[must_use]
    pub fn storage(&self) -> &SqliteStorage {
        &self.storage
    }

    /// All symbols matching `filter`.
    pub async fn symbols(&self, filter: SymbolFilter) -> Vec<SymbolRecord> {
        self.storage
            .list_symbols(&filter)
            .await
            .expect("list_symbols")
    }

    /// Every symbol in the project.
    pub async fn all_symbols(&self) -> Vec<SymbolRecord> {
        self.symbols(SymbolFilter::default()).await
    }

    /// All symbols with exactly this name.
    pub async fn symbols_named(&self, name: &str) -> Vec<SymbolRecord> {
        self.symbols(SymbolFilter {
            name_exact: Some(name.to_string()),
            ..SymbolFilter::default()
        })
        .await
    }

    /// Exactly one symbol with this name — panics if zero or many match.
    /// Convenience for unambiguous fixtures.
    pub async fn symbol(&self, name: &str) -> SymbolRecord {
        let mut matches = self.symbols_named(name).await;
        assert_eq!(
            matches.len(),
            1,
            "expected exactly one symbol named `{name}`, got {}: {:?}",
            matches.len(),
            matches
                .iter()
                .map(|s| format!("{} ({}) @ {}", s.name, s.kind, s.file_path))
                .collect::<Vec<_>>(),
        );
        matches.pop().unwrap()
    }

    /// All references pointing **to** any symbol named `name`, paired with the
    /// file path of the referencing (`from`) symbol. This is the cross-file
    /// edge introspection primitive the ref assertions are built on.
    pub async fn refs_into(&self, name: &str, kind: Option<RefKind>) -> Vec<RefEdge> {
        let id_to_file = self.id_to_file().await;
        let mut edges = Vec::new();
        for target in self.symbols_named(name).await {
            let refs = self
                .storage
                .query_refs(&target.id, kind)
                .await
                .expect("query_refs");
            for r in refs {
                let from_file = id_to_file
                    .get(&r.from_symbol_id)
                    .cloned()
                    .unwrap_or_default();
                edges.push(RefEdge {
                    from_symbol_id: r.from_symbol_id,
                    from_file,
                    to_symbol_id: r.to_symbol_id,
                    to_name: target.name.clone(),
                    to_file: target.file_path.clone(),
                    kind: r.ref_kind,
                });
            }
        }
        edges
    }

    /// Build an id → file-path map over every symbol in the project.
    async fn id_to_file(&self) -> HashMap<SymbolId, String> {
        self.all_symbols()
            .await
            .into_iter()
            .map(|s| (s.id, s.file_path))
            .collect()
    }
}

/// A resolved cross-reference edge, flattened for assertions: who references
/// whom, in which files, and how.
#[derive(Debug, Clone)]
pub struct RefEdge {
    pub from_symbol_id: SymbolId,
    pub from_file: String,
    pub to_symbol_id: SymbolId,
    pub to_name: String,
    pub to_file: String,
    pub kind: RefKind,
}

async fn ingest_into_storage(dir: &Path) -> SqliteStorage {
    let storage = SqliteStorage::open_in_memory().expect("open in-memory storage");
    let embedder = MockEmbedder::default();
    let index = HnswIndex::new(embedder.dimension(), 10_000).expect("hnsw index");
    let pipeline = IngestionPipeline::new();

    let stats = pipeline
        .ingest_directory_with_embeddings(dir, &storage, &embedder, &index)
        .await
        .expect("ingest fixture directory");

    assert!(
        stats.files_indexed > 0,
        "harness ingested no files from {}",
        dir.display(),
    );

    storage
}

// ---------------------------------------------------------------------------
// Assertion layer — decoupled from ingestion + graph access.
// ---------------------------------------------------------------------------

/// Assert a symbol with `name` + `kind` exists, defined in a file whose path
/// ends with `file_suffix`. Returns it for further assertions.
pub async fn assert_symbol(
    proj: &IngestedProject,
    name: &str,
    kind: &str,
    file_suffix: &str,
) -> SymbolRecord {
    let candidates = proj.symbols_named(name).await;
    assert!(
        !candidates.is_empty(),
        "expected a symbol named `{name}` ({kind}); none extracted.\nAll symbols: {}",
        render_symbols(&proj.all_symbols().await),
    );
    let found = candidates
        .iter()
        .find(|s| s.kind == kind && s.file_path.ends_with(file_suffix));
    assert!(
        found.is_some(),
        "expected `{name}` as kind `{kind}` in a file ending `{file_suffix}`.\n\
         Candidates named `{name}`: {}",
        render_symbols(&candidates),
    );
    found.cloned().unwrap()
}

/// Assert a multi-line symbol's line range is well-formed: `line_end >
/// line_start` (the range invariant whose violation was a candidate root cause
/// for dropped refs). Single-line symbols are allowed `line_end == line_start`.
pub fn assert_range_invariant(sym: &SymbolRecord) {
    assert!(
        sym.line_end >= sym.line_start && sym.line_start >= 1,
        "symbol `{}` has a malformed line range: start={}, end={}",
        sym.name,
        sym.line_start,
        sym.line_end,
    );
}

/// Assert that at least one symbol defined in `def_file_suffix` and named
/// `def_name` is referenced from a symbol in `importer_file_suffix` — i.e. the
/// cross-file edge resolved. `kind = None` accepts any reference kind.
pub async fn assert_cross_file_ref(
    proj: &IngestedProject,
    def_name: &str,
    def_file_suffix: &str,
    importer_file_suffix: &str,
    kind: Option<RefKind>,
) {
    let edges = proj.refs_into(def_name, kind).await;
    let hit = edges.iter().any(|e| {
        e.to_file.ends_with(def_file_suffix) && e.from_file.ends_with(importer_file_suffix)
    });
    assert!(
        hit,
        "expected a cross-file reference to `{def_name}` (defined in `…{def_file_suffix}`) \
         from `…{importer_file_suffix}`{}, but none resolved.\nResolved edges into `{def_name}`: {}",
        kind.map_or(String::new(), |k| format!(" of kind {k:?}")),
        render_edges(&edges),
    );
}

fn render_symbols(symbols: &[SymbolRecord]) -> String {
    use std::fmt::Write;
    if symbols.is_empty() {
        return "<none>".to_string();
    }
    symbols.iter().fold(String::new(), |mut acc, s| {
        let _ = write!(
            acc,
            "\n  - {} ({}) {}..{} @ {}",
            s.name, s.kind, s.line_start, s.line_end, s.file_path
        );
        acc
    })
}

fn render_edges(edges: &[RefEdge]) -> String {
    use std::fmt::Write;
    if edges.is_empty() {
        return "<none>".to_string();
    }
    edges.iter().fold(String::new(), |mut acc, e| {
        let _ = write!(
            acc,
            "\n  - {} ({:?}) from {} → {}",
            e.to_name, e.kind, e.from_file, e.to_file
        );
        acc
    })
}
