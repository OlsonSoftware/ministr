//! F36 — store-seam benchmark: current in-memory HNSW vs an mmap-backed
//! candidate (usearch), driven through the D4 `IndexedVectorStore` seam.
//!
//! This is NOT a criterion statistical bench: it is a single deterministic
//! targeted run (seeded RNG, fixed corpus) that measures the five decision
//! axes for the adopt/keep/revisit call:
//!
//!   - ingest/build throughput (vectors → searchable index, incl. persist)
//!   - query latency (p50/p95 over 100 queries, k=10)
//!   - recall@10 vs brute-force cosine ground truth
//!   - startup time (persisted artifacts → searchable index)
//!   - peak RSS, measured in a SEPARATE process per store+phase so numbers
//!     don't conflate (the parent re-execs itself into child roles)
//!
//! The HNSW side goes through the real seam: a `BenchStore` implementing
//! [`IndexedVectorStore`] drives `load_cached_or_rebuild_hnsw` for both the
//! rebuild+persist path (build) and the token-validated cache-hit path
//! (serve/startup). Production code paths are exercised, never modified.
//!
//! Both serve children hold the same dataset + ground-truth arrays, so the
//! peak-RSS DIFFERENCE between them isolates index memory.
//!
//! Run: `cargo bench -p ministr-core --bench store_seam`

// Numeric casts between counters, durations, and reported floats are inherent
// to a measurement harness; precision far exceeds what the metrics need.
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use std::path::Path;
use std::process::Command;
use std::time::Instant;

use ministr_core::error::StorageError;
use ministr_core::index::{
    IndexedVectorStore, VectorFingerprint, VectorIndex, load_cached_or_rebuild_hnsw,
};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use usearch::{IndexOptions, MetricKind, ScalarKind, new_index};

const N: usize = 20_000;
const DIM: usize = 384;
const CLUSTERS: usize = 200;
const QUERIES: usize = 100;
const K: usize = 10;
const SEED: u64 = 0x00C0_FFEE;
const MODEL: &str = "bench-model";

const ROLE_ENV: &str = "STORE_SEAM_ROLE";
const DIR_ENV: &str = "STORE_SEAM_DIR";

fn main() {
    match std::env::var(ROLE_ENV).ok().as_deref() {
        None => orchestrate(),
        Some("hnsw-build") => hnsw_build(),
        Some("hnsw-serve") => hnsw_serve(),
        Some("usearch-build") => usearch_build(),
        Some("usearch-serve") => usearch_serve(),
        Some(other) => panic!("unknown {ROLE_ENV}: {other}"),
    }
}

// ---------------------------------------------------------------- dataset

/// Deterministic clustered corpus: unit vectors around `CLUSTERS` centers.
///
/// Clustered (not uniform-random) so recall@10 is a meaningful measurement —
/// uniform random vectors at this dimension make every neighbor equidistant
/// and any index looks perfect.
fn corpus() -> Vec<Vec<f32>> {
    let mut rng = StdRng::seed_from_u64(SEED);
    let centers: Vec<Vec<f32>> = (0..CLUSTERS).map(|_| unit_vector(&mut rng)).collect();
    (0..N)
        .map(|i| {
            let center = &centers[i % CLUSTERS];
            let mut v: Vec<f32> = center
                .iter()
                .map(|c| c + 0.15 * rng.gen_range(-0.5..0.5))
                .collect();
            normalize(&mut v);
            v
        })
        .collect()
}

/// Queries are perturbations of corpus vectors — near-duplicate lookups,
/// the realistic retrieval shape.
fn queries(corpus: &[Vec<f32>]) -> Vec<Vec<f32>> {
    let mut rng = StdRng::seed_from_u64(SEED ^ 0x5EED);
    (0..QUERIES)
        .map(|q| {
            let base = &corpus[(q * (N / QUERIES)) % N];
            let mut v: Vec<f32> = base
                .iter()
                .map(|c| c + 0.10 * rng.gen_range(-0.5..0.5))
                .collect();
            normalize(&mut v);
            v
        })
        .collect()
}

/// Brute-force cosine top-K ground truth (corpus indices).
fn ground_truth(corpus: &[Vec<f32>], queries: &[Vec<f32>]) -> Vec<Vec<usize>> {
    queries
        .iter()
        .map(|q| {
            let mut scored: Vec<(usize, f32)> = corpus
                .iter()
                .enumerate()
                .map(|(i, v)| (i, dot(q, v)))
                .collect();
            scored.sort_by(|a, b| b.1.total_cmp(&a.1));
            scored.truncate(K);
            scored.into_iter().map(|(i, _)| i).collect()
        })
        .collect()
}

