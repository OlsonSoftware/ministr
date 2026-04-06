//! Metal-compatible BERT model for embedding inference.
//!
//! This is a vendored copy of `candle_transformers::models::bert` (v0.10.2) with
//! [`LayerNorm`] replaced by a decomposed [`MetalLayerNorm`] that uses only
//! primitive tensor ops (mean, sub, sqr, sqrt, mul, add). Candle's Metal backend
//! does not implement a fused `layer-norm` kernel, so the upstream `BertModel`
//! crashes on GPU with "no metal implementation for layer-norm".
//!
//! Approach adapted from [`metal-candle`](https://github.com/GarthDB/metal-candle).

#![allow(
    clippy::needless_pass_by_value,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::semicolon_if_nothing_returned
)]

use candle_core::{D, DType, Device, Module, Result, Tensor};
use candle_nn::{Embedding, VarBuilder, embedding};
use candle_transformers::models::bert::{Config, HiddenAct};
use candle_transformers::models::with_tracing::{Linear, linear};

// ---------------------------------------------------------------------------
// MetalLayerNorm — decomposed into ops Metal supports
// ---------------------------------------------------------------------------

/// Drop-in replacement for `candle_nn::LayerNorm` that works on Metal GPU.
///
/// Decomposes `layer_norm(x)` into:
/// ```text
/// mean = x.mean(dim=-1, keepdim=True)
/// var  = ((x - mean)²).mean(dim=-1, keepdim=True)
/// norm = (x - mean) / sqrt(var + eps)
/// out  = norm * weight + bias
/// ```
///
/// All of these are primitive tensor ops that Metal supports.
#[derive(Clone)]
pub(crate) struct MetalLayerNorm {
    weight: Tensor,
    bias: Tensor,
    eps: f64,
}

impl MetalLayerNorm {
    fn load(size: usize, eps: f64, vb: VarBuilder) -> Result<Self> {
        let weight = vb.get(size, "weight")?;
        let bias = vb.get(size, "bias")?;
        Ok(Self { weight, bias, eps })
    }
}

impl Module for MetalLayerNorm {
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        let mean = x.mean_keepdim(D::Minus1)?;
        let centered = x.broadcast_sub(&mean)?;
        let variance = centered.sqr()?.mean_keepdim(D::Minus1)?;
        let std = (variance + self.eps)?.sqrt()?;
        let normalized = centered.broadcast_div(&std)?;
        let scaled = normalized.broadcast_mul(&self.weight)?;
        scaled.broadcast_add(&self.bias)
    }
}

// ---------------------------------------------------------------------------
// Activation
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct HiddenActLayer {
    act: HiddenAct,
}

impl HiddenActLayer {
    fn new(act: HiddenAct) -> Self {
        Self { act }
    }

    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        match self.act {
            HiddenAct::Gelu => xs.gelu_erf(),
            HiddenAct::GeluApproximate => xs.gelu(),
            HiddenAct::Relu => xs.relu(),
        }
    }
}

// ---------------------------------------------------------------------------
// BertEmbeddings
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct BertEmbeddings {
    word_embeddings: Embedding,
    position_embeddings: Option<Embedding>,
    token_type_embeddings: Embedding,
    layer_norm: MetalLayerNorm,
}

impl BertEmbeddings {
    fn load(vb: VarBuilder, config: &Config) -> Result<Self> {
        let word_embeddings = embedding(
            config.vocab_size,
            config.hidden_size,
            vb.pp("word_embeddings"),
        )?;
        let position_embeddings = embedding(
            config.max_position_embeddings,
            config.hidden_size,
            vb.pp("position_embeddings"),
        )?;
        let token_type_embeddings = embedding(
            config.type_vocab_size,
            config.hidden_size,
            vb.pp("token_type_embeddings"),
        )?;
        let layer_norm = MetalLayerNorm::load(
            config.hidden_size,
            config.layer_norm_eps,
            vb.pp("LayerNorm"),
        )?;
        Ok(Self {
            word_embeddings,
            position_embeddings: Some(position_embeddings),
            token_type_embeddings,
            layer_norm,
        })
    }

    fn forward(&self, input_ids: &Tensor, token_type_ids: &Tensor) -> Result<Tensor> {
        let (_bsize, seq_len) = input_ids.dims2()?;
        let input_embeddings = self.word_embeddings.forward(input_ids)?;
        let token_type_embeddings = self.token_type_embeddings.forward(token_type_ids)?;
        let mut embeddings = (&input_embeddings + token_type_embeddings)?;
        if let Some(ref position_embeddings) = self.position_embeddings {
            let position_ids = (0..seq_len as u32).collect::<Vec<_>>();
            let position_ids = Tensor::new(&position_ids[..], input_ids.device())?;
            embeddings = embeddings.broadcast_add(&position_embeddings.forward(&position_ids)?)?;
        }
        self.layer_norm.forward(&embeddings)
    }
}

