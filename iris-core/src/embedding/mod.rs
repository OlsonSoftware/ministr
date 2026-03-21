//! Embedding subsystem for generating vector representations of text.
//!
//! The [`Embedder`] trait defines the interface for text embedding models.
//! The [`FastEmbedder`] implementation wraps the `fastembed` crate for local
//! ONNX-based inference with automatic model download and caching.

mod fastembed_impl;

pub use fastembed_impl::FastEmbedder;

use crate::error::IndexError;

/// Interface for text embedding models.
///
/// Implementations must be `Send + Sync` so they can be shared across async
/// tasks (typically behind an `Arc`). The `embed` method is synchronous because
/// ONNX inference is CPU-bound — callers should use `spawn_blocking` when
/// invoking from an async context.
///
/// # Examples
///
/// ```no_run
/// use iris_core::embedding::{Embedder, FastEmbedder};
///
/// let embedder = FastEmbedder::new("all-MiniLM-L6-v2", None)?;
/// let vectors = embedder.embed(&["hello world", "how are you"])?;
/// assert_eq!(vectors.len(), 2);
/// assert_eq!(vectors[0].len(), embedder.dimension());
/// # Ok::<(), iris_core::error::IndexError>(())
/// ```
pub trait Embedder: Send + Sync {
    /// Generate embedding vectors for a batch of text inputs.
    ///
    /// Returns one vector per input text. All vectors have the same
    /// dimensionality, equal to [`Embedder::dimension`].
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if inference fails.
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError>;

    /// The dimensionality of vectors produced by this model.
    fn dimension(&self) -> usize;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trivial embedder for unit-testing trait usage.
    struct MockEmbedder {
        dim: usize,
    }

    impl Embedder for MockEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
            Ok(texts.iter().map(|_| vec![0.0; self.dim]).collect())
        }

        fn dimension(&self) -> usize {
            self.dim
        }
    }

    #[test]
    fn mock_embedder_produces_correct_dimensions() {
        let embedder = MockEmbedder { dim: 384 };
        let vectors = embedder.embed(&["hello", "world"]).unwrap();
        assert_eq!(vectors.len(), 2);
        assert_eq!(vectors[0].len(), 384);
        assert_eq!(vectors[1].len(), 384);
    }

    #[test]
    fn mock_embedder_empty_input() {
        let embedder = MockEmbedder { dim: 128 };
        let vectors = embedder.embed(&[]).unwrap();
        assert!(vectors.is_empty());
    }

    #[test]
    fn trait_object_works() {
        let embedder: Box<dyn Embedder> = Box::new(MockEmbedder { dim: 384 });
        assert_eq!(embedder.dimension(), 384);
        let vectors = embedder.embed(&["test"]).unwrap();
        assert_eq!(vectors.len(), 1);
    }
}
