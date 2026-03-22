//! [`FastEmbedder`] — local embedding via the `fastembed` ONNX runtime.
//!
//! Wraps [`fastembed::TextEmbedding`] with automatic model download, caching
//! under a configurable directory, and model selection by name string.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use tracing::{info, instrument};

use crate::error::IndexError;

use super::Embedder;

/// Default batch size for embedding inference.
const DEFAULT_BATCH_SIZE: usize = 256;

/// Local embedding model powered by ONNX Runtime via the `fastembed` crate.
///
/// Supports automatic model download on first use with caching under a
/// configurable directory (defaults to `~/.iris/models/`).
///
/// Uses an interior `Mutex` because `TextEmbedding::embed` requires `&mut self`,
/// while the [`Embedder`] trait exposes `&self` for `Send + Sync` compatibility.
///
/// # Examples
///
/// ```no_run
/// use iris_core::embedding::{Embedder, FastEmbedder};
///
/// // Use default model with default cache directory
/// let embedder = FastEmbedder::new("all-MiniLM-L6-v2", None)?;
///
/// // Use a specific cache directory
/// let embedder = FastEmbedder::new("all-MiniLM-L6-v2", Some("/tmp/models"))?;
///
/// let vectors = embedder.embed(&["hello world"])?;
/// assert_eq!(vectors[0].len(), 384);
/// # Ok::<(), iris_core::error::IndexError>(())
/// ```
pub struct FastEmbedder {
    model: Mutex<TextEmbedding>,
    dim: usize,
}

impl FastEmbedder {
    /// Create a new `FastEmbedder` with the given model name and optional cache directory.
    ///
    /// The model is downloaded on first use and cached for subsequent runs.
    /// If `cache_dir` is `None`, the fastembed default cache location is used.
    ///
    /// # Supported Models
    ///
    /// - `"all-MiniLM-L6-v2"` — 384 dimensions, fast general-purpose (default)
    /// - `"all-MiniLM-L6-v2-q"` — 384 dimensions, quantized variant (faster, smaller)
    /// - `"all-MiniLM-L12-v2"` — 384 dimensions, slightly higher quality
    /// - `"all-MiniLM-L12-v2-q"` — 384 dimensions, quantized variant
    /// - `"bge-small-en-v1.5"` — 384 dimensions, BAAI small English
    /// - `"bge-small-en-v1.5-q"` — 384 dimensions, quantized variant
    /// - `"bge-base-en-v1.5"` — 768 dimensions, BAAI base English
    /// - `"bge-base-en-v1.5-q"` — 768 dimensions, quantized variant
    /// - `"bge-large-en-v1.5"` — 1024 dimensions, BAAI large English
    /// - `"bge-large-en-v1.5-q"` — 1024 dimensions, quantized variant
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the model name is unknown or
    /// the model cannot be loaded.
    #[instrument(skip_all, fields(model = model_name))]
    pub fn new(model_name: &str, cache_dir: Option<&str>) -> Result<Self, IndexError> {
        let embedding_model = parse_model_name(model_name)?;
        let dim = model_dimension(&embedding_model);

        let mut options = InitOptions::new(embedding_model).with_show_download_progress(true);

        if let Some(dir) = cache_dir {
            options = options.with_cache_dir(PathBuf::from(dir));
        }

        let model = TextEmbedding::try_new(options).map_err(|e| IndexError::EmbeddingFailed {
            reason: format!("failed to initialize model '{model_name}': {e}"),
        })?;

        info!(model = model_name, dim, "embedding model loaded");

        Ok(Self {
            model: Mutex::new(model),
            dim,
        })
    }

    /// Create a `FastEmbedder` with a cache directory under the iris data directory.
    ///
    /// Resolves the cache path as `{data_dir}/models/`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the model cannot be loaded.
    pub fn with_data_dir(model_name: &str, data_dir: &Path) -> Result<Self, IndexError> {
        let cache_dir = data_dir.join("models");
        let cache_str = cache_dir.to_string_lossy();
        Self::new(model_name, Some(&cache_str))
    }
}

impl Embedder for FastEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let docs: Vec<&str> = texts.to_vec();
        let mut model = self.model.lock().map_err(|e| IndexError::EmbeddingFailed {
            reason: format!("model lock poisoned: {e}"),
        })?;
        model
            .embed(docs, Some(DEFAULT_BATCH_SIZE))
            .map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("embedding inference failed: {e}"),
            })
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

