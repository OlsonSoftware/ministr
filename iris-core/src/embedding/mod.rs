//! Embedding subsystem for generating vector representations of text.
//!
//! The [`Embedder`] trait defines the interface for dense text embedding models.
//! The [`SparseEmbedder`] trait defines the interface for sparse (SPLADE-style) models.
//! The [`FastEmbedder`] and [`FastSparseEmbedder`] implementations wrap the
//! `fastembed` crate for local ONNX-based inference with automatic model download.

pub mod cache;
#[cfg(feature = "candle")]
mod candle_impl;
mod fastembed_impl;
pub mod hybrid;
#[cfg(feature = "candle")]
mod metal_bert;
mod rerank;
mod sparse;

pub use cache::CachedEmbedder;
#[cfg(feature = "candle")]
pub use candle_impl::{CandleEmbedder, CandleModelInfo, candle_supported_models, is_candle_model};
pub use fastembed_impl::{FastEmbedder, ModelInfo, TruncatingEmbedder, supported_models};
pub use hybrid::HybridEmbedder;
pub use rerank::FastReranker;
pub use sparse::FastSparseEmbedder;

use crate::error::IndexError;

/// Supported model serialization formats.
///
/// Tracks which runtime backend a model uses, enabling format-aware
/// loading and future GGUF quantized model support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelFormat {
    /// ONNX Runtime format (used by FastEmbed / `ort`).
    Onnx,
    /// Candle native format (Hugging Face safetensors, Metal-accelerated).
    Candle,
    /// GGUF quantized format (future support).
    Gguf,
}

/// Result of a model compatibility check against a stored index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelCompatibility {
    /// Models match — no re-embedding needed.
    Compatible,
    /// The stored model differs from the current model; re-embedding is required.
    IncompatibleModel {
        /// The model name stored in the index.
        stored: String,
        /// The model name currently configured.
        current: String,
    },
    /// No model was previously stored (fresh index); embedding can proceed.
    NoPreviousModel,
}

/// Check whether the current embedding model is compatible with what was
/// previously used to build the index.
///
/// Returns [`ModelCompatibility::Compatible`] when the names match,
/// [`ModelCompatibility::NoPreviousModel`] when no model was stored (first run),
/// or [`ModelCompatibility::IncompatibleModel`] when they differ and
/// re-embedding is required.
///
/// # Examples
///
/// ```
/// use iris_core::embedding::{check_model_compatibility, ModelCompatibility};
///
/// let result = check_model_compatibility("all-MiniLM-L6-v2", Some("all-MiniLM-L6-v2"));
/// assert_eq!(result, ModelCompatibility::Compatible);
///
/// let result = check_model_compatibility("bge-small-en-v1.5", Some("all-MiniLM-L6-v2"));
/// assert!(matches!(result, ModelCompatibility::IncompatibleModel { .. }));
///
/// let result = check_model_compatibility("all-MiniLM-L6-v2", None);
/// assert_eq!(result, ModelCompatibility::NoPreviousModel);
/// ```
#[must_use]
pub fn check_model_compatibility(
    current_model: &str,
    stored_model: Option<&str>,
) -> ModelCompatibility {
    match stored_model {
        None => ModelCompatibility::NoPreviousModel,
        Some(stored) if stored == current_model => ModelCompatibility::Compatible,
        Some(stored) => ModelCompatibility::IncompatibleModel {
            stored: stored.to_owned(),
            current: current_model.to_owned(),
        },
    }
}

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

/// A single reranking score: original document index paired with relevance.
#[derive(Debug, Clone, PartialEq)]
pub struct RerankScore {
    /// Original index in the input documents slice.
    pub index: usize,
    /// Cross-encoder relevance score (higher = more relevant).
    pub score: f32,
}

/// Interface for cross-encoder reranking models.
///
/// Rerankers take a query and a set of candidate documents, scoring each
/// document for relevance using a cross-encoder architecture. Unlike
/// bi-encoder embeddings, cross-encoders jointly attend to query and
/// document tokens for higher-quality relevance judgments.
///
/// Results are returned sorted by score descending (most relevant first).
pub trait Reranker: Send + Sync {
    /// Score documents for relevance to the query.
    ///
    /// Returns one [`RerankScore`] per input document, sorted by score
    /// descending. Each score includes the original index so callers can
    /// map back to their candidate list.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if inference fails.
    fn rerank(&self, query: &str, documents: &[&str]) -> Result<Vec<RerankScore>, IndexError>;
}

