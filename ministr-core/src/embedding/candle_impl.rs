//! Candle-based embedding backend with native Metal GPU acceleration.
//!
//! Uses HuggingFace's [`candle`](https://github.com/huggingface/candle) framework
//! for BERT-family embedding models. On Apple Silicon, this runs inference
//! directly on the Metal GPU — significantly faster than the ONNX/CoreML path
//! used by [`FastEmbedder`](super::FastEmbedder).
//!
//! # Feature Flag
//!
//! This module is only available when the `candle` feature is enabled:
//!
//! ```toml
//! ministr-core = { version = "0.1", features = ["candle"] }
//! ```

use std::path::{Path, PathBuf};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::Config as BertConfig;
use hf_hub::api::sync::ApiBuilder;
use parking_lot::Mutex;
use tokenizers::Tokenizer;
use tracing::{info, instrument};

use super::Embedder;
use super::metal_bert::MetalBertModel;
use crate::error::IndexError;

/// Maximum batch size for Candle inference.
///
/// Candle's Metal backend handles large batches well, but we cap to avoid
/// excessive memory allocation for very large ingestion runs.
const MAX_BATCH_SIZE: usize = 256;

/// Supported Candle embedding models and their dimensions.
const CANDLE_MODELS: &[CandleModelInfo] = &[
    CandleModelInfo {
        name: "all-MiniLM-L6-v2",
        repo_id: "sentence-transformers/all-MiniLM-L6-v2",
        dimension: 384,
        description: "Fast general-purpose 384d model (Candle Metal)",
    },
    CandleModelInfo {
        name: "bge-small-en-v1.5",
        repo_id: "BAAI/bge-small-en-v1.5",
        dimension: 384,
        description: "BAAI BGE small English 384d (Candle Metal)",
    },
    CandleModelInfo {
        name: "bge-base-en-v1.5",
        repo_id: "BAAI/bge-base-en-v1.5",
        dimension: 768,
        description: "BAAI BGE base English 768d (Candle Metal)",
    },
    CandleModelInfo {
        name: "bge-large-en-v1.5",
        repo_id: "BAAI/bge-large-en-v1.5",
        dimension: 1024,
        description: "BAAI BGE large English 1024d (Candle Metal)",
    },
    CandleModelInfo {
        name: "nomic-embed-text-v1.5",
        repo_id: "nomic-ai/nomic-embed-text-v1.5",
        dimension: 768,
        description: "Nomic 768d with Matryoshka support (Candle Metal)",
    },
];

/// Metadata for a supported Candle embedding model.
#[derive(Debug, Clone)]
pub struct CandleModelInfo {
    /// CLI/config name (matches fastembed names where possible).
    pub name: &'static str,
    /// HuggingFace Hub repository ID.
    pub repo_id: &'static str,
    /// Output embedding dimension.
    pub dimension: usize,
    /// Human-readable description.
    pub description: &'static str,
}

/// Return the list of embedding models supported by the Candle backend.
#[must_use]
pub fn candle_supported_models() -> &'static [CandleModelInfo] {
    CANDLE_MODELS
}

/// Check if a model name is supported by the Candle backend.
#[must_use]
pub fn is_candle_model(name: &str) -> bool {
    CANDLE_MODELS.iter().any(|m| m.name == name)
}

/// Look up model info by name.
fn find_model(name: &str) -> Result<&'static CandleModelInfo, IndexError> {
    CANDLE_MODELS
        .iter()
        .find(|m| m.name == name)
        .ok_or_else(|| IndexError::EmbeddingFailed {
            reason: format!(
                "unknown candle model '{name}'. Supported: {}",
                CANDLE_MODELS
                    .iter()
                    .map(|m| m.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        })
}

/// Select the best available device for inference.
///
/// Prefers Metal on macOS, falls back to CPU.
fn select_device() -> Result<Device, IndexError> {
    #[cfg(target_os = "macos")]
    {
        if candle_core::utils::metal_is_available() {
            info!("Candle: using Metal GPU device");
            return Device::new_metal(0).map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("failed to create Metal device: {e}"),
            });
        }
    }
    info!("Candle: using CPU device");
    Ok(Device::Cpu)
}

