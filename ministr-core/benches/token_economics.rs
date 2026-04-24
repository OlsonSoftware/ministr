//! Token economics benchmarks — compares token consumption of raw file
//! concatenation (grep/cat workflow) against ministr-style targeted retrieval.
//!
//! Measures token counts at various codebase sizes to quantify the savings
//! ministr provides over naive approaches.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use ministr_core::token::count_tokens;

/// A synthetic "source file" of approximately 200 tokens.
const SYNTHETIC_FILE: &str = r#"
//! Authentication middleware for the HTTP server.
//!
//! Validates JWT tokens, manages refresh flows, and enforces
//! role-based access control on protected endpoints.

use std::collections::HashMap;

pub struct AuthService {
    jwt_secret: String,
    token_cache: HashMap<String, Claims>,
    refresh_window: u64,
}

pub struct Claims {
    pub sub: String,
    pub exp: u64,
    pub roles: Vec<String>,
}

impl AuthService {
    pub fn new(secret: String) -> Self {
        Self {
            jwt_secret: secret,
            token_cache: HashMap::new(),
            refresh_window: 300,
        }
    }

    pub fn validate(&self, token: &str) -> Result<Claims, String> {
        if token.is_empty() {
            return Err("empty token".into());
        }
        Ok(Claims {
            sub: "user-1".into(),
            exp: 0,
            roles: vec!["reader".into()],
        })
    }

    pub fn has_role(&self, claims: &Claims, role: &str) -> bool {
        claims.roles.iter().any(|r| r == role)
    }
}
"#;

/// A targeted extract (what ministr would deliver for "how does auth validation work?").
const TARGETED_EXTRACT: &str = r#"
pub fn validate(&self, token: &str) -> Result<Claims, String> {
    if token.is_empty() {
        return Err("empty token".into());
    }
    Ok(Claims { sub: "user-1".into(), exp: 0, roles: vec!["reader".into()] })
}
"#;

/// Build a synthetic codebase of N files (1 relevant + N-1 irrelevant).
fn build_codebase(num_files: usize) -> Vec<String> {
    (0..num_files)
        .map(|i| format!("// File {i}\n{SYNTHETIC_FILE}\n// End file {i}"))
        .collect()
}

fn bench_raw_cat_tokens(c: &mut Criterion) {
    let mut group = c.benchmark_group("raw_cat_tokens");

    for size in [5, 20, 50, 100] {
        let codebase = build_codebase(size);
        let refs: Vec<&str> = codebase.iter().map(String::as_str).collect();

        group.bench_with_input(BenchmarkId::new("files", size), &refs, |b, files| {
            b.iter(|| {
                let total: usize = files.iter().map(|f| count_tokens(f)).sum();
                total
            });
        });
    }

    group.finish();
}

fn bench_ministr_targeted_tokens(c: &mut Criterion) {
    c.bench_function("ministr_targeted_extract", |b| {
        b.iter(|| count_tokens(TARGETED_EXTRACT));
    });
}

#[allow(clippy::cast_precision_loss)]
fn bench_token_savings_ratio(c: &mut Criterion) {
    let mut group = c.benchmark_group("token_savings_ratio");

    for size in [5, 20, 50, 100] {
        let codebase = build_codebase(size);

        group.bench_with_input(BenchmarkId::new("files", size), &codebase, |b, files| {
            b.iter(|| {
                let raw: usize = files.iter().map(|f| count_tokens(f)).sum();
                let ministr = count_tokens(TARGETED_EXTRACT);
                1.0 - (ministr as f64 / raw as f64)
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_raw_cat_tokens,
    bench_ministr_targeted_tokens,
    bench_token_savings_ratio,
);
criterion_main!(benches);
