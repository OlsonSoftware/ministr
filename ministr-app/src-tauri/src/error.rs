//! Structured error type for Tauri commands.
//!
//! Replaces the stringly-typed `Result<T, String>` /
//! `map_err(|e| e.to_string())` pattern with a classified error that
//! carries a [`ErrorKind`] for server-side logging, matching, and
//! testing, and `From` conversions so command bodies use `?` instead of
//! hand-rolled `.to_string()` mapping.
//!
//! ## Wire representation — deliberately a plain string
//!
//! [`CommandError`] serializes as its `message` **string**, not a
//! `{kind, message}` object. The React frontend renders command
//! failures with `String(e)` in ~20 call sites and has no central
//! `invoke` chokepoint; emitting an object would render as
//! `"[object Object]"` everywhere. Keeping the wire a string is
//! byte-identical to the previous behaviour while the Rust side gains
//! the structure (typed construction, a `kind` for `tracing`, testable
//! error paths, a single place errors are built). Switching the wire to
//! an object and consuming `kind` in the UI is a deliberate, separate,
//! UI-coordinated change — only the `Serialize` impl here and the
//! frontend catch sites would move.

use serde::{Serialize, Serializer};

/// Coarse classification of a command failure. Used server-side for
/// `tracing` and tests; not currently sent over the wire (see module
/// docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Corpus registry / lifecycle failure (register, unregister, …).
    Registry,
    /// Filesystem / IO failure.
    Io,
    /// A requested entity (corpus, client, path) does not exist.
    NotFound,
    /// Caller supplied invalid or out-of-policy input.
    InvalidInput,
    /// Background-task join failure or other internal fault.
    Internal,
}

impl ErrorKind {
    /// Stable lowercase tag for structured logging.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Registry => "registry",
            Self::Io => "io",
            Self::NotFound => "not_found",
            Self::InvalidInput => "invalid_input",
            Self::Internal => "internal",
        }
    }
}

/// A structured, classified error returned by `#[tauri::command]` fns.
#[derive(Debug, Clone)]
pub struct CommandError {
    /// Coarse failure class (server-side only).
    pub kind: ErrorKind,
    /// Human-readable message — this is what reaches the frontend.
    pub message: String,
}

impl CommandError {
    /// Construct an error with an explicit kind.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }

    /// A "requested entity does not exist" error.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }

    /// An "invalid/out-of-policy input" error.
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, message)
    }

    /// A generic internal error.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Internal, message)
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for CommandError {}

impl Serialize for CommandError {
    /// See module docs: the wire form is the bare message string for
    /// frontend `String(e)` compatibility.
    ///
    /// Serialization happens exactly once, at the command→frontend
    /// boundary, so it's also the natural single chokepoint to emit a
    /// *structured* server-side log (carrying the `kind` the wire
    /// deliberately drops) without per-command instrumentation.
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        tracing::warn!(
            kind = self.kind.as_str(),
            message = %self.message,
            "command returned an error to the frontend"
        );
        serializer.serialize_str(&self.message)
    }
}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self::new(ErrorKind::Internal, message)
    }
}

impl From<&str> for CommandError {
    fn from(message: &str) -> Self {
        Self::new(ErrorKind::Internal, message.to_owned())
    }
}

impl From<std::io::Error> for CommandError {
    fn from(e: std::io::Error) -> Self {
        let kind = if e.kind() == std::io::ErrorKind::NotFound {
            ErrorKind::NotFound
        } else {
            ErrorKind::Io
        };
        Self::new(kind, e.to_string())
    }
}

impl From<ministr_daemon::registry::RegistryError> for CommandError {
    fn from(e: ministr_daemon::registry::RegistryError) -> Self {
        Self::new(ErrorKind::Registry, e.to_string())
    }
}
