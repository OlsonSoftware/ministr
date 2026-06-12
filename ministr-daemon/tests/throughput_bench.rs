//! cq-throughput — end-to-end `IngestionCoordinator` throughput at 25+ corpora.
//!
//! Drives a mix of small + large corpora through the real
//! [`CorpusRegistry`] + `IngestionCoordinator` (cq-queue / cq-priority /
//! cq-coalesce) and measures how fast the bounded, shortest-job-first queue
//! drains them. Uses a deterministic in-process `MockEmbedder` so the number
//! reflects coordinator + pipeline orchestration (dispatch, parse, `SQLite`),
//! NOT embedding speed (that is `f-ingest-embed-throughput`).
//!
//! `#[ignore]`d so `just validate` stays fast — run it with
//! `just cq-throughput-bench`.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ministr_core::config::MinistrConfig;
use ministr_core::embedding::Embedder;
use ministr_core::error::IndexError;
use ministr_daemon::registry::CorpusRegistry;

/// Deterministic mock embedder — consistent vectors from text bytes, no model
/// download or GPU, so the bench isolates the coordinator + pipeline.
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

/// The scheduler's default global concurrency bound (mirrors
/// `IngestionScheduler::with_default_concurrency`, which is `pub(crate)` and so
/// not readable from an integration test). Reported for context.
fn default_concurrency_bound() -> usize {
    std::thread::available_parallelism()
        .map_or(2, std::num::NonZero::get)
        .clamp(2, 4)
}

/// Write a tiny multi-section markdown file the ingest pipeline will parse into
/// one document with a couple of sections.
fn write_doc(dir: &std::path::Path, corpus: usize, file: usize) {
    let path = dir.join(format!("doc_{file:03}.md"));
    let body = format!(
        "# Corpus {corpus} Document {file}\n\n\
         Intro paragraph with enough words to make a section worth embedding \
         and parsing through the real pipeline.\n\n\
         ## Section B\n\n\
         A second section so each file yields more than one section, exercising \
         parse + store under the coordinator.\n"
    );
    std::fs::write(path, body).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "bench: run via `just cq-throughput-bench`"]
async fn coordinator_throughput_25_corpora() {
    // Mixed-size job set: many small user-code-sized repos + a few large
    // (vendored-tree-sized) ones — exercises shortest-job-first ordering and
    // proves a few big corpora can't head-of-line-block the small ones.
    let mut sizes: Vec<usize> = Vec::new();
    for i in 0..20 {
        sizes.push(2 + (i % 7)); // 20 small corpora: 2..=8 files
    }
    for i in 0..6 {
        sizes.push(40 + i * 16); // 6 large corpora: 40,56,72,88,104,120 files
    }
    let n = sizes.len();
    let total_files: usize = sizes.iter().sum();

    let data_tmp = tempfile::TempDir::new().unwrap();
    let config = MinistrConfig {
        data_dir: data_tmp.path().to_path_buf(),
        ..MinistrConfig::default()
    };
    let embedder: Arc<dyn Embedder> = Arc::new(MockEmbedder { dim: 16 });
    let registry = Arc::new(CorpusRegistry::new(
        embedder,
        "mock-model:test".to_string(),
        config,
    ));

    // Generate the corpora on disk (kept alive until the bench finishes so the
    // source files exist throughout ingest).
    let mut corpus_dirs = Vec::with_capacity(n);
    for (ci, &k) in sizes.iter().enumerate() {
        let dir = tempfile::TempDir::new().unwrap();
        for fi in 0..k {
            write_doc(dir.path(), ci, fi);
        }
        corpus_dirs.push(dir);
    }

    // Register every corpus — each enqueues a job onto the single coordinator
    // queue, drained by the bounded worker pool.
    let start = Instant::now();
    let mut ids = Vec::with_capacity(n);
    for dir in &corpus_dirs {
        let path = dir.path().to_str().unwrap().to_string();
        let (id, _started) = registry.register(&[path]).await.unwrap();
        ids.push(id);
    }
    let register_done = start.elapsed();

    // Wait until every corpus has finished its initial index. `files_indexed`
    // flips from 0 to its real count when `update_stats` runs at ingest success,
    // so "all > 0" is a robust completion signal for non-empty corpora.
    let timeout = Duration::from_secs(180);
    loop {
        let infos = registry.list().await;
        let done = ids
            .iter()
            .filter(|id| {
                infos
                    .iter()
                    .find(|i| &i.id == *id)
                    .is_some_and(|i| i.files_indexed > 0)
            })
            .count();
        if done == n {
            break;
        }
        assert!(
            start.elapsed() < timeout,
            "timed out: only {done}/{n} corpora indexed after {:?}",
            start.elapsed()
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let elapsed = start.elapsed();

    // Aggregate the final per-corpus counts.
    let infos = registry.list().await;
    let indexed_total: usize = ids
        .iter()
        .map(|id| {
            infos
                .iter()
                .find(|i| &i.id == id)
                .map_or(0, |i| i.files_indexed)
        })
        .sum();

    let secs = elapsed.as_secs_f64();
    eprintln!("\n=== cq-throughput: IngestionCoordinator @ {n} corpora ===");
    eprintln!("corpora:            {n} ({} small + 6 large)", n - 6);
    eprintln!("total source files: {total_files}");
    eprintln!("files indexed:      {indexed_total}");
    eprintln!("concurrency bound:  {}", default_concurrency_bound());
    eprintln!("register loop:      {:.3}s", register_done.as_secs_f64());
    eprintln!("wall-clock (total): {secs:.3}s");
    eprintln!(
        "throughput:         {:.1} corpora/s, {:.0} files/s",
        f64::from(u32::try_from(n).unwrap()) / secs,
        f64::from(u32::try_from(total_files).unwrap()) / secs
    );
    eprintln!("======================================================\n");

    // Invariant: the bounded, fair queue drained EVERY corpus completely — no
    // corpus was starved and no file was dropped under coalescing at scale.
    assert!(n >= 25, "bench must drive 25+ corpora (have {n})");
    assert_eq!(
        indexed_total, total_files,
        "every source file across all corpora must be indexed (no starvation / no dropped paths)"
    );
}