/// Create an embedding model using the best available backend.
///
/// Checks the `IRIS_BACKEND` environment variable:
/// - `"candle"` — force Candle Metal backend (requires `candle` feature)
/// - `"onnx"` or `"fastembed"` — force ONNX/FastEmbed backend
/// - empty/unset — auto-detect: on macOS with the `candle` feature,
///   use Candle if the model is supported; otherwise fall back to FastEmbed
///
/// # Errors
///
/// Returns [`IndexError::EmbeddingFailed`] if the selected backend fails to
/// initialize (e.g. model download failure, unsupported model name).
pub fn create_embedder(
    model_name: &str,
    data_dir: &std::path::Path,
) -> Result<std::sync::Arc<dyn Embedder>, IndexError> {
    let backend = std::env::var("IRIS_BACKEND").unwrap_or_default();

    match backend.as_str() {
        "candle" => {
            #[cfg(feature = "candle")]
            {
                tracing::info!(model = %model_name, "using Candle Metal backend (IRIS_BACKEND=candle)");
                let embedder = CandleEmbedder::with_data_dir(model_name, data_dir)?;
                return Ok(std::sync::Arc::new(embedder));
            }
            #[cfg(not(feature = "candle"))]
            {
                tracing::warn!(
                    "IRIS_BACKEND=candle requested but candle feature not enabled, falling back to ONNX"
                );
            }
        }
        "onnx" | "fastembed" => {
            tracing::info!(model = %model_name, "using ONNX/FastEmbed backend (IRIS_BACKEND={backend})");
        }
        "" =>
        {
            #[cfg(all(feature = "candle", target_os = "macos"))]
            if is_candle_model(model_name) {
                tracing::info!(
                    model = %model_name,
                    "auto-selected Candle Metal backend (Apple Silicon detected, model supported)"
                );
                let embedder = CandleEmbedder::with_data_dir(model_name, data_dir)?;
                return Ok(std::sync::Arc::new(embedder));
            }
        }
        other => {
            tracing::warn!(
                backend = other,
                "unknown IRIS_BACKEND value, falling back to ONNX"
            );
        }
    }

    // Default: FastEmbed/ONNX Runtime.
    let embedder = FastEmbedder::with_data_dir(model_name, data_dir)?;
    Ok(std::sync::Arc::new(embedder))
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

    /// A trivial reranker for unit-testing trait usage.
    /// Scores documents by their length (longer = higher score).
    struct MockReranker;

    impl Reranker for MockReranker {
        #[allow(clippy::cast_precision_loss)]
        fn rerank(&self, _query: &str, documents: &[&str]) -> Result<Vec<RerankScore>, IndexError> {
            let mut scores: Vec<RerankScore> = documents
                .iter()
                .enumerate()
                .map(|(i, doc)| RerankScore {
                    index: i,
                    score: doc.len() as f32,
                })
                .collect();
            scores.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            Ok(scores)
        }
    }

    #[test]
    fn mock_reranker_produces_sorted_scores() {
        let reranker = MockReranker;
        let scores = reranker
            .rerank("query", &["short", "a much longer document", "medium len"])
            .unwrap();
        assert_eq!(scores.len(), 3);
        // Sorted descending by score (length)
        assert!(scores[0].score >= scores[1].score);
        assert!(scores[1].score >= scores[2].score);
        // Longest doc should be first
        assert_eq!(scores[0].index, 1);
    }

    #[test]
    fn mock_reranker_empty_input() {
        let reranker = MockReranker;
        let scores = reranker.rerank("query", &[]).unwrap();
        assert!(scores.is_empty());
    }

    #[test]
    fn reranker_trait_object_works() {
        let reranker: Box<dyn Reranker> = Box::new(MockReranker);
        let scores = reranker.rerank("query", &["doc1", "doc2"]).unwrap();
        assert_eq!(scores.len(), 2);
    }
}