/// Embedding model backed by HuggingFace Candle with native Metal GPU acceleration.
///
/// On Apple Silicon Macs, inference runs directly on the GPU via Metal — typically
/// 7-12x faster than the ONNX/CoreML path for batch embedding.
///
/// Uses an interior `Mutex` because `BertModel::forward` requires `&self` but
/// we need exclusive access for the tokenizer's internal state during batch
/// encoding.
///
/// # Examples
///
/// ```no_run
/// use ministr_core::embedding::{Embedder, CandleEmbedder};
///
/// let embedder = CandleEmbedder::new("all-MiniLM-L6-v2", None)?;
/// let vectors = embedder.embed(&["hello world"])?;
/// assert_eq!(vectors[0].len(), 384);
/// # Ok::<(), ministr_core::error::IndexError>(())
/// ```
pub struct CandleEmbedder {
    model: MetalBertModel,
    tokenizer: Mutex<Tokenizer>,
    device: Device,
    dim: usize,
}

impl CandleEmbedder {
    /// Create a new `CandleEmbedder` for the given model.
    ///
    /// Downloads model weights and tokenizer from HuggingFace Hub on first use.
    /// Subsequent runs use cached files under `cache_dir` (or the HF default cache).
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the model name is unknown,
    /// download fails, or model loading fails.
    #[instrument(skip_all, fields(model = model_name))]
    #[must_use = "constructors return a new value"]
    pub fn new(model_name: &str, cache_dir: Option<&str>) -> Result<Self, IndexError> {
        let model_info = find_model(model_name)?;
        let device = select_device()?;

        // Set up HuggingFace Hub API with optional cache directory.
        let mut builder = ApiBuilder::new();
        if let Some(dir) = cache_dir {
            builder = builder.with_cache_dir(PathBuf::from(dir));
        }
        let api = builder.build().map_err(|e| IndexError::EmbeddingFailed {
            reason: format!("failed to initialize HuggingFace Hub API: {e}"),
        })?;

        let repo = api.model(model_info.repo_id.to_string());

        // Download model files.
        info!(repo_id = model_info.repo_id, "downloading model files");
        let config_path = repo
            .get("config.json")
            .map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("failed to download config.json: {e}"),
            })?;
        let tokenizer_path =
            repo.get("tokenizer.json")
                .map_err(|e| IndexError::EmbeddingFailed {
                    reason: format!("failed to download tokenizer.json: {e}"),
                })?;
        let weights_path = Self::download_weights(&repo)?;

        // Load config.
        let config: BertConfig =
            serde_json::from_reader(std::fs::File::open(&config_path).map_err(|e| {
                IndexError::EmbeddingFailed {
                    reason: format!("failed to open config.json: {e}"),
                }
            })?)
            .map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("failed to parse config.json: {e}"),
            })?;

        // Load tokenizer.
        let tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("failed to load tokenizer: {e}"),
            })?;

        // Load model weights (safe — reads entire file into memory, no mmap).
        info!("loading model weights onto {:?}", device);
        let weights_data =
            std::fs::read(&weights_path).map_err(|e| IndexError::EmbeddingFailed {
                reason: format!("failed to read model weights: {e}"),
            })?;
        let vb = VarBuilder::from_buffered_safetensors(weights_data, DType::F32, &device).map_err(
            |e| IndexError::EmbeddingFailed {
                reason: format!("failed to load model weights: {e}"),
            },
        )?;

        let model = MetalBertModel::load(vb, &config).map_err(|e| IndexError::EmbeddingFailed {
            reason: format!("failed to load BERT model: {e}"),
        })?;

        info!(
            model = model_name,
            dim = model_info.dimension,
            device = ?device,
            "Candle embedding model loaded"
        );

        Ok(Self {
            model,
            tokenizer: Mutex::new(tokenizer),
            device,
            dim: model_info.dimension,
        })
    }

    /// Create a `CandleEmbedder` with cache under the ministr data directory.
    ///
    /// Resolves the cache path as `{data_dir}/models/candle/`.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the model cannot be loaded.
    #[must_use = "constructors return a new value"]
    pub fn with_data_dir(model_name: &str, data_dir: &Path) -> Result<Self, IndexError> {
        let cache_dir = data_dir.join("models").join("candle");
        std::fs::create_dir_all(&cache_dir).map_err(|e| IndexError::EmbeddingFailed {
            reason: format!("failed to create candle cache dir: {e}"),
        })?;
        Self::new(model_name, Some(&cache_dir.to_string_lossy()))
    }

    /// Try downloading model.safetensors first, fall back to pytorch_model.bin → convert.
    fn download_weights(repo: &hf_hub::api::sync::ApiRepo) -> Result<PathBuf, IndexError> {
        // Try safetensors first (preferred — zero-copy mmap).
        if let Ok(path) = repo.get("model.safetensors") {
            return Ok(path);
        }
        // Some models split weights across multiple shards.
        if let Ok(path) = repo.get("model.safetensors.index.json") {
            // For sharded models, return the index — VarBuilder handles it.
            return Ok(path);
        }
        Err(IndexError::EmbeddingFailed {
            reason: "model.safetensors not found in repository".to_string(),
        })
    }

    /// Run mean pooling over BERT output with attention mask.
    fn mean_pool(embeddings: &Tensor, attention_mask: &Tensor) -> Result<Tensor, IndexError> {
        // embeddings: [batch, seq_len, hidden]
        // attention_mask: [batch, seq_len]
        let (batch, _seq_len, hidden) = embeddings.dims3().map_err(candle_err)?;

        let mask_f32 = attention_mask.to_dtype(DType::F32).map_err(candle_err)?;

        // Expand mask to [batch, seq_len, hidden] to match embeddings shape.
        // Candle's Metal backend doesn't auto-broadcast [b,s,1] * [b,s,h].
        let mask_expanded = mask_f32
            .unsqueeze(2)
            .map_err(candle_err)?
            .broadcast_as(embeddings.shape())
            .map_err(candle_err)?;

        // Masked sum over seq_len dimension.
        let sum = embeddings
            .mul(&mask_expanded)
            .map_err(candle_err)?
            .sum(1)
            .map_err(candle_err)?;

        // Count non-masked tokens per batch item: [batch, 1] → [batch, hidden].
        let count = mask_f32
            .sum(1)
            .map_err(candle_err)?
            .unsqueeze(1)
            .map_err(candle_err)?
            .broadcast_as(&[batch, hidden])
            .map_err(candle_err)?
            .clamp(1e-9, f64::MAX)
            .map_err(candle_err)?;

        sum.div(&count).map_err(candle_err)
    }

    /// L2-normalize a batch of vectors.
    fn l2_normalize(tensor: &Tensor) -> Result<Tensor, IndexError> {
        // tensor: [batch, hidden]
        let norm = tensor
            .sqr()
            .map_err(candle_err)?
            .sum_keepdim(1)
            .map_err(candle_err)?
            .sqrt()
            .map_err(candle_err)?
            .clamp(1e-12, f64::MAX)
            .map_err(candle_err)?
            .broadcast_as(tensor.shape())
            .map_err(candle_err)?;
        tensor.div(&norm).map_err(candle_err)
    }
}

