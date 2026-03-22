//! Benchmarks for ingestion pipeline throughput.
//!
//! Measures end-to-end ingestion performance (parse → extract → store) using
//! in-memory `SQLite` storage and the evaluation corpus. No embedding model is
//! needed — the pipeline stores sections and claims without vectorization.
//!
//! Run with: `cargo bench --bench ingestion -p iris-core`

use std::path::Path;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use iris_core::ingestion::IngestionPipeline;
use iris_core::storage::SqliteStorage;
use tempfile::TempDir;
use tokio::runtime::Runtime;

/// Path to the evaluation corpus (relative to workspace root).
const EVAL_CORPUS: &str = "eval/corpus";

/// Create a temporary corpus directory with `n` synthetic markdown files.
fn create_synthetic_corpus(n: usize) -> TempDir {
    let dir = TempDir::new().expect("failed to create temp dir");
    for i in 0..n {
        let content = format!(
            "# Document {i}\n\n\
             ## Overview\n\n\
             This is document number {i} in the synthetic benchmark corpus. \
             It contains multiple sections with varying content to simulate \
             a realistic documentation structure.\n\n\
             ## Configuration\n\n\
             The configuration for component {i} involves setting up environment \
             variables, connection strings, and feature flags. Each setting is \
             validated at startup and missing required values cause a fatal error.\n\n\
             ## API Reference\n\n\
             The API exposes endpoints for creating, reading, updating, and deleting \
             resources. All endpoints require authentication via bearer token. \
             Rate limiting is applied per client at 100 requests per minute.\n\n\
             ## Error Handling\n\n\
             Errors are categorized into client errors (4xx) and server errors (5xx). \
             Client errors include validation failures, authentication errors, and \
             resource not found. Server errors are logged with correlation IDs \
             and trigger alerts when the error rate exceeds thresholds.\n\n\
             ## Deployment\n\n\
             The service is deployed as a container with health checks on /health/live \
             and /health/ready endpoints. Rolling updates ensure zero downtime. \
             Canary deployments validate each release before full rollout.\n"
        );
        let path = dir.path().join(format!("doc_{i:04}.md"));
        std::fs::write(&path, content).expect("failed to write synthetic file");
    }
    dir
}

fn bench_ingestion_synthetic(c: &mut Criterion) {
    let rt = Runtime::new().expect("failed to create tokio runtime");
    let mut group = c.benchmark_group("ingestion_synthetic");
    group.sample_size(10);

    for &corpus_size in &[5, 20, 50] {
        group.bench_with_input(
            BenchmarkId::from_parameter(corpus_size),
            &corpus_size,
            |b, &n| {
                b.iter_with_setup(
                    || {
                        let dir = create_synthetic_corpus(n);
                        let storage =
                            SqliteStorage::open_in_memory().expect("failed to create storage");
                        (dir, storage)
                    },
                    |(dir, storage)| {
                        let pipeline = IngestionPipeline::new();
                        rt.block_on(pipeline.ingest_directory(dir.path(), &storage))
                            .expect("ingestion failed");
                    },
                );
            },
        );
    }

    group.finish();
}

fn bench_ingestion_eval_corpus(c: &mut Criterion) {
    let rt = Runtime::new().expect("failed to create tokio runtime");

    // Resolve eval corpus path relative to workspace root
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let workspace_root = Path::new(manifest_dir)
        .parent()
        .expect("failed to find workspace root");
    let corpus_path = workspace_root.join(EVAL_CORPUS);

    if !corpus_path.exists() {
        eprintln!(
            "Skipping eval corpus benchmark: {} not found",
            corpus_path.display()
        );
        return;
    }

    let mut group = c.benchmark_group("ingestion_eval_corpus");
    group.sample_size(10);

    group.bench_function("eval_corpus", |b| {
        b.iter_with_setup(
            || SqliteStorage::open_in_memory().expect("failed to create storage"),
            |storage| {
                let pipeline = IngestionPipeline::new();
                rt.block_on(pipeline.ingest_directory(&corpus_path, &storage))
                    .expect("ingestion failed");
            },
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_ingestion_synthetic,
    bench_ingestion_eval_corpus
);
criterion_main!(benches);
