//! Benchmarks for ingestion pipeline throughput.
//!
//! Measures end-to-end ingestion performance across three scenarios:
//! 1. **Synthetic markdown** — parse → extract → store (no embeddings)
//! 2. **With embeddings** — full pipeline including mock embedding + HNSW insert
//! 3. **Mixed corpus** — markdown + Rust code files exercising tree-sitter
//!
//! Run with: `cargo bench --bench ingestion -p iris-core`

use std::path::Path;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use iris_core::embedding::Embedder;
use iris_core::error::IndexError;
use iris_core::index::HnswIndex;
use iris_core::ingestion::IngestionPipeline;
use iris_core::storage::SqliteStorage;
use tempfile::TempDir;
use tokio::runtime::Runtime;

/// Path to the evaluation corpus (relative to workspace root).
const EVAL_CORPUS: &str = "eval/corpus";

/// Lightweight mock embedder for benchmarking pipeline overhead without ONNX.
struct BenchEmbedder {
    dim: usize,
}

impl Embedder for BenchEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        Ok(texts.iter().map(|_| vec![0.0; self.dim]).collect())
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

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

/// Create a mixed corpus with both markdown and Rust source files.
///
/// Generates `n_md` markdown files and `n_rs` Rust files with realistic
/// struct/fn/impl structure to exercise tree-sitter parsing and symbol
/// extraction alongside document parsing.
#[allow(clippy::too_many_lines)]
fn create_mixed_corpus(n_md: usize, n_rs: usize) -> TempDir {
    let dir = TempDir::new().expect("failed to create temp dir");

    // Generate markdown files
    for i in 0..n_md {
        let content = format!(
            "# Module {i}\n\n\
             ## Overview\n\n\
             This module provides utilities for processing data streams. \
             It handles buffering, transformation, and output formatting.\n\n\
             ## Usage\n\n\
             ```rust\n\
             let processor = StreamProcessor::new(config);\n\
             processor.run().await?;\n\
             ```\n\n\
             ## Configuration\n\n\
             Set `BUFFER_SIZE` to control memory usage. Default is 8192 bytes.\n"
        );
        let path = dir.path().join(format!("doc_{i:04}.md"));
        std::fs::write(&path, content).expect("failed to write markdown file");
    }

    // Generate Rust source files with realistic structure
    for i in 0..n_rs {
        let content = format!(
            "//! Module {i} — data processing utilities.\n\
             \n\
             use std::collections::HashMap;\n\
             use std::io::{{self, Read, Write}};\n\
             \n\
             /// Configuration for the processing pipeline.\n\
             #[derive(Debug, Clone)]\n\
             pub struct Config{i} {{\n\
                 /// Maximum buffer size in bytes.\n\
                 pub buffer_size: usize,\n\
                 /// Whether to enable compression.\n\
                 pub compress: bool,\n\
                 /// Optional output path override.\n\
                 pub output_path: Option<String>,\n\
             }}\n\
             \n\
             impl Config{i} {{\n\
                 /// Create a new configuration with default values.\n\
                 pub fn new() -> Self {{\n\
                     Self {{\n\
                         buffer_size: 8192,\n\
                         compress: false,\n\
                         output_path: None,\n\
                     }}\n\
                 }}\n\
                 \n\
                 /// Set the buffer size.\n\
                 pub fn with_buffer_size(mut self, size: usize) -> Self {{\n\
                     self.buffer_size = size;\n\
                     self\n\
                 }}\n\
             }}\n\
             \n\
             /// Error type for processing operations.\n\
             #[derive(Debug)]\n\
             pub enum ProcessError{i} {{\n\
                 /// An I/O error occurred.\n\
                 Io(io::Error),\n\
                 /// The input data was malformed.\n\
                 InvalidInput(String),\n\
                 /// Buffer overflow.\n\
                 Overflow {{ limit: usize, actual: usize }},\n\
             }}\n\
             \n\
             impl std::fmt::Display for ProcessError{i} {{\n\
                 fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{\n\
                     match self {{\n\
                         Self::Io(e) => write!(f, \"I/O error: {{e}}\"),\n\
                         Self::InvalidInput(msg) => write!(f, \"invalid input: {{msg}}\"),\n\
                         Self::Overflow {{ limit, actual }} => {{\n\
                             write!(f, \"buffer overflow: {{actual}} exceeds limit {{limit}}\")\n\
                         }}\n\
                     }}\n\
                 }}\n\
             }}\n\
             \n\
             /// A data processor that transforms input records.\n\
             pub struct Processor{i} {{\n\
                 config: Config{i},\n\
                 cache: HashMap<String, Vec<u8>>,\n\
                 stats: ProcessStats,\n\
             }}\n\
             \n\
             /// Processing statistics.\n\
             #[derive(Debug, Default)]\n\
             struct ProcessStats {{\n\
                 records_processed: usize,\n\
                 bytes_read: usize,\n\
                 bytes_written: usize,\n\
             }}\n\
             \n\
             impl Processor{i} {{\n\
                 /// Create a new processor with the given configuration.\n\
                 pub fn new(config: Config{i}) -> Self {{\n\
                     Self {{\n\
                         config,\n\
                         cache: HashMap::new(),\n\
                         stats: ProcessStats::default(),\n\
                     }}\n\
                 }}\n\
                 \n\
                 /// Process a single record and return the transformed output.\n\
                 pub fn process_record(&mut self, key: &str, data: &[u8]) -> Result<Vec<u8>, ProcessError{i}> {{\n\
                     if data.len() > self.config.buffer_size {{\n\
                         return Err(ProcessError{i}::Overflow {{\n\
                             limit: self.config.buffer_size,\n\
                             actual: data.len(),\n\
                         }});\n\
                     }}\n\
                     \n\
                     let output = data.to_vec();\n\
                     self.cache.insert(key.to_string(), output.clone());\n\
                     self.stats.records_processed += 1;\n\
                     self.stats.bytes_read += data.len();\n\
                     self.stats.bytes_written += output.len();\n\
                     Ok(output)\n\
                 }}\n\
                 \n\
                 /// Return the number of records processed.\n\
                 pub fn records_processed(&self) -> usize {{\n\
                     self.stats.records_processed\n\
                 }}\n\
                 \n\
                 /// Flush the internal cache.\n\
                 pub fn flush(&mut self) {{\n\
                     self.cache.clear();\n\
                 }}\n\
             }}\n\
             \n\
             /// Trait for pluggable transformation strategies.\n\
             pub trait Transform{i} {{\n\
                 /// Transform the input bytes into output bytes.\n\
                 fn transform(&self, input: &[u8]) -> Result<Vec<u8>, ProcessError{i}>;\n\
             }}\n\
             \n\
             #[cfg(test)]\n\
             mod tests {{\n\
                 use super::*;\n\
                 \n\
                 #[test]\n\
                 fn test_default_config() {{\n\
                     let config = Config{i}::new();\n\
                     assert_eq!(config.buffer_size, 8192);\n\
                     assert!(!config.compress);\n\
                 }}\n\
                 \n\
                 #[test]\n\
                 fn test_process_record() {{\n\
                     let config = Config{i}::new();\n\
                     let mut proc = Processor{i}::new(config);\n\
                     let result = proc.process_record(\"key1\", b\"hello\");\n\
                     assert!(result.is_ok());\n\
                     assert_eq!(proc.records_processed(), 1);\n\
                 }}\n\
             }}\n"
        );
        let path = dir.path().join(format!("module_{i:04}.rs"));
        std::fs::write(&path, content).expect("failed to write Rust file");
    }

    dir
}

