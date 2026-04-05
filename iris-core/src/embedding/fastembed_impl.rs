//! [`FastEmbedder`] — local embedding via the `fastembed` ONNX runtime.
//!
//! Wraps [`fastembed::TextEmbedding`] with automatic model download, caching
//! under a configurable directory, and model selection by name string.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use tracing::{info, instrument};

use crate::error::IndexError;

use super::Embedder;

/// Metadata for a supported embedding model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    /// CLI/config name (e.g. `"jina-embeddings-v2-base-code"`).
    pub name: &'static str,
    /// Output vector dimensionality.
    pub dimension: usize,
    /// Short human-readable description.
    pub description: &'static str,
    /// Whether this model is optimized for source code.
    pub code_optimized: bool,
}

/// All embedding models supported by iris via fastembed.
///
/// This is the single source of truth for model names, dimensions, and
/// descriptions. Used by `parse_model_name()`, `model_dimension()`, the
/// CLI `list-models` command, and the MCP server.
pub static SUPPORTED_MODELS: &[ModelInfo] = &[
    // -- General-purpose (MiniLM) --
    ModelInfo {
        name: "all-MiniLM-L6-v2",
        dimension: 384,
        description: "Fast general-purpose (default)",
        code_optimized: false,
    },
    ModelInfo {
        name: "all-MiniLM-L6-v2-q",
        dimension: 384,
        description: "Quantized, faster/smaller",
        code_optimized: false,
    },
    ModelInfo {
        name: "all-MiniLM-L12-v2",
        dimension: 384,
        description: "Slightly higher quality than L6",
        code_optimized: false,
    },
    ModelInfo {
        name: "all-MiniLM-L12-v2-q",
        dimension: 384,
        description: "Quantized L12 variant",
        code_optimized: false,
    },
    ModelInfo {
        name: "all-mpnet-base-v2",
        dimension: 768,
        description: "Higher quality, slower than MiniLM",
        code_optimized: false,
    },
    // -- Paraphrase --
    ModelInfo {
        name: "paraphrase-ml-MiniLM-L12-v2",
        dimension: 384,
        description: "Multilingual paraphrase MiniLM",
        code_optimized: false,
    },
    ModelInfo {
        name: "paraphrase-ml-MiniLM-L12-v2-q",
        dimension: 384,
        description: "Quantized multilingual paraphrase",
        code_optimized: false,
    },
    ModelInfo {
        name: "paraphrase-ml-mpnet-base-v2",
        dimension: 768,
        description: "Multilingual paraphrase MPNet",
        code_optimized: false,
    },
    // -- BGE English --
    ModelInfo {
        name: "bge-small-en-v1.5",
        dimension: 384,
        description: "BAAI small English",
        code_optimized: false,
    },
    ModelInfo {
        name: "bge-small-en-v1.5-q",
        dimension: 384,
        description: "Quantized BAAI small",
        code_optimized: false,
    },
    ModelInfo {
        name: "bge-base-en-v1.5",
        dimension: 768,
        description: "BAAI base English",
        code_optimized: false,
    },
    ModelInfo {
        name: "bge-base-en-v1.5-q",
        dimension: 768,
        description: "Quantized BAAI base",
        code_optimized: false,
    },
    ModelInfo {
        name: "bge-large-en-v1.5",
        dimension: 1024,
        description: "BAAI large English",
        code_optimized: false,
    },
    ModelInfo {
        name: "bge-large-en-v1.5-q",
        dimension: 1024,
        description: "Quantized BAAI large",
        code_optimized: false,
    },
    // -- BGE Chinese --
    ModelInfo {
        name: "bge-small-zh-v1.5",
        dimension: 512,
        description: "BAAI small Chinese",
        code_optimized: false,
    },
    ModelInfo {
        name: "bge-large-zh-v1.5",
        dimension: 1024,
        description: "BAAI large Chinese",
        code_optimized: false,
    },
    // -- BGE M3 --
    ModelInfo {
        name: "bge-m3",
        dimension: 1024,
        description: "Multilingual, high quality",
        code_optimized: false,
    },
    // -- Nomic --
    ModelInfo {
        name: "nomic-embed-text-v1",
        dimension: 768,
        description: "Nomic open text embeddings",
        code_optimized: false,
    },
    ModelInfo {
        name: "nomic-embed-text-v1.5",
        dimension: 768,
        description: "Nomic v1.5, Matryoshka support",
        code_optimized: false,
    },
    ModelInfo {
        name: "nomic-embed-text-v1.5-q",
        dimension: 768,
        description: "Quantized Nomic v1.5",
        code_optimized: false,
    },
    // -- Jina --
    ModelInfo {
        name: "jina-embeddings-v2-base-en",
        dimension: 768,
        description: "Jina English, 8192 context",
        code_optimized: false,
    },
    ModelInfo {
        name: "jina-embeddings-v2-base-code",
        dimension: 768,
        description: "Jina code-specialized, 8192 context",
        code_optimized: true,
    },
    // -- GTE --
    ModelInfo {
        name: "gte-base-en-v1.5",
        dimension: 768,
        description: "GTE base English",
        code_optimized: false,
    },
    ModelInfo {
        name: "gte-base-en-v1.5-q",
        dimension: 768,
        description: "Quantized GTE base",
        code_optimized: false,
    },
    ModelInfo {
        name: "gte-large-en-v1.5",
        dimension: 1024,
        description: "GTE large English",
        code_optimized: false,
    },
    ModelInfo {
        name: "gte-large-en-v1.5-q",
        dimension: 1024,
        description: "Quantized GTE large",
        code_optimized: false,
    },
    // -- Multilingual E5 --
    ModelInfo {
        name: "multilingual-e5-small",
        dimension: 384,
        description: "Multilingual E5 small",
        code_optimized: false,
    },
    ModelInfo {
        name: "multilingual-e5-base",
        dimension: 768,
        description: "Multilingual E5 base",
        code_optimized: false,
    },
    ModelInfo {
        name: "multilingual-e5-large",
        dimension: 1024,
        description: "Multilingual E5 large",
        code_optimized: false,
    },
    // -- MxBai --
    ModelInfo {
        name: "mxbai-embed-large-v1",
        dimension: 1024,
        description: "MixedBread large",
        code_optimized: false,
    },
    ModelInfo {
        name: "mxbai-embed-large-v1-q",
        dimension: 1024,
        description: "Quantized MixedBread large",
        code_optimized: false,
    },
    // -- Snowflake Arctic --
    ModelInfo {
        name: "snowflake-arctic-embed-xs",
        dimension: 384,
        description: "Arctic extra-small",
        code_optimized: false,
    },
    ModelInfo {
        name: "snowflake-arctic-embed-xs-q",
        dimension: 384,
        description: "Quantized Arctic XS",
        code_optimized: false,
    },
    ModelInfo {
        name: "snowflake-arctic-embed-s",
        dimension: 384,
        description: "Arctic small",
        code_optimized: false,
    },
    ModelInfo {
        name: "snowflake-arctic-embed-s-q",
        dimension: 384,
        description: "Quantized Arctic small",
        code_optimized: false,
    },
    ModelInfo {
        name: "snowflake-arctic-embed-m",
        dimension: 768,
        description: "Arctic medium",
        code_optimized: false,
    },
    ModelInfo {
        name: "snowflake-arctic-embed-m-q",
        dimension: 768,
        description: "Quantized Arctic medium",
        code_optimized: false,
    },
    ModelInfo {
        name: "snowflake-arctic-embed-m-long",
        dimension: 768,
        description: "Arctic medium, long context",
        code_optimized: false,
    },
    ModelInfo {
        name: "snowflake-arctic-embed-m-long-q",
        dimension: 768,
        description: "Quantized Arctic medium long",
        code_optimized: false,
    },
    ModelInfo {
        name: "snowflake-arctic-embed-l",
        dimension: 1024,
        description: "Arctic large",
        code_optimized: false,
    },
    ModelInfo {
        name: "snowflake-arctic-embed-l-q",
        dimension: 1024,
        description: "Quantized Arctic large",
        code_optimized: false,
    },
    // -- Modern BERT --
    ModelInfo {
        name: "modern-bert-embed-large",
        dimension: 1024,
        description: "ModernBERT large",
        code_optimized: false,
    },
    // -- Gemma --
    ModelInfo {
        name: "embedding-gemma-300m",
        dimension: 768,
        description: "Gemma 300M embedding",
        code_optimized: false,
    },
];

