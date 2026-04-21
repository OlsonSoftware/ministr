//! Error types for ministr-core.
//!
//! Each module area defines its own error enum, all using `thiserror` for
//! automatic `Display` and `Error` implementations. Errors are matchable
//! by variant so callers can handle specific failure modes.

use std::path::PathBuf;

/// Errors from the vector index and embedding pipeline.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    /// Failed to generate an embedding vector for the given text.
    #[error("embedding failed: {reason}")]
    EmbeddingFailed { reason: String },

    /// Vector index query returned an unexpected result.
    #[error("index query failed: {reason}")]
    QueryFailed { reason: String },

    /// The index file could not be loaded or is corrupted.
    #[error("index load failed for {path}: {reason}")]
    LoadFailed { path: PathBuf, reason: String },

    /// Attempted to access a vector ID that does not exist.
    #[error("vector not found: {id}")]
    VectorNotFound { id: String },
}

/// Errors from session shadow tracking and budget management.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// The requested session does not exist.
    #[error("session not found: {id}")]
    NotFound { id: String },

    /// Session state is inconsistent (e.g. delivered items reference unknown content).
    #[error("session state inconsistent: {reason}")]
    Inconsistent { reason: String },

    /// Budget limit has been exceeded.
    #[error("context budget exceeded: used {used} of {budget} tokens")]
    BudgetExceeded { used: usize, budget: usize },

    /// Failed to persist session state.
    #[error("session persistence failed: {source}")]
    PersistFailed {
        #[source]
        source: std::io::Error,
    },
}

/// Errors from the storage layer (`SQLite`, file I/O, serialization).
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// `SQLite` operation failed.
    #[error("database error: {reason}")]
    Database { reason: String },

    /// Database is busy — the `busy_timeout` elapsed without the lock
    /// being acquired. Callers can safely retry transient busy errors.
    #[error("database busy: {reason}")]
    Busy { reason: String },

    /// File I/O operation failed.
    #[error("I/O error: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },

    /// Serialization or deserialization failed.
    #[error("serialization error: {reason}")]
    Serialization { reason: String },

    /// The requested item was not found in storage.
    #[error("not found: {entity} with id {id}")]
    NotFound { entity: String, id: String },

    /// Schema migration failed.
    #[error("migration failed: {reason}")]
    MigrationFailed { reason: String },
}

impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        // Classify SQLITE_BUSY / SQLITE_LOCKED into the transient variant
        // so callers that want to retry can match on it; everything else
        // flows into the generic Database error.
        if let rusqlite::Error::SqliteFailure(err, _) = &e
            && matches!(
                err.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
        {
            return StorageError::Busy {
                reason: e.to_string(),
            };
        }
        StorageError::Database {
            reason: e.to_string(),
        }
    }
}

/// Errors from document parsing and content extraction.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// The document format is not supported.
    #[error("unsupported format: {format}")]
    UnsupportedFormat { format: String },

    /// The document could not be parsed.
    #[error("parse failed for {path}: {reason}")]
    Failed { path: PathBuf, reason: String },

    /// Section extraction produced no sections (document may be empty).
    #[error("no sections found in {path}")]
    NoSections { path: PathBuf },

    /// File I/O error while reading the document.
    #[error("read error: {source}")]
    ReadError {
        #[from]
        source: std::io::Error,
    },

    /// Encoding error (e.g. file is not valid UTF-8).
    #[error("encoding error in {path}: {reason}")]
    EncodingError { path: PathBuf, reason: String },
}

/// Errors from the coherence subsystem (file watching and change detection).
#[derive(Debug, thiserror::Error)]
pub enum CoherenceError {
    /// Failed to initialize the file watcher.
    #[error("watcher initialization failed: {reason}")]
    WatcherInit { reason: String },

    /// Failed to watch a directory path.
    #[error("failed to watch {path}: {reason}")]
    WatchFailed {
        path: std::path::PathBuf,
        reason: String,
    },

    /// Re-indexing a changed file failed.
    #[error("re-index failed for {path}: {source}")]
    ReindexFailed {
        path: std::path::PathBuf,
        #[source]
        source: Box<IngestionError>,
    },

    /// The watcher channel was disconnected.
    #[error("watcher channel closed")]
    ChannelClosed,
}

/// Errors from the ingestion pipeline.
#[derive(Debug, thiserror::Error)]
pub enum IngestionError {
    /// File I/O error during ingestion.
    #[error("I/O error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// Document parsing failed.
    #[error("parse error: {source}")]
    Parse {
        #[from]
        source: ParseError,
    },

    /// Storage operation failed.
    #[error("storage error: {source}")]
    Storage {
        #[from]
        source: StorageError,
    },

    /// File encoding is not valid UTF-8.
    #[error("encoding error in {path}: file is not valid UTF-8")]
    Encoding { path: PathBuf },

    /// Embedding or vector index operation failed.
    #[error("embedding error: {reason}")]
    Embedding { reason: String },

    /// The operation was cancelled via a cancellation token.
    #[error("ingestion cancelled")]
    Cancelled,
}

/// Errors from HTTP fetching and URL handling.
#[derive(Debug, thiserror::Error)]
pub enum WebError {
    /// The URL is invalid or uses an unsupported scheme.
    #[error("invalid URL {url}: {reason}")]
    InvalidUrl { url: String, reason: String },