// ---------------------------------------------------------------------------
// Benchmark groups
// ---------------------------------------------------------------------------

fn bench_ingestion_synthetic(c: &mut Criterion) {
    let rt = Runtime::new().expect("failed to create tokio runtime");
    let mut group = c.benchmark_group("ingestion_synthetic");
    group.sample_size(10);

    for &corpus_size in &[5usize, 20, 50] {
        group.throughput(Throughput::Elements(corpus_size as u64));
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

fn bench_ingestion_with_embeddings(c: &mut Criterion) {
    let rt = Runtime::new().expect("failed to create tokio runtime");
    let mut group = c.benchmark_group("ingestion_with_embeddings");
    group.sample_size(10);

    let embedder = BenchEmbedder { dim: 384 };

    for &corpus_size in &[20usize, 100, 500] {
        group.throughput(Throughput::Elements(corpus_size as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(corpus_size),
            &corpus_size,
            |b, &n| {
                b.iter_with_setup(
                    || {
                        let dir = create_synthetic_corpus(n);
                        let storage =
                            SqliteStorage::open_in_memory().expect("failed to create storage");
                        let index =
                            HnswIndex::new(384, 100_000).expect("failed to create HNSW index");
                        (dir, storage, index)
                    },
                    |(dir, storage, index)| {
                        let pipeline = IngestionPipeline::new();
                        let paths = vec![dir.path().to_path_buf()];
                        rt.block_on(
                            pipeline
                                .ingest_paths_with_embeddings(&paths, &storage, &embedder, &index),
                        )
                        .expect("ingestion with embeddings failed");
                    },
                );
            },
        );
    }

    group.finish();
}

fn bench_ingestion_mixed_corpus(c: &mut Criterion) {
    let rt = Runtime::new().expect("failed to create tokio runtime");
    let mut group = c.benchmark_group("ingestion_mixed_corpus");
    group.sample_size(10);

    let embedder = BenchEmbedder { dim: 384 };

    // (markdown_files, rust_files)
    for &(n_md, n_rs) in &[(10, 10), (25, 75), (50, 450)] {
        let total = n_md + n_rs;
        group.throughput(Throughput::Elements(total as u64));
        group.bench_with_input(
            BenchmarkId::new("md_rs", format!("{n_md}md_{n_rs}rs")),
            &(n_md, n_rs),
            |b, &(n_md, n_rs)| {
                b.iter_with_setup(
                    || {
                        let dir = create_mixed_corpus(n_md, n_rs);
                        let storage =
                            SqliteStorage::open_in_memory().expect("failed to create storage");
                        let index =
                            HnswIndex::new(384, 100_000).expect("failed to create HNSW index");
                        (dir, storage, index)
                    },
                    |(dir, storage, index)| {
                        let pipeline = IngestionPipeline::new();
                        let paths = vec![dir.path().to_path_buf()];
                        rt.block_on(
                            pipeline
                                .ingest_paths_with_embeddings(&paths, &storage, &embedder, &index),
                        )
                        .expect("mixed corpus ingestion failed");
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
    bench_ingestion_with_embeddings,
    bench_ingestion_mixed_corpus,
    bench_ingestion_eval_corpus
);
criterion_main!(benches);
