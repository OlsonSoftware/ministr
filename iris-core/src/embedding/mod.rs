//! Embedding subsystem for generating vector representations of text.
//!
//! The [`Embedder`] trait defines the interface for dense text embedding models.
//! The [`SparseEmbedder`] trait defines the interface for sparse (SPLADE-style) models.
//! The [`FastEmbedder`] and [`FastSparseEmbedder`] implementations wrap the
//! `fastembed` crate for local ONNX-based inference with automatic model download.

mod fastembed_impl;
mod sparse;

pub use fastembed_impl::FastEmbedder;
pub use sparse::FastSparseEmbedder;

use crate::error::IndexError;

/// A sparse embedding: parallel arrays of token indices and their weights.
#[derive(Debug, Clone, PartialEq)]
pub struct SparseVector {
    /// Token/vocabulary indices with non-zero activation.
    pub indices: Vec<u32>,
    /// Corresponding weight for each index.
    pub values: Vec<f32>,
}

/// Interface for sparse text embedding models (e.g. SPLADE).
///
/// Produces sparse vectors where most dimensions are zero — only activated
/// token positions carry weight. Used alongside dense embeddings for
/// hybrid search with keyword-level matching.
pub trait SparseEmbedder: Send + Sync {
    /// Generate sparse embedding vectors for a batch of text inputs.
    ///
    /// Returns one [`SparseVector`] per input text.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if inference fails.
    fn embed_sparse(&self, texts: &[&str]) -> Result<Vec<SparseVector>, IndexError>;
}

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

    /// A trivial sparse embedder for unit-testing trait usage.
    struct MockSparseEmbedder;

    impl SparseEmbedder for MockSparseEmbedder {
        fn embed_sparse(&self, texts: &[&str]) -> Result<Vec<SparseVector>, IndexError> {
            Ok(texts
                .iter()
                .enumerate()
                .map(|(i, _)| SparseVector {
                    indices: vec![u32::try_from(i).unwrap(), u32::try_from(i + 10).unwrap()],
                    values: vec![1.0, 0.5],
                })
                .collect())
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

    #[test]
    fn mock_sparse_embedder_produces_vectors() {
        let embedder = MockSparseEmbedder;
        let vecs = embedder.embed_sparse(&["hello", "world"]).unwrap();
        assert_eq!(vecs.len(), 2);
        assert_eq!(vecs[0].indices.len(), 2);
        assert_eq!(vecs[0].values.len(), 2);
    }

    #[test]
    fn mock_sparse_embedder_empty_input() {
        let embedder = MockSparseEmbedder;
        let vecs = embedder.embed_sparse(&[]).unwrap();
        assert!(vecs.is_empty());
    }

    #[test]
    fn sparse_trait_object_works() {
        let embedder: Box<dyn SparseEmbedder> = Box::new(MockSparseEmbedder);
        let vecs = embedder.embed_sparse(&["test"]).unwrap();
        assert_eq!(vecs.len(), 1);
    }
}
