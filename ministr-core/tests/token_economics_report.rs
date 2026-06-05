//! Illustrative token-economics *scaling model* (synthetic).
//!
//! Deterministic, dependency-free companion to `benches/token_economics.rs`
//! (which measures *latency*, not the savings figures). This test computes
//! token counts for a naive "read every candidate file" workflow versus a
//! single ministr-style targeted extract across growing codebase sizes, prints
//! a Markdown table, and asserts the *shape* (the gap widens with size).
//!
//! It is NOT the published headline figure — that is a real end-to-end
//! measurement in `ministr-mcp/tests/token_economics_e2e.rs` (index a real
//! corpus, run real `ministr_survey` calls, count the literal response bytes).
//! This model just isolates *why* the advantage grows with repo size: a
//! targeted lookup is bounded while "read every candidate" scales with the
//! file count. Treat the percentages as an illustrative upper bound, not a
//! measurement of a real lookup (a real survey returns the top-k slices, ~500
//! tokens here, not a single 68-token extract).
//!
//! Reproduce:  `cargo test -p ministr-core --test token_economics_report -- --nocapture`
//!
//! It is a *synthetic micro-benchmark*: one relevant function in a codebase
//! of otherwise-irrelevant same-shaped files. It isolates the retrieval
//! cost (what crosses the context window), not end-to-end agent quality.

use ministr_core::token::count_tokens;

/// ~200-token synthetic source file (mirrors `benches/token_economics.rs`).
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
        println!("| {size} | {raw} | {targeted} | {:.1}% |", ratio * 100.0);
        // Monotonic: more irrelevant candidates ⇒ a larger share avoided.
        assert!(
            ratio > prev_ratio,
            "reduction must increase with codebase size ({size} files: {ratio} <= {prev_ratio})"
        );
        prev_ratio = ratio;
    }
    println!();

    // Guards the illustrative model's shape (NOT the published headline — that
    // is the real e2e measurement): a single targeted extract is well under
    // 10% of a 100-file naive read. This is an upper bound on the savings, not
    // the real per-lookup cost.
    let ratio_100 = 1.0 - (targeted as f64 / raw_cat_tokens(100) as f64);
    assert!(
        ratio_100 >= 0.90,
        "expected >=90% reduction at 100 files (illustrative model), got {:.1}%",
        ratio_100 * 100.0
    );

    // Token counting is deterministic — same inputs, same numbers, every run.
    assert_eq!(raw_cat_tokens(20), raw_cat_tokens(20));
}