/// Return the full list of supported embedding models.
///
/// This is useful for CLI help text and MCP tool responses.
#[must_use]
pub fn supported_models() -> &'static [ModelInfo] {
    SUPPORTED_MODELS
}

/// Batch size for embedding inference.
///
/// Controls the internal batch size passed to ONNX Runtime via fastembed.
/// With the `CoreML` ANE leak fixed (default `CPUAndGPU`), larger batches
/// are safe and significantly improve throughput.
const DEFAULT_BATCH_SIZE: usize = 128;

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
    /// Use [`supported_models()`] for the full programmatic list. Key models:
    ///
    /// - `"all-MiniLM-L6-v2"` — 384d, fast general-purpose (default)
    /// - `"jina-embeddings-v2-base-code"` — 768d, code-specialized
    /// - `"nomic-embed-text-v1.5"` — 768d, Matryoshka support
    /// - `"bge-m3"` — 1024d, multilingual
    /// - `"gte-large-en-v1.5"` — 1024d, high quality
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the model name is unknown or
    /// the model cannot be loaded.
    #[instrument(skip_all, fields(model = model_name))]
    #[must_use = "constructors return a new value"]
    pub fn new(model_name: &str, cache_dir: Option<&str>) -> Result<Self, IndexError> {
        let embedding_model = parse_model_name(model_name)?;
        let dim = model_dimension(&embedding_model);

        let mut options = InitOptions::new(embedding_model).with_show_download_progress(true);

        if let Some(dir) = cache_dir {
            options = options.with_cache_dir(PathBuf::from(dir));
        }

        // CoreML on macOS: enabled by default with CPUAndGPU compute units.
        //
        // The Neural Engine (ANE) path leaks ~12 GB per inference batch due to
        // an Apple-side memory management bug in the CoreML/ANE bridge.
        // See: https://github.com/microsoft/onnxruntime/issues/14455
        //
        // Override via IRIS_COMPUTE_UNITS: "cpu_and_gpu" (default), "cpu_only",
        // "cpu_and_ane", or "all". Set IRIS_COREML=0 to disable CoreML entirely.
        #[cfg(target_os = "macos")]
        {
            let coreml_disabled =
                std::env::var("IRIS_COREML").is_ok_and(|v| v == "0" || v == "false");

            if coreml_disabled {
                info!("CoreML disabled (IRIS_COREML=0), using CPU execution provider");
            } else {
                let compute_units = match std::env::var("IRIS_COMPUTE_UNITS")
                    .unwrap_or_default()
                    .as_str()
                {
                    "cpu_only" => ort::ep::coreml::ComputeUnits::CPUOnly,
                    "cpu_and_ane" => ort::ep::coreml::ComputeUnits::CPUAndNeuralEngine,
                    "all" => ort::ep::coreml::ComputeUnits::All,
                    // Default: CPU+GPU avoids the ANE memory leak while still
                    // getting Metal GPU acceleration on Apple Silicon.
                    _ => ort::ep::coreml::ComputeUnits::CPUAndGPU,
                };

                let mut coreml = ort::ep::CoreML::default()
                    .with_compute_units(compute_units)
                    .with_subgraphs(true)
                    .with_static_input_shapes(true)
                    .with_model_format(ort::ep::coreml::ModelFormat::MLProgram);

                // Cache compiled CoreML models to avoid recompilation each session.
                if let Some(dir) = cache_dir {
                    let coreml_cache = PathBuf::from(dir).join("coreml_cache");
                    coreml = coreml.with_model_cache_dir(coreml_cache.to_string_lossy());
                }

                options = options.with_execution_providers(vec![coreml.build()]);
                info!(?compute_units, "CoreML execution provider enabled");
            }
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
    #[must_use = "constructors return a new value"]
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

/// Wrapper that truncates embeddings to a lower dimensionality.
///
/// Useful for Matryoshka-capable models (e.g. `nomic-embed-text-v1.5`) where
/// the first `N` dimensions of a full-size embedding retain most of the
/// semantic information. After truncation, vectors are L2-normalized so
/// cosine similarity remains well-calibrated.
///
/// # Examples
///
/// ```no_run
/// use iris_core::embedding::{Embedder, FastEmbedder, TruncatingEmbedder};
/// use std::sync::Arc;
///
/// let inner = Arc::new(FastEmbedder::new("nomic-embed-text-v1.5", None)?);
/// let truncated = TruncatingEmbedder::new(inner, 256)?;
/// assert_eq!(truncated.dimension(), 256);
/// # Ok::<(), iris_core::error::IndexError>(())
/// ```
pub struct TruncatingEmbedder {
    inner: Arc<dyn Embedder>,
    target_dim: usize,
}

impl std::fmt::Debug for TruncatingEmbedder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TruncatingEmbedder")
            .field("target_dim", &self.target_dim)
            .field("inner_dim", &self.inner.dimension())
            .finish()
    }
}