/// Convert a Candle error into an `IndexError`.
#[allow(clippy::needless_pass_by_value)]
fn candle_err(e: candle_core::Error) -> IndexError {
    IndexError::EmbeddingFailed {
        reason: format!("candle inference error: {e}"),
    }
}

impl Embedder for CandleEmbedder {
    #[instrument(skip(self, texts), fields(count = texts.len()))]
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        // Length-sorted batching: group nearby-length texts so each forward pads
        // to a similar (shorter) max sequence length instead of the global
        // longest, cutting the padding waste that dominates batched transformer
        // inference on heterogeneous corpora (short claims + long code sections).
        // This mirrors what the sentence-transformers reference does internally.
        // The per-text embedding is independent of its batchmates (masked
        // mean-pool over a per-sequence BERT forward), so this changes only
        // padding — never the returned vectors — and output is restored to the
        // caller's input order.
        embed_length_sorted(texts, MAX_BATCH_SIZE, |chunk| self.embed_batch(chunk))
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

impl CandleEmbedder {
    /// Embed a single batch (up to `MAX_BATCH_SIZE` texts).
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        let device = &self.device;

        // Tokenize all texts (needs mutable tokenizer for internal state).
        let t_tokenize = std::time::Instant::now();
        let tokenizer = self.tokenizer.lock();
        let encodings = tokenizer.encode_batch(texts.to_vec(), true).map_err(|e| {
            IndexError::EmbeddingFailed {
                reason: format!("tokenization failed: {e}"),
            }
        })?;
        drop(tokenizer);
        let tokenize_ms = t_tokenize.elapsed().as_secs_f64() * 1000.0;

