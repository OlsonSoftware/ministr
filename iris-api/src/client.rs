//! HTTP client for communicating with the iris daemon over Unix domain socket.
//!
//! [`DaemonClient`] provides typed methods for all daemon API endpoints.
//! Used by `iris-mcp` (MCP proxy) and `iris-cli` (CLI tool).

use std::path::PathBuf;

use serde::de::DeserializeOwned;

use crate::ApiError;
use crate::corpus::{
    CorpusInfo, ListCorporaResponse, RegisterCorpusRequest, RegisterCorpusResponse,
};
use crate::query::{
    ExtractRequest, ExtractResponse, ReferencesResponse, SectionDetail, SurveyRequest,
    SurveyResponse, SymbolDefinition, SymbolsRequest, SymbolsResponse,
};
use crate::status::DaemonStatus;

/// Errors from daemon client operations.
#[derive(Debug)]
pub enum ClientError {
    /// Failed to connect to the daemon socket.
    Connect(String),
    /// HTTP request failed.
    Request(String),
    /// Failed to deserialize the response.
    Deserialize(String),
    /// Daemon returned an error response.
    Api(ApiError),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(msg) => write!(f, "daemon connection failed: {msg}"),
            Self::Request(msg) => write!(f, "request failed: {msg}"),
            Self::Deserialize(msg) => write!(f, "response decode failed: {msg}"),
            Self::Api(err) => write!(f, "daemon error: {err}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// HTTP client for the iris daemon API over Unix domain socket.
///
/// All methods are async and return typed responses. The client holds
/// a connection to `~/.iris/irisd.sock`.
pub struct DaemonClient {
    socket_path: PathBuf,
}

impl DaemonClient {
    /// Create a new client connecting to the default daemon socket.
    #[must_use]
    pub fn new() -> Self {
        Self {
            socket_path: crate::daemon_socket_path(),
        }
    }

    /// Create a client connecting to a specific socket path.
    #[must_use]
    pub fn with_socket(path: PathBuf) -> Self {
        Self { socket_path: path }
    }

    /// Check if the daemon socket file exists (fast, no I/O beyond stat).
    ///
    /// Use this for polling during startup. For a definitive health check,
    /// call [`is_healthy`](Self::is_healthy) instead.
    #[must_use]
    pub fn is_socket_present(&self) -> bool {
        self.socket_path.exists()
    }

    /// Check if the daemon is actually running and responding.
    ///
    /// Attempts a real HTTP request to `/api/v1/status`. Returns `true`
    /// only if the daemon responds successfully within 2 seconds.
    pub async fn is_healthy(&self) -> bool {
        if !self.socket_path.exists() {
            return false;
        }
        tokio::time::timeout(std::time::Duration::from_secs(2), self.status())
            .await
            .is_ok_and(|r| r.is_ok())
    }

    /// Alias for [`is_socket_present`] — quick file-existence check.
    ///
    /// Prefer [`is_healthy`] when you need to confirm the daemon is responsive.
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.is_socket_present()
    }

    /// The socket path this client connects to.
    #[must_use]
    pub fn socket_path(&self) -> &std::path::Path {
        &self.socket_path
    }

    // -- Corpus management --

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
        self.post(&format!("/api/v1/corpora/{corpus_id}/survey"), &req)
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
    ) -> Result<SymbolDefinition, ClientError> {
        let encoded = encode_path_component(symbol_id);
        self.get(&format!("/api/v1/corpora/{corpus_id}/definition/{encoded}"))
            .await
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
    ) -> Result<ReferencesResponse, ClientError> {
        let encoded = encode_path_component(symbol_id);
        self.get(&format!("/api/v1/corpora/{corpus_id}/references/{encoded}"))
            .await
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
    /// in the session shadow and budget tracker so `session_budget` reflects
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

    /// Export a corpus to a portable `.iris-index` bundle.
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

    /// Import an `.iris-index` bundle into the daemon.
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
    pub async fn session_budget(
        &self,
        corpus_id: &str,
        session_id: &str,
    ) -> Result<crate::session::SessionBudgetResponse, ClientError> {
        self.get(&format!(
            "/api/v1/corpora/{corpus_id}/sessions/{session_id}/budget"
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
    pub async fn evict_content(
        &self,
        corpus_id: &str,
        session_id: &str,
        req: &crate::session::EvictRequest,
    ) -> Result<crate::session::EvictResponse, ClientError> {
        self.post(
            &format!("/api/v1/corpora/{corpus_id}/sessions/{session_id}/evicted"),
            req,
        )
        .await
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
        let resp = self.raw_request("GET", path, None).await?;
        Self::parse_response(&resp)
    }

    async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl serde::Serialize,
    ) -> Result<T, ClientError> {
        let json = serde_json::to_vec(body).map_err(|e| ClientError::Request(e.to_string()))?;
        let resp = self.raw_request("POST", path, Some(json)).await?;
        Self::parse_response(&resp)
    }

    async fn delete(&self, path: &str) -> Result<Vec<u8>, ClientError> {
        self.raw_request("DELETE", path, None).await
    }

    /// Send a raw HTTP request over the platform IPC channel.
    ///
    /// Uses Unix domain sockets on macOS/Linux and named pipes on Windows.
    /// Implements a minimal HTTP/1.1 client to avoid heavy dependencies.
    async fn raw_request(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, ClientError> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut stream = self.connect().await?;

        // Build HTTP/1.1 request.
        let content_length = body.as_ref().map_or(0, Vec::len);
        let request = format!(
            "{method} {path} HTTP/1.1\r\n\
             Host: localhost\r\n\
             Content-Type: application/json\r\n\
             Content-Length: {content_length}\r\n\
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

        // Read entire response.
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .map_err(|e| ClientError::Request(e.to_string()))?;

        // Extract body (after the \r\n\r\n header terminator).
        let header_end = response
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .ok_or_else(|| ClientError::Request("malformed HTTP response".to_string()))?;

        Ok(response[header_end + 4..].to_vec())
    }

    /// Connect to the daemon's IPC channel (UDS on Unix, named pipe on Windows).
    #[cfg(unix)]
    async fn connect(&self) -> Result<tokio::net::UnixStream, ClientError> {
        tokio::net::UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| ClientError::Connect(format!("{}: {e}", self.socket_path.display())))
    }

    fn parse_response<T: DeserializeOwned>(body: &[u8]) -> Result<T, ClientError> {
        // Try to parse as the expected type first.
        if let Ok(value) = serde_json::from_slice::<T>(body) {
            return Ok(value);
        }
        // If that fails, try to parse as an API error.
        if let Ok(err) = serde_json::from_slice::<ApiError>(body) {
            return Err(ClientError::Api(err));
        }
        // Last resort: raw deserialization error.
        Err(ClientError::Deserialize(
            String::from_utf8_lossy(body).into_owned(),
        ))
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
