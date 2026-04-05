//! [`FastReranker`] — cross-encoder reranking via the `fastembed` ONNX runtime.
//!
//! Wraps [`fastembed::TextRerank`] with automatic model download, caching
//! under a configurable directory, and model selection by name string.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fastembed::{RerankInitOptions, RerankerModel, TextRerank};
use tracing::{info, instrument};

use crate::error::IndexError;

use super::{RerankScore, Reranker};

/// Default batch size for reranking inference.
const DEFAULT_BATCH_SIZE: usize = 256;

/// Cross-encoder reranking model powered by ONNX Runtime via the `fastembed` crate.
///
/// Scores query-document pairs jointly using a cross-encoder architecture,
/// producing higher-quality relevance scores than bi-encoder cosine similarity.
///
/// Supports automatic model download on first use with caching under a
/// configurable directory (defaults to the fastembed cache location).
///
/// Uses an interior `Mutex` because `TextRerank::rerank` requires `&mut self`,
/// while the [`Reranker`] trait exposes `&self` for `Send + Sync` compatibility.
///
/// # Examples
///
/// ```no_run
/// use iris_core::embedding::{Reranker, FastReranker};
///
/// let reranker = FastReranker::new("bge-reranker-base", None)?;
/// let scores = reranker.rerank("what is rust?", &["Rust is a language", "Python is fun"])?;
/// assert_eq!(scores.len(), 2);
/// assert!(scores[0].score >= scores[1].score); // sorted descending
/// # Ok::<(), iris_core::error::IndexError>(())
/// ```
pub struct FastReranker {
    model: Mutex<TextRerank>,
}

impl FastReranker {
    /// Create a new `FastReranker` with the given model name and optional cache directory.
    ///
    /// The model is downloaded on first use and cached for subsequent runs.
    /// If `cache_dir` is `None`, the fastembed default cache location is used.
    ///
    /// # Supported Models
    ///
    /// - `"bge-reranker-base"` — BAAI BGE reranker base (default)
    /// - `"bge-reranker-v2-m3"` — BAAI BGE reranker v2 M3, multilingual
    /// - `"jina-reranker-v1-turbo-en"` — Jina reranker v1 turbo, English
    /// - `"jina-reranker-v2-base-multilingual"` — Jina reranker v2 base, multilingual
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the model name is unknown or
    /// the model cannot be loaded.
    #[instrument(skip_all, fields(model = model_name))]
    #[must_use = "constructors return a new value"]
    pub fn new(model_name: &str, cache_dir: Option<&str>) -> Result<Self, IndexError> {
        let reranker_model = parse_reranker_model(model_name)?;

        let mut options = RerankInitOptions::new(reranker_model).with_show_download_progress(true);

        if let Some(dir) = cache_dir {
            options = options.with_cache_dir(PathBuf::from(dir));
        }

        let model = TextRerank::try_new(options).map_err(|e| IndexError::EmbeddingFailed {
            reason: format!("failed to initialize reranker '{model_name}': {e}"),
        })?;

        info!(model = model_name, "reranker model loaded");

        Ok(Self {
            model: Mutex::new(model),
        })
    }

    /// Create a `FastReranker` with a cache directory under the iris data directory.
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

impl Reranker for FastReranker {
    fn rerank(&self, query: &str, documents: &[&str]) -> Result<Vec<RerankScore>, IndexError> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }

        let docs: Vec<&str> = documents.to_vec();
        let mut model = self.model.lock().map_err(|e| IndexError::EmbeddingFailed {
            reason: format!("reranker lock poisoned: {e}"),
        })?;

        let results = model
            .rerank(query, docs, false, Some(DEFAULT_BATCH_SIZE))
            .map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("reranking inference failed: {e}"),
            })?;

        // Results from fastembed are already sorted by score descending
        Ok(results
            .into_iter()
            .map(|r| RerankScore {
                index: r.index,
                score: r.score,
            })
            .collect())
    }
}

/// Map a model name string to the corresponding `RerankerModel` enum variant.
fn parse_reranker_model(name: &str) -> Result<RerankerModel, IndexError> {
    match name {
        "bge-reranker-base" => Ok(RerankerModel::BGERerankerBase),
        "bge-reranker-v2-m3" => Ok(RerankerModel::BGERerankerV2M3),
        "jina-reranker-v1-turbo-en" => Ok(RerankerModel::JINARerankerV1TurboEn),
        "jina-reranker-v2-base-multilingual" => Ok(RerankerModel::JINARerankerV2BaseMultiligual),
        _ => Err(IndexError::EmbeddingFailed {
            reason: format!(
                "unknown reranker model '{name}'. Supported: \
                 bge-reranker-base, bge-reranker-v2-m3, \
                 jina-reranker-v1-turbo-en, jina-reranker-v2-base-multilingual"
            ),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_reranker_models() {
        assert!(parse_reranker_model("bge-reranker-base").is_ok());
        assert!(parse_reranker_model("bge-reranker-v2-m3").is_ok());
        assert!(parse_reranker_model("jina-reranker-v1-turbo-en").is_ok());
        assert!(parse_reranker_model("jina-reranker-v2-base-multilingual").is_ok());
    }

    #[test]
    fn parse_unknown_reranker_model_returns_error() {
        let err = parse_reranker_model("nonexistent-model").unwrap_err();
        assert!(err.to_string().contains("unknown reranker model"));
        assert!(err.to_string().contains("nonexistent-model"));
    }

    // Integration test: requires model download
    #[test]
    #[ignore = "requires reranker model download"]
    fn fast_reranker_produces_scores() {
        let reranker = FastReranker::new("bge-reranker-base", None).unwrap();
        let scores = reranker
            .rerank(
                "what is rust?",
                &[
                    "Rust is a systems programming language",
                    "Python is an interpreted language",
                    "Rust focuses on memory safety",
                ],
            )
            .unwrap();
        assert_eq!(scores.len(), 3);
        // Scores should be sorted descending
        assert!(scores[0].score >= scores[1].score);
        assert!(scores[1].score >= scores[2].score);
    }

    #[test]
    #[ignore = "requires reranker model download"]
    fn fast_reranker_empty_input() {
        let reranker = FastReranker::new("bge-reranker-base", None).unwrap();
        let scores = reranker.rerank("test query", &[]).unwrap();
        assert!(scores.is_empty());
    }
}
