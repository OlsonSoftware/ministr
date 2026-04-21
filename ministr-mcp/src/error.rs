//! Diagnostic error types for the MCP server layer.
//!
//! These errors wrap ministr-core error types and add `miette::Diagnostic`
//! metadata for user-facing error reports with codes, help text, and context.

use miette::Diagnostic;
use thiserror::Error;

use ministr_core::error::{IndexError, ParseError, SessionError, StorageError};

/// Top-level error type for MCP tool handlers.
///
/// Each variant wraps a core error and adds a diagnostic code so that
/// errors are identifiable in logs and user reports.
#[derive(Debug, Error, Diagnostic)]
pub enum McpError {
    /// An index or embedding operation failed.
    #[error(transparent)]
    #[diagnostic(code(ministr::mcp::index))]
    Index(#[from] IndexError),

    /// A session tracking operation failed.
    #[error(transparent)]
    #[diagnostic(code(ministr::mcp::session))]
    Session(#[from] SessionError),

    /// A storage operation failed.
    #[error(transparent)]
    #[diagnostic(code(ministr::mcp::storage))]
    Storage(#[from] StorageError),

    /// A document parsing operation failed.
    #[error(transparent)]
    #[diagnostic(code(ministr::mcp::parse))]
    Parse(#[from] ParseError),

    /// An MCP protocol-level error (e.g. invalid tool parameters).
    #[error("MCP protocol error: {reason}")]
    #[diagnostic(code(ministr::mcp::protocol), help("check the tool call parameters"))]
    Protocol { reason: String },
}