// ---------------------------------------------------------------------------
// Self-Attention
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct BertSelfAttention {
    query: Linear,
    key: Linear,
    value: Linear,
    num_attention_heads: usize,
    attention_head_size: usize,
}

impl BertSelfAttention {
    fn load(vb: VarBuilder, config: &Config) -> Result<Self> {
        let attention_head_size = config.hidden_size / config.num_attention_heads;
        let all_head_size = config.num_attention_heads * attention_head_size;
        let hidden_size = config.hidden_size;
        let query = linear(hidden_size, all_head_size, vb.pp("query"))?;
        let value = linear(hidden_size, all_head_size, vb.pp("value"))?;
        let key = linear(hidden_size, all_head_size, vb.pp("key"))?;
        Ok(Self {
            query,
            key,
            value,
            num_attention_heads: config.num_attention_heads,
            attention_head_size,
        })
    }

    fn transpose_for_scores(&self, xs: &Tensor) -> Result<Tensor> {
        let mut new_x_shape = xs.dims().to_vec();
        new_x_shape.pop();
        new_x_shape.push(self.num_attention_heads);
        new_x_shape.push(self.attention_head_size);
        let xs = xs.reshape(new_x_shape.as_slice())?.transpose(1, 2)?;
        xs.contiguous()
    }

    fn forward(&self, hidden_states: &Tensor, attention_mask: &Tensor) -> Result<Tensor> {
        let query_layer = self.query.forward(hidden_states)?;
        let key_layer = self.key.forward(hidden_states)?;
        let value_layer = self.value.forward(hidden_states)?;

        let query_layer = self.transpose_for_scores(&query_layer)?;
        let key_layer = self.transpose_for_scores(&key_layer)?;
        let value_layer = self.transpose_for_scores(&value_layer)?;

        let attention_scores = query_layer.matmul(&key_layer.t()?)?;
        let attention_scores = (attention_scores / (self.attention_head_size as f64).sqrt())?;
        let attention_scores = attention_scores.broadcast_add(attention_mask)?;
        let attention_probs = candle_nn::ops::softmax(&attention_scores, D::Minus1)?;

        let context_layer = attention_probs.matmul(&value_layer)?;
        let context_layer = context_layer.transpose(1, 2)?.contiguous()?;
        context_layer.flatten_from(D::Minus2)
    }
}

// ---------------------------------------------------------------------------
// Self-Output (dense + layer_norm + residual)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct BertSelfOutput {
    dense: Linear,
    layer_norm: MetalLayerNorm,
}

impl BertSelfOutput {
    fn load(vb: VarBuilder, config: &Config) -> Result<Self> {
        let dense = linear(config.hidden_size, config.hidden_size, vb.pp("dense"))?;
        let layer_norm = MetalLayerNorm::load(
            config.hidden_size,
            config.layer_norm_eps,
            vb.pp("LayerNorm"),
        )?;
        Ok(Self { dense, layer_norm })
    }

    fn forward(&self, hidden_states: &Tensor, input_tensor: &Tensor) -> Result<Tensor> {
        let hidden_states = self.dense.forward(hidden_states)?;
        self.layer_norm.forward(&(hidden_states + input_tensor)?)
    }
}

// ---------------------------------------------------------------------------
// Attention block
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct BertAttention {
    self_attention: BertSelfAttention,
    self_output: BertSelfOutput,
}

impl BertAttention {
    fn load(vb: VarBuilder, config: &Config) -> Result<Self> {
        let self_attention = BertSelfAttention::load(vb.pp("self"), config)?;
        let self_output = BertSelfOutput::load(vb.pp("output"), config)?;
        Ok(Self {
            self_attention,
            self_output,
        })
    }

    fn forward(&self, hidden_states: &Tensor, attention_mask: &Tensor) -> Result<Tensor> {
        let self_outputs = self.self_attention.forward(hidden_states, attention_mask)?;
        self.self_output.forward(&self_outputs, hidden_states)
    }
}

// ---------------------------------------------------------------------------
// FFN intermediate
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct BertIntermediate {
    dense: Linear,
    intermediate_act: HiddenActLayer,
}

impl BertIntermediate {
    fn load(vb: VarBuilder, config: &Config) -> Result<Self> {
        let dense = linear(config.hidden_size, config.intermediate_size, vb.pp("dense"))?;
        Ok(Self {
            dense,
            intermediate_act: HiddenActLayer::new(config.hidden_act),
        })
    }
}

impl Module for BertIntermediate {
    fn forward(&self, hidden_states: &Tensor) -> Result<Tensor> {
        let hidden_states = self.dense.forward(hidden_states)?;
        self.intermediate_act.forward(&hidden_states)
    }
}

// ---------------------------------------------------------------------------
// FFN output (dense + layer_norm + residual)
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct BertOutput {
    dense: Linear,
    layer_norm: MetalLayerNorm,
}

impl BertOutput {
    fn load(vb: VarBuilder, config: &Config) -> Result<Self> {
        let dense = linear(config.intermediate_size, config.hidden_size, vb.pp("dense"))?;
        let layer_norm = MetalLayerNorm::load(
            config.hidden_size,
            config.layer_norm_eps,
            vb.pp("LayerNorm"),
        )?;
        Ok(Self { dense, layer_norm })
    }