    /// HTTP request failed after exhausting all retries.
    #[error("HTTP request failed for {url} after {attempts} attempts: {reason}")]
    TooManyRetries {
        url: String,
        attempts: u32,
        reason: String,
    },

    /// HTTP request returned a non-success status code (not retryable).
    #[error("HTTP {status} for {url}")]
    HttpStatus { url: String, status: u16 },

    /// Underlying transport error (DNS, connection refused, etc.).
    #[error("request error: {source}")]
    Request {
        #[from]
        source: reqwest::Error,
    },

    /// Neither `llms-full.txt` nor `llms.txt` was found for the domain.
    #[error("llms.txt not found for {domain}")]
    LlmsTxtNotFound { domain: String },

    /// Cache I/O error when reading or writing cached web content.
    #[error("cache I/O error for {path}: {reason}")]
    CacheIo {
        path: std::path::PathBuf,
        reason: String,
    },

    /// Ingestion of fetched web content failed.
    #[error("web ingestion failed: {reason}")]
    IngestionFailed { reason: String },

    /// Sitemap XML parsing failed.
    #[error("sitemap parse error: {reason}")]
    SitemapParse { reason: String },

    /// The operation was cancelled via a cancellation token.
    #[error("web fetch cancelled")]
    Cancelled,
}

/// Errors from git clone and repository operations.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    /// A git subprocess exited with a non-zero status.
    #[error("git {command} failed (exit {exit_code}): {stderr}")]
    CommandFailed {
        command: String,
        exit_code: i32,
        stderr: String,
    },

    /// Could not create or access the clone directory.
    #[error("clone directory error for {path}: {reason}")]
    CloneDirectory { path: PathBuf, reason: String },

    /// Failed to read or write clone metadata.
    #[error("metadata error for {path}: {reason}")]
    Metadata { path: PathBuf, reason: String },

    /// The `git` binary was not found on `PATH`.
    #[error("git is not installed or not on PATH")]
    NotInstalled,

    /// The repository URL is invalid or empty.
    #[error("invalid repository URL: {url}")]
    InvalidRepo { url: String },

    /// The operation was cancelled via a cancellation token.
    #[error("git operation cancelled")]
    Cancelled,

    /// A git subprocess exceeded its timeout.
    #[error("git command timed out after {timeout_secs}s")]
    Timeout { timeout_secs: u64 },
}

/// Errors from index bundle export/import operations.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    /// A required file is missing from the corpus directory.
    #[error("missing file {path}: {reason}")]
    MissingFile { path: PathBuf, reason: String },

    /// File I/O failed during export or import.
    #[error("I/O error for {path}: {reason}")]
    Io { path: PathBuf, reason: String },

    /// Serialization or deserialization failed.
    #[error("serialization error: {reason}")]
    SerializationFailed { reason: String },

    /// The bundle's database operations failed.
    #[error("database error: {reason}")]
    DatabaseError { reason: String },

    /// The bundle file is malformed or missing required entries.
    #[error("invalid bundle: {reason}")]
    InvalidBundle { reason: String },

    /// The bundle was created with a newer format version than we support.
    #[error("incompatible bundle version {bundle_version} (max supported: {max_supported})")]
    IncompatibleVersion {
        bundle_version: u32,
        max_supported: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_error_display() {
        let err = IndexError::EmbeddingFailed {
            reason: "model not loaded".into(),
        };
        assert_eq!(err.to_string(), "embedding failed: model not loaded");

        let err = IndexError::LoadFailed {
            path: PathBuf::from("/tmp/index.hnsw"),
            reason: "corrupted header".into(),
        };
        assert!(err.to_string().contains("/tmp/index.hnsw"));
    }

    #[test]
    fn session_error_display() {
        let err = SessionError::BudgetExceeded {
            used: 120_000,
            budget: 100_000,
        };
        assert_eq!(
            err.to_string(),
            "context budget exceeded: used 120000 of 100000 tokens"
        );

        let err = SessionError::NotFound {
            id: "sess-42".into(),
        };
        assert_eq!(err.to_string(), "session not found: sess-42");
    }

    #[test]
    fn storage_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "gone");
        let err: StorageError = io_err.into();
        assert!(matches!(err, StorageError::Io { .. }));
        assert!(err.to_string().contains("gone"));
    }

    #[test]
    fn parse_error_display() {
        let err = ParseError::UnsupportedFormat {
            format: "docx".into(),
        };
        assert_eq!(err.to_string(), "unsupported format: docx");

        let err = ParseError::NoSections {
            path: PathBuf::from("empty.md"),
        };
        assert!(err.to_string().contains("empty.md"));
    }

    #[test]
    fn parse_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        let err: ParseError = io_err.into();
        assert!(matches!(err, ParseError::ReadError { .. }));
    }

    #[test]
    fn git_error_display() {
        let err = GitError::CommandFailed {
            command: "clone".into(),
            exit_code: 128,
            stderr: "fatal: repository not found".into(),
        };
        assert!(err.to_string().contains("clone"));
        assert!(err.to_string().contains("128"));

        let err = GitError::NotInstalled;
        assert!(err.to_string().contains("not installed"));

        let err = GitError::InvalidRepo { url: String::new() };
        assert!(err.to_string().contains("invalid repository URL"));
    }
}
