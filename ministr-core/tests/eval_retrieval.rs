//! Evaluation retrieval test — ingests the eval corpus, runs ground-truth
//! queries, and measures precision@k, recall@k, MRR, and nDCG@k.
//!
//! Uses a deterministic hash-based embedder (no model download needed).
//! These tests serve as smoke tests for retrieval quality, not strict
//! benchmarks — the mock embedder lacks real semantic understanding.

mod common;

use std::path::Path;

use common::{
    ExpectedResult, GroundTruth, ndcg_at_k, precision_at_k, probe_corpus_ids, recall_at_k,
    reciprocal_rank, run_eval_with_embedder,
};
use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;

/// Deterministic hash-based embedder for evaluation (no model download).
///
/// Produces unit vectors where each dimension is derived from the byte values
/// of the input text. Texts with overlapping vocabulary will have similar
/// vectors, providing a rough proxy for semantic similarity.
struct HashEmbedder {
    dim: usize,
}

impl Embedder for HashEmbedder {
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

/// Wraps an inner embedder and records the exact text of every input it is
/// asked to embed — i.e. one string per embedded section. Used by the RQ1
/// truncation content-loss measurement to recover, faithfully, the units that
/// actually get embedded (rather than re-deriving them from raw files).
struct CapturingEmbedder {
    inner: HashEmbedder,
    seen: std::sync::Mutex<Vec<String>>,
}

impl Embedder for CapturingEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        self.seen
            .lock()
            .expect("seen mutex poisoned")
            .extend(texts.iter().map(|t| (*t).to_string()));
        self.inner.embed(texts)
    }

    fn dimension(&self) -> usize {
        self.inner.dimension()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn eval_corpus_retrieval_quality() {
    let Some((corpus_path, ground_truth)) = load_eval_data() else {
        return;
    };

    let embedder = HashEmbedder { dim: 64 };
    let results = run_eval_with_embedder(&corpus_path, &ground_truth, &embedder, true).await;

    eprintln!();
    eprintln!("=== Evaluation Results ===");
    eprintln!("Queries:          {}", results.query_count);
    eprintln!("Mean P@5:         {:.3}", results.mean_precision);
    eprintln!("Mean R@5:         {:.3}", results.mean_recall);
    eprintln!("MRR:              {:.3}", results.mrr);
    eprintln!("Mean nDCG@5:      {:.3}", results.mean_ndcg);

    // Smoke test: with a hash-based embedder, we expect at least some retrieval
    // quality because overlapping vocabulary creates correlated vectors.
    // These thresholds are intentionally lenient for the mock embedder.
    assert!(
        results.mean_recall > 0.0,
        "mean recall@5 is zero — retrieval is completely broken"
    );
}

/// Regression gate: asserts that retrieval metrics stay above minimum thresholds.
///
/// Thresholds are calibrated for the deterministic hash-based embedder.
/// They catch total retrieval breakage without being so strict that they
/// fail on minor scoring fluctuations.
#[tokio::test]
async fn eval_retrieval_regression_gate() {
    // Minimum thresholds for the hash-based embedder.
    // These are intentionally lenient — the hash embedder has limited
    // semantic capability. With a real model, thresholds would be higher.
    const MIN_MRR: f64 = 0.01;
    const MIN_RECALL: f64 = 0.01;
    const MIN_NDCG: f64 = 0.01;

    let Some((corpus_path, ground_truth)) = load_eval_data() else {
        return;
    };

    let embedder = HashEmbedder { dim: 64 };
    let results = run_eval_with_embedder(&corpus_path, &ground_truth, &embedder, false).await;

    assert!(
        results.mrr >= MIN_MRR,
        "MRR {:.3} dropped below minimum threshold {MIN_MRR}",
        results.mrr
    );
    assert!(
        results.mean_recall >= MIN_RECALL,
        "Mean recall@5 {:.3} dropped below minimum threshold {MIN_RECALL}",
        results.mean_recall
    );
    assert!(
        results.mean_ndcg >= MIN_NDCG,
        "Mean nDCG@5 {:.3} dropped below minimum threshold {MIN_NDCG}",
        results.mean_ndcg
    );

    eprintln!("✓ Retrieval regression gate passed");
    eprintln!(
        "  MRR={:.3} (min {MIN_MRR})  recall={:.3} (min {MIN_RECALL})  nDCG={:.3} (min {MIN_NDCG})",
        results.mrr, results.mean_recall, results.mean_ndcg
    );
}

/// Real-embedder retrieval quality on the committed eval corpus — the
/// instrument the whole retrieval-quality (rq-epic) program is judged against.
///
/// Unlike [`eval_corpus_retrieval_quality`] (which uses the `HashEmbedder` mock
/// and therefore measures nothing about real semantics), this loads the REAL
/// default embedding model (`all-MiniLM-L6-v2` via ONNX/fastembed) and reports
/// actual recall@k / nDCG@k / MRR, plus a regression gate against committed
/// baseline floors.
///
/// `#[ignore]` on purpose: it downloads/loads a model (network + compute), so
/// it must NEVER run in the default `cargo test` / CI gate. Run it with:
///
/// ```text
/// just eval-quality
/// ```
///
/// SEEDING / TIGHTENING THE GATE: the `BASELINE_*` floors below are
/// conservative real-model lower bounds chosen to catch a degenerate index
/// without false-failing on first run. After a `just eval-quality` run, read
/// the printed metrics and raise each floor to ~0.05 under the observed value
/// so the gate becomes a real regression detector for the RQ chunks
/// (rq1 truncation, rq2 model swap, rq3 chunking, rq4 hybrid, rq5 rerank).
#[tokio::test]
#[ignore = "loads a real embedding model (network/compute); run via `just eval-quality`"]
async fn eval_retrieval_real_embedder() {
    use ministr_core::embedding::FastEmbedder;

    // Seeded 2026-06-02 from a real `just eval-quality` run on all-MiniLM-L6-v2.
    // Expanded corpus (now incl. long code + doc sections >256 tokens; 72
    // queries): R@5=0.819, MRR=0.939, nDCG@5=0.872 (the last with the corrected,
    // [0,1]-bounded ndcg_at_k). Floors sit ~0.05 under each observed value:
    // tight enough to catch a real regression (rq2 model swap, rq4 hybrid, rq5
    // rerank), loose enough to absorb minor scoring jitter. The eval is
    // deterministic (in-memory corpus + fixed weights), so re-seed only when the
    // model, corpus, or metric definition changes.
    //
    // rq3-eval-confirm A/B (2026-06-02): the cAST split (rq3a) is NEUTRAL on this
    // doc-heavy corpus. CODE_CHUNK_BUDGET=256 (split ON): R@5 0.812 / MRR 0.939 /
    // nDCG 0.870; budget=1_000_000 (split OFF): R@5 0.819 / MRR 0.939 / nDCG 0.872
    // — within ±0.01 jitter (the OFF arm reproduces the seeded baseline exactly).
    // The corpus has ~1 code file, so the split barely fires here; it is kept on
    // CORRECTNESS grounds (lossless, no silent truncation of over-budget symbols),
    // not a doc-corpus metric gain. A code-heavy corpus with genuinely
    // over-budget symbols is needed to positively measure cAST (see
    // rq-eval-corpus-bigcode). Floors NOT raised (split did not improve them).
    const BASELINE_RECALL_AT_5: f64 = 0.77;
    const BASELINE_NDCG_AT_5: f64 = 0.82;
    const BASELINE_MRR: f64 = 0.88;

    let Some((corpus_path, ground_truth)) = load_eval_data() else {
        eprintln!("Skipping eval: eval/ data not found");
        return;
    };

    let embedder = FastEmbedder::new("all-MiniLM-L6-v2", None)
        .expect("failed to load real embedding model (all-MiniLM-L6-v2)");
    let results = run_eval_with_embedder(&corpus_path, &ground_truth, &embedder, true).await;

    eprintln!();
    eprintln!("=== Real-embedder retrieval quality (all-MiniLM-L6-v2) ===");
    eprintln!("Queries:     {}", results.query_count);
    eprintln!("Mean P@5:    {:.3}", results.mean_precision);
    eprintln!(
        "Mean R@5:    {:.3}   (baseline floor {BASELINE_RECALL_AT_5})",
        results.mean_recall
    );
    eprintln!(
        "MRR:         {:.3}   (baseline floor {BASELINE_MRR})",
        results.mrr
    );
    eprintln!(
        "Mean nDCG@5: {:.3}   (baseline floor {BASELINE_NDCG_AT_5})",
        results.mean_ndcg
    );
    eprintln!("(to tighten the gate: raise the BASELINE_* floors to ~0.05 under these)");

    assert!(
        results.mean_recall >= BASELINE_RECALL_AT_5,
        "recall@5 {:.3} regressed below baseline floor {BASELINE_RECALL_AT_5}",
        results.mean_recall
    );
    assert!(
        results.mean_ndcg >= BASELINE_NDCG_AT_5,
        "nDCG@5 {:.3} regressed below baseline floor {BASELINE_NDCG_AT_5}",
        results.mean_ndcg
    );
    assert!(
        results.mrr >= BASELINE_MRR,
        "MRR {:.3} regressed below baseline floor {BASELINE_MRR}",
        results.mrr
    );
}

/// RQ2 — embedder bake-off: benchmark candidate embedding models against the
/// eval golden set and print a comparison table (dim + P@5/R@5/MRR/nDCG@5).
///
/// Use the printed spread to pick a default; the production swap is a separate
/// step (a dimension change forces a full re-index of every corpus). The
/// candidate set is the fastembed-runnable 2026 field — `nomic-embed-code` (7B)
/// and `voyage-code-3` (API-only) are not locally runnable and are excluded.
/// `jina-embeddings-v2-base-code` is the code-specialized entry; `bge-m3` is the
/// 2026 general SOTA (large download, listed last). Per-model load failures are
/// reported and skipped rather than aborting the run.
///
/// `#[ignore]`: downloads several embedding models (network/compute). Run via:
///
/// ```text
/// just eval-bakeoff
/// ```
#[tokio::test]
#[ignore = "downloads several embedding models; run via `just eval-bakeoff`"]
async fn eval_model_bakeoff() {
    let Some((corpus_path, ground_truth)) = load_eval_data() else {
        eprintln!("Skipping: eval/ data not found");
        return;
    };
    run_model_bakeoff(
        &corpus_path,
        &ground_truth,
        "RQ2 embedder bake-off (doc-heavy corpus)",
    )
    .await;
}

/// RQ2-followup — the SAME bake-off run against the code-heavy corpus
/// (`eval/corpus-code` + `eval/ground-truth-code.json`: 26 text-to-code queries
/// over Rust/Python/Go/TypeScript/Java/C++).
///
/// The doc-heavy [`eval_model_bakeoff`] concluded "no swap" (every candidate
/// regressed vs all-MiniLM-L6-v2), but flagged that `jina-embeddings-v2-base-code`
/// was the closest non-baseline and might overtake on a code-representative
/// corpus — which is what agents actually retrieve.
///
/// RESULT (2026-06-02, 26 code queries; `just eval-bakeoff-code`):
/// ```text
/// model                         dim    P@5    R@5    MRR  nDCG@5
/// all-MiniLM-L6-v2              384  0.323  0.635  0.604  0.550
/// gte-base-en-v1.5             768  0.285  0.558  0.705  0.528
/// jina-embeddings-v2-base-code 768  0.315  0.673  0.681  0.604   <- wins R@5 + nDCG
/// bge-m3                      1024  0.277  0.500  0.618  0.503
/// ```
/// Hypothesis CONFIRMED: jina-code's edge is CODE-SPECIFIC — it overtakes
/// `MiniLM` on recall + nDCG on code, yet `MiniLM` still wins every metric on the
/// doc corpus.
/// A single global default can't be best at both, so the architecturally-right
/// answer is per-corpus embedder routing, NOT a global swap (which would also
/// double the vector dim 384->768 and force a full re-index). bge-m3 now loads
/// fine via fastembed (the earlier failure was transient) but is not a contender.
///
/// `#[ignore]`: downloads several embedding models (network/compute). Run via:
///
/// ```text
/// just eval-bakeoff-code
/// ```
#[tokio::test]
#[ignore = "downloads several embedding models; run via `just eval-bakeoff-code`"]
async fn eval_model_bakeoff_code() {
    let Some((corpus_path, ground_truth)) =
        load_eval_data_from("eval/corpus-code", "eval/ground-truth-code.json")
    else {
        eprintln!("Skipping: eval/corpus-code data not found");
        return;
    };
    run_model_bakeoff(
        &corpus_path,
        &ground_truth,
        "RQ2-followup embedder bake-off (code-heavy corpus)",
    )
    .await;
}

/// Shared bake-off body: load each 2026-candidate model, run the eval against
/// `(corpus_path, ground_truth)`, and print one comparison row per model
/// (dim + P@5/R@5/MRR/nDCG@5). Per-model load failures are reported and skipped
/// rather than aborting the run.
///
/// The candidate set is the fastembed-runnable 2026 field. `nomic-embed-code`
/// (7B) and `voyage-code-3` (API-only) are not locally runnable and excluded;
/// `bge-m3` is listed (2026 general SOTA) but currently fails to load via
/// fastembed — tracked by the rq2-bge-m3-candle-runner chunk.
async fn run_model_bakeoff(corpus_path: &Path, ground_truth: &GroundTruth, title: &str) {
    use ministr_core::embedding::FastEmbedder;

    const CANDIDATES: &[&str] = &[
        "all-MiniLM-L6-v2", // baseline (current default)
        "bge-small-en-v1.5",
        "bge-base-en-v1.5",
        "gte-base-en-v1.5",
        "jina-embeddings-v2-base-code", // code-specialized
        "nomic-embed-text-v1.5",        // Matryoshka
        "all-mpnet-base-v2",
        "bge-m3", // 2026 general SOTA (large)
    ];

    eprintln!();
    eprintln!("=== {title} ({} queries) ===", ground_truth.queries.len());
    eprintln!(
        "{:<32} {:>4}  {:>6} {:>6} {:>6} {:>6}",
        "model", "dim", "P@5", "R@5", "MRR", "nDCG@5"
    );
    for name in CANDIDATES {
        match FastEmbedder::new(name, None) {
            Ok(embedder) => {
                let r = run_eval_with_embedder(corpus_path, ground_truth, &embedder, false).await;
                eprintln!(
                    "{:<32} {:>4}  {:>6.3} {:>6.3} {:>6.3} {:>6.3}",
                    name,
                    embedder.dimension(),
                    r.mean_precision,
                    r.mean_recall,
                    r.mrr,
                    r.mean_ndcg
                );
            }
            Err(e) => eprintln!("{name:<32} FAILED to load: {e}"),
        }
    }
    eprintln!(
        "(directional: small {}-query corpus; read the spread, not a single point)",
        ground_truth.queries.len()
    );
}

/// RQ1 — quantify how much section content the embedding truncation cap
/// silently drops.
///
/// Ingests the committed eval corpus through the real ingestion pipeline with a
/// [`CapturingEmbedder`], capturing the exact string of every embedded section,
/// then tokenizes each with the real `all-MiniLM-L6-v2` `WordPiece` tokenizer
/// (truncation DISABLED, so true lengths are measured) and reports the
/// token-length distribution plus the sections / tokens lost at the old 128-token
/// cap and at the model's real 256-token cap (this chunk's fix).
///
/// `#[ignore]`: downloads `tokenizer.json` on first run (network). The ingest
/// itself uses the hash mock — no embedding model is downloaded. Run via:
///
/// ```text
/// just eval-truncation
/// ```
#[tokio::test]
#[ignore = "downloads a tokenizer (network); run via `just eval-truncation`"]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
async fn measure_truncation_content_loss() {
    use ministr_core::index::HnswIndex;
    use ministr_core::ingestion::IngestionPipeline;
    use ministr_core::storage::SqliteStorage;

    let Some((corpus_path, _gt)) = load_eval_data() else {
        eprintln!("Skipping: eval/ data not found");
        return;
    };

    // Ingest the corpus, capturing every embedded section's text. The mock
    // embedder keeps this model-free; only the token COUNT below uses the real
    // tokenizer.
    let embedder = CapturingEmbedder {
        inner: HashEmbedder { dim: 64 },
        seen: std::sync::Mutex::new(Vec::new()),
    };
    let storage = SqliteStorage::open_in_memory().expect("failed to create storage");
    let index = HnswIndex::new(embedder.dimension(), 10_000).expect("failed to create index");
    IngestionPipeline::new()
        .ingest_directory_with_embeddings(&corpus_path, &storage, &embedder, &index)
        .await
        .expect("ingestion failed");

    let texts = embedder.seen.into_inner().expect("seen mutex poisoned");
    assert!(!texts.is_empty(), "no sections were embedded");

    // Real WordPiece tokenizer with truncation disabled, so we see true lengths.
    let api = hf_hub::api::sync::Api::new().expect("failed to init hf-hub api");
    let tok_path = api
        .model("sentence-transformers/all-MiniLM-L6-v2".to_string())
        .get("tokenizer.json")
        .expect("failed to download tokenizer.json");
    let mut tokenizer =
        tokenizers::Tokenizer::from_file(&tok_path).expect("failed to load tokenizer");
    // Disable BOTH truncation and padding: padding (the loaded tokenizer.json
    // pads to a fixed length) would otherwise inflate every short input's
    // get_ids().len() up to the pad length, masking the true content length.
    tokenizer
        .with_truncation(None)
        .expect("failed to disable truncation");
    tokenizer.with_padding(None);

    // Token length per embedded section (with special tokens, as embed() uses).
    let mut lens: Vec<usize> = texts
        .iter()
        .map(|t| {
            tokenizer
                .encode(t.as_str(), true)
                .expect("tokenization failed")
                .get_ids()
                .len()
        })
        .collect();
    lens.sort_unstable();
    let n = lens.len();
    let total_tokens: usize = lens.iter().sum();

    // Nearest-rank percentile.
    let pct = |p: f64| -> usize {
        let idx = ((p / 100.0) * (n as f64)).ceil() as usize;
        lens[idx.clamp(1, n) - 1]
    };
    // (#sections over the cap, #tokens dropped by the cap).
    let lost_at = |cap: usize| -> (usize, usize) {
        let sections = lens.iter().filter(|&&l| l > cap).count();
        let tokens: usize = lens.iter().map(|&l| l.saturating_sub(cap)).sum();
        (sections, tokens)
    };
    let (over128, lost128) = lost_at(128);
    let (over256, lost256) = lost_at(256);
    let frac = |x: usize, whole: usize| -> f64 {
        if whole == 0 {
            0.0
        } else {
            100.0 * x as f64 / whole as f64
        }
    };

    eprintln!();
    eprintln!("=== RQ1 truncation content-loss (all-MiniLM-L6-v2 WordPiece) ===");
    eprintln!("Embedded sections:          {n}");
    eprintln!(
        "Token length p50/p90/p99/max: {} / {} / {} / {}",
        pct(50.0),
        pct(90.0),
        pct(99.0),
        lens.last().copied().unwrap_or(0)
    );
    eprintln!(
        "Sections > 128 tokens:      {over128} ({:.1}%)   tokens dropped @128: {lost128} ({:.1}% of all tokens)",
        frac(over128, n),
        frac(lost128, total_tokens)
    );
    eprintln!(
        "Sections > 256 tokens:      {over256} ({:.1}%)   tokens dropped @256: {lost256} ({:.1}% of all tokens)",
        frac(over256, n),
        frac(lost256, total_tokens)
    );
    eprintln!(
        "=> Raising the cap 128 -> 256 (this chunk) recovers content for {over128} section(s); \
         {over256} still exceed 256 (candidates for AST/late chunking, rq3/rq6)."
    );
}

/// Calibration utility (not a real test): ingest the code-heavy corpus with the
/// model-free hash embedder and print the real `content_ids` the index emits, so
/// `eval/ground-truth-code.json` can be authored against verified substrings
/// rather than guessed language-dependent symbol id formats. Run via:
///
/// ```text
/// just eval-dump-code-ids
/// ```
#[tokio::test]
#[ignore = "calibration dump, not an assertion; run via `just eval-dump-code-ids`"]
async fn dump_code_corpus_ids() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .parent()
        .expect("failed to find workspace root");
    let corpus_path = workspace_root.join("eval/corpus-code");
    if !corpus_path.exists() {
        eprintln!("Skipping: eval/corpus-code not found");
        return;
    }

    // Broad single-word probes; a wide top_k surfaces the full id universe of
    // this small corpus regardless of the (weak) hash-embedder ranking.
    let probes = [
        "rate limit token bucket",
        "least recently used cache eviction",
        "retry exponential backoff circuit breaker",
        "debounce throttle coalesce events",
        "disjoint set union find connected",
        "binary search lower upper bound sorted array",
    ];
    let embedder = HashEmbedder { dim: 64 };
    let hits = probe_corpus_ids(&corpus_path, &embedder, &probes, 200).await;

    let mut universe: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (_, ids) in &hits {
        universe.extend(ids.iter().cloned());
    }

    eprintln!();
    eprintln!(
        "=== code corpus content_ids ({} unique) ===",
        universe.len()
    );
    for id in &universe {
        eprintln!("{id}");
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load the default (doc-heavy) eval corpus and ground truth.
fn load_eval_data() -> Option<(std::path::PathBuf, GroundTruth)> {
    load_eval_data_from("eval/corpus", "eval/ground-truth.json")
}

/// Load an eval corpus + ground-truth pair by workspace-relative paths,
/// returning `None` (with a skip message) if either is missing. Lets the
/// doc-heavy and code-heavy bake-offs share one loader.
fn load_eval_data_from(
    corpus_rel: &str,
    ground_truth_rel: &str,
) -> Option<(std::path::PathBuf, GroundTruth)> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .parent()
        .expect("failed to find workspace root");
    let corpus_path = workspace_root.join(corpus_rel);
    let ground_truth_path = workspace_root.join(ground_truth_rel);

    if !corpus_path.exists() || !ground_truth_path.exists() {
        eprintln!("Skipping eval test: {corpus_rel} or {ground_truth_rel} not found");
        return None;
    }

    let gt_json = std::fs::read_to_string(&ground_truth_path).expect("failed to read ground truth");
    let ground_truth: GroundTruth =
        serde_json::from_str(&gt_json).expect("failed to parse ground truth");

    Some((corpus_path, ground_truth))
}

// ---------------------------------------------------------------------------
// Unit tests for metrics (edge cases)
// ---------------------------------------------------------------------------

#[test]
fn ground_truth_file_parses() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .parent()
        .expect("failed to find workspace root");
    let gt_path = workspace_root.join("eval/ground-truth.json");

    if !gt_path.exists() {
        eprintln!("Skipping: ground-truth.json not found");
        return;
    }

    let gt_json = std::fs::read_to_string(&gt_path).unwrap();
    let gt: GroundTruth = serde_json::from_str(&gt_json).unwrap();

    assert!(
        gt.queries.len() >= 50,
        "ground truth must have at least 50 queries, found {}",
        gt.queries.len()
    );
    for q in &gt.queries {
        assert!(!q.query.is_empty(), "empty query in ground truth");
        assert!(
            !q.expected.is_empty(),
            "query '{}' has no expected results",
            q.query
        );
        for e in &q.expected {
            assert!(!e.section_id.is_empty(), "empty section_id in ground truth");
            assert!(
                (1..=3).contains(&e.relevance),
                "relevance must be 1-3, got {} for {}",
                e.relevance,
                e.section_id
            );
        }
    }
}

