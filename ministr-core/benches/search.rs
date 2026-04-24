//! Benchmarks for HNSW vector index insert throughput and search latency.
//!
//! Tests performance at 1k, 10k, and 100k index sizes using synthetic random
//! vectors. These benchmarks do NOT require a model download and run quickly.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ministr_core::index::{HnswIndex, VectorIndex};
use rand::Rng;

const DIMENSION: usize = 384;
const SEARCH_K: usize = 10;

/// Generate a random unit vector of the given dimension.
fn random_vector(rng: &mut impl Rng, dim: usize) -> Vec<f32> {
    let mut v: Vec<f32> = (0..dim).map(|_| rng.gen_range(-1.0..1.0)).collect();
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

/// Build an HNSW index populated with `n` random vectors.
fn build_index(n: usize) -> HnswIndex {
    let mut rng = rand::thread_rng();
    let index = HnswIndex::new(DIMENSION, n + 1000).unwrap();
    for i in 0..n {
        let v = random_vector(&mut rng, DIMENSION);
        index.insert(&format!("section::{i}"), &v).unwrap();
    }
    index
}

fn bench_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_insert");

    for &size in &[1_000, 10_000] {
        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, &n| {
            b.iter_with_setup(
                || {
                    let mut rng = rand::thread_rng();
                    let vectors: Vec<Vec<f32>> =
                        (0..n).map(|_| random_vector(&mut rng, DIMENSION)).collect();
                    (HnswIndex::new(DIMENSION, n + 1000).unwrap(), vectors)
                },
                |(index, vectors)| {
                    for (i, v) in vectors.iter().enumerate() {
                        index.insert(&format!("section::{i}"), v).unwrap();
                    }
                },
            );
        });
    }

    group.finish();
}

fn bench_search_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_search");
    group.sample_size(100);

    for &size in &[1_000, 10_000, 100_000] {
        let index = build_index(size);
        let mut rng = rand::thread_rng();

        group.bench_with_input(BenchmarkId::from_parameter(size), &size, |b, _| {
            b.iter_with_setup(
                || random_vector(&mut rng, DIMENSION),
                |query| {
                    index.search_knn(&query, SEARCH_K).unwrap();
                },
            );
        });
    }

    group.finish();
}

fn bench_search_varying_k(c: &mut Criterion) {
    let mut group = c.benchmark_group("hnsw_search_k");
    let index = build_index(10_000);
    let mut rng = rand::thread_rng();

    for &k in &[1, 5, 10, 30, 50] {
        group.bench_with_input(BenchmarkId::from_parameter(k), &k, |b, &k| {
            b.iter_with_setup(
                || random_vector(&mut rng, DIMENSION),
                |query| {
                    index.search_knn(&query, k).unwrap();
                },
            );
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_insert,
    bench_search_latency,
    bench_search_varying_k
);
criterion_main!(benches);
