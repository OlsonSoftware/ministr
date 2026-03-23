//! Evaluation retrieval test — ingests the eval corpus, runs ground-truth
//! queries, and measures precision@k, recall@k, MRR, and nDCG@k.
//!
//! Uses a deterministic hash-based embedder (no model download needed).
//! These tests serve as smoke tests for retrieval quality, not strict
//! benchmarks — the mock embedder lacks real semantic understanding.

use std::path::Path;

use iris_core::embedding::Embedder;
use iris_core::error::IndexError;
use iris_core::index::HnswIndex;
use iris_core::ingestion::IngestionPipeline;
use iris_core::search::{MultiResolutionSearch, SearchConfig};
use iris_core::storage::SqliteStorage;

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

/// Ground-truth query with expected section IDs and relevance grades.
#[derive(serde::Deserialize)]
struct GroundTruth {
    queries: Vec<QueryAnnotation>,
}

#[derive(serde::Deserialize)]
struct QueryAnnotation {
    query: String,
    expected: Vec<ExpectedResult>,
}

#[derive(serde::Deserialize)]
struct ExpectedResult {
    section_id: String,
    relevance: u8,
}

// ---------------------------------------------------------------------------
// Retrieval metrics
// ---------------------------------------------------------------------------

/// Compute precision@k: fraction of top-k results that are in the expected set.
#[allow(clippy::cast_precision_loss)]
fn precision_at_k(result_ids: &[String], expected_ids: &[String], k: usize) -> f64 {
    let top_k: Vec<_> = result_ids.iter().take(k).collect();
    if top_k.is_empty() {
        return 0.0;
    }
    let hits = top_k
        .iter()
        .filter(|id| expected_ids.iter().any(|e| id.contains(e)))
        .count();
    hits as f64 / top_k.len() as f64
}

/// Compute recall@k: fraction of expected results found in the top-k results.
#[allow(clippy::cast_precision_loss)]
fn recall_at_k(result_ids: &[String], expected_ids: &[String], k: usize) -> f64 {
    if expected_ids.is_empty() {
        return 1.0;
    }
    let top_k: Vec<_> = result_ids.iter().take(k).collect();
    let found = expected_ids
        .iter()
        .filter(|e| top_k.iter().any(|id| id.contains(e.as_str())))
        .count();
    found as f64 / expected_ids.len() as f64
}

/// Compute Mean Reciprocal Rank (MRR): 1/rank of the first relevant result.
///
/// Returns 0.0 if no relevant result is found in the result list.
#[allow(clippy::cast_precision_loss)]
fn reciprocal_rank(result_ids: &[String], expected_ids: &[String]) -> f64 {
    for (rank, id) in result_ids.iter().enumerate() {
        if expected_ids.iter().any(|e| id.contains(e)) {
            return 1.0 / (rank as f64 + 1.0);
        }
    }
    0.0
}

/// Compute nDCG@k (Normalized Discounted Cumulative Gain) using graded relevance.
///
/// Each result is scored by its relevance grade (from the ground-truth annotations).
/// DCG = sum of (2^rel - 1) / log2(rank + 1) for the top-k results.
/// nDCG = DCG / ideal DCG (where ideal sorts by relevance descending).
#[allow(clippy::cast_precision_loss)]
fn ndcg_at_k(result_ids: &[String], expected: &[ExpectedResult], k: usize) -> f64 {
    // Build a lookup from section_id to relevance grade
    let relevance_of = |id: &str| -> f64 {
        expected
            .iter()
            .find(|e| id.contains(&e.section_id))
            .map_or(0.0, |e| f64::from(e.relevance))
    };

    // DCG for the actual ranking
    let dcg: f64 = result_ids
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, id)| {
            let rel = relevance_of(id);
            (2.0_f64.powf(rel) - 1.0) / ((i + 2) as f64).log2()
        })
        .sum();

    // Ideal DCG: sort expected relevances descending, take top-k
    let mut ideal_rels: Vec<f64> = expected.iter().map(|e| f64::from(e.relevance)).collect();
    ideal_rels.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
    ideal_rels.truncate(k);

    let idcg: f64 = ideal_rels
        .iter()
        .enumerate()
        .map(|(i, &rel)| (2.0_f64.powf(rel) - 1.0) / ((i + 2) as f64).log2())
        .sum();

    if idcg == 0.0 {
        return 0.0;
    }

    dcg / idcg
}