#[test]
fn ground_truth_code_file_parses() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .parent()
        .expect("failed to find workspace root");
    let gt_path = workspace_root.join("eval/ground-truth-code.json");

    if !gt_path.exists() {
        eprintln!("Skipping: ground-truth-code.json not found");
        return;
    }

    let gt_json = std::fs::read_to_string(&gt_path).unwrap();
    let gt: GroundTruth = serde_json::from_str(&gt_json).unwrap();

    assert!(
        gt.queries.len() >= 20,
        "code ground truth must have at least 20 queries, found {}",
        gt.queries.len()
    );
    for q in &gt.queries {
        assert!(!q.query.is_empty(), "empty query in code ground truth");
        assert!(
            !q.expected.is_empty(),
            "query '{}' has no expected results",
            q.query
        );
        for e in &q.expected {
            assert!(
                !e.section_id.is_empty(),
                "empty section_id in code ground truth"
            );
            // Code section_ids follow the `<file>#<module>::<symbol>` convention
            // (verified against real index ids by the dump_code_corpus_ids test).
            assert!(
                e.section_id.contains('#'),
                "code section_id '{}' should reference a file section",
                e.section_id
            );
            assert!(
                (1..=3).contains(&e.relevance),
                "relevance must be 1-3, got {} for {}",
                e.relevance,
                e.section_id
            );
        }
    }
}