    fn forward(&self, hidden_states: &Tensor, input_tensor: &Tensor) -> Result<Tensor> {
        let hidden_states = self.dense.forward(hidden_states)?;
        self.layer_norm.forward(&(hidden_states + input_tensor)?)
    }
}

// ---------------------------------------------------------------------------
// Transformer layer
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MetalBertLayer {
    attention: BertAttention,
    intermediate: BertIntermediate,
    output: BertOutput,
}

impl MetalBertLayer {
    fn load(vb: VarBuilder, config: &Config) -> Result<Self> {
        let attention = BertAttention::load(vb.pp("attention"), config)?;
        let intermediate = BertIntermediate::load(vb.pp("intermediate"), config)?;
        let output = BertOutput::load(vb.pp("output"), config)?;
        Ok(Self {
            attention,
            intermediate,
            output,
        })
    }

    fn forward(&self, hidden_states: &Tensor, attention_mask: &Tensor) -> Result<Tensor> {
        let attention_output = self.attention.forward(hidden_states, attention_mask)?;
        let intermediate_output = self.intermediate.forward(&attention_output)?;
        self.output.forward(&intermediate_output, &attention_output)
    }
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MetalBertEncoder {
    layers: Vec<MetalBertLayer>,
}

impl MetalBertEncoder {
    fn load(vb: VarBuilder, config: &Config) -> Result<Self> {
        let layers = (0..config.num_hidden_layers)
            .map(|index| MetalBertLayer::load(vb.pp(format!("layer.{index}")), config))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self { layers })
    }

    fn forward(&self, hidden_states: &Tensor, attention_mask: &Tensor) -> Result<Tensor> {
        let mut hidden_states = hidden_states.clone();
        for layer in &self.layers {
            hidden_states = layer.forward(&hidden_states, attention_mask)?;
        }
        Ok(hidden_states)
    }
}

// ---------------------------------------------------------------------------
// Public model
// ---------------------------------------------------------------------------

/// BERT model with Metal-compatible layer normalization.
///
/// Identical to `candle_transformers::models::bert::BertModel` except all
/// `LayerNorm` layers are replaced with [`MetalLayerNorm`], which decomposes
/// normalization into primitive tensor ops that Metal supports.
pub(crate) struct MetalBertModel {
    embeddings: BertEmbeddings,
    encoder: MetalBertEncoder,
    #[allow(dead_code)]
    pub device: Device,
}

impl MetalBertModel {
    /// Load model weights. Compatible with the same safetensors files as
    /// the upstream `BertModel`.
    pub fn load(vb: VarBuilder, config: &Config) -> Result<Self> {
        let (embeddings, encoder) = match (
            BertEmbeddings::load(vb.pp("embeddings"), config),
            MetalBertEncoder::load(vb.pp("encoder"), config),
        ) {
            (Ok(embeddings), Ok(encoder)) => (embeddings, encoder),
            (Err(err), _) | (_, Err(err)) => {
                if let Some(model_type) = &config.model_type {
                    if let (Ok(embeddings), Ok(encoder)) = (
                        BertEmbeddings::load(vb.pp(format!("{model_type}.embeddings")), config),
                        MetalBertEncoder::load(vb.pp(format!("{model_type}.encoder")), config),
                    ) {
                        (embeddings, encoder)
                    } else {
                        return Err(err);
                    }
                } else {
                    return Err(err);
                }
            }
        };
        Ok(Self {
            embeddings,
            encoder,
            device: vb.device().clone(),
        })
    }

    /// Forward pass — same signature as `BertModel::forward`.
    pub fn forward(
        &self,
        input_ids: &Tensor,
        token_type_ids: &Tensor,
        attention_mask: Option<&Tensor>,
    ) -> Result<Tensor> {
        let embedding_output = self.embeddings.forward(input_ids, token_type_ids)?;
        let attention_mask = match attention_mask {
            Some(attention_mask) => attention_mask.clone(),
            None => input_ids.ones_like()?,
        };
        let dtype = embedding_output.dtype();
        let attention_mask = get_extended_attention_mask(&attention_mask, dtype)?;
        self.encoder.forward(&embedding_output, &attention_mask)
    }
}

fn get_extended_attention_mask(attention_mask: &Tensor, dtype: DType) -> Result<Tensor> {
    let attention_mask = match attention_mask.rank() {
        3 => attention_mask.unsqueeze(1)?,
        2 => attention_mask.unsqueeze(1)?.unsqueeze(1)?,
        _ => candle_core::bail!("Wrong shape for attention_mask"),
    };
    let attention_mask = attention_mask.to_dtype(dtype)?;
    (attention_mask.ones_like()? - &attention_mask)?.broadcast_mul(
        &Tensor::try_from(f32::MIN)?
            .to_device(attention_mask.device())?
            .to_dtype(dtype)?,
    )
}
