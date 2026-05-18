//! ministr-vs-LSP code-navigation benchmark — Phase 2 runner.
//!
//! Indexes this repository with ministr in-process, then for every task in
//! `eval/lsp-nav/ground-truth.json` compares ministr's answer (and, when
//! present, rust-analyzer's answer resolved from `eval/lsp-nav/ra.lsif`)
//! against the hand-verified expected locations, printing the metrics
//! table documented in `eval/lsp-nav/README.md`.
//!
//! `#[ignore]` — both indexers are minutes-long and memory-heavy, so this
//! never runs in the default test pass. Invoke via `just bench-lsp`.
//!
//! Report-only: the only hard assertions are "ground truth exists and
//! parses". It is a benchmark, not a regression gate (a CI gate over a
//! committed baseline is a deliberate later step, mirroring
//! `just eval-gate`).
//!
//! VALIDATION STATUS: compile-checked. The end-to-end comparison (full
//! self-index + a real `rust-analyzer lsif`) has NOT been run in the
//! authoring environment — the numbers are produced by the harness, not
//! vetted here. The RA side degrades to "n/a" (never to wrong numbers)
//! when `ra.lsif` is absent or a task can't be resolved in it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_core::index::HnswIndex;
use ministr_core::ingestion::IngestionPipeline;
use ministr_core::service::QueryService;
use ministr_core::storage::{SqliteStorage, SymbolFilter};

/// Deterministic mock embedder — navigation is symbol/AST-driven, so the
/// embedding quality is irrelevant here; this avoids a model download.
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

// ---------------------------------------------------------------------------
// Ground truth
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ExpectLoc {
    file: String,
    line_start: u32,
    line_end: u32,
}

#[derive(Debug, Clone)]
struct Task {
    id: String,
    kind: String, // definition | references | bridge
    symbol: String,
    /// The source position to query FROM (definition/references); the
    /// schema's `{ file, line }`. Absent for bridge tasks.
    from: Option<(String, u32)>,
    expect: Vec<ExpectLoc>,
    lsp_can_answer: bool,
}

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR = <repo>/ministr-core
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("ministr-core has a parent (repo root)")
        .to_path_buf()
}

