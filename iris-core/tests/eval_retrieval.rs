//! Evaluation retrieval test — ingests the eval corpus, runs ground-truth
//! queries, and measures precision@k and recall@k.
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
    #[allow(dead_code)]
    relevance: u8,
}

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

    // Load ground truth
    let gt_json = std::fs::read_to_string(&ground_truth_path).expect("failed to read ground truth");
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
        .ingest_directory_with_embeddings(&corpus_path, &storage, &embedder, &index)
        .await
        .expect("ingestion failed");

    assert!(stats.files_indexed > 0, "no files were indexed");
    assert!(
        stats.total_sections > 0,
        "no sections were extracted from the corpus"
    );

    // Run all ground-truth queries and collect metrics
    let searcher = MultiResolutionSearch::new(&embedder, &index);
    let config = SearchConfig {
        raw_k: 30,
        top_k: 10,
    };

    let k = 5;
    let mut total_precision = 0.0;
    let mut total_recall = 0.0;
    let mut query_count = 0;

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

        total_precision += p;
        total_recall += r;
        query_count += 1;

        // Print per-query metrics for debugging
        eprintln!(
            "Query: {:50} P@{k}={p:.2}  R@{k}={r:.2}  results={}",
            &annotation.query,
            result_ids.len()
        );
    }

    let mean_precision = total_precision / f64::from(query_count);
    let mean_recall = total_recall / f64::from(query_count);

    eprintln!();
    eprintln!("=== Evaluation Results ===");
    eprintln!("Queries:          {query_count}");
    eprintln!("Mean P@{k}:        {mean_precision:.3}");
    eprintln!("Mean R@{k}:        {mean_recall:.3}");
    eprintln!("Files indexed:    {}", stats.files_indexed);
    eprintln!("Total sections:   {}", stats.total_sections);
    eprintln!("Total claims:     {}", stats.total_claims);
    eprintln!("Total embeddings: {}", stats.total_embeddings);

    // Smoke test: with a hash-based embedder, we expect at least some retrieval
    // quality because overlapping vocabulary creates correlated vectors.
    // These thresholds are intentionally lenient for the mock embedder.
    assert!(
        mean_recall > 0.0,
        "mean recall@{k} is zero — retrieval is completely broken"
    );
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

    assert!(!gt.queries.is_empty(), "ground truth has no queries");
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
