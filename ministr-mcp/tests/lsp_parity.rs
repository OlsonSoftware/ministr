//! FL4 — the always-on LSP-equivalence parity gate.
//!
//! Proves that ministr's EXISTING MCP op contract (`QueryBackend`, exercised
//! here via the in-process `Backend::local`) collectively covers what an agent
//! needs from a per-language LSP: go-to-definition, find-references,
//! implementation, type-hierarchy refs, call hierarchy (incoming/outgoing),
//! document/workspace symbols, position→symbol resolution, and the verify-stage
//! ops (diagnostics). Each navigation/hierarchy/verify row of
//! `eval/lsp-nav/PARITY.md` is asserted wired and non-degenerate over a small
//! committed fixture corpus.
//!
//! Unlike `eval/lsp-nav`'s heavy accuracy-vs-rust-analyzer benchmark (report
//! only, `#[ignore]`), this is a fast regression gate that runs in the default
//! `cargo test` pass — "block, don't monitor".

use std::path::PathBuf;
use std::sync::Arc;

use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_core::index::HnswIndex;
use ministr_core::ingestion::IngestionPipeline;
use ministr_core::service::{CallDirection, QueryService};
use ministr_core::storage::{SqliteStorage, SymbolFilter};
use ministr_core::types::RefKind;
use ministr_mcp::backend::Backend;

/// Deterministic mock embedder — navigation is symbol/AST-driven, so embedding
/// quality is irrelevant and this avoids a model download.
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

