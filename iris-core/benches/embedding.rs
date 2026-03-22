//! Benchmarks for embedding throughput (docs/sec).
//!
//! Uses the real `FastEmbedder` with the all-MiniLM-L6-v2 model.
//! Requires the ONNX model to be downloaded (~80MB on first run).
//!
//! Run with: `cargo bench --bench embedding`

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use iris_core::embedding::{Embedder, FastEmbedder};

/// Sample documents of varying lengths for realistic throughput measurement.
const SHORT_DOC: &str = "Authentication uses JWT tokens with RS256 signing.";

const MEDIUM_DOC: &str = "\
    The rate limiting system enforces per-client quotas using a sliding window \
    algorithm. Each client is identified by API key and tracked independently. \
    When a client exceeds their quota, requests receive a 429 status code with \
    a Retry-After header indicating when the window resets. The default quota \
    is 100 requests per minute for standard tier and 1000 for premium tier.";

const LONG_DOC: &str = "\
    The authentication subsystem implements a multi-layered security model \
    combining OAuth 2.0 with PKCE for public clients and client credentials \
    for service-to-service communication. Access tokens are JWTs signed with \
    RS256 and contain claims for user identity, roles, and permissions. \
    Refresh tokens are opaque strings stored in the database with a 30-day \
    expiration. The token introspection endpoint validates tokens and returns \
    active status along with associated metadata. Session management uses \
    secure HTTP-only cookies with SameSite=Strict attribute. CSRF protection \
    is implemented via double-submit cookie pattern. Rate limiting on the \
    login endpoint prevents brute force attacks with exponential backoff \
    after 5 failed attempts. All sensitive operations require re-authentication \
    within a 15-minute window. Audit logging captures all authentication events \
    including IP address, user agent, and geographic location derived from IP.";

fn create_batch(doc: &str, size: usize) -> Vec<&str> {
    vec![doc; size]
}

fn bench_embedding_throughput(c: &mut Criterion) {
    let embedder = FastEmbedder::new("all-MiniLM-L6-v2", None)
        .expect("failed to load embedding model — run `cargo bench --bench embedding` to download");

    let mut group = c.benchmark_group("embedding_throughput");
    group.sample_size(20);

    for &batch_size in &[1, 10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("short", batch_size),
            &batch_size,
            |b, &n| {
                let batch = create_batch(SHORT_DOC, n);
                b.iter(|| embedder.embed(&batch).unwrap());
            },
        );

        group.bench_with_input(
            BenchmarkId::new("medium", batch_size),
            &batch_size,
            |b, &n| {
                let batch = create_batch(MEDIUM_DOC, n);
                b.iter(|| embedder.embed(&batch).unwrap());
            },
        );

        group.bench_with_input(
            BenchmarkId::new("long", batch_size),
            &batch_size,
            |b, &n| {
                let batch = create_batch(LONG_DOC, n);
                b.iter(|| embedder.embed(&batch).unwrap());
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_embedding_throughput);
criterion_main!(benches);