impl TruncatingEmbedder {
    /// Create a truncating wrapper around an existing embedder.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if `target_dim` exceeds the
    /// inner embedder's dimension.
    #[must_use = "constructors return a new value"]
    pub fn new(inner: Arc<dyn Embedder>, target_dim: usize) -> Result<Self, IndexError> {
        if target_dim > inner.dimension() {
            return Err(IndexError::EmbeddingFailed {
                reason: format!(
                    "target dimension {target_dim} exceeds model dimension {}",
                    inner.dimension()
                ),
            });
        }
        Ok(Self { inner, target_dim })
    }
}

impl Embedder for TruncatingEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        let mut vectors = self.inner.embed(texts)?;
        if self.target_dim == self.inner.dimension() {
            return Ok(vectors);
        }
        for vec in &mut vectors {
            Vec::truncate(vec, self.target_dim);
            // L2-normalize after truncation so cosine similarity stays calibrated.
            let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm > 0.0 {
                for x in vec.iter_mut() {
                    *x /= norm;
                }
            }
        }
        Ok(vectors)
    }

    fn dimension(&self) -> usize {
        self.target_dim
    }
}

/// Map a model name string to the corresponding `EmbeddingModel` enum variant.
fn parse_model_name(name: &str) -> Result<EmbeddingModel, IndexError> {
    match name {
        "all-MiniLM-L6-v2" => Ok(EmbeddingModel::AllMiniLML6V2),
        "all-MiniLM-L6-v2-q" => Ok(EmbeddingModel::AllMiniLML6V2Q),
        "all-MiniLM-L12-v2" => Ok(EmbeddingModel::AllMiniLML12V2),
        "all-MiniLM-L12-v2-q" => Ok(EmbeddingModel::AllMiniLML12V2Q),
        "all-mpnet-base-v2" => Ok(EmbeddingModel::AllMpnetBaseV2),
        "paraphrase-ml-MiniLM-L12-v2" => Ok(EmbeddingModel::ParaphraseMLMiniLML12V2),
        "paraphrase-ml-MiniLM-L12-v2-q" => Ok(EmbeddingModel::ParaphraseMLMiniLML12V2Q),
        "paraphrase-ml-mpnet-base-v2" => Ok(EmbeddingModel::ParaphraseMLMpnetBaseV2),
        "bge-small-en-v1.5" => Ok(EmbeddingModel::BGESmallENV15),
        "bge-small-en-v1.5-q" => Ok(EmbeddingModel::BGESmallENV15Q),
        "bge-base-en-v1.5" => Ok(EmbeddingModel::BGEBaseENV15),
        "bge-base-en-v1.5-q" => Ok(EmbeddingModel::BGEBaseENV15Q),
        "bge-large-en-v1.5" => Ok(EmbeddingModel::BGELargeENV15),
        "bge-large-en-v1.5-q" => Ok(EmbeddingModel::BGELargeENV15Q),
        "bge-small-zh-v1.5" => Ok(EmbeddingModel::BGESmallZHV15),
        "bge-large-zh-v1.5" => Ok(EmbeddingModel::BGELargeZHV15),
        "bge-m3" => Ok(EmbeddingModel::BGEM3),
        "nomic-embed-text-v1" => Ok(EmbeddingModel::NomicEmbedTextV1),
        "nomic-embed-text-v1.5" => Ok(EmbeddingModel::NomicEmbedTextV15),
        "nomic-embed-text-v1.5-q" => Ok(EmbeddingModel::NomicEmbedTextV15Q),
        "jina-embeddings-v2-base-en" => Ok(EmbeddingModel::JinaEmbeddingsV2BaseEN),
        "jina-embeddings-v2-base-code" => Ok(EmbeddingModel::JinaEmbeddingsV2BaseCode),
        "gte-base-en-v1.5" => Ok(EmbeddingModel::GTEBaseENV15),
        "gte-base-en-v1.5-q" => Ok(EmbeddingModel::GTEBaseENV15Q),
        "gte-large-en-v1.5" => Ok(EmbeddingModel::GTELargeENV15),
        "gte-large-en-v1.5-q" => Ok(EmbeddingModel::GTELargeENV15Q),
        "multilingual-e5-small" => Ok(EmbeddingModel::MultilingualE5Small),
        "multilingual-e5-base" => Ok(EmbeddingModel::MultilingualE5Base),
        "multilingual-e5-large" => Ok(EmbeddingModel::MultilingualE5Large),
        "mxbai-embed-large-v1" => Ok(EmbeddingModel::MxbaiEmbedLargeV1),
        "mxbai-embed-large-v1-q" => Ok(EmbeddingModel::MxbaiEmbedLargeV1Q),
        "snowflake-arctic-embed-xs" => Ok(EmbeddingModel::SnowflakeArcticEmbedXS),
        "snowflake-arctic-embed-xs-q" => Ok(EmbeddingModel::SnowflakeArcticEmbedXSQ),
        "snowflake-arctic-embed-s" => Ok(EmbeddingModel::SnowflakeArcticEmbedS),
        "snowflake-arctic-embed-s-q" => Ok(EmbeddingModel::SnowflakeArcticEmbedSQ),
        "snowflake-arctic-embed-m" => Ok(EmbeddingModel::SnowflakeArcticEmbedM),
        "snowflake-arctic-embed-m-q" => Ok(EmbeddingModel::SnowflakeArcticEmbedMQ),
        "snowflake-arctic-embed-m-long" => Ok(EmbeddingModel::SnowflakeArcticEmbedMLong),
        "snowflake-arctic-embed-m-long-q" => Ok(EmbeddingModel::SnowflakeArcticEmbedMLongQ),
        "snowflake-arctic-embed-l" => Ok(EmbeddingModel::SnowflakeArcticEmbedL),
        "snowflake-arctic-embed-l-q" => Ok(EmbeddingModel::SnowflakeArcticEmbedLQ),
        "modern-bert-embed-large" => Ok(EmbeddingModel::ModernBertEmbedLarge),
        "embedding-gemma-300m" => Ok(EmbeddingModel::EmbeddingGemma300M),
        _ => {
            let names: Vec<&str> = SUPPORTED_MODELS.iter().map(|m| m.name).collect();
            Err(IndexError::EmbeddingFailed {
                reason: format!(
                    "unknown embedding model '{name}'. Supported models: {}",
                    names.join(", ")
                ),
            })
        }
    }
}