fn unit_vector(rng: &mut StdRng) -> Vec<f32> {
    let mut v: Vec<f32> = (0..DIM).map(|_| rng.gen_range(-1.0..1.0)).collect();
    normalize(&mut v);
    v
}

fn normalize(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v {
            *x /= norm;
        }
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

// ------------------------------------------------------------- seam store

/// In-memory [`IndexedVectorStore`] with a stable fingerprint, so the build
/// child's persisted cache token validates in the serve child (cache hit).
struct BenchStore {
    data: Vec<(String, Vec<f32>)>,
}

impl BenchStore {
    fn new(corpus: &[Vec<f32>]) -> Self {
        Self {
            data: corpus
                .iter()
                .enumerate()
                .map(|(i, v)| (format!("v{i}"), v.clone()))
                .collect(),
        }
    }
}

impl IndexedVectorStore for BenchStore {
    async fn list_indexed_vectors(&self) -> Result<Vec<(String, Vec<f32>)>, StorageError> {
        Ok(self.data.clone())
    }

    async fn indexed_vector_fingerprint(&self) -> Result<Option<VectorFingerprint>, StorageError> {
        Ok(Some(VectorFingerprint {
            count: self.data.len() as u64,
            generation: 1,
        }))
    }
}

// ------------------------------------------------------------ child roles

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

fn hnsw_dir() -> std::path::PathBuf {
    Path::new(&std::env::var(DIR_ENV).expect(DIR_ENV)).join("hnsw")
}

fn usearch_path() -> String {
    Path::new(&std::env::var(DIR_ENV).expect(DIR_ENV))
        .join("usearch.bin")
        .to_string_lossy()
        .into_owned()
}

/// Build child: rebuild-from-store + persist through the REAL seam function.
fn hnsw_build() {
    let store = BenchStore::new(&corpus());
    let dir = hnsw_dir();
    let t = Instant::now();
    let index = rt()
        .block_on(load_cached_or_rebuild_hnsw(
            &store,
            &dir,
            DIM,
            Some(MODEL),
            None,
        ))
        .expect("rebuild")
        .expect("non-empty corpus");
    let build_s = t.elapsed().as_secs_f64();
    assert_eq!(index.len(), N);
    metric("hnsw", "build_s", build_s);
    metric("hnsw", "ingest_vps", N as f64 / build_s);
    metric("hnsw", "build_peak_rss_mb", peak_rss_mb());
}

/// Serve child: token-validated cache-hit load (the real restart path).
fn hnsw_serve() {
    let corpus = corpus();
    let queries = queries(&corpus);
    let gt = ground_truth(&corpus, &queries);
    let store = BenchStore::new(&corpus);
    let dir = hnsw_dir();

    let t = Instant::now();
    let index = rt()
        .block_on(load_cached_or_rebuild_hnsw(
            &store,
            &dir,
            DIM,
            Some(MODEL),
            None,
        ))
        .expect("load")
        .expect("cache present");
    let startup_s = t.elapsed().as_secs_f64();
    assert_eq!(index.len(), N, "serve child must hit the persisted cache");

    let mut latencies = Vec::with_capacity(QUERIES);
    let mut hits = 0usize;
    for (q, truth) in queries.iter().zip(&gt) {
        let t = Instant::now();
        let results = index.search_knn(q, K).expect("search");
        latencies.push(t.elapsed().as_secs_f64() * 1e6);
        hits += results
            .iter()
            .filter_map(|r| r.id.strip_prefix('v').and_then(|s| s.parse::<usize>().ok()))
            .filter(|i| truth.contains(i))
            .count();
    }
    report_serve("hnsw", startup_s, &mut latencies, hits);
}

fn usearch_options() -> IndexOptions {
    IndexOptions {
        dimensions: DIM,
        metric: MetricKind::Cos,
        quantization: ScalarKind::F32,
        ..Default::default()
    }
}

/// Build child: add-loop + save (usearch has no rebuild seam — its ingest IS
/// the add loop, which is what the seam's rebuild would drive).
fn usearch_build() {
    let corpus = corpus();
    let index = new_index(&usearch_options()).expect("usearch index");
    index.reserve(N).expect("reserve");
    let t = Instant::now();
    for (i, v) in corpus.iter().enumerate() {
        index.add(i as u64, v).expect("add");
    }
    index.save(&usearch_path()).expect("save");
    let build_s = t.elapsed().as_secs_f64();
    metric("usearch", "build_s", build_s);
    metric("usearch", "ingest_vps", N as f64 / build_s);
    metric("usearch", "build_peak_rss_mb", peak_rss_mb());
}

/// Serve child: mmap `view` — the whole point of the candidate.
fn usearch_serve() {
    let corpus = corpus();
    let queries = queries(&corpus);
    let gt = ground_truth(&corpus, &queries);

    let index = new_index(&usearch_options()).expect("usearch index");
    let t = Instant::now();
    index.view(&usearch_path()).expect("view");
    let startup_s = t.elapsed().as_secs_f64();
    assert_eq!(index.size(), N);

    let mut latencies = Vec::with_capacity(QUERIES);
    let mut hits = 0usize;
    for (q, truth) in queries.iter().zip(&gt) {
        let t = Instant::now();
        let matches = index.search(q, K).expect("search");
        latencies.push(t.elapsed().as_secs_f64() * 1e6);
        hits += matches
            .keys
            .iter()
            .filter(|&&k| truth.contains(&(k as usize)))
            .count();
    }
    report_serve("usearch", startup_s, &mut latencies, hits);
}

// ------------------------------------------------------------- reporting

fn metric(store: &str, key: &str, value: f64) {
    println!("METRIC {store} {key} {value:.6}");
}

fn report_serve(store: &str, startup_s: f64, latencies_us: &mut [f64], hits: usize) {
    latencies_us.sort_by(f64::total_cmp);
    let p = |q: f64| latencies_us[((latencies_us.len() as f64 - 1.0) * q) as usize];
    metric(store, "startup_s", startup_s);
    metric(store, "query_p50_us", p(0.50));
    metric(store, "query_p95_us", p(0.95));
    metric(store, "recall_at_10", hits as f64 / (QUERIES * K) as f64);
    metric(store, "serve_peak_rss_mb", peak_rss_mb());
}

/// Peak RSS of THIS process, in MiB (`ru_maxrss`: bytes on macOS, KiB on Linux).
///
/// Peak RSS is only readable via the getrusage syscall — bench-only exception
/// to the workspace unsafe-code deny.
#[allow(unsafe_code)]
fn peak_rss_mb() -> f64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) };
    assert_eq!(rc, 0, "getrusage failed");
    let max = usage.ru_maxrss as f64;
    if cfg!(target_os = "macos") {
        max / (1024.0 * 1024.0)
    } else {
        max / 1024.0
    }
}

