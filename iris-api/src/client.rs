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

    /// Check if the daemon socket exists (daemon may be running).
    #[must_use]
    pub fn is_available(&self) -> bool {
        self.socket_path.exists()
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
        self.get(&format!(
            "/api/v1/corpora/{corpus_id}/definition/{symbol_id}"
        ))
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
        self.get(&format!(
            "/api/v1/corpora/{corpus_id}/references/{symbol_id}"
        ))
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
        self.get(&format!("/api/v1/corpora/{corpus_id}/read/{section_id}"))
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

    /// Send a raw HTTP request over the Unix domain socket.
    ///
    /// This uses a simple HTTP/1.1 implementation over `tokio::net::UnixStream`
    /// to avoid pulling in a full HTTP client with UDS support.
    async fn raw_request(
        &self,
        method: &str,
        path: &str,
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, ClientError> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixStream;

        let mut stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| ClientError::Connect(format!("{}: {e}", self.socket_path.display())))?;

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
