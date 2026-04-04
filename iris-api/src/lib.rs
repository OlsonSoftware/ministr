//! Shared API types for the iris daemon, MCP proxy, and CLI.
//!
//! This crate defines the request and response types exchanged over the
//! daemon's HTTP API (Unix domain socket). It is the single source of truth
//! for the wire format — used by `iris-app` (daemon), `iris-mcp` (MCP proxy),
//! and `iris-cli` (CLI client).
//!
//! No heavy dependencies here: only serde, schemars, and std.

pub mod client;
pub mod corpus;
pub mod query;
pub mod session;
pub mod status;

/// Well-known Unix domain socket path for the iris daemon.
pub const DAEMON_SOCKET_PATH: &str = "~/.iris/irisd.sock";

/// Resolve the daemon socket path, expanding `~` to the user's home directory.
#[must_use]
pub fn daemon_socket_path() -> std::path::PathBuf {
    if let Some(home) = home_dir() {
        home.join(".iris").join("irisd.sock")
    } else {
        std::path::PathBuf::from("/tmp/irisd.sock")
    }
}

/// Platform-independent home directory lookup.
fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

/// Standard API error response.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ApiError {
    /// Machine-readable error code.
    pub code: String,
    /// Human-readable error message.
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}