// ----------------------------------------------------------- orchestrator

fn orchestrate() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let exe = std::env::current_exe().expect("current_exe");
    let mut metrics: Vec<(String, String, f64)> = Vec::new();

    for role in ["hnsw-build", "hnsw-serve", "usearch-build", "usearch-serve"] {
        eprintln!("── running {role} …");
        let out = Command::new(&exe)
            .env(ROLE_ENV, role)
            .env(DIR_ENV, tmp.path())
            .output()
            .expect("spawn child");
        assert!(
            out.status.success(),
            "{role} failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let mut parts = line.split_whitespace();
            if parts.next() == Some("METRIC") {
                let store = parts.next().expect("store").to_owned();
                let key = parts.next().expect("key").to_owned();
                let value: f64 = parts.next().expect("value").parse().expect("f64");
                metrics.push((store, key, value));
            }
        }
    }

    let get = |store: &str, key: &str| -> f64 {
        metrics
            .iter()
            .find(|(s, k, _)| s == store && k == key)
            .map_or(f64::NAN, |(_, _, v)| *v)
    };

    println!();
    println!(
        "store-seam benchmark — N={N} dim={DIM} clusters={CLUSTERS} queries={QUERIES} k={K} (seed {SEED:#x})"
    );
    println!();
    println!(
        "{:<26} {:>14} {:>14}",
        "metric", "hnsw (current)", "usearch (mmap)"
    );
    for (label, key) in [
        ("build+persist (s)", "build_s"),
        ("ingest (vectors/s)", "ingest_vps"),
        ("build peak RSS (MiB)", "build_peak_rss_mb"),
        ("startup (s)", "startup_s"),
        ("query p50 (µs)", "query_p50_us"),
        ("query p95 (µs)", "query_p95_us"),
        ("recall@10", "recall_at_10"),
        ("serve peak RSS (MiB)", "serve_peak_rss_mb"),
    ] {
        println!(
            "{:<26} {:>14.3} {:>14.3}",
            label,
            get("hnsw", key),
            get("usearch", key)
        );
    }
    println!();
    println!("notes:");
    println!("  - each column is measured in its own process; serve children hold identical");
    println!(
        "    dataset + ground-truth arrays, so the serve-RSS DIFFERENCE isolates index memory"
    );
    println!("  - hnsw path = real seam (load_cached_or_rebuild_hnsw: rebuild+persist, then");
    println!("    token-validated cache-hit load); usearch path = add/save then mmap view()");
    println!("  - both stores run their as-shipped default graph parameters");
}