#[test]
fn precision_recall_edge_cases() {
    // Empty results
    let empty: Vec<String> = vec![];
    let expected = vec!["a".to_string()];
    assert!((precision_at_k(&empty, &expected, 5) - 0.0).abs() < f64::EPSILON);
    assert!((recall_at_k(&empty, &expected, 5) - 0.0).abs() < f64::EPSILON);

    // Empty expected
    let results = vec!["a".to_string()];
    let empty_exp: Vec<String> = vec![];
    assert!((recall_at_k(&results, &empty_exp, 5) - 1.0).abs() < f64::EPSILON);

    // Perfect match
    let results = vec!["a".to_string(), "b".to_string()];
    let expected = vec!["a".to_string(), "b".to_string()];
    assert!((precision_at_k(&results, &expected, 5) - 1.0).abs() < f64::EPSILON);
    assert!((recall_at_k(&results, &expected, 5) - 1.0).abs() < f64::EPSILON);
}

#[test]
fn mrr_edge_cases() {
    // No relevant results
    let results = vec!["x".to_string(), "y".to_string()];
    let expected = vec!["a".to_string()];
    assert!((reciprocal_rank(&results, &expected) - 0.0).abs() < f64::EPSILON);

    // First result is relevant → RR = 1.0
    let results = vec!["a".to_string(), "b".to_string()];
    let expected = vec!["a".to_string()];
    assert!((reciprocal_rank(&results, &expected) - 1.0).abs() < f64::EPSILON);

    // Second result is relevant → RR = 0.5
    let results = vec!["x".to_string(), "a".to_string()];
    let expected = vec!["a".to_string()];
    assert!((reciprocal_rank(&results, &expected) - 0.5).abs() < f64::EPSILON);

    // Empty results
    let empty: Vec<String> = vec![];
    assert!((reciprocal_rank(&empty, &expected) - 0.0).abs() < f64::EPSILON);
}

