//! Shared API types for the ministr daemon, MCP proxy, and CLI.
//!
//! This crate defines the request and response types exchanged over the
//! daemon's HTTP API. It is the single source of truth for the wire format —
//! used by `ministr-app` (daemon), `ministr-mcp` (MCP proxy), and
//! `ministr-cli` (CLI client).
//!
//! Transport is platform-native IPC: Unix domain sockets on macOS/Linux,
//! Windows named pipes on Windows. See [`transport`] for the connect side.
//!
//! No heavy dependencies here: only serde, schemars, tokio, and std.

pub mod activity;
pub mod api_key;
pub mod audit;
pub mod blob_sink;
pub mod client;
pub mod coherence;
pub mod corpora_repo;
pub mod corpus;
pub mod corpus_restorer;
pub mod drops_ledger;
pub mod github_app;
pub mod index_job_sink;
pub mod mail;
pub mod plan_resolver;
pub mod query;
pub mod session;
pub mod session_bundle_store;
pub mod session_storage;
pub mod sla_window_store;
pub mod status;
pub mod tenant;
pub mod tenant_filter;
pub mod transport;
pub mod usage;

pub use api_key::{
    ApiKeyError, ApiKeyResolver, ResolveApiKeyFuture, ResolvedApiKey, TouchLastUsedFuture,
};
pub use audit::{AuditEntry, AuditSink};
pub use blob_sink::BlobSink;
pub use drops_ledger::{
    AppendDropFuture, DropEntry, DropsLedger, DropsLedgerError, ListDropsFuture,
};
pub use corpora_repo::{CorporaRepo, CorporaRepoError, CorpusRegistration, RepoFuture};
pub use corpus_restorer::{CorpusRestoreError, CorpusRestorer, RestoreFuture};
pub use github_app::{InstallationTokenMinter, MintError};
pub use index_job_sink::{
    IndexJobError, IndexJobFuture, IndexJobSink, IndexJobSnapshot, IndexJobStatus,
};
pub use mail::{InviteMessage, MailSender};
pub use plan_resolver::{PlanResolver, PlanResolverError, ResolvePlanFuture};
pub use sla_window_store::{MaxP95Future, SlaWindowStore, SlaWindowStoreError};
pub use session_bundle_store::{
    PutAndSignFuture, SessionBundleStore, SessionBundleStoreError, SignedBundleUrl,
    VerifyAndGetFuture,
};
pub use session_storage::{
    LoadSessionFuture, SaveSessionFuture, SessionMutFuture, SessionSnapshot, SessionStorage,
    SessionStorageError,
};
pub use tenant::TenantId;
pub use tenant_filter::{
    CorpusRegistrationView, DefaultCorpusFuture, PendingCorporaFuture, TenantCorpusFilter,
    TenantCorpusVisibility, TenantFilterError, TenantFilterFuture, VisibleCorpusFuture,
};
pub use usage::UsageSink;

/// Canonical data directory for the ministr daemon (`~/.ministr` on Unix,
/// `%USERPROFILE%\.ministr` on Windows).
///
/// This is the single source of truth for where daemon-owned state lives:
/// the socket / pipe sentinel, PID file, log file, onboarding markers,
/// per-corpus index data.
///
/// Falls back to the system temp dir when no home can be resolved — this
/// should only happen on minimal/headless environments with no `HOME` or
/// `USERPROFILE` set.
#[must_use]
pub fn daemon_data_dir() -> std::path::PathBuf {
    if let Some(home) = home_dir() {
        home.join(".ministr")
    } else {
        std::env::temp_dir().join("ministr")
    }
}

/// Resolve the daemon Unix socket path (`~/.ministr/ministrd.sock`).
///
/// Provided for Unix-specific tooling; on Windows the daemon uses a
/// named pipe instead — prefer [`daemon_ipc_addr`] for portable code.
#[must_use]
pub fn daemon_socket_path() -> std::path::PathBuf {
    daemon_data_dir().join("ministrd.sock")
}

/// Resolve the daemon PID file path (`~/.ministr/ministrd.pid`).
#[must_use]
pub fn daemon_pid_path() -> std::path::PathBuf {
    daemon_data_dir().join("ministrd.pid")
}

/// IPC address for the daemon — Unix domain socket or Windows named pipe.
///
/// Both variants exist on every platform (to keep the type portable in
/// public APIs), but only the platform-native variant is functional at
/// runtime. See [`transport::connect`].
#[derive(Debug, Clone)]
pub enum IpcAddr {
    /// Unix domain socket path (macOS, Linux).
    Unix(std::path::PathBuf),
    /// Windows named pipe name (e.g. `\\.\pipe\ministr`).
    NamedPipe(String),
}

impl IpcAddr {
    /// Advisory check for whether the endpoint is currently reachable —
    /// cheaper than a full health probe.
    ///
    /// Unix: whether the socket file exists on disk.
    /// Windows: whether the named pipe namespace entry exists — this
    /// returns `true` iff a daemon has an instance waiting, which is
    /// stronger than the Unix file-exists check (on Windows there is no
    /// "stale socket file" analogue).
    #[must_use]
    pub fn exists(&self) -> bool {
        match self {
            Self::Unix(path) => path.exists(),
            // Windows named pipes live under `\\.\pipe\` and can be
            // probed via the standard file-metadata path. When no server
            // has created the pipe, metadata lookup returns NotFound.
            Self::NamedPipe(name) => std::fs::metadata(name).is_ok(),
        }
    }
}

impl std::fmt::Display for IpcAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unix(path) => write!(f, "{}", path.display()),
            Self::NamedPipe(name) => f.write_str(name),
        }
    }
}

/// Get the platform-appropriate IPC address for the daemon.
///
/// Uses a per-user named pipe on Windows to avoid cross-account collisions
/// on multi-user machines (two different logins running their own daemon).
#[must_use]
pub fn daemon_ipc_addr() -> IpcAddr {
    #[cfg(unix)]
    {
        IpcAddr::Unix(daemon_socket_path())
    }
    #[cfg(windows)]
    {
        let suffix = std::env::var("USERNAME")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "default".to_string());
        IpcAddr::NamedPipe(format!(r"\\.\pipe\ministrd-{suffix}"))
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
