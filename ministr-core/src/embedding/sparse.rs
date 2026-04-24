//! [`FastSparseEmbedder`] — sparse embedding via SPLADE models in `fastembed`.
//!
//! Wraps [`fastembed::SparseTextEmbedding`] with automatic model download
//! and caching, producing sparse vectors for hybrid search.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fastembed::{SparseInitOptions, SparseModel, SparseTextEmbedding};
use tracing::{info, instrument};

use crate::error::IndexError;

use super::{SparseEmbedder, SparseVector};

/// Default batch size for sparse embedding inference.
const DEFAULT_BATCH_SIZE: usize = 256;

/// Sparse embedding model powered by SPLADE via the `fastembed` crate.
///
/// Produces sparse vectors where only activated vocabulary positions carry
/// non-zero weight. These are used for keyword-level matching in hybrid search.
///
/// Uses an interior `Mutex` because `SparseTextEmbedding::embed` requires
/// `&mut self`.
///
/// # Examples
///
/// ```no_run
/// use ministr_core::embedding::{SparseEmbedder, FastSparseEmbedder};
///
/// let embedder = FastSparseEmbedder::new("splade-pp-v1", None)?;
/// let sparse_vecs = embedder.embed_sparse(&["hello world"])?;
/// assert!(!sparse_vecs[0].indices.is_empty());
/// # Ok::<(), ministr_core::error::IndexError>(())
/// ```
pub struct FastSparseEmbedder {
    model: Mutex<SparseTextEmbedding>,
}

impl FastSparseEmbedder {
    /// Create a new `FastSparseEmbedder` with the given model name and optional cache directory.
    ///
    /// # Supported Models
    ///
    /// - `"splade-pp-v1"` — SPLADE++ v1, general-purpose sparse model (default)
    /// - `"bge-m3-sparse"` — BGE-M3 sparse mode, multilingual
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the model name is unknown or
    /// the model cannot be loaded.
    #[instrument(skip_all, fields(model = model_name))]
    #[must_use = "constructors return a new value"]
    pub fn new(model_name: &str, cache_dir: Option<&str>) -> Result<Self, IndexError> {
        let sparse_model = parse_sparse_model(model_name)?;

        let mut options = SparseInitOptions::new(sparse_model).with_show_download_progress(true);

        if let Some(dir) = cache_dir {
            options = options.with_cache_dir(PathBuf::from(dir));
        }

        let model =
            SparseTextEmbedding::try_new(options).map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("failed to initialize sparse model '{model_name}': {e}"),
            })?;

        info!(model = model_name, "sparse embedding model loaded");

        Ok(Self {
            model: Mutex::new(model),
        })
    }

    /// Create a `FastSparseEmbedder` with a cache directory under the ministr data directory.
    ///
    /// Resolves the cache path as `{data_dir}/models/`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the model cannot be loaded.
    #[must_use = "constructors return a new value"]
    pub fn with_data_dir(model_name: &str, data_dir: &Path) -> Result<Self, IndexError> {
        let cache_dir = data_dir.join("models");
        let cache_str = cache_dir.to_string_lossy();
        Self::new(model_name, Some(&cache_str))
    }
}

impl SparseEmbedder for FastSparseEmbedder {
    fn embed_sparse(&self, texts: &[&str]) -> Result<Vec<SparseVector>, IndexError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let docs: Vec<&str> = texts.to_vec();
        let mut model = self.model.lock().map_err(|e| IndexError::EmbeddingFailed {
            reason: format!("sparse model lock poisoned: {e}"),
        })?;

        let raw_embeddings = model.embed(docs, Some(DEFAULT_BATCH_SIZE)).map_err(|e| {
            IndexError::EmbeddingFailed {
                reason: format!("sparse embedding inference failed: {e}"),
            }
        })?;

        Ok(raw_embeddings
            .into_iter()
            .map(|se| SparseVector {
                indices: se
                    .indices
                    .iter()
                    .map(|&i| u32::try_from(i).unwrap_or(u32::MAX))
                    .collect(),
                values: se.values,
            })
            .collect())
    }
}

/// Map a model name string to the corresponding `SparseModel` enum variant.
fn parse_sparse_model(name: &str) -> Result<SparseModel, IndexError> {
    match name {
        "splade-pp-v1" => Ok(SparseModel::SPLADEPPV1),
        "bge-m3-sparse" => Ok(SparseModel::BGEM3),
        _ => Err(IndexError::EmbeddingFailed {
            reason: format!(
                "unknown sparse model '{name}'. Supported: splade-pp-v1, bge-m3-sparse"
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_sparse_models() {
        assert!(parse_sparse_model("splade-pp-v1").is_ok());
        assert!(parse_sparse_model("bge-m3-sparse").is_ok());
    }

    #[test]
    fn parse_unknown_sparse_model_returns_error() {
        let err = parse_sparse_model("nonexistent").unwrap_err();
        assert!(err.to_string().contains("unknown sparse model"));
    }

    #[test]
    fn sparse_vector_construction() {
        let sv = SparseVector {
            indices: vec![10, 42, 100],
            values: vec![0.5, 1.2, 0.3],
        };
        assert_eq!(sv.indices.len(), 3);
        assert_eq!(sv.values.len(), 3);
    }

    // Integration test: requires model download
    #[test]
    #[ignore = "requires sparse model download"]
    fn fast_sparse_embedder_produces_vectors() {
        let embedder = FastSparseEmbedder::new("splade-pp-v1", None).unwrap();
        let vecs = embedder
            .embed_sparse(&["hello world", "rust programming"])
            .unwrap();
        assert_eq!(vecs.len(), 2);
        assert!(!vecs[0].indices.is_empty());
        assert_eq!(vecs[0].indices.len(), vecs[0].values.len());
    }

    #[test]
    #[ignore = "requires sparse model download"]
    fn fast_sparse_embedder_empty_input() {
        let embedder = FastSparseEmbedder::new("splade-pp-v1", None).unwrap();
        let vecs = embedder.embed_sparse(&[]).unwrap();
        assert!(vecs.is_empty());
    }
}