#[test]
fn ndcg_edge_cases() {
    // Perfect single result → nDCG = 1.0
    let results = vec!["a".to_string()];
    let expected = vec![ExpectedResult {
        section_id: "a".to_string(),
        relevance: 3,
    }];
    assert!((ndcg_at_k(&results, &expected, 5) - 1.0).abs() < f64::EPSILON);

    // No relevant results → nDCG = 0.0
    let results = vec!["x".to_string()];
    let expected = vec![ExpectedResult {
        section_id: "a".to_string(),
        relevance: 3,
    }];
    assert!((ndcg_at_k(&results, &expected, 5) - 0.0).abs() < 0.001);

    // Empty results → nDCG = 0.0
    let empty: Vec<String> = vec![];
    assert!((ndcg_at_k(&empty, &expected, 5) - 0.0).abs() < 0.001);

    // No expected results → nDCG = 0.0 (idcg is 0)
    let results = vec!["a".to_string()];
    let empty_expected: Vec<ExpectedResult> = vec![];
    assert!((ndcg_at_k(&results, &empty_expected, 5) - 0.0).abs() < f64::EPSILON);
}

#[test]
fn ndcg_never_exceeds_one_on_duplicate_matches() {
    // Two DISTINCT result ids that both contain the same expected section_id
    // (loose substring matching). Each expected item must be credited at most
    // once, so nDCG stays within [0, 1] — the regression for the observed
    // Mean nDCG@5 = 1.612 (rq0-eval-hardening).
    let results = vec![
        "src/foo.rs#bar".to_string(),
        "src/foo.rs#bar-helper".to_string(),
    ];
    let expected = vec![ExpectedResult {
        section_id: "foo.rs#bar".to_string(),
        relevance: 3,
    }];
    let n = ndcg_at_k(&results, &expected, 5);
    assert!(n <= 1.0 + f64::EPSILON, "nDCG must not exceed 1.0, got {n}");
    // The first result is a perfect rank-1 hit, so the ideal is achieved.
    assert!((n - 1.0).abs() < 1e-9, "expected perfect nDCG=1.0, got {n}");
}
