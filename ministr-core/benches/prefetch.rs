//! Benchmarks for the prefetch cache — insert/lookup throughput and simulated
//! hit rates under different access patterns.
//!
//! Run with: `cargo bench --bench prefetch -p ministr-core`

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ministr_core::session::prefetch::{CacheEntry, PrefetchStrategy, PriorityCache};
use ministr_core::types::Resolution;

/// Create a cache entry with the given ID and strategy.
fn make_entry(id: &str, strategy: PrefetchStrategy) -> CacheEntry {
    CacheEntry {
        content_id: id.to_string(),
        text: format!("Content for {id}. This is filler text to simulate a realistic section."),
        token_count: 20,
        heading_path: Some(vec!["Doc".to_string(), id.to_string()]),
        summary: Some(format!("Summary of {id}.")),
        resolution: Resolution::Section,
        claims_available: 3,
        strategy,
    }
}

fn bench_cache_insert(c: &mut Criterion) {
    let mut group = c.benchmark_group("prefetch_insert");

    for &capacity in &[10, 50, 200] {
        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            &capacity,
            |b, &cap| {
                b.iter_with_setup(
                    || PriorityCache::new(cap),
                    |mut cache| {
                        // Insert 2x capacity to exercise eviction
                        for i in 0..cap * 2 {
                            let id = format!("section::{i}");
                            cache.insert_default(
                                id,
                                make_entry(&format!("s{i}"), PrefetchStrategy::Sequential),
                            );
                        }
                    },
                );
            },
        );
    }

    group.finish();
}

fn bench_cache_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("prefetch_lookup");

    for &capacity in &[10, 50, 200] {
        // Pre-fill the cache
        let mut cache = PriorityCache::new(capacity);
        for i in 0..capacity {
            let id = format!("section::{i}");
            cache.insert_default(
                id,
                make_entry(&format!("s{i}"), PrefetchStrategy::Sequential),
            );
        }

        group.bench_with_input(
            BenchmarkId::from_parameter(capacity),
            &capacity,
            |b, &cap| {
                let mut idx = 0usize;
                b.iter(|| {
                    let key = format!("section::{}", idx % cap);
                    let _ = cache.get(&key);
                    idx = idx.wrapping_add(1);
                });
            },
        );
    }

    group.finish();
}

/// Simulate sequential access pattern: reads sections 0, 1, 2, 3, ...
/// With sequential prefetch, cache should have high hit rate.
fn bench_hit_rate_sequential(c: &mut Criterion) {
    let mut group = c.benchmark_group("prefetch_hitrate");
    let capacity = 50;
    let num_sections = 200;

    group.bench_function("sequential_access", |b| {
        b.iter_with_setup(
            || PriorityCache::new(capacity),
            |mut cache| {
                // Simulate: for each access, the "prefetch engine" inserts the next section
                for i in 0..num_sections {
                    let current = format!("section::{i}");
                    let _ = cache.get(&current);

                    // Simulate prefetch: warm the next section
                    let next = format!("section::{}", i + 1);
                    cache.insert_default(
                        next,
                        make_entry(&format!("s{}", i + 1), PrefetchStrategy::Sequential),
                    );
                }
            },
        );
    });

    group.bench_function("random_access", |b| {
        b.iter_with_setup(
            || {
                let mut cache = PriorityCache::new(capacity);
                // Pre-fill with sequential sections
                for i in 0..capacity {
                    let id = format!("section::{i}");
                    cache.insert_default(
                        id,
                        make_entry(&format!("s{i}"), PrefetchStrategy::Topical),
                    );
                }
                cache
            },
            |mut cache| {
                // Access pattern: pseudo-random (hash-based for determinism)
                for i in 0..num_sections {
                    let key = format!("section::{}", (i * 37 + 13) % (capacity * 3));
                    let _ = cache.get(&key);
                }
            },
        );
    });

    group.bench_function("clustered_access", |b| {
        b.iter_with_setup(
            || PriorityCache::new(capacity),
            |mut cache| {
                // Access pattern: clusters of 5 sequential reads, then jump
                for cluster in 0..(num_sections / 5) {
                    let base = cluster * 10; // Jump by 10, read 5
                    // Simulate prefetch for the cluster
                    for j in 0..5 {
                        let id = format!("section::{}", base + j);
                        cache.insert_default(
                            id,
                            make_entry(&format!("s{}", base + j), PrefetchStrategy::Structural),
                        );
                    }
                    // Now read the cluster
                    for j in 0..5 {
                        let key = format!("section::{}", base + j);
                        let _ = cache.get(&key);
                    }
                }
            },
        );
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_cache_insert,
    bench_cache_lookup,
    bench_hit_rate_sequential
);
criterion_main!(benches);