/// Map a model name string to the corresponding `EmbeddingModel` enum variant.
fn parse_model_name(name: &str) -> Result<EmbeddingModel, IndexError> {
    match name {
        "all-MiniLM-L6-v2" => Ok(EmbeddingModel::AllMiniLML6V2),
        "all-MiniLM-L6-v2-q" => Ok(EmbeddingModel::AllMiniLML6V2Q),
        "all-MiniLM-L12-v2" => Ok(EmbeddingModel::AllMiniLML12V2),
        "all-MiniLM-L12-v2-q" => Ok(EmbeddingModel::AllMiniLML12V2Q),
        "bge-small-en-v1.5" => Ok(EmbeddingModel::BGESmallENV15),
        "bge-small-en-v1.5-q" => Ok(EmbeddingModel::BGESmallENV15Q),
        "bge-base-en-v1.5" => Ok(EmbeddingModel::BGEBaseENV15),
        "bge-base-en-v1.5-q" => Ok(EmbeddingModel::BGEBaseENV15Q),
        "bge-large-en-v1.5" => Ok(EmbeddingModel::BGELargeENV15),
        "bge-large-en-v1.5-q" => Ok(EmbeddingModel::BGELargeENV15Q),
        _ => Err(IndexError::EmbeddingFailed {
            reason: format!(
                "unknown embedding model '{name}'. Supported: \
                 all-MiniLM-L6-v2, all-MiniLM-L6-v2-q, \
                 all-MiniLM-L12-v2, all-MiniLM-L12-v2-q, \
                 bge-small-en-v1.5, bge-small-en-v1.5-q, \
                 bge-base-en-v1.5, bge-base-en-v1.5-q, \
                 bge-large-en-v1.5, bge-large-en-v1.5-q"
            ),
        }),
    }
}

/// Return the output dimension for a known embedding model.
fn model_dimension(model: &EmbeddingModel) -> usize {
    match model {
        EmbeddingModel::BGEBaseENV15 | EmbeddingModel::BGEBaseENV15Q => 768,
        EmbeddingModel::BGELargeENV15 | EmbeddingModel::BGELargeENV15Q => 1024,
        // All other supported models (MiniLM, BGE-small) produce 384-dim vectors
        _ => 384,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_models() {
        assert!(parse_model_name("all-MiniLM-L6-v2").is_ok());
        assert!(parse_model_name("all-MiniLM-L6-v2-q").is_ok());
        assert!(parse_model_name("all-MiniLM-L12-v2").is_ok());
        assert!(parse_model_name("all-MiniLM-L12-v2-q").is_ok());
        assert!(parse_model_name("bge-small-en-v1.5").is_ok());
        assert!(parse_model_name("bge-small-en-v1.5-q").is_ok());
        assert!(parse_model_name("bge-base-en-v1.5").is_ok());
        assert!(parse_model_name("bge-base-en-v1.5-q").is_ok());
        assert!(parse_model_name("bge-large-en-v1.5").is_ok());
        assert!(parse_model_name("bge-large-en-v1.5-q").is_ok());
    }

    #[test]
    fn parse_unknown_model_returns_error() {
        let err = parse_model_name("nonexistent-model").unwrap_err();
        assert!(err.to_string().contains("unknown embedding model"));
        assert!(err.to_string().contains("nonexistent-model"));
    }

    #[test]
    fn model_dimensions_correct() {
        assert_eq!(model_dimension(&EmbeddingModel::AllMiniLML6V2), 384);
        assert_eq!(model_dimension(&EmbeddingModel::AllMiniLML6V2Q), 384);
        assert_eq!(model_dimension(&EmbeddingModel::AllMiniLML12V2), 384);
        assert_eq!(model_dimension(&EmbeddingModel::AllMiniLML12V2Q), 384);
        assert_eq!(model_dimension(&EmbeddingModel::BGESmallENV15), 384);
        assert_eq!(model_dimension(&EmbeddingModel::BGESmallENV15Q), 384);
        assert_eq!(model_dimension(&EmbeddingModel::BGEBaseENV15), 768);
        assert_eq!(model_dimension(&EmbeddingModel::BGEBaseENV15Q), 768);
        assert_eq!(model_dimension(&EmbeddingModel::BGELargeENV15), 1024);
        assert_eq!(model_dimension(&EmbeddingModel::BGELargeENV15Q), 1024);
    }

    // Integration test: requires model download, so only run with --ignored
    #[test]
    #[ignore = "requires model download (~80MB)"]
    fn fast_embedder_produces_vectors() {
        let embedder = FastEmbedder::new("all-MiniLM-L6-v2", None).unwrap();
        assert_eq!(embedder.dimension(), 384);

        let vectors = embedder.embed(&["hello world", "how are you"]).unwrap();
        assert_eq!(vectors.len(), 2);
        assert_eq!(vectors[0].len(), 384);
        assert_eq!(vectors[1].len(), 384);

        // Vectors should be non-zero
        assert!(vectors[0].iter().any(|&v| v != 0.0));
    }

    #[test]
    #[ignore = "requires model download (~80MB)"]
    fn fast_embedder_empty_input() {
        let embedder = FastEmbedder::new("all-MiniLM-L6-v2", None).unwrap();
        let vectors = embedder.embed(&[]).unwrap();
        assert!(vectors.is_empty());
    }

    #[test]
    #[ignore = "requires model download (~80MB)"]
    fn fast_embedder_with_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let embedder = FastEmbedder::with_data_dir("all-MiniLM-L6-v2", tmp.path()).unwrap();
        assert_eq!(embedder.dimension(), 384);

        // Verify cache directory was created
        assert!(tmp.path().join("models").exists());
    }
}