/// Return the output dimension for a known embedding model.
///
/// Looks up the dimension from [`SUPPORTED_MODELS`]. Falls back to the
/// fastembed default for any unrecognized variant (should not happen if
/// `parse_model_name` is the only entry point).
fn model_dimension(model: &EmbeddingModel) -> usize {
    // Map EmbeddingModel enum back to its name and look up in SUPPORTED_MODELS.
    // This keeps SUPPORTED_MODELS as the single source of truth.
    match model {
        // 384-dimensional models
        EmbeddingModel::AllMiniLML6V2
        | EmbeddingModel::AllMiniLML6V2Q
        | EmbeddingModel::AllMiniLML12V2
        | EmbeddingModel::AllMiniLML12V2Q
        | EmbeddingModel::ParaphraseMLMiniLML12V2
        | EmbeddingModel::ParaphraseMLMiniLML12V2Q
        | EmbeddingModel::BGESmallENV15
        | EmbeddingModel::BGESmallENV15Q
        | EmbeddingModel::MultilingualE5Small
        | EmbeddingModel::SnowflakeArcticEmbedXS
        | EmbeddingModel::SnowflakeArcticEmbedXSQ
        | EmbeddingModel::SnowflakeArcticEmbedS
        | EmbeddingModel::SnowflakeArcticEmbedSQ => 384,

        // 512-dimensional models
        EmbeddingModel::BGESmallZHV15 | EmbeddingModel::ClipVitB32 => 512,

        // 768-dimensional models
        EmbeddingModel::AllMpnetBaseV2
        | EmbeddingModel::ParaphraseMLMpnetBaseV2
        | EmbeddingModel::BGEBaseENV15
        | EmbeddingModel::BGEBaseENV15Q
        | EmbeddingModel::NomicEmbedTextV1
        | EmbeddingModel::NomicEmbedTextV15
        | EmbeddingModel::NomicEmbedTextV15Q
        | EmbeddingModel::JinaEmbeddingsV2BaseEN
        | EmbeddingModel::JinaEmbeddingsV2BaseCode
        | EmbeddingModel::GTEBaseENV15
        | EmbeddingModel::GTEBaseENV15Q
        | EmbeddingModel::MultilingualE5Base
        | EmbeddingModel::SnowflakeArcticEmbedM
        | EmbeddingModel::SnowflakeArcticEmbedMQ
        | EmbeddingModel::SnowflakeArcticEmbedMLong
        | EmbeddingModel::SnowflakeArcticEmbedMLongQ
        | EmbeddingModel::EmbeddingGemma300M => 768,

        // 1024-dimensional models
        EmbeddingModel::BGELargeENV15
        | EmbeddingModel::BGELargeENV15Q
        | EmbeddingModel::BGELargeZHV15
        | EmbeddingModel::BGEM3
        | EmbeddingModel::GTELargeENV15
        | EmbeddingModel::GTELargeENV15Q
        | EmbeddingModel::MultilingualE5Large
        | EmbeddingModel::MxbaiEmbedLargeV1
        | EmbeddingModel::MxbaiEmbedLargeV1Q
        | EmbeddingModel::SnowflakeArcticEmbedL
        | EmbeddingModel::SnowflakeArcticEmbedLQ
        | EmbeddingModel::ModernBertEmbedLarge => 1024,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_all_supported_models() {
        for info in SUPPORTED_MODELS {
            let result = parse_model_name(info.name);
            assert!(
                result.is_ok(),
                "parse_model_name({}) should succeed",
                info.name
            );
        }
    }

    #[test]
    fn parse_unknown_model_returns_error() {
        let err = parse_model_name("nonexistent-model").unwrap_err();
        assert!(err.to_string().contains("unknown embedding model"));
        assert!(err.to_string().contains("nonexistent-model"));
    }

    #[test]
    fn dimensions_match_supported_models_table() {
        for info in SUPPORTED_MODELS {
            let model = parse_model_name(info.name).unwrap();
            let dim = model_dimension(&model);
            assert_eq!(
                dim, info.dimension,
                "dimension mismatch for {}: model_dimension()={dim}, SUPPORTED_MODELS.dimension={}",
                info.name, info.dimension
            );
        }
    }

    #[test]
    fn supported_models_not_empty() {
        assert!(SUPPORTED_MODELS.len() >= 10);
    }

    #[test]
    fn code_optimized_models_exist() {
        let code_models: Vec<_> = SUPPORTED_MODELS
            .iter()
            .filter(|m| m.code_optimized)
            .collect();
        assert!(
            !code_models.is_empty(),
            "at least one code-optimized model should exist"
        );
        assert!(
            code_models
                .iter()
                .any(|m| m.name == "jina-embeddings-v2-base-code"),
            "jina-embeddings-v2-base-code should be code-optimized"
        );
    }

    #[test]
    fn supported_models_function_returns_same_slice() {
        assert_eq!(supported_models().len(), SUPPORTED_MODELS.len());
    }

    #[test]
    fn model_dimensions_key_models() {
        assert_eq!(model_dimension(&EmbeddingModel::AllMiniLML6V2), 384);
        assert_eq!(model_dimension(&EmbeddingModel::BGEBaseENV15), 768);
        assert_eq!(model_dimension(&EmbeddingModel::BGELargeENV15), 1024);
        assert_eq!(
            model_dimension(&EmbeddingModel::JinaEmbeddingsV2BaseCode),
            768
        );
        assert_eq!(model_dimension(&EmbeddingModel::NomicEmbedTextV15), 768);
        assert_eq!(model_dimension(&EmbeddingModel::BGEM3), 1024);
        assert_eq!(model_dimension(&EmbeddingModel::BGESmallZHV15), 512);
    }

    // Integration test: requires model download, so only run with --ignored
    // -- TruncatingEmbedder tests --

    /// Mock embedder producing predictable vectors for truncation tests.
    struct FixedEmbedder {
        dim: usize,
    }

    #[allow(clippy::cast_precision_loss)]
    impl Embedder for FixedEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
            Ok(texts
                .iter()
                .enumerate()
                .map(|(i, _)| vec![(i + 1) as f32; self.dim])
                .collect())
        }

        fn dimension(&self) -> usize {
            self.dim
        }
    }

    #[test]
    fn truncating_embedder_reduces_dimension() {
        let inner = Arc::new(FixedEmbedder { dim: 768 });
        let truncated = TruncatingEmbedder::new(inner, 256).unwrap();
        assert_eq!(truncated.dimension(), 256);

        let vectors = truncated.embed(&["hello"]).unwrap();
        assert_eq!(vectors[0].len(), 256);
    }

    #[test]
    fn truncating_embedder_normalizes_output() {
        let inner = Arc::new(FixedEmbedder { dim: 768 });
        let truncated = TruncatingEmbedder::new(inner, 256).unwrap();

        let vectors = truncated.embed(&["hello"]).unwrap();
        let norm: f32 = vectors[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "truncated vector should be L2-normalized, got norm={norm}"
        );
    }

    #[test]
    fn truncating_embedder_passthrough() {
        let inner = Arc::new(FixedEmbedder { dim: 384 });
        let truncated = TruncatingEmbedder::new(inner, 384).unwrap();
        assert_eq!(truncated.dimension(), 384);

        let vectors = truncated.embed(&["hello"]).unwrap();
        assert_eq!(vectors[0].len(), 384);
        // Passthrough should NOT normalize (no truncation happened).
        // The vector should be raw (all 1.0 values).
        assert!((vectors[0][0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn truncating_embedder_rejects_larger_target() {
        let inner = Arc::new(FixedEmbedder { dim: 384 });
        let err = TruncatingEmbedder::new(inner, 768).unwrap_err();
        assert!(err.to_string().contains("exceeds model dimension"));
    }

    #[test]
    fn truncating_embedder_empty_input() {
        let inner = Arc::new(FixedEmbedder { dim: 768 });
        let truncated = TruncatingEmbedder::new(inner, 256).unwrap();
        let vectors = truncated.embed(&[]).unwrap();
        assert!(vectors.is_empty());
    }

    // Integration tests: require model download
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
