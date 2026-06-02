//! HTTP client for communicating with the ministr daemon over its native
//! IPC transport (Unix domain socket on macOS/Linux, named pipe on Windows).
//!
//! [`DaemonClient`] provides typed methods for all daemon API endpoints.
//! Used by `ministr-mcp` (MCP proxy) and `ministr-cli` (CLI tool).
//!
//! ## Per-request session attribution
//!
//! When a caller wraps a future in [`with_session_id`], every HTTP request
//! issued inside that future automatically carries an
//! `X-Ministr-Session-Id` header. The daemon's activity middleware uses
//! that header to tag activity events with their originating session even
//! when the underlying tool route is not session-scoped (e.g. corpus-wide
//! `survey` / `symbols` / `definition` / `references` / `extract` / `toc`
//! / `bridge` calls). Scopes are per-async-task, so concurrent tool
//! handlers from different sessions never cross-contaminate.

use std::future::Future;

use serde::de::DeserializeOwned;

tokio::task_local! {
    /// Active MCP session id for the duration of [`with_session_id`].
    /// Read by [`DaemonClient::raw_request`] to stamp every outbound HTTP
    /// request with `X-Ministr-Session-Id` when set.
    static REQUEST_SESSION_ID: Option<String>;
}

/// Run `fut` with `session_id` bound as the active MCP session for every
/// [`DaemonClient`] HTTP request issued inside it.
///
/// The session id is read from a `tokio::task_local!` slot in
/// [`DaemonClient::raw_request`] and forwarded to the daemon as an
/// `X-Ministr-Session-Id` header. The daemon's activity middleware uses
/// that header to attribute corpus-wide tool calls (which have no session
/// id in the URL path) to the originating session.
///
/// Pass `None` to leave the slot empty inside `fut` — useful for short
/// administrative calls (`status`, `list_corpora`) explicitly executed
/// outside a session context.
pub fn with_session_id<F>(session_id: Option<String>, fut: F) -> impl Future<Output = F::Output>
where
    F: Future,
{
    REQUEST_SESSION_ID.scope(session_id, fut)
}

/// Read the active session id from the current async task's scope, if any.
/// Returns `None` outside a [`with_session_id`] frame.
fn current_session_id() -> Option<String> {
    REQUEST_SESSION_ID
        .try_with(std::clone::Clone::clone)
        .unwrap_or(None)
}

use crate::activity::ActivityResponse;
use crate::coherence::CoherenceEventsResponse;
use crate::corpus::{
    CloneRepoRequest, CloneRepoResponse, CorpusInfo, ListCorporaResponse, RegisterCorpusRequest,
    RegisterCorpusResponse, UpdateCorpusPathsRequest,
};
use crate::query::{
    DeadCodeRequest, DeadCodeResponse, ExtractRequest, ExtractResponse, ImpactResponse,
    ReferencesResponse, SectionDetail, SolidRequest, SolidResponse, SurveyRequest, SurveyResponse,
    SymbolDefinition, SymbolsRequest, SymbolsResponse,
};
use crate::status::DaemonStatus;
use crate::transport;
use crate::{ApiError, IpcAddr};

/// Errors from daemon client operations.
#[derive(Debug)]
pub enum ClientError {
    /// Failed to connect to the daemon socket.
    Connect(String),
    /// HTTP request failed (write error, read error, malformed response).
    Request(String),
    /// Failed to deserialize a 2xx response body.
    Deserialize(String),
    /// Daemon returned a structured `ApiError` (any 4xx/5xx whose body
    /// parses as `ApiError`).
    Api(ApiError),
    /// Daemon returned a non-2xx status with a body that wasn't
    /// `ApiError`-shaped (e.g. a hyper internal error page). Callers
    /// that need to distinguish transient server faults from app-level
    /// errors can branch on `code`.
    HttpStatus {
        /// HTTP status code from the response status line.
        code: u16,
        /// Raw response body (lossily decoded to UTF-8).
        body: String,
    },
    /// The whole request–response cycle exceeded [`REQUEST_TIMEOUT`].
    Timeout {
        /// Human description of the method + path that timed out.
        operation: String,
        /// How long we waited before giving up (ms).
        elapsed_ms: u128,
    },
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(msg) => write!(f, "daemon connection failed: {msg}"),
            Self::Request(msg) => write!(f, "request failed: {msg}"),
            Self::Deserialize(msg) => write!(f, "response decode failed: {msg}"),
            Self::Api(err) => write!(f, "daemon error: {err}"),
            Self::HttpStatus { code, body } => {
                write!(f, "daemon returned HTTP {code}: {body}")
            }
            Self::Timeout {
                operation,
                elapsed_ms,
            } => write!(f, "daemon {operation} timed out after {elapsed_ms}ms"),
        }
    }
}