        // Find max sequence length for padding.
        let max_len = encodings
            .iter()
            .map(|e| e.get_ids().len())
            .max()
            .unwrap_or(0);

        // Build padded input tensors.
        let mut all_input_ids = Vec::with_capacity(texts.len() * max_len);
        let mut all_type_ids = Vec::with_capacity(texts.len() * max_len);
        let mut all_attention_mask = Vec::with_capacity(texts.len() * max_len);

        for encoding in &encodings {
            let ids = encoding.get_ids();
            let type_ids = encoding.get_type_ids();
            let mask = encoding.get_attention_mask();
            let seq_len = ids.len();

            all_input_ids.extend(ids.iter().copied());
            all_type_ids.extend(type_ids.iter().copied());
            all_attention_mask.extend(mask.iter().copied());

            // Pad to max_len.
            let padding = max_len - seq_len;
            all_input_ids.extend(std::iter::repeat_n(0u32, padding));
            all_type_ids.extend(std::iter::repeat_n(0u32, padding));
            all_attention_mask.extend(std::iter::repeat_n(0u32, padding));
        }

        let batch_size = texts.len();
        let shape = (batch_size, max_len);

        // Time the GPU section (tensor upload → forward → pool → normalize →
        // download). `to_vec2` forces a Metal sync, so this captures true GPU
        // wall time rather than the async-dispatch time `forward` alone would
        // show. Paired with `tokenize_ms` + `batch`/`max_len` it pinpoints where
        // an embed batch actually spends its ~ms (embed-throughput profiling —
        // is the cost tokenization, padding/forward, or fixed per-call overhead?).
        let t_compute = std::time::Instant::now();

        let input_ids = Tensor::from_vec(all_input_ids, shape, device).map_err(candle_err)?;
        let token_type_ids = Tensor::from_vec(all_type_ids, shape, device).map_err(candle_err)?;
        let attention_mask =
            Tensor::from_vec(all_attention_mask, shape, device).map_err(candle_err)?;

        // Forward pass.
        let output = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))
            .map_err(candle_err)?;

        // Mean pooling + L2 normalization.
        let pooled = Self::mean_pool(&output, &attention_mask)?;
        let normalized = Self::l2_normalize(&pooled)?;

        // Convert to Vec<Vec<f32>> (downloads from the GPU, forcing a sync).
        let vectors = normalized
            .to_dtype(DType::F32)
            .map_err(candle_err)?
            .to_vec2()
            .map_err(candle_err)?;

        let compute_ms = t_compute.elapsed().as_secs_f64() * 1000.0;
        tracing::debug!(
            batch = batch_size,
            max_len,
            tokenize_ms,
            compute_ms,
            "candle embed_batch timing"
        );

        Ok(vectors)
    }
}

