//! Evaluation retrieval test — ingests the eval corpus, runs ground-truth
//! queries, and measures precision@k, recall@k, MRR, and nDCG@k.
//!
//! Uses a deterministic hash-based embedder (no model download needed).
//! These tests serve as smoke tests for retrieval quality, not strict
//! benchmarks — the mock embedder lacks real semantic understanding.

mod common;

use std::path::Path;

use common::{
    ExpectedResult, GroundTruth, ndcg_at_k, precision_at_k, recall_at_k, reciprocal_rank,
    run_eval_with_embedder,
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

    // Conservative floors: a working real model clears these comfortably; a
    // degenerate/broken index does not. Re-seed from a `just eval-quality` run.
    const BASELINE_RECALL_AT_5: f64 = 0.10;
    const BASELINE_NDCG_AT_5: f64 = 0.08;
    const BASELINE_MRR: f64 = 0.10;

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
    tokenizer
        .with_truncation(None)
        .expect("failed to disable truncation");

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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Load eval corpus and ground truth, returning `None` (with a skip message)
/// if the eval directory is missing.
fn load_eval_data() -> Option<(std::path::PathBuf, GroundTruth)> {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .parent()
        .expect("failed to find workspace root");
    let corpus_path = workspace_root.join("eval/corpus");
    let ground_truth_path = workspace_root.join("eval/ground-truth.json");

    if !corpus_path.exists() || !ground_truth_path.exists() {
        eprintln!("Skipping eval test: eval/ directory not found");
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
