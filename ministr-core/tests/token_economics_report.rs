//! Reproducible token-economics report.
//!
//! Deterministic, dependency-free companion to `benches/token_economics.rs`
//! (which measures *latency*, not the savings figures). This test computes
//! the actual token counts for a naive "read every candidate file" workflow
//! versus a ministr-style targeted extract, prints a Markdown table, and
//! asserts the published claim so it can't silently drift.
//!
//! Reproduce:  `cargo test -p ministr-core --test token_economics_report -- --nocapture`
//!
//! It is a *synthetic micro-benchmark*: one relevant function in a codebase
//! of otherwise-irrelevant same-shaped files. It isolates the retrieval
//! cost (what crosses the context window), not end-to-end agent quality.

use ministr_core::token::count_tokens;

/// ~200-token synthetic source file (mirrors benches/token_economics.rs).
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
        Self { jwt_secret: secret, token_cache: HashMap::new(), refresh_window: 300 }
    }

    pub fn validate(&self, token: &str) -> Result<Claims, String> {
        if token.is_empty() {
            return Err("empty token".into());
        }
        Ok(Claims { sub: "user-1".into(), exp: 0, roles: vec!["reader".into()] })
    }

    pub fn has_role(&self, claims: &Claims, role: &str) -> bool {
        claims.roles.iter().any(|r| r == role)
    }
}
"#;

/// What ministr delivers for "how does auth validation work?" — the one
/// function that answers the question, not the file it lives in.
const TARGETED_EXTRACT: &str = r#"
pub fn validate(&self, token: &str) -> Result<Claims, String> {
    if token.is_empty() {
        return Err("empty token".into());
    }
    Ok(Claims { sub: "user-1".into(), exp: 0, roles: vec!["reader".into()] })
}
"#;

fn raw_cat_tokens(num_files: usize) -> usize {
    (0..num_files)
        .map(|i| count_tokens(&format!("// File {i}\n{SYNTHETIC_FILE}\n// End file {i}")))
        .sum()
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn token_economics_report() {
    let targeted = count_tokens(TARGETED_EXTRACT);
    let sizes = [5usize, 20, 50, 100];

    println!("\n| candidate files | grep+read tokens | ministr tokens | reduction |");
    println!("|---|---|---|---|");
    let mut prev_ratio = 0.0_f64;
    for size in sizes {
        let raw = raw_cat_tokens(size);
        let ratio = 1.0 - (targeted as f64 / raw as f64);
        println!(
            "| {size} | {raw} | {targeted} | {:.1}% |",
            ratio * 100.0
        );
        // Monotonic: more irrelevant candidates ⇒ a larger share avoided.
        assert!(
            ratio > prev_ratio,
            "reduction must increase with codebase size ({size} files: {ratio} <= {prev_ratio})"
        );
        prev_ratio = ratio;
    }
    println!();

    // Guards the published headline figure: at 100 candidate files the
    // targeted extract is well under 10% of the naive read.
    let ratio_100 = 1.0 - (targeted as f64 / raw_cat_tokens(100) as f64);
    assert!(
        ratio_100 >= 0.90,
        "expected >=90% reduction at 100 files, got {:.1}%",
        ratio_100 * 100.0
    );

    // Token counting is deterministic — same inputs, same numbers, every run.
    assert_eq!(raw_cat_tokens(20), raw_cat_tokens(20));
}
