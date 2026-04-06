//! Token economics baseline — measures tokens consumed by raw file
//! concatenation vs iris-style targeted retrieval.
//!
//! Demonstrates that semantic search + targeted section delivery uses
//! fewer tokens than naive grep/cat workflows for equivalent information
//! retrieval tasks.

use iris_core::token::count_tokens;

/// Simulate a "raw cat" workflow: concatenate entire files to answer a query.
///
/// Returns the total token count of all concatenated file contents.
fn raw_cat_tokens(files: &[&str]) -> usize {
    files.iter().map(|f| count_tokens(f)).sum()
}

/// Simulate an iris-style workflow: deliver only relevant sections/claims.
///
/// Returns the total token count of targeted extracts.
fn iris_targeted_tokens(extracts: &[&str]) -> usize {
    extracts.iter().map(|e| count_tokens(e)).sum()
}

// Synthetic "files" representing a small codebase.
const FILE_A: &str = r#"
//! Module A — session management.
//!
//! This module handles user session lifecycle including creation,
//! validation, expiry, and cleanup of session state.

use std::collections::HashMap;
use std::time::{Duration, Instant};

pub struct SessionManager {
    sessions: HashMap<String, Session>,
    timeout: Duration,
}

pub struct Session {
    id: String,
    created_at: Instant,
    last_access: Instant,
    data: HashMap<String, String>,
}

impl SessionManager {
    pub fn new(timeout: Duration) -> Self {
        Self {
            sessions: HashMap::new(),
            timeout,
        }
    }

    pub fn create_session(&mut self) -> String {
        let id = format!("session-{}", self.sessions.len());
        let now = Instant::now();
        self.sessions.insert(id.clone(), Session {
            id: id.clone(),
            created_at: now,
            last_access: now,
            data: HashMap::new(),
        });
        id
    }

    pub fn get_session(&self, id: &str) -> Option<&Session> {
        self.sessions.get(id)
    }

    pub fn cleanup_expired(&mut self) {
        let timeout = self.timeout;
        self.sessions.retain(|_, s| s.last_access.elapsed() < timeout);
    }
}
"#;

const FILE_B: &str = r#"
//! Module B — authentication middleware.
//!
//! Validates JWT tokens, checks permissions, and manages OAuth flows.
//! Not related to session management directly.

use std::collections::HashSet;

pub struct AuthMiddleware {
    allowed_origins: HashSet<String>,
    jwt_secret: String,
}

pub struct JwtClaims {
    sub: String,
    exp: u64,
    roles: Vec<String>,
}

impl AuthMiddleware {
    pub fn new(secret: String) -> Self {
        Self {
            allowed_origins: HashSet::new(),
            jwt_secret: secret,
        }
    }

    pub fn validate_token(&self, token: &str) -> Result<JwtClaims, String> {
        // JWT validation logic would go here
        if token.is_empty() {
            return Err("empty token".into());
        }
        Ok(JwtClaims {
            sub: "user".into(),
            exp: 0,
            roles: vec!["reader".into()],
        })
    }

    pub fn check_permission(&self, claims: &JwtClaims, required: &str) -> bool {
        claims.roles.iter().any(|r| r == required)
    }
}
"#;

const FILE_C: &str = r#"
//! Module C — configuration loading.
//!
//! Reads TOML config files, environment variables, and CLI flags.
//! Completely unrelated to session management.

use std::path::PathBuf;
use std::collections::HashMap;

pub struct Config {
    pub data_dir: PathBuf,
    pub log_level: String,
    pub port: u16,
    pub features: HashMap<String, bool>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("./data"),
            log_level: "info".into(),
            port: 8080,
            features: HashMap::new(),
        }
    }
}

impl Config {
    pub fn from_file(path: &str) -> Result<Self, String> {
        // TOML parsing would go here
        let _ = path;
        Ok(Self::default())
    }

    pub fn merge_env(&mut self) {
        if let Ok(level) = std::env::var("LOG_LEVEL") {
            self.log_level = level;
        }
        if let Ok(port) = std::env::var("PORT") {
            if let Ok(p) = port.parse() {
                self.port = p;
            }
        }
    }
}
"#;

/// The relevant extract that answers "how does session cleanup work?"
const IRIS_EXTRACT: &str = r"
pub fn cleanup_expired(&mut self) {
    let timeout = self.timeout;
    self.sessions.retain(|_, s| s.last_access.elapsed() < timeout);
}
";

/// Section-level context iris would deliver (the `SessionManager` struct + impl).
const IRIS_SECTION: &str = r"
pub struct SessionManager {
    sessions: HashMap<String, Session>,
    timeout: Duration,
}

impl SessionManager {
    pub fn new(timeout: Duration) -> Self { ... }
    pub fn create_session(&mut self) -> String { ... }
    pub fn get_session(&self, id: &str) -> Option<&Session> { ... }
    pub fn cleanup_expired(&mut self) { ... }
}
";

#[test]
#[allow(clippy::cast_precision_loss)]
fn iris_uses_fewer_tokens_than_raw_cat_for_targeted_query() {
    // Query: "how does session cleanup work?"
    //
    // Raw workflow: cat all 3 files to find the answer
    let raw_tokens = raw_cat_tokens(&[FILE_A, FILE_B, FILE_C]);

    // iris workflow: survey returns FILE_A section, extract returns cleanup fn
    let iris_tokens = iris_targeted_tokens(&[IRIS_SECTION, IRIS_EXTRACT]);

    // iris should use significantly fewer tokens
    let savings_pct = 1.0 - (iris_tokens as f64 / raw_tokens as f64);

    assert!(
        iris_tokens < raw_tokens,
        "iris should use fewer tokens: iris={iris_tokens}, raw={raw_tokens}"
    );
    assert!(
        savings_pct > 0.5,
        "iris should save at least 50% of tokens: savings={savings_pct:.1}%"
    );
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn raw_cat_includes_irrelevant_content() {
    let total_raw = raw_cat_tokens(&[FILE_A, FILE_B, FILE_C]);
    let relevant_only = raw_cat_tokens(&[FILE_A]);

    // 2 of 3 files are completely irrelevant to session questions
    let irrelevant_fraction = 1.0 - (relevant_only as f64 / total_raw as f64);

    assert!(
        irrelevant_fraction > 0.5,
        "more than half of raw content is irrelevant: {irrelevant_fraction:.1}"
    );
}

#[test]
#[allow(clippy::cast_precision_loss)]
fn token_savings_scale_with_codebase_size() {
    // Simulate a larger codebase: 10 files, only 1 relevant
    let irrelevant_files: Vec<String> = (0..9)
        .map(|i| format!("// Module {i}\n{FILE_B}\n// End module {i}"))
        .collect();
    let all_files: Vec<&str> = std::iter::once(FILE_A)
        .chain(irrelevant_files.iter().map(String::as_str))
        .collect();

    let raw_tokens = raw_cat_tokens(&all_files);
    let iris_tokens = iris_targeted_tokens(&[IRIS_SECTION, IRIS_EXTRACT]);

    let savings_pct = 1.0 - (iris_tokens as f64 / raw_tokens as f64);

    assert!(
        savings_pct > 0.90,
        "with 10 files, iris should save >90% tokens: savings={savings_pct:.2}"
    );
}