/// Parse `eval/lsp-nav/ground-truth.json` (skips the `_schema` block).
fn load_ground_truth(root: &Path) -> Vec<Task> {
    let path = root.join("eval/lsp-nav/ground-truth.json");
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let json: serde_json::Value =
        serde_json::from_str(&raw).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));

    let arr = json
        .get("tasks")
        .and_then(|t| t.as_array())
        .expect("ground-truth.json has a `tasks` array");

    arr.iter()
        .map(|t| {
            let expect = t
                .get("expect")
                .and_then(|e| e.as_array())
                .map(|locs| {
                    locs.iter()
                        .map(|l| ExpectLoc {
                            file: l["file"].as_str().unwrap_or_default().to_string(),
                            line_start: u32::try_from(l["line_start"].as_u64().unwrap_or(0))
                                .unwrap_or(0),
                            line_end: u32::try_from(l["line_end"].as_u64().unwrap_or(0))
                                .unwrap_or(0),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let from = t.get("from").and_then(|f| {
                Some((
                    f.get("file")?.as_str()?.to_string(),
                    u32::try_from(f.get("line")?.as_u64()?).ok()?,
                ))
            });
            Task {
                id: t["id"].as_str().unwrap_or_default().to_string(),
                kind: t["kind"].as_str().unwrap_or_default().to_string(),
                symbol: t["symbol"].as_str().unwrap_or_default().to_string(),
                from,
                expect,
                lsp_can_answer: t["lsp_can_answer"].as_bool().unwrap_or(false),
            }
        })
        .collect()
}

/// Workspace source roots to index. Scoped (not the whole tree) so the
/// run is bounded and deterministic; covers the Rust crates rust-analyzer
/// also sees, plus the Tauri TS side for the cross-language bridge tasks.
fn corpus_paths(root: &Path) -> Vec<PathBuf> {
    [
        "ministr-api/src",
        "ministr-core/src",
        "ministr-daemon/src",
        "ministr-mcp/src",
        "ministr-cli/src",
        "ministr-app/src-tauri/src",
        "ministr-app/src",
    ]
    .iter()
    .map(|p| root.join(p))
    .filter(|p| p.exists())
    .collect()
}

/// A normalized location for comparison: repo-relative POSIX path + 1-based line.
fn norm_path(root: &Path, p: &str) -> String {
    let rel = Path::new(p)
        .strip_prefix(root)
        .map_or_else(|_| PathBuf::from(p), Path::to_path_buf);
    rel.to_string_lossy().replace('\\', "/")
}

/// Does `line` fall inside any expected location's range (file match by suffix)?
fn matches_expect(file: &str, line: u32, expect: &[ExpectLoc]) -> bool {
    expect.iter().any(|e| {
        (file.ends_with(&e.file) || e.file.ends_with(file))
            && line >= e.line_start.saturating_sub(2)
            && line <= e.line_end.saturating_add(2)
    })
}

// ---------------------------------------------------------------------------
// Minimal LSIF reader (definition + references), graceful on anything it
// can't resolve — never emits a wrong location, only "unresolved".
// ---------------------------------------------------------------------------

mod lsif {
    use super::HashMap;
    use std::path::Path;

    #[derive(Default)]
    pub struct Index {
        /// range/result/resultSet id -> resultSet id (the `next` edge).
        next: HashMap<i64, i64>,
        /// resultSet id -> definitionResult id.
        def_edge: HashMap<i64, i64>,
        /// resultSet id -> referenceResult id.
        ref_edge: HashMap<i64, i64>,
        /// result id -> range ids it contains (`item` edges).
        items: HashMap<i64, Vec<i64>>,
        /// range id -> (document id, 0-based start line).
        range: HashMap<i64, (i64, u32)>,
        /// document id -> uri.
        doc_uri: HashMap<i64, String>,
        /// document id -> contained range ids.
        doc_ranges: HashMap<i64, Vec<i64>>,
    }

    impl Index {
        /// Parse a line-delimited LSIF dump. Unknown shapes are skipped;
        /// the reader is intentionally lenient (RA's exact emission order
        /// varies) and only records what definition/reference resolution
        /// needs.
        pub fn parse(text: &str) -> Self {
            let mut idx = Index::default();
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                let Some(id) = v.get("id").and_then(serde_json::Value::as_i64) else {
                    continue;
                };
                let label = v.get("label").and_then(|l| l.as_str()).unwrap_or("");
                match v.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                    "vertex" => match label {
                        "document" => {
                            if let Some(uri) = v.get("uri").and_then(|u| u.as_str()) {
                                idx.doc_uri.insert(id, uri.to_string());
                            }
                        }
                        "range" => {
                            if let Some(sl) = v
                                .get("start")
                                .and_then(|s| s.get("line"))
                                .and_then(serde_json::Value::as_u64)
                            {
                                // document is attached via a `contains`
                                // edge; line now, doc filled in below.
                                idx.range.insert(id, (-1, u32::try_from(sl).unwrap_or(0)));
                            }
                        }
                        _ => {}
                    },
                    "edge" => {
                        let out = v.get("outV").and_then(serde_json::Value::as_i64);
                        let in_single = v.get("inV").and_then(serde_json::Value::as_i64);
                        let in_many: Vec<i64> = v
                            .get("inVs")
                            .and_then(|a| a.as_array())
                            .map(|a| a.iter().filter_map(serde_json::Value::as_i64).collect())
                            .unwrap_or_default();
                        match label {
                            "contains" => {
                                if let Some(doc) = out {
                                    for r in &in_many {
                                        idx.doc_ranges.entry(doc).or_default().push(*r);
                                        if let Some(slot) = idx.range.get_mut(r) {
                                            slot.0 = doc;
                                        }
                                    }
                                }
                            }
                            "next" => {
                                if let (Some(o), Some(i)) = (out, in_single) {
                                    idx.next.insert(o, i);
                                }
                            }
                            "textDocument/definition" => {
                                if let (Some(o), Some(i)) = (out, in_single) {
                                    idx.def_edge.insert(o, i);
                                }
                            }
                            "textDocument/references" => {
                                if let (Some(o), Some(i)) = (out, in_single) {
                                    idx.ref_edge.insert(o, i);
                                }
                            }
                            "item" => {
                                if let Some(o) = out {
                                    idx.items.entry(o).or_default().extend(in_many);
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            idx
        }

        fn doc_for_file(&self, file_suffix: &str) -> Option<i64> {
            self.doc_uri
                .iter()
                .find(|(_, uri)| {
                    let p = uri.strip_prefix("file://").unwrap_or(uri);
                    let p = p.replace('\\', "/");
                    p.ends_with(file_suffix)
                })
                .map(|(d, _)| *d)
        }

        /// Range id whose 0-based start line == `line0` in the doc for `file_suffix`.
        fn range_at(&self, file_suffix: &str, line0: u32) -> Option<i64> {
            let doc = self.doc_for_file(file_suffix)?;
            self.doc_ranges
                .get(&doc)?
                .iter()
                .find(|r| self.range.get(r).is_some_and(|(_, l)| *l == line0))
                .copied()
        }

        fn result_set(&self, range_id: i64) -> i64 {
            // Follow `next` to the resultSet (RA chains range -> resultSet).
            self.next.get(&range_id).copied().unwrap_or(range_id)
        }

        fn resolve(&self, result_id: i64) -> Vec<(String, u32)> {
            let Some(items) = self.items.get(&result_id) else {
                return Vec::new();
            };
            items
                .iter()
                .filter_map(|r| {
                    let (doc, line0) = self.range.get(r)?;
                    let uri = self.doc_uri.get(doc)?;
                    let path = uri.strip_prefix("file://").unwrap_or(uri).to_string();
                    Some((
                        Path::new(&path).to_string_lossy().replace('\\', "/"),
                        line0 + 1,
                    ))
                })
                .collect()
        }

        /// rust-analyzer's go-to-definition for a position, if resolvable.
        pub fn definition(&self, file_suffix: &str, line_1based: u32) -> Vec<(String, u32)> {
            let Some(rid) = self.range_at(file_suffix, line_1based.saturating_sub(1)) else {
                return Vec::new();
            };
            let rs = self.result_set(rid);
            self.def_edge
                .get(&rs)
                .map(|d| self.resolve(*d))
                .unwrap_or_default()
        }

        /// rust-analyzer's find-references for a position, if resolvable.
        pub fn references(&self, file_suffix: &str, line_1based: u32) -> Vec<(String, u32)> {
            let Some(rid) = self.range_at(file_suffix, line_1based.saturating_sub(1)) else {
                return Vec::new();
            };
            let rs = self.result_set(rid);
            self.ref_edge
                .get(&rs)
                .map(|r| self.resolve(*r))
                .unwrap_or_default()
        }
    }
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "heavy: full self-index + rust-analyzer lsif; run via `just bench-lsp`"]
// Report-only benchmark driver: one long linear scoring loop, and the
// timing average is an intentionally lossy usize→f64.
#[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
async fn lsp_nav_benchmark() {
    let root = repo_root();
    let tasks = load_ground_truth(&root);
    assert!(!tasks.is_empty(), "ground-truth.json must define tasks");

    // --- index ministr's own source in-process ---
    let storage = SqliteStorage::open_in_memory().expect("open in-memory storage");
    let dim = 8;
    let embedder = MockEmbedder { dim };
    let index = HnswIndex::new(dim, 200_000).expect("create index");
    let paths = corpus_paths(&root);
    assert!(!paths.is_empty(), "no corpus source roots found under repo");

    let t_index = Instant::now();
    IngestionPipeline::new()
        .ingest_paths_with_embeddings(&paths, &storage, &embedder, &index)
        .await
        .expect("ingest ministr source");
    let ministr_index_secs = t_index.elapsed().as_secs_f64();

    let query = QueryService::new(
        storage,
        Arc::new(MockEmbedder { dim }),
        Arc::new(HnswIndex::new(dim, 1).expect("placeholder index")),
    );

    // --- optional rust-analyzer LSIF side ---
    let ra_path = root.join("eval/lsp-nav/ra.lsif");
    let ra = if let Ok(text) = std::fs::read_to_string(&ra_path) {
        Some(lsif::Index::parse(&text))
    } else {
        println!(
            "::notice:: {} absent — RA side reported as n/a (run `just bench-lsp-index`)",
            ra_path.display()
        );
        None
    };

    // --- score ---
    let mut m_def_ok = 0u32;
    let mut m_def_total = 0u32;
    let mut ra_def_ok = 0u32;
    let mut ra_answerable = 0u32;
    let mut m_cov = 0u32;
    let mut ra_cov = 0u32;
    let mut ref_lines = Vec::new();
    let mut per_query_ms = Vec::new();

    for task in &tasks {
        let leaf = task.symbol.split("::").last().unwrap_or(&task.symbol);
        let leaf = leaf.split('(').next().unwrap_or(leaf).trim();

        let t0 = Instant::now();
        let syms = query
            .search_symbols(&SymbolFilter {
                name_exact: Some(leaf.to_string()),
                ..SymbolFilter::default()
            })
            .await
            .unwrap_or_default();
        per_query_ms.push(t0.elapsed().as_secs_f64() * 1000.0);

        let ministr_answered = !syms.is_empty();
        if ministr_answered {
            m_cov += 1;
        }

        match task.kind.as_str() {
            "definition" => {
                m_def_total += 1;
                if let Some(s) = syms.first()
                    && let Ok(def) = query.get_symbol_definition(&s.id.0).await
                {
                    let f = norm_path(&root, &def.file_path);
                    if matches_expect(&f, def.line_start, &task.expect) {
                        m_def_ok += 1;
                    }
                }
                if task.lsp_can_answer {
                    ra_answerable += 1;
                    if let Some(ra) = &ra {
                        // RA is position-keyed; query at the expected def
                        // line in the expected file (a stable anchor that
                        // doesn't depend on choosing a use-site).
                        if let Some(e) = task.expect.first() {
                            let hits = ra.definition(&e.file, e.line_start);
                            if hits
                                .iter()
                                .any(|(f, l)| matches_expect(f, *l, &task.expect))
                            {
                                ra_def_ok += 1;
                                ra_cov += 1;
                            }
                        }
                    }
                }
            }
            "references" => {
                if let Some(s) = syms.first()
                    && let Ok(refs) = query.get_symbol_references(&s.id.0, None).await
                {
                    let got = refs.len();
                    let hit = refs
                        .iter()
                        .filter(|r| {
                            matches_expect(
                                &norm_path(&root, &r.from_file),
                                r.from_line,
                                &task.expect,
                            )
                        })
                        .count();
                    ref_lines.push(format!(
                        "  refs {:<32} ministr: {hit}/{} matched of {got} returned",
                        task.id,
                        task.expect.len()
                    ));
                }
                if task.lsp_can_answer {
                    ra_answerable += 1;
                    if let (Some(ra), Some((ffile, fline))) = (&ra, &task.from) {
                        let hits = ra.references(ffile, *fline);
                        let hit = hits
                            .iter()
                            .filter(|(f, l)| matches_expect(f, *l, &task.expect))
                            .count();
                        ref_lines.push(format!(
                            "  refs {:<32} rust-analyzer: {hit}/{} matched of {} returned",
                            task.id,
                            task.expect.len(),
                            hits.len()
                        ));
                    }
                }
            }
            "bridge" => {
                // Cross-language: ministr resolves via bridge links; a
                // Rust-only LSP is structurally blind (lsp_can_answer:false).
                if ministr_answered {
                    ra_cov += 0; // explicit: RA contributes nothing here
                }
                println!(
                    "  bridge {:<31} ministr: {} | rust-analyzer: n/a (Rust-only, cross-language)",
                    task.id,
                    if ministr_answered { "resolved" } else { "MISS" }
                );
            }
            other => println!("  (skipping unknown task kind: {other})"),
        }
    }

    let avg_q = if per_query_ms.is_empty() {
        0.0
    } else {
        per_query_ms.iter().sum::<f64>() / per_query_ms.len() as f64
    };

    println!("\n======== ministr vs LSP — code navigation ========");
    println!("tasks: {}", tasks.len());
    println!(
        "definition accuracy   ministr {m_def_ok}/{m_def_total}   rust-analyzer {ra_def_ok}/{ra_answerable}{}",
        if ra.is_none() {
            "  (RA n/a — no ra.lsif)"
        } else {
            ""
        }
    );
    println!(
        "coverage (answerable) ministr {m_cov}/{}   rust-analyzer {ra_cov}/{} — the cross-language gap",
        tasks.len(),
        tasks.len()
    );
    println!(
        "ministr index time    {ministr_index_secs:.1}s over {} source roots",
        paths.len()
    );
    println!("avg ministr query     {avg_q:.2} ms");
    for l in &ref_lines {
        println!("{l}");
    }
    println!("==================================================");
    println!(
        "NOTE: report-only, not a gate. RA numbers are n/a unless `just bench-lsp-index` \
         produced eval/lsp-nav/ra.lsif. Re-verify ground-truth line ranges if source moved."
    );
}