/// Evaluation results for a single run across all queries.
struct EvalResults {
    query_count: u32,
    mean_precision: f64,
    mean_recall: f64,
    mrr: f64,
    mean_ndcg: f64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn eval_corpus_retrieval_quality() {
    // Resolve paths relative to workspace root
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .parent()
        .expect("failed to find workspace root");
    let corpus_path = workspace_root.join("eval/corpus");
    let ground_truth_path = workspace_root.join("eval/ground-truth.json");

    if !corpus_path.exists() || !ground_truth_path.exists() {
        eprintln!("Skipping eval test: eval/ directory not found");
        return;
    }

    let results = run_eval(&corpus_path, &ground_truth_path).await;

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

    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .parent()
        .expect("failed to find workspace root");
    let corpus_path = workspace_root.join("eval/corpus");
    let ground_truth_path = workspace_root.join("eval/ground-truth.json");

    if !corpus_path.exists() || !ground_truth_path.exists() {
        eprintln!("Skipping eval regression gate: eval/ directory not found");
        return;
    }

    let results = run_eval(&corpus_path, &ground_truth_path).await;

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

/// Run the full evaluation pipeline and return aggregated metrics.
#[allow(clippy::cast_precision_loss)]
async fn run_eval(corpus_path: &Path, ground_truth_path: &Path) -> EvalResults {
    // Load ground truth
    let gt_json = std::fs::read_to_string(ground_truth_path).expect("failed to read ground truth");
    let ground_truth: GroundTruth =
        serde_json::from_str(&gt_json).expect("failed to parse ground truth");

    // Set up storage, embedder, and index
    let storage = SqliteStorage::open_in_memory().expect("failed to create storage");
    let dim = 64;
    let embedder = HashEmbedder { dim };
    let index = HnswIndex::new(dim, 10_000).expect("failed to create index");

    // Ingest the eval corpus with embeddings
    let pipeline = IngestionPipeline::new();
    let stats = pipeline
        .ingest_directory_with_embeddings(corpus_path, &storage, &embedder, &index)
        .await
        .expect("ingestion failed");

    assert!(stats.files_indexed > 0, "no files were indexed");
    assert!(
        stats.total_sections > 0,
        "no sections were extracted from the corpus"
    );

    eprintln!("Files indexed:    {}", stats.files_indexed);
    eprintln!("Total sections:   {}", stats.total_sections);
    eprintln!("Total claims:     {}", stats.total_claims);
    eprintln!("Total embeddings: {}", stats.total_embeddings);

    // Run all ground-truth queries and collect metrics
    let searcher = MultiResolutionSearch::new(&embedder, &index);
    let config = SearchConfig {
        raw_k: 30,
        top_k: 10,
        sparse_weight: 0.0,
        rerank_top_k: None,
    };

    let k = 5;
    let mut total_precision = 0.0;
    let mut total_recall = 0.0;
    let mut total_rr = 0.0;
    let mut total_ndcg = 0.0;
    let mut query_count: u32 = 0;

    for annotation in &ground_truth.queries {
        let results = searcher
            .search(&annotation.query, config)
            .expect("search failed");

        let result_ids: Vec<String> = results
            .iter()
            .map(|r| r.vector_id.content_id().to_string())
            .collect();

        let expected_ids: Vec<String> = annotation
            .expected
            .iter()
            .map(|e| e.section_id.clone())
            .collect();

        let p = precision_at_k(&result_ids, &expected_ids, k);
        let r = recall_at_k(&result_ids, &expected_ids, k);
        let rr = reciprocal_rank(&result_ids, &expected_ids);
        let ndcg = ndcg_at_k(&result_ids, &annotation.expected, k);

        total_precision += p;
        total_recall += r;
        total_rr += rr;
        total_ndcg += ndcg;
        query_count += 1;

        // Print per-query metrics for debugging
        eprintln!(
            "Query: {:60} P@{k}={p:.2}  R@{k}={r:.2}  RR={rr:.2}  nDCG@{k}={ndcg:.2}",
            &annotation.query,
        );
    }

    let count_f = f64::from(query_count);
    EvalResults {
        query_count,
        mean_precision: total_precision / count_f,
        mean_recall: total_recall / count_f,
        mrr: total_rr / count_f,
        mean_ndcg: total_ndcg / count_f,
    }
}

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