/// Order `texts` into length-sorted batches, embed each batch via `embed_batch`,
/// and scatter the results back into the caller's original order.
///
/// Sorting by length means each batch pads to a similar, shorter max sequence
/// length — the padding-minimizing batching the sentence-transformers reference
/// applies. The per-text embedding is independent of its batchmates (masked
/// mean-pool over a per-sequence BERT forward), so reordering changes only the
/// padding, not the vectors; results are returned in the input order.
///
/// `embed_batch` is injected so the ordering logic is unit-testable without
/// loading a model.
fn embed_length_sorted<F>(
    texts: &[&str],
    max_batch: usize,
    mut embed_batch: F,
) -> Result<Vec<Vec<f32>>, IndexError>
where
    F: FnMut(&[&str]) -> Result<Vec<Vec<f32>>, IndexError>,
{
    if texts.is_empty() {
        return Ok(Vec::new());
    }

    // Stable sort by length (ascending) so equal-length texts keep input order.
    let mut order: Vec<usize> = (0..texts.len()).collect();
    order.sort_by_key(|&i| texts[i].len());

    let mut results: Vec<Vec<f32>> = vec![Vec::new(); texts.len()];
    for chunk in order.chunks(max_batch.max(1)) {
        let batch_texts: Vec<&str> = chunk.iter().map(|&i| texts[i]).collect();
        let vectors = embed_batch(&batch_texts)?;
        if vectors.len() != chunk.len() {
            return Err(IndexError::EmbeddingFailed {
                reason: format!(
                    "embed batch returned {} vectors for {} inputs",
                    vectors.len(),
                    chunk.len()
                ),
            });
        }
        for (&orig_idx, vector) in chunk.iter().zip(vectors) {
            results[orig_idx] = vector;
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candle_supported_models_is_not_empty() {
        assert!(!candle_supported_models().is_empty());
    }

    #[test]
    fn is_candle_model_works() {
        assert!(is_candle_model("all-MiniLM-L6-v2"));
        assert!(is_candle_model("bge-small-en-v1.5"));
        assert!(!is_candle_model("nonexistent-model"));
    }

    #[test]
    fn find_model_returns_correct_info() {
        let info = find_model("all-MiniLM-L6-v2").unwrap();
        assert_eq!(info.dimension, 384);
        assert_eq!(info.repo_id, "sentence-transformers/all-MiniLM-L6-v2");
    }

    #[test]
    fn find_model_returns_error_for_unknown() {
        assert!(find_model("nonexistent").is_err());
    }

    #[test]
    fn select_device_succeeds() {
        let device = select_device();
        assert!(device.is_ok());
    }

    #[test]
    fn embed_length_sorted_preserves_input_order() {
        // First byte distinguishes each text; lengths are deliberately out of
        // order so the internal length-sort must be undone before returning.
        let texts = ["dddd", "a", "ccc", "bb"];
        let out = embed_length_sorted(&texts, 2, |batch| {
            Ok(batch
                .iter()
                .map(|t| vec![f32::from(t.as_bytes()[0])])
                .collect())
        })
        .expect("embed");
        // Each result must encode its OWN text's first byte at its INPUT index.
        for (got, &b) in out.iter().zip([b'd', b'a', b'c', b'b'].iter()) {
            assert!(
                (got[0] - f32::from(b)).abs() < f32::EPSILON,
                "expected first byte {b}, got {}",
                got[0]
            );
        }
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn embed_length_sorted_batches_by_ascending_length() {
        let texts = ["dddd", "a", "ccc", "bb"];
        let seen: std::cell::RefCell<Vec<usize>> = std::cell::RefCell::new(Vec::new());
        embed_length_sorted(&texts, 2, |batch| {
            seen.borrow_mut().extend(batch.iter().map(|t| t.len()));
            Ok(batch.iter().map(|_| vec![0.0_f32]).collect())
        })
        .expect("embed");
        // Texts are processed shortest-first, so the lengths the batches saw are
        // globally non-decreasing (1,2 then 3,4 across two batches of 2).
        assert_eq!(seen.into_inner(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn embed_length_sorted_empty_is_empty() {
        let out = embed_length_sorted(&[], 8, |_| Ok(Vec::new())).expect("embed");
        assert!(out.is_empty());
    }

    #[test]
    fn embed_length_sorted_propagates_batch_error() {
        let texts = ["a", "bb"];
        let err = embed_length_sorted(&texts, 8, |_| {
            Err(IndexError::EmbeddingFailed {
                reason: "boom".to_owned(),
            })
        });
        assert!(matches!(err, Err(IndexError::EmbeddingFailed { .. })));
    }
}
