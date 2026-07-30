//! F37 — cross-encoder rerank stage A/B measurement.
//!
//! Runs the full `QueryService` survey path (RRF + shaping, NOT the bare
//! `MultiResolutionSearch` the other evals use) over the eval ground truth,
//! twice: baseline vs `with_reranker` (fastembed cross-encoder, depth-capped
//! at `CROSS_ENCODER_RERANK_DEPTH`). Reports precision@5 / recall@5 / MRR /
//! nDCG@5 for both arms plus the per-query latency the reranker adds — the
//! measurement the F37 acceptance requires BEFORE any thought of defaulting
//! the stage on.
//!
//! Informational (no quality floors): the point is the measured delta, not a
//! gate. Ignored by default — it downloads the real embedding AND reranker
//! models. Run explicitly:
//!
//! ```sh
//! MINISTR_COREML=0 cargo test -p ministr-core --test eval_rerank -- --ignored --nocapture
//! ```

// Counter/latency casts are inherent to a measurement harness.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use common::{GroundTruth, ndcg_at_k, precision_at_k, recall_at_k, reciprocal_rank};
use ministr_core::embedding::{Embedder, FastEmbedder, FastReranker};
use ministr_core::index::{ExactScanIndex, VectorIndex};
use ministr_core::ingestion::IngestionPipeline;
use ministr_core::service::QueryService;
use ministr_core::storage::SqliteStorage;

/// Reranker model under measurement — the smallest supported cross-encoder,
/// chosen to bound both download size and CPU latency for the local default
/// question. Swap the name to measure the larger models.
const RERANKER_MODEL: &str = "jina-reranker-v1-turbo-en";

struct ArmResults {
    mean_precision: f64,
    mean_recall: f64,
    mrr: f64,
    mean_ndcg: f64,
    latencies_ms: Vec<f64>,
}

async fn run_arm(service: &QueryService, ground_truth: &GroundTruth) -> ArmResults {
    let k = 5;
    let mut total_precision = 0.0;
    let mut total_recall = 0.0;
    let mut total_rr = 0.0;
    let mut total_ndcg = 0.0;
    let mut latencies_ms = Vec::with_capacity(ground_truth.queries.len());

    for annotation in &ground_truth.queries {
        let t = Instant::now();
        let results = service
            .survey(&annotation.query, 10)
            .await
            .expect("survey failed");
        latencies_ms.push(t.elapsed().as_secs_f64() * 1e3);

        let result_ids: Vec<String> = results.iter().map(|r| r.content_id.clone()).collect();
        let expected_ids: Vec<String> = annotation
            .expected
            .iter()
            .map(|e| e.section_id.clone())
            .collect();

        total_precision += precision_at_k(&result_ids, &expected_ids, k);
        total_recall += recall_at_k(&result_ids, &expected_ids, k);
        total_rr += reciprocal_rank(&result_ids, &expected_ids);
        total_ndcg += ndcg_at_k(&result_ids, &annotation.expected, k);
    }

    let n = ground_truth.queries.len() as f64;
    ArmResults {
        mean_precision: total_precision / n,
        mean_recall: total_recall / n,
        mrr: total_rr / n,
        mean_ndcg: total_ndcg / n,
        latencies_ms,
    }
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn p95(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[((sorted.len() as f64 - 1.0) * 0.95) as usize]
}

#[tokio::test]
#[ignore = "downloads real embedding + reranker models; run: MINISTR_COREML=0 cargo test -p ministr-core --test eval_rerank -- --ignored --nocapture"]
async fn eval_rerank_ab() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .parent()
        .expect("failed to find workspace root");
    let corpus_path = workspace_root.join("eval/corpus");
    let ground_truth_path = workspace_root.join("eval/ground-truth.json");
    if !corpus_path.exists() || !ground_truth_path.exists() {
        eprintln!("Skipping eval: eval/ data not found");
        return;
    }
    let ground_truth: GroundTruth = serde_json::from_str(
        &std::fs::read_to_string(&ground_truth_path).expect("failed to read ground truth"),
    )
    .expect("failed to parse ground truth");

    let embedder: Arc<dyn Embedder> = Arc::new(
        FastEmbedder::new("all-MiniLM-L6-v2", None)
            .expect("failed to load real embedding model (all-MiniLM-L6-v2)"),
    );
    let dim = embedder.dimension();
    let storage = SqliteStorage::open_in_memory().expect("failed to create storage");
    let index: Arc<dyn VectorIndex> = Arc::new(ExactScanIndex::new(dim));

    let pipeline = IngestionPipeline::new();
    let stats = pipeline
        .ingest_directory_with_embeddings(&corpus_path, &storage, &*embedder, &*index)
        .await
        .expect("ingestion failed");
    assert!(stats.files_indexed > 0, "no files were indexed");

    let baseline_service =
        QueryService::new(storage.clone(), Arc::clone(&embedder), Arc::clone(&index));

    let reranker = FastReranker::new(RERANKER_MODEL, None)
        .expect("failed to load reranker model (network required on first run)");
    let reranked_service = QueryService::new(storage.clone(), Arc::clone(&embedder), index)
        .with_reranker(Arc::new(reranker));

    // Warm both arms once so first-inference session setup doesn't pollute
    // the latency numbers.
    let _ = baseline_service.survey("warmup query", 10).await;
    let _ = reranked_service.survey("warmup query", 10).await;

    let base = run_arm(&baseline_service, &ground_truth).await;
    let rerank = run_arm(&reranked_service, &ground_truth).await;

    let added_mean = mean(&rerank.latencies_ms) - mean(&base.latencies_ms);
    eprintln!();
    eprintln!("=== F37 cross-encoder rerank A/B ({RERANKER_MODEL}, depth 20) ===");
    eprintln!("Queries: {}", ground_truth.queries.len());
    eprintln!();
    eprintln!(
        "{:<14} {:>10} {:>12} {:>10}",
        "metric", "baseline", "reranked", "delta"
    );
    for (label, b, r) in [
        ("P@5", base.mean_precision, rerank.mean_precision),
        ("R@5", base.mean_recall, rerank.mean_recall),
        ("MRR", base.mrr, rerank.mrr),
        ("nDCG@5", base.mean_ndcg, rerank.mean_ndcg),
    ] {
        eprintln!("{label:<14} {b:>10.3} {r:>12.3} {:>+10.3}", r - b);
    }
    eprintln!();
    eprintln!(
        "latency mean   {:>8.1}ms {:>10.1}ms {:>+9.1}ms",
        mean(&base.latencies_ms),
        mean(&rerank.latencies_ms),
        added_mean
    );
    eprintln!(
        "latency p95    {:>8.1}ms {:>10.1}ms {:>+9.1}ms",
        p95(&base.latencies_ms),
        p95(&rerank.latencies_ms),
        p95(&rerank.latencies_ms) - p95(&base.latencies_ms)
    );
    eprintln!();
    eprintln!("(informational — no floors; record these numbers in the F37 decision trace)");

    assert!(
        !rerank.latencies_ms.is_empty(),
        "reranked arm produced no measurements"
    );
}