impl std::error::Error for ClientError {}

/// Request timeout for every call that goes through [`DaemonClient::raw_request`].
///
/// Chosen to be longer than the daemon's `ministr_ask` inference timeout
/// (120s today) so legitimate sub-inference calls can complete, while
/// still surfacing genuine daemon hangs in bounded time.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

/// Connect-time retry backoffs — applied only *before* any bytes go
/// over the wire, so non-idempotent requests aren't replayed. Tuned to
/// cover a daemon-restart-and-socket-rebind window (typically <1s).
const CONNECT_RETRY_BACKOFFS: [std::time::Duration; 3] = [
    std::time::Duration::from_millis(200),
    std::time::Duration::from_millis(500),
    std::time::Duration::from_secs(1),
];

/// Backoff schedule for retrying a transient *server* fault (5xx or
/// timeout) on an idempotent GET. Distinct from
/// [`CONNECT_RETRY_BACKOFFS`], which covers pre-write connect failures
/// for all verbs; these retries replay a fully-sent request and so are
/// gated to GET only.
const SERVER_RETRY_BACKOFFS: [std::time::Duration; 3] = [
    std::time::Duration::from_millis(200),
    std::time::Duration::from_millis(500),
    std::time::Duration::from_secs(1),
];

/// HTTP client for the ministr daemon API over its native IPC transport.
///
/// All methods are async and return typed responses. The client is bound
/// to a single [`IpcAddr`] (the default platform endpoint from
/// [`crate::daemon_ipc_addr`] unless overridden).
pub struct DaemonClient {
    addr: IpcAddr,
}

impl DaemonClient {
    /// Create a new client connecting to the default platform daemon endpoint.
    #[must_use]
    pub fn new() -> Self {
        Self {
            addr: crate::daemon_ipc_addr(),
        }
    }

    /// Create a client bound to a specific IPC endpoint. Primarily used by
    /// tests that spin up an in-process daemon on a temp endpoint.
    #[must_use]
    pub fn with_addr(addr: IpcAddr) -> Self {
        Self { addr }
    }

    /// Advisory check for whether the endpoint is currently reachable
    /// (fast stat on Unix, pipe-metadata lookup on Windows).
    ///
    /// Use this for polling during startup. For a definitive health check,
    /// call [`is_healthy`](Self::is_healthy) instead.
    #[must_use]
    pub fn is_endpoint_present(&self) -> bool {
        self.addr.exists()
    }

