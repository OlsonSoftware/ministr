//! MCP sampling-based abstractive compression.
//!
//! Implements [`AbstractiveCompressor`] using MCP `sampling/createMessage`
//! to request LLM-assisted abstractive summarization through the client.

use ministr_core::extraction::abstractive::{AbstractiveCompressor, CompressError};
use ministr_core::token::count_tokens;
use rmcp::RoleServer;
use rmcp::model::{
    CreateMessageRequestParams, ModelHint, ModelPreferences, SamplingMessage,
    SamplingMessageContent,
};

use rmcp::service::Peer;

/// System prompt for the compression LLM.
///
/// Instructs the model to produce maximally dense summaries that preserve
/// all factual content, relationships, and technical details.
const COMPRESS_SYSTEM_PROMPT: &str = "\
You are a compression engine. Output ONLY the compressed summary — no preamble, \
no meta-commentary, no markdown formatting. Preserve all factual claims, names, \
numbers, code identifiers, and relationships. Be maximally dense: every word must \
carry information. Omit filler, transitions, and redundant phrasing.";

/// Abstractive compressor backed by MCP sampling.
///
/// Sends `sampling/createMessage` requests to the MCP client, which
/// forwards them to its host LLM. This achieves 90%+ compression ratios
/// compared to 60–80% for extractive methods.
///
/// Falls back gracefully — the caller should catch errors and use
/// extractive compression instead.
pub struct SamplingCompressor {
    peer: Peer<RoleServer>,
}

impl SamplingCompressor {
    /// Create a new sampling compressor wrapping the given MCP peer.
    #[must_use]
    pub fn new(peer: Peer<RoleServer>) -> Self {
        Self { peer }
    }
}

impl AbstractiveCompressor for SamplingCompressor {
    async fn compress(&self, text: &str, context_hint: &str) -> Result<String, CompressError> {
        // Target ~10% of original token count for max_tokens (90% compression).
        // Floor at 32 tokens to avoid degenerate cases; cap at 1024 for long sections.
        let original_tokens = count_tokens(text);
        let target_tokens = (original_tokens / 10).clamp(32, 1024);

        let user_message = if context_hint.is_empty() {
            format!("Compress the following text:\n\n{text}")
        } else {
            format!("Compress the following text (from: {context_hint}):\n\n{text}")
        };

        #[allow(clippy::cast_possible_truncation)]
        let params = CreateMessageRequestParams::new(
            vec![SamplingMessage::user_text(user_message)],
            target_tokens as u32,
        )
        .with_model_preferences(
            ModelPreferences::new()
                .with_hints(vec![ModelHint::new("claude")])
                .with_cost_priority(0.8)
                .with_speed_priority(0.9)
                .with_intelligence_priority(0.3),
        )
        .with_system_prompt(COMPRESS_SYSTEM_PROMPT)
        .with_temperature(0.0);

        let result = self
            .peer
            .create_message(params)
            .await
            .map_err(|e| CompressError::Failed(e.to_string()))?;

        // Extract text from the response message
        let text = result
            .message
            .content
            .into_vec()
            .into_iter()
            .filter_map(|c| match c {
                SamplingMessageContent::Text(t) => Some(t.text),
                _ => None,
            })
            .collect::<String>();

        if text.trim().is_empty() {
            return Err(CompressError::Failed(
                "sampling returned empty response".into(),
            ));
        }

        Ok(text)
    }
}
