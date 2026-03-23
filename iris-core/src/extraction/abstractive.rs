//! Abstractive compression trait for LLM-assisted summarization.
//!
//! Defines the [`AbstractiveCompressor`] trait used by the service layer
//! to delegate compression to an external LLM (e.g. via MCP sampling).
//! The trait is transport-agnostic — iris-core defines the interface,
//! while iris-mcp provides the MCP sampling implementation.

use std::future::Future;

/// Errors from abstractive compression.
///
/// These represent failures in the LLM-assisted compression path.
/// Callers should fall back to extractive compression when abstractive
/// fails.
#[derive(Debug, thiserror::Error)]
pub enum CompressError {
    /// Sampling is not available (e.g. peer not connected, client denies).
    #[error("sampling unavailable: {0}")]
    Unavailable(String),

    /// The sampling request was sent but failed.
    #[error("sampling failed: {0}")]
    Failed(String),
}

/// Trait for LLM-assisted abstractive compression.
///
/// Implementations use an external LLM to generate dense summaries that
/// preserve semantic content with higher compression ratios (90%+) than
/// extractive methods (60–80%).
///
/// The trait is async because it involves a network round-trip to the LLM.
/// Implementations must be `Send + Sync` for use in async service methods.
///
/// # Fallback behavior
///
/// The service layer treats abstractive compression as best-effort.
/// When [`compress`](Self::compress) returns an error, the caller falls
/// back to extractive summarization automatically.
pub trait AbstractiveCompressor: Send + Sync {
    /// Compress text into a dense abstractive summary.
    ///
    /// `context_hint` provides context about the content (e.g. heading path
    /// or section title) to help the LLM generate a more accurate summary.
    ///
    /// # Errors
    ///
    /// Returns [`CompressError::Unavailable`] if the LLM backend is not
    /// reachable, or [`CompressError::Failed`] if the request fails.
    fn compress(
        &self,
        text: &str,
        context_hint: &str,
    ) -> impl Future<Output = Result<String, CompressError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mock compressor for testing that returns a canned summary.
    struct MockCompressor {
        response: String,
    }

    impl MockCompressor {
        fn new(response: &str) -> Self {
            Self {
                response: response.to_string(),
            }
        }
    }

    impl AbstractiveCompressor for MockCompressor {
        async fn compress(
            &self,
            _text: &str,
            _context_hint: &str,
        ) -> Result<String, CompressError> {
            Ok(self.response.clone())
        }
    }

    /// A mock compressor that always fails.
    struct FailingCompressor;

    impl AbstractiveCompressor for FailingCompressor {
        async fn compress(
            &self,
            _text: &str,
            _context_hint: &str,
        ) -> Result<String, CompressError> {
            Err(CompressError::Unavailable("test: no peer".into()))
        }
    }

    #[tokio::test]
    async fn mock_compressor_returns_summary() {
        let compressor = MockCompressor::new("Dense summary.");
        let result = compressor.compress("Long text here.", "test/section").await;
        assert_eq!(result.unwrap(), "Dense summary.");
    }

    #[tokio::test]
    async fn failing_compressor_returns_error() {
        let compressor = FailingCompressor;
        let result = compressor.compress("Text.", "test/section").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no peer"));
    }

    #[test]
    fn compress_error_display() {
        let err = CompressError::Unavailable("no client".into());
        assert_eq!(err.to_string(), "sampling unavailable: no client");

        let err = CompressError::Failed("timeout".into());
        assert_eq!(err.to_string(), "sampling failed: timeout");
    }
}