    /// Check if the daemon is actually running and responding.
    ///
    /// Attempts a real HTTP request to `/api/v1/status`. Returns `true`
    /// only if the daemon responds successfully within 2 seconds.
    pub async fn is_healthy(&self) -> bool {
        if !self.addr.exists() {
            return false;
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), self.status())
            .await
            .is_ok_and(|r| r.is_ok())
    }

    /// Alias for [`is_endpoint_present`](Self::is_endpoint_present) — cheap
    /// reachability probe. Prefer [`is_healthy`](Self::is_healthy) when you
    /// need to confirm the daemon is responsive.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.is_endpoint_present()
    }

    /// The IPC endpoint this client connects to.
    #[must_use]
    pub fn endpoint(&self) -> &IpcAddr {
        &self.addr
    }

    /// Ensure a daemon is running and reachable, spawning a **detached**
    /// `<daemon_bin> __daemon` sidecar if none is currently healthy.
    ///
    /// This is the spawn-if-not-alive handshake the desktop app and the MCP
    /// proxy share: probe [`is_healthy`](Self::is_healthy); if a daemon
    /// already owns the socket, attach to it (returns `Ok(false)` — "was
    /// already running"). Otherwise spawn the sidecar so it **outlives the
    /// spawning process** — a GUI close or proxy exit must leave the daemon,
    /// and its warm corpora/indexing, running — then poll until it answers
    /// `/api/v1/status` or `timeout` elapses.
    ///
    /// Detachment uses std only (no `libc`/`unsafe` `pre_exec`): on Unix a
    /// fresh process group (`process_group(0)`) so a group-wide signal when
    /// the parent quits never reaches the daemon; on Windows
    /// `DETACHED_PROCESS | CREATE_NO_WINDOW`. stdio is nulled — the daemon
    /// writes its own tracing log file, so discarding it loses nothing and
    /// avoids a child that blocks on an inherited pipe.
    ///
    /// `daemon_bin` must be the `ministr` CLI binary that hosts the hidden
    /// `__daemon` subcommand. The caller resolves it (bundle sidecar /
    /// staged `~/.ministr/bin` / system install) so this crate stays free of
    /// any app-bundle layout knowledge.
    ///
    /// # Errors
    ///
    /// [`ClientError::Connect`] if the spawn syscall fails;
    /// [`ClientError::Timeout`] if the freshly-spawned daemon never becomes
    /// healthy within `timeout`.
    pub async fn ensure_daemon_spawned(
        &self,
        daemon_bin: &std::path::Path,
        timeout: std::time::Duration,
    ) -> Result<bool, ClientError> {
        // Fast path: a daemon already owns the socket — just attach.
        if self.is_healthy().await {
            return Ok(false);
        }

        let mut cmd = std::process::Command::new(daemon_bin);
        cmd.arg("__daemon")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            // New process group (pgid = child pid) so the daemon escapes the
            // spawner's group and survives its exit. Stable std since 1.64.
            cmd.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            // DETACHED_PROCESS: no inherited console. CREATE_NO_WINDOW:
            // never flash a console window from a GUI parent.
            const DETACHED_PROCESS: u32 = 0x0000_0008;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(DETACHED_PROCESS | CREATE_NO_WINDOW);
        }

        // Dropping the returned Child does not wait on or kill the process
        // (std only closes the handle) — exactly the detached semantics we
        // want.
        cmd.spawn().map_err(|e| {
            ClientError::Connect(format!(
                "failed to spawn daemon `{} __daemon`: {e}",
                daemon_bin.display()
            ))
        })?;

        // Poll until the daemon binds the socket and answers, or give up.
        // bootstrap::run binds the listener before restoring corpora, so
        // health typically arrives well under a second.
        let start = std::time::Instant::now();
        loop {
            if self.is_healthy().await {
                return Ok(true);
            }
            if start.elapsed() >= timeout {
                return Err(ClientError::Timeout {
                    operation: format!("daemon spawn via `{}`", daemon_bin.display()),
                    elapsed_ms: start.elapsed().as_millis(),
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        }
    }

    // -- Corpus management --

    /// Register a corpus with the daemon, optionally overriding the
    /// daemon's path-derived display name.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or
    /// deserialization failure.
    pub async fn register_corpus_with_display_name(
        &self,
        paths: &[String],
        display_name: Option<String>,
    ) -> Result<RegisterCorpusResponse, ClientError> {
        let req = RegisterCorpusRequest {
            paths: paths.to_vec(),
            git_includes: Vec::new(),
            display_name,
        };
        self.post("/api/v1/corpora", &req).await
    }

    /// Register a corpus with the daemon.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn register_corpus(
        &self,
        paths: &[String],
    ) -> Result<RegisterCorpusResponse, ClientError> {
        let req = RegisterCorpusRequest {
            paths: paths.to_vec(),
            git_includes: Vec::new(),
            display_name: None,
        };
        self.post("/api/v1/corpora", &req).await
    }

    /// List all registered corpora.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn list_corpora(&self) -> Result<Vec<CorpusInfo>, ClientError> {
        let resp: ListCorporaResponse = self.get("/api/v1/corpora").await?;
        Ok(resp.corpora)
    }

    /// List a corpus's indexed files with content hashes + section counts.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn list_corpus_files(
        &self,
        corpus_id: &str,
    ) -> Result<Vec<crate::corpus::FileInfo>, ClientError> {
        let resp: crate::corpus::ListFilesResponse = self
            .get(&format!("/api/v1/corpora/{corpus_id}/files"))
            .await?;
        Ok(resp.files)
    }

    /// Get status of a specific corpus.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn corpus_status(&self, corpus_id: &str) -> Result<CorpusInfo, ClientError> {
        self.get(&format!("/api/v1/corpora/{corpus_id}")).await
    }

    /// Unregister a corpus.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection or request failure.
    pub async fn unregister_corpus(&self, corpus_id: &str) -> Result<(), ClientError> {
        self.delete(&format!("/api/v1/corpora/{corpus_id}")).await?;
        Ok(())
    }

    /// Unregister a corpus AND purge its on-disk index directory.
    ///
    /// Same as [`Self::unregister_corpus`] but passes `?purge=true` so the
    /// daemon — which owns the corpus data directory — deletes it after
    /// teardown. Used by the desktop "remove project" action so the GUI
    /// never has to reach into `~/.ministr/corpora` itself.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if the corpus doesn't exist or the purge fails.
    pub async fn unregister_corpus_purge(&self, corpus_id: &str) -> Result<(), ClientError> {
        self.delete(&format!("/api/v1/corpora/{corpus_id}?purge=true"))
            .await?;
        Ok(())
    }

    /// Reindex a corpus: purge its on-disk index and re-register it, so the
    /// daemon re-resolves the corpus's `.ministr.toml` config and re-embeds
    /// from scratch. The rebuild is daemon-side (it owns the data directory).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if the corpus doesn't exist or the rebuild fails.
    pub async fn reindex_corpus(
        &self,
        corpus_id: &str,
    ) -> Result<RegisterCorpusResponse, ClientError> {
        self.post(&format!("/api/v1/corpora/{corpus_id}/reindex"), &())
            .await
    }

    /// Replace an existing corpus's path set without dropping its sessions.
    ///
    /// The new paths must canonicalise to the same `corpus_id` as the
    /// existing corpus. To change identity, call [`Self::unregister_corpus`]
    /// followed by [`Self::register_corpus`] instead.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if the corpus doesn't exist (`NOT_FOUND`),
    /// the new paths canonicalise to a different id (`BAD_REQUEST` with
    /// code `identity_changed`), or the request fails.
    pub async fn update_corpus_paths(
        &self,
        corpus_id: &str,
        paths: &[String],
    ) -> Result<(), ClientError> {
        let req = UpdateCorpusPathsRequest {
            paths: paths.to_vec(),
        };
        self.put_no_content(&format!("/api/v1/corpora/{corpus_id}/paths"), &req)
            .await
    }

    /// Clone a git repo, register it as a new corpus, and append a
    /// `[[linked]]` entry to `parent_corpus_id`'s `.ministr.toml`.
    ///
    /// This is the daemon-mediated implementation of the `ministr_clone`
    /// MCP tool's "create new linked project" semantics. The daemon
    /// chooses the clone target path (under `~/.ministr/clones/`),
    /// performs the clone, registers the new corpus, mutates the parent
    /// TOML, and returns the new corpus's id + label.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, clone, registration, or
    /// TOML-mutation failure.
    pub async fn clone_repo(
        &self,
        parent_corpus_id: &str,
        req: &CloneRepoRequest,
    ) -> Result<CloneRepoResponse, ClientError> {
        self.post(&format!("/api/v1/corpora/{parent_corpus_id}/clone"), req)
            .await
    }

    // -- Query endpoints --

    /// Semantic search across the corpus.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn survey(
        &self,
        corpus_id: &str,
        query: &str,
        top_k: Option<usize>,
    ) -> Result<SurveyResponse, ClientError> {
        let req = SurveyRequest {
            query: query.to_string(),
            top_k,
            session_id: None,
        };
        self.survey_req(corpus_id, &req).await
    }

    /// Semantic search with a full pre-built request (e.g. to include
    /// `session_id`). Prefer [`survey`](Self::survey) for simple calls.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn survey_req(
        &self,
        corpus_id: &str,
        req: &SurveyRequest,
    ) -> Result<SurveyResponse, ClientError> {
        self.post(&format!("/api/v1/corpora/{corpus_id}/survey"), req)
            .await
    }

    /// Search for code symbols.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn symbols(
        &self,
        corpus_id: &str,
        req: &SymbolsRequest,
    ) -> Result<SymbolsResponse, ClientError> {
        self.post(&format!("/api/v1/corpora/{corpus_id}/symbols"), req)
            .await
    }

    /// Get a symbol definition.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn definition(
        &self,
        corpus_id: &str,
        symbol_id: &str,
        session_id: Option<&str>,
    ) -> Result<SymbolDefinition, ClientError> {
        let encoded = encode_path_component(symbol_id);
        let path = match session_id {
            Some(sid) => {
                let sid_enc = encode_path_component(sid);
                format!("/api/v1/corpora/{corpus_id}/definition/{encoded}?session_id={sid_enc}")
            }
            None => format!("/api/v1/corpora/{corpus_id}/definition/{encoded}"),
        };
        self.get(&path).await
    }

    /// Get references to a symbol.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn references(
        &self,
        corpus_id: &str,
        symbol_id: &str,
        session_id: Option<&str>,
    ) -> Result<ReferencesResponse, ClientError> {
        let encoded = encode_path_component(symbol_id);
        let path = match session_id {
            Some(sid) => {
                let sid_enc = encode_path_component(sid);
                format!("/api/v1/corpora/{corpus_id}/references/{encoded}?session_id={sid_enc}")
            }
            None => format!("/api/v1/corpora/{corpus_id}/references/{encoded}"),
        };
        self.get(&path).await
    }

    /// Compute the transitive impact (blast radius) of changing a symbol.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn impact(
        &self,
        corpus_id: &str,
        symbol_id: &str,
        max_depth: Option<u32>,
        session_id: Option<&str>,
    ) -> Result<ImpactResponse, ClientError> {
        let encoded = encode_path_component(symbol_id);
        let mut qs: Vec<String> = Vec::new();
        if let Some(d) = max_depth {
            qs.push(format!("max_depth={d}"));
        }
        if let Some(sid) = session_id {
            qs.push(format!("session_id={}", encode_path_component(sid)));
        }
        let suffix = if qs.is_empty() {
            String::new()
        } else {
            format!("?{}", qs.join("&"))
        };
        let path = format!("/api/v1/corpora/{corpus_id}/impact/{encoded}{suffix}");
        self.get(&path).await
    }

    /// Find symbols with zero references (dead-code candidates).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn dead_code(
        &self,
        corpus_id: &str,
        req: &DeadCodeRequest,
        session_id: Option<&str>,
    ) -> Result<DeadCodeResponse, ClientError> {
        let path = match session_id {
            Some(sid) => format!(
                "/api/v1/corpora/{corpus_id}/dead?session_id={}",
                encode_path_component(sid)
            ),
            None => format!("/api/v1/corpora/{corpus_id}/dead"),
        };
        self.post(&path, req).await
    }

    /// Detect possible SOLID violations across the corpus.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn solid(
        &self,
        corpus_id: &str,
        req: &SolidRequest,
        session_id: Option<&str>,
    ) -> Result<SolidResponse, ClientError> {
        let path = match session_id {
            Some(sid) => format!(
                "/api/v1/corpora/{corpus_id}/solid?session_id={}",
                encode_path_component(sid)
            ),
            None => format!("/api/v1/corpora/{corpus_id}/solid"),
        };
        self.post(&path, req).await
    }

    /// Read a source file's full contents + the symbol-definition spans the
    /// index knows for it (the desktop code browser's file view).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] if the file is unavailable or the request fails.
    pub async fn read_file_content(
        &self,
        corpus_id: &str,
        path: String,
    ) -> Result<crate::query::FileContentResponse, ClientError> {
        let req = crate::query::FilePathRequest { path };
        self.post(&format!("/api/v1/corpora/{corpus_id}/file"), &req)
            .await
    }

    /// List a file's resolved identifier occurrences (click-any-token index).
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on request or deserialization failure.
    pub async fn file_occurrences(
        &self,
        corpus_id: &str,
        path: String,
    ) -> Result<Vec<crate::query::Occurrence>, ClientError> {
        let req = crate::query::FilePathRequest { path };
        let resp: crate::query::OccurrencesResponse = self
            .post(&format!("/api/v1/corpora/{corpus_id}/occurrences"), &req)
            .await?;
        Ok(resp.occurrences)
    }

    /// Read a section by ID.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn read_section(
        &self,
        corpus_id: &str,
        section_id: &str,
    ) -> Result<SectionDetail, ClientError> {
        let encoded = encode_path_component(section_id);
        self.get(&format!("/api/v1/corpora/{corpus_id}/read/{encoded}"))
            .await
    }

    /// Read a section with session tracking (records delivery in budget).
    ///
    /// Like [`read_section`](Self::read_section) but also records the delivery
    /// in the session shadow and budget tracker so `session_usage` reflects
    /// actual token usage.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn session_read_section(
        &self,
        corpus_id: &str,
        session_id: &str,
        section_id: &str,
    ) -> Result<SectionDetail, ClientError> {
        let encoded = encode_path_component(section_id);
        self.get(&format!(
            "/api/v1/corpora/{corpus_id}/sessions/{session_id}/read/{encoded}"
        ))
        .await
    }

    /// Extract claims from a section.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn extract(
        &self,
        corpus_id: &str,
        req: &ExtractRequest,
    ) -> Result<ExtractResponse, ClientError> {
        self.post(&format!("/api/v1/corpora/{corpus_id}/extract"), req)
            .await
    }

    /// Get table of contents.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn toc(
        &self,
        corpus_id: &str,
        req: &crate::query::TocRequest,
    ) -> Result<crate::query::TocResponse, ClientError> {
        self.post(&format!("/api/v1/corpora/{corpus_id}/toc"), req)
            .await
    }

    /// Find related claims.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn related(
        &self,
        corpus_id: &str,
        req: &crate::query::RelatedRequest,
    ) -> Result<crate::query::RelatedResponse, ClientError> {
        self.post(&format!("/api/v1/corpora/{corpus_id}/related"), req)
            .await
    }

    /// Query cross-language bridge links.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn bridge(
        &self,
        corpus_id: &str,
        req: &crate::query::BridgeRequest,
    ) -> Result<crate::query::BridgeResponse, ClientError> {
        self.post(&format!("/api/v1/corpora/{corpus_id}/bridge"), req)
            .await
    }

    // -- Prefetch --

    /// Get prefetch cache metrics for a corpus.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn prefetch_metrics(
        &self,
        corpus_id: &str,
    ) -> Result<crate::session::PrefetchMetricsResponse, ClientError> {
        self.get(&format!("/api/v1/corpora/{corpus_id}/prefetch"))
            .await
    }

    // -- Bundles --

    /// Export a corpus to a portable `.ministr-index` bundle.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn export_bundle(
        &self,
        corpus_id: &str,
    ) -> Result<crate::corpus::ExportBundleResponse, ClientError> {
        // POST with empty body to trigger export.
        self.post(
            &format!("/api/v1/corpora/{corpus_id}/export"),
            &serde_json::Value::Null,
        )
        .await
    }

    /// Import an `.ministr-index` bundle into the daemon.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn import_bundle(
        &self,
        req: &crate::corpus::ImportBundleRequest,
    ) -> Result<crate::corpus::ImportBundleResponse, ClientError> {
        self.post("/api/v1/corpora/import", req).await
    }

    // -- Compress --

    /// Compress content items for budget-efficient eviction.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn compress(
        &self,
        corpus_id: &str,
        req: &crate::session::CompressRequest,
    ) -> Result<crate::session::CompressResponse, ClientError> {
        self.post(&format!("/api/v1/corpora/{corpus_id}/compress"), req)
            .await
    }

    /// Ask a question and get a synthesized answer from sub-inference.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn ask(
        &self,
        corpus_id: &str,
        query: &str,
        session_id: Option<&str>,
    ) -> Result<crate::query::AskResponse, ClientError> {
        let req = crate::query::AskRequest {
            query: query.to_string(),
            session_id: session_id.map(String::from),
        };
        self.post(&format!("/api/v1/corpora/{corpus_id}/ask"), &req)
            .await
    }

    // -- Sessions --

    /// Create a new session for a corpus.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn create_session(
        &self,
        corpus_id: &str,
        budget_tokens: Option<usize>,
    ) -> Result<crate::session::CreateSessionResponse, ClientError> {
        let req = crate::session::CreateSessionRequest { budget_tokens };
        self.post(&format!("/api/v1/corpora/{corpus_id}/sessions"), &req)
            .await
    }

    /// Get the budget status for a session.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn session_usage(
        &self,
        corpus_id: &str,
        session_id: &str,
    ) -> Result<crate::session::SessionUsageResponse, ClientError> {
        self.get(&format!(
            "/api/v1/corpora/{corpus_id}/sessions/{session_id}/usage"
        ))
        .await
    }

    /// Destroy a session.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection failure.
    pub async fn destroy_session(
        &self,
        corpus_id: &str,
        session_id: &str,
    ) -> Result<(), ClientError> {
        self.delete(&format!(
            "/api/v1/corpora/{corpus_id}/sessions/{session_id}"
        ))
        .await?;
        Ok(())
    }

    /// Remove all sessions for a corpus.
    ///
    /// Useful when a proxy reconnects and wants to clean up stale sessions
    /// from a previous connection.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection failure.
    pub async fn clear_sessions(&self, corpus_id: &str) -> Result<(), ClientError> {
        self.delete(&format!("/api/v1/corpora/{corpus_id}/sessions"))
            .await?;
        Ok(())
    }

    /// Signal that content has been evicted from the agent's context window.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn drop_content(
        &self,
        corpus_id: &str,
        session_id: &str,
        req: &crate::session::DropRequest,
    ) -> Result<crate::session::DropResponse, ClientError> {
        self.post(
            &format!("/api/v1/corpora/{corpus_id}/sessions/{session_id}/dropped"),
            req,
        )
        .await
    }

    // -- Activity --

    /// Snapshot recent tool-call activity events from the daemon.
    ///
    /// Returns up to `limit` events, newest first. If `since_ms` is
    /// provided, only events with `timestamp_ms > since_ms` are returned
    /// — useful for incremental polling.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn recent_activity(
        &self,
        limit: Option<usize>,
        since_ms: Option<u64>,
    ) -> Result<ActivityResponse, ClientError> {
        use std::fmt::Write as _;
        let mut path = String::from("/activity");
        let mut sep = '?';
        if let Some(limit) = limit {
            path.push(sep);
            let _ = write!(path, "limit={limit}");
            sep = '&';
        }
        if let Some(since) = since_ms {
            path.push(sep);
            let _ = write!(path, "since={since}");
        }
        self.get(&path).await
    }

    // -- Coherence --

    /// Snapshot recent coherence (file-change) events from the daemon.
    ///
    /// Returns up to `limit` events, newest first. If `since_ms` is
    /// provided, only events with `timestamp_ms > since_ms` are returned
    /// — useful for incremental polling alongside `recent_activity`.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn recent_coherence_events(
        &self,
        limit: Option<usize>,
        since_ms: Option<u64>,
    ) -> Result<CoherenceEventsResponse, ClientError> {
        use std::fmt::Write as _;
        let mut path = String::from("/coherence-events");
        let mut sep = '?';
        if let Some(limit) = limit {
            path.push(sep);
            let _ = write!(path, "limit={limit}");
            sep = '&';
        }
        if let Some(since) = since_ms {
            path.push(sep);
            let _ = write!(path, "since={since}");
        }
        self.get(&path).await
    }

    // -- Admin --

    /// Get daemon status.
    ///
    /// # Errors
    ///
    /// Returns [`ClientError`] on connection, request, or deserialization failure.
    pub async fn status(&self) -> Result<DaemonStatus, ClientError> {
        self.get("/api/v1/status").await
    }

    // -- HTTP primitives over UDS --

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        // GET is safe + idempotent, so a transient 5xx or a timeout can
        // be retried without risk of double-applying a side effect.
        let (code, body) = self.get_with_retry(path).await?;
        Self::parse_response(code, &body)
    }

    /// `GET` with bounded backoff retry for transient server faults.
    ///
    /// Retries only on a 5xx response or a [`ClientError::Timeout`] —
    /// both of which a safe, idempotent GET can replay. Any other error
    /// (connect failure, 4xx, decode) is returned with its kind intact,
    /// never retried. Non-idempotent verbs (POST/PUT/DELETE) bypass this
    /// entirely and are issued exactly once.
    async fn get_with_retry(&self, path: &str) -> Result<(u16, Vec<u8>), ClientError> {
        fn is_transient(outcome: &Result<(u16, Vec<u8>), ClientError>) -> bool {
            matches!(outcome, Ok((code, _)) if (500..600).contains(code))
                || matches!(outcome, Err(ClientError::Timeout { .. }))
        }

        let mut outcome = self.raw_request("GET", path, None).await;
        for backoff in SERVER_RETRY_BACKOFFS {
            if !is_transient(&outcome) {
                return outcome;
            }
            // `ministr-api` is intentionally dependency-light (no
            // `tracing`); the retry is silent and the final error, if
            // any, reaches the caller with its kind intact.
            tokio::time::sleep(backoff).await;
            outcome = self.raw_request("GET", path, None).await;
        }
        outcome
    }

    async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        req: &impl serde::Serialize,
    ) -> Result<T, ClientError> {
        let json = serde_json::to_vec(req).map_err(|e| ClientError::Request(e.to_string()))?;
        let (code, body) = self.raw_request("POST", path, Some(json)).await?;
        Self::parse_response(code, &body)
    }

    async fn delete(&self, path: &str) -> Result<Vec<u8>, ClientError> {
        let (code, body) = self.raw_request("DELETE", path, None).await?;
        if !(200..300).contains(&code) {
            return Err(err_for_status(code, &body));
        }
        Ok(body)
    }

    async fn put_no_content(
        &self,
        path: &str,
        req: &impl serde::Serialize,
    ) -> Result<(), ClientError> {
        let json = serde_json::to_vec(req).map_err(|e| ClientError::Request(e.to_string()))?;
        let (code, body) = self.raw_request("PUT", path, Some(json)).await?;
        if !(200..300).contains(&code) {
            return Err(err_for_status(code, &body));
        }
        Ok(())
    }

    /// Send a raw HTTP request over the platform IPC channel.
    ///
    /// Uses Unix domain sockets on macOS/Linux and named pipes on Windows.
    /// Implements a minimal HTTP/1.1 client to avoid heavy dependencies.
    ///
    /// Returns `(status_code, body)` — the caller is responsible for
    /// interpreting the status. Each invocation:
    ///
    /// 1. Runs under a single [`REQUEST_TIMEOUT`] so a hung daemon can't
    ///    stall the caller indefinitely.
    /// 2. Retries [`transport::connect`] with [`CONNECT_RETRY_BACKOFFS`]
    ///    so a daemon-restart window (brief endpoint unavailability)
    ///    doesn't fail every concurrent tool call. Retries only before
    ///    any bytes are written — non-idempotent requests are never
    ///    replayed.
    async fn raw_request(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<(u16, Vec<u8>), ClientError> {
        let op = format!("{method} {path}");
        let started = std::time::Instant::now();

        let result = tokio::time::timeout(REQUEST_TIMEOUT, async {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};

            let mut stream = self.connect_with_retry().await?;

            // Build HTTP/1.1 request.
            let content_length = body.as_ref().map_or(0, Vec::len);
            // Stamp the active MCP session id when one is in scope (see
            // `with_session_id`). The daemon's activity middleware reads
            // this to attribute corpus-wide tool calls to a session.
            let session_header = current_session_id()
                .map(|sid| format!("X-Ministr-Session-Id: {sid}\r\n"))
                .unwrap_or_default();
            let request = format!(
                "{method} {path} HTTP/1.1\r\n\
                 Host: localhost\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {content_length}\r\n\
                 {session_header}\
                 Connection: close\r\n\
                 \r\n"
            );

            stream
                .write_all(request.as_bytes())
                .await
                .map_err(|e| ClientError::Request(e.to_string()))?;
            if let Some(body) = body {
                stream
                    .write_all(&body)
                    .await
                    .map_err(|e| ClientError::Request(e.to_string()))?;
            }

            // Read entire response. Server sends `Connection: close`
            // framing so EOF terminates the body.
            let mut response = Vec::new();
            stream
                .read_to_end(&mut response)
                .await
                .map_err(|e| ClientError::Request(e.to_string()))?;

            // Parse the status line: "HTTP/1.1 200 OK\r\n..."
            let status_line_end = response
                .windows(2)
                .position(|w| w == b"\r\n")
                .ok_or_else(|| ClientError::Request("response missing status line".into()))?;
            let status_line = std::str::from_utf8(&response[..status_line_end])
                .map_err(|_| ClientError::Request("status line is not UTF-8".into()))?;
            let code: u16 = status_line
                .split_whitespace()
                .nth(1)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| ClientError::Request(format!("bad status line: {status_line}")))?;

            // Extract body (after the \r\n\r\n header terminator).
            let header_end = response
                .windows(4)
                .position(|w| w == b"\r\n\r\n")
                .ok_or_else(|| ClientError::Request("malformed HTTP response".into()))?;

            Ok::<_, ClientError>((code, response[header_end + 4..].to_vec()))
        })
        .await;

        match result {
            Ok(inner) => inner,
            Err(_) => Err(ClientError::Timeout {
                operation: op,
                elapsed_ms: started.elapsed().as_millis(),
            }),
        }
    }

    /// Connect with small retry/backoff to ride out a daemon-restart window.
    async fn connect_with_retry(&self) -> Result<transport::Stream, ClientError> {
        let mut last_err: Option<std::io::Error> = None;
        for (attempt, delay) in std::iter::once(None)
            .chain(CONNECT_RETRY_BACKOFFS.iter().copied().map(Some))
            .enumerate()
        {
            if let Some(d) = delay {
                tokio::time::sleep(d).await;
            }
            match transport::connect(&self.addr).await {
                Ok(s) => return Ok(s),
                Err(e) => {
                    // ministr-api has no tracing dependency by design — the
                    // retry loop is silent. The final `Connect` error
                    // carries the last OS error for diagnostics.
                    let _ = attempt;
                    last_err = Some(e);
                }
            }
        }
        Err(ClientError::Connect(format!(
            "{}: {}",
            self.addr,
            last_err.map_or_else(|| "unknown".to_string(), |e| e.to_string()),
        )))
    }

    fn parse_response<T: DeserializeOwned>(code: u16, body: &[u8]) -> Result<T, ClientError> {
        if (200..300).contains(&code) {
            // 2xx: parse body as the expected type. An empty 2xx body
            // is unusual (204 routes return via `delete` which takes a
            // different path) — surface it as a clear deserialize error
            // instead of a cryptic serde message.
            if body.is_empty() {
                return Err(ClientError::Deserialize(format!(
                    "empty body from daemon (HTTP {code})"
                )));
            }
            return serde_json::from_slice::<T>(body).map_err(|e| {
                ClientError::Deserialize(format!("{e}: {}", String::from_utf8_lossy(body)))
            });
        }
        // Non-2xx: prefer structured ApiError, fall back to HttpStatus.
        Err(err_for_status(code, body))
    }
}

/// Build a [`ClientError`] for a non-2xx response — prefer `Api` when
/// the body parses as the structured error shape, otherwise surface
/// the raw status + body via `HttpStatus`.
fn err_for_status(code: u16, body: &[u8]) -> ClientError {
    if let Ok(err) = serde_json::from_slice::<ApiError>(body) {
        return ClientError::Api(err);
    }
    ClientError::HttpStatus {
        code,
        body: String::from_utf8_lossy(body).into_owned(),
    }
}

impl Default for DaemonClient {
    fn default() -> Self {
        Self::new()
    }
}

/// Percent-encode a path component for use in HTTP URLs.
///
/// Encodes characters that are not unreserved in RFC 3986 (e.g. `/`, `#`, `?`, `%`).
fn encode_path_component(s: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            // Unreserved characters (RFC 3986 section 2.3) plus `:` and `@`.
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' | b'@' => {
                encoded.push(byte as char);
            }
            _ => {
                let _ = write!(encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}
