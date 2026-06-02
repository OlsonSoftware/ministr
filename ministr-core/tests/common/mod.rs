//! Shared evaluation helpers for retrieval quality tests.
//!
//! Provides ground-truth data types, retrieval metrics (P@k, R@k, MRR, nDCG@k),
//! and a reusable eval pipeline that ingests the eval corpus and scores queries.

use std::path::Path;

use ministr_core::embedding::Embedder;
use ministr_core::index::HnswIndex;
use ministr_core::ingestion::IngestionPipeline;
use ministr_core::search::{MultiResolutionSearch, SearchConfig};
use ministr_core::storage::SqliteStorage;

/// Ground-truth query with expected section IDs and relevance grades.
#[derive(serde::Deserialize)]
pub struct GroundTruth {
    pub queries: Vec<QueryAnnotation>,
}

#[derive(serde::Deserialize)]
pub struct QueryAnnotation {
    pub query: String,
    pub expected: Vec<ExpectedResult>,
}

#[derive(serde::Deserialize)]
pub struct ExpectedResult {
    pub section_id: String,
    pub relevance: u8,
}

/// Aggregated evaluation results for a single model run.
pub struct EvalResults {
    pub query_count: u32,
    pub mean_precision: f64,
    pub mean_recall: f64,
    pub mrr: f64,
    pub mean_ndcg: f64,
}

// ---------------------------------------------------------------------------
// Retrieval metrics
// ---------------------------------------------------------------------------

/// Compute precision@k: fraction of top-k results that are in the expected set.
#[allow(clippy::cast_precision_loss)]
pub fn precision_at_k(result_ids: &[String], expected_ids: &[String], k: usize) -> f64 {
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
pub fn recall_at_k(result_ids: &[String], expected_ids: &[String], k: usize) -> f64 {
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
pub fn reciprocal_rank(result_ids: &[String], expected_ids: &[String]) -> f64 {
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
pub fn ndcg_at_k(result_ids: &[String], expected: &[ExpectedResult], k: usize) -> f64 {
    // Credit each expected item at most once, at the rank of the first result
    // that matches it. Without this dedup, several top-k results matching the
    // SAME expected id each add its gain to DCG while IDCG counts it once,
    // which lets nDCG exceed 1.0 (it is normalized to [0, 1]). A result that
    // only re-matches already-credited expecteds contributes zero gain.
    let mut credited = vec![false; expected.len()];
    let mut dcg = 0.0_f64;
    for (i, id) in result_ids.iter().take(k).enumerate() {
        let mut rel = 0.0;
        for (j, e) in expected.iter().enumerate() {
            if !credited[j] && id.contains(&e.section_id) {
                credited[j] = true;
                rel = f64::from(e.relevance);
                break;
            }
        }
        dcg += (2.0_f64.powf(rel) - 1.0) / ((i + 2) as f64).log2();
    }

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

// ---------------------------------------------------------------------------
// Eval pipeline
// ---------------------------------------------------------------------------

/// Run the full eval pipeline: ingest corpus, run all ground-truth queries,
/// return aggregated metrics.
///
/// Each call creates fresh in-memory storage and HNSW index, so different
/// embedders (with different dimensions) can be compared fairly.
#[allow(clippy::cast_precision_loss)]
pub async fn run_eval_with_embedder(
    corpus_path: &Path,
    ground_truth: &GroundTruth,
    embedder: &dyn Embedder,
    verbose: bool,
) -> EvalResults {
    let dim = embedder.dimension();
    let storage = SqliteStorage::open_in_memory().expect("failed to create storage");
    let index = HnswIndex::new(dim, 10_000).expect("failed to create index");

    let pipeline = IngestionPipeline::new();
    let stats = pipeline
        .ingest_directory_with_embeddings(corpus_path, &storage, embedder, &index)
        .await
        .expect("ingestion failed");

    assert!(stats.files_indexed > 0, "no files were indexed");
    assert!(
        stats.total_sections > 0,
        "no sections were extracted from the corpus"
    );

    if verbose {
        eprintln!("Files indexed:    {}", stats.files_indexed);
        eprintln!("Total sections:   {}", stats.total_sections);
        eprintln!("Total claims:     {}", stats.total_claims);
        eprintln!("Total embeddings: {}", stats.total_embeddings);
    }

    let searcher = MultiResolutionSearch::new(embedder, &index);
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

        if verbose {
            eprintln!(
                "Query: {:60} P@{k}={p:.2}  R@{k}={r:.2}  RR={rr:.2}  nDCG@{k}={ndcg:.2}",
                &annotation.query,
            );
        }
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

/// Ingest a corpus and, for each probe query, return the top result `content_ids`.
///
/// Calibration helper: code symbols are stored under language-dependent
/// `content_ids` (e.g. `retry.go#resilience::RetryWithBackoff`), so authoring a
/// ground-truth file by guessing those ids is error-prone. This returns the
/// real ids the index emits for a set of probe queries, so a ground-truth file
/// can be written against verified substrings rather than guesses. Uses a wide
/// `top_k` so a handful of broad probes surface the full id universe of a small
/// corpus.
///
/// Only the `eval_retrieval` test binary uses this; `#[allow(dead_code)]` keeps
/// the other binaries that share `common` (e.g. `eval_model_comparison`) clean.
#[allow(dead_code)]
pub async fn probe_corpus_ids(
    corpus_path: &Path,
    embedder: &dyn Embedder,
    queries: &[&str],
    top_k: usize,
) -> Vec<(String, Vec<String>)> {
    let dim = embedder.dimension();
    let storage = SqliteStorage::open_in_memory().expect("failed to create storage");
    let index = HnswIndex::new(dim, 10_000).expect("failed to create index");
    let pipeline = IngestionPipeline::new();
    pipeline
        .ingest_directory_with_embeddings(corpus_path, &storage, embedder, &index)
        .await
        .expect("ingestion failed");

    let searcher = MultiResolutionSearch::new(embedder, &index);
    let config = SearchConfig {
        raw_k: top_k.max(50),
        top_k,
        sparse_weight: 0.0,
        rerank_top_k: None,
    };

    queries
        .iter()
        .map(|q| {
            let results = searcher.search(q, config).expect("search failed");
            let ids = results
                .iter()
                .map(|r| r.vector_id.content_id().to_string())
                .collect();
            ((*q).to_string(), ids)
        })
        .collect()
}