/// A `name_exact` symbol filter.
fn by_name(name: &str) -> SymbolFilter {
    SymbolFilter {
        name_exact: Some(name.to_string()),
        ..SymbolFilter::default()
    }
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn lsp_parity_gate() {
    // --- build a small fixture corpus in-process (no model, no daemon) ---
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/lsp_parity");
    let storage = SqliteStorage::open_in_memory().expect("open in-memory storage");
    let dim = 8;
    let embedder = MockEmbedder { dim };
    let index = HnswIndex::new(dim, 4096).expect("create index");
    let paths = vec![fixture];

    IngestionPipeline::new()
        .ingest_paths_with_embeddings(&paths, &storage, &embedder, &index)
        .await
        .expect("ingest fixture corpus");

    let query = QueryService::new(
        storage,
        Arc::new(MockEmbedder { dim }),
        Arc::new(HnswIndex::new(dim, 1).expect("placeholder index")),
    );
    let backend = Backend::local(Arc::new(query));

    // ── workspace/symbol — find a symbol anywhere by name ───────────────────
    let helpers = backend
        .search_symbols(None, None, by_name("helper"))
        .await
        .expect("search_symbols");
    assert_eq!(helpers.len(), 1, "workspace/symbol: exactly one `helper`");
    let helper_id = helpers[0].id.0.clone();

    // ── textDocument/definition (hover: signature/doc ride along) ───────────
    let def = backend
        .definition(None, None, &helper_id)
        .await
        .expect("definition");
    assert!(
        def.file_path.ends_with("shapes.rs"),
        "definition resolves to the fixture file: {}",
        def.file_path
    );
    let shapes_path = def.file_path.clone();
    let helper_line = def.line_start;

    // ── textDocument/documentSymbol — symbols in one file ───────────────────
    let doc_syms = backend
        .search_symbols(
            None,
            None,
            SymbolFilter {
                file_path: Some(shapes_path.clone()),
                ..SymbolFilter::default()
            },
        )
        .await
        .expect("documentSymbol");
    assert!(
        doc_syms.len() >= 4,
        "documentSymbol: the fixture file has many symbols, got {}",
        doc_syms.len()
    );

    // ── textDocument/references — every use of a symbol ─────────────────────
    let refs = backend
        .references(None, None, &helper_id, None, false)
        .await
        .expect("references");
    assert!(
        refs.len() >= 2,
        "find-references: `helper` is called by at least two functions, got {}",
        refs.len()
    );
    assert!(
        refs.iter().any(|r| r.from_name == "caller_one"),
        "references include the caller_one call site"
    );

    // ── textDocument/implementation — implementors of a trait ───────────────
    let shape = backend
        .search_symbols(None, None, by_name("Shape"))
        .await
        .expect("search Shape");
    assert!(!shape.is_empty(), "the Shape trait is indexed");
    let impls = backend
        .references(None, None, &shape[0].id.0, Some(RefKind::Implements), false)
        .await
        .expect("implementation refs");
    assert!(
        impls.len() >= 2,
        "implementation: Shape has two implementors (Circle, Square), got {}",
        impls.len()
    );

    // ── typeHierarchy / interface-method refs through implementors (FL3b) ───
    let area = backend
        .search_symbols(None, None, by_name("area"))
        .await
        .expect("search area");
    assert!(!area.is_empty(), "the area method is indexed");
    let area_id = area[0].id.0.clone();
    let through = backend
        .references(None, None, &area_id, None, true)
        .await
        .expect("refs through implementors")
        .len();
    let direct = backend
        .references(None, None, &area_id, None, false)
        .await
        .expect("direct refs")
        .len();
    assert!(
        through >= direct,
        "through_implementors is additive (peer co-implementor hop): {through} >= {direct}"
    );

    // ── callHierarchy/incomingCalls — who calls this (FL3) ──────────────────
    let incoming = backend
        .impact(None, None, &helper_id, 5, CallDirection::Incoming, false)
        .await
        .expect("incoming calls");
    assert!(
        incoming.symbols >= 2,
        "incoming: at least two callers reach `helper`, got {}",
        incoming.symbols
    );

    // ── callHierarchy/outgoingCalls — what this calls (FL3) ─────────────────
    let caller_one = backend
        .search_symbols(None, None, by_name("caller_one"))
        .await
        .expect("search caller_one");
    assert!(!caller_one.is_empty(), "caller_one is indexed");
    let outgoing = backend
        .impact(
            None,
            None,
            &caller_one[0].id.0,
            5,
            CallDirection::Outgoing,
            false,
        )
        .await
        .expect("outgoing calls");
    assert!(
        outgoing.symbols >= 1,
        "outgoing: caller_one reaches at least `helper`, got {}",
        outgoing.symbols
    );

    // ── test↔code mapping — tests_only is a non-increasing filter (FL6) ─────
    let tests_only = backend
        .impact(None, None, &helper_id, 5, CallDirection::Incoming, true)
        .await
        .expect("tests-only impact");
    assert!(
        tests_only.symbols <= incoming.symbols,
        "tests_only filters the incoming set: {} <= {}",
        tests_only.symbols,
        incoming.symbols
    );

    // ── textDocument/publishDiagnostics — the verify stage is wired (FL5) ───
    // A bogus language filter guarantees no real toolchain executes; the op
    // must still be callable and return a (bounded, here empty) list.
    let no_lang = ["zzz_no_toolchain".to_string()];
    let diags = backend
        .diagnostics(None, None, Some(&no_lang), 1)
        .await
        .expect("diagnostics op is wired");
    assert!(
        diags.is_empty(),
        "no toolchain matches the bogus language filter"
    );

    // ── ministr-only differentiator — cross-language bridges are wired ──────
    let bridges = backend
        .bridges(None, None, None, None, None, None)
        .await
        .expect("bridges op is wired");
    assert!(
        bridges.is_empty(),
        "the single-file Rust fixture has no cross-language bridges"
    );

    // ── textDocument/{definition,references} BY POSITION (FL2) ──────────────
    // The FL2 resolver runs on the occurrence index, an opt-in ingest pass
    // (`MINISTR_INDEX_OCCURRENCES`, off by default and not enabled here, since
    // this crate forbids the `unsafe` `env::set_var`), so it reports `None` for
    // this fixture. This asserts the op is wired into the contract and returns
    // the right shape; when the index *is* present it must resolve `helper`
    // (the `pub fn helper` identifier sits at byte cols 7..13, so col 9 lands
    // inside it). Occurrence extraction itself is covered by ministr-core's
    // `code::occurrence` / `fe5_invariants` tests.
    let resolved = backend
        .symbol_at_position(None, None, &shapes_path, helper_line, 9)
        .await
        .expect("symbol_at_position is wired");
    if let Some(id) = resolved {
        assert_eq!(id, helper_id, "FL2: a cursor on `helper` resolves to it");
    }
}
