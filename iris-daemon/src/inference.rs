//! Sub-inference via the Claude CLI for `iris_ask` answer synthesis.
//!
//! Defines the [`Inference`] trait for testability and provides
//! [`ClaudeCliInference`] (production, spawns `claude -p`) and
//! [`MockInference`] (tests).

use tokio::io::AsyncWriteExt as _;

/// Error from inference operations.
#[derive(Debug, thiserror::Error)]
pub enum InferenceError {
    /// The `claude` CLI exited with a non-zero status.
    #[error("claude CLI failed (exit {exit_code}): {stderr}")]
    CliFailed {
        /// Process exit code.
        exit_code: i32,
        /// Captured stderr output.
        stderr: String,
    },

    /// The `claude` CLI binary was not found on `PATH`.
    #[error("claude CLI not found: {reason}")]
    CliNotFound {
        /// Underlying IO error description.
        reason: String,
    },

    /// Failed to write the prompt to the child's stdin (broken pipe, child
    /// died mid-startup, etc.). Keeps this distinct from a non-zero exit
    /// so the underlying cause isn't obscured when triage'ing logs.
    #[error("failed to send prompt to claude: {reason}")]
    StdinWriteFailed {
        /// Underlying IO error description.
        reason: String,
    },

    /// Failed to parse the inference output.
    #[error("failed to parse inference output: {reason}")]
    ParseFailed {
        /// What went wrong during parsing.
        reason: String,
    },

    /// The inference call timed out.
    #[error("inference timed out after {timeout_secs}s")]
    Timeout {
        /// The configured timeout in seconds.
        timeout_secs: u64,
    },
}

/// Response from an inference call.
#[derive(Debug, Clone)]
pub struct InferenceResponse {
    /// The synthesized answer text.
    pub answer: String,
    /// The model that produced the answer.
    pub model: String,
}

/// Trait for sub-inference operations, enabling mock injection in tests.
///
/// Uses `Pin<Box<dyn Future>>` for dyn-compatibility so the daemon can
/// store `Arc<dyn Inference>` and pass `&dyn Inference` to the ask module.
pub trait Inference: Send + Sync {
    /// Synthesize an answer from the given prompt.
    fn infer(
        &self,
        prompt: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<InferenceResponse, InferenceError>> + Send>,
    >;
}

/// Default inference timeout in seconds.
/// Agentic mode needs more time for multi-turn tool use.
const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Production implementation: spawns `claude -p` for pure text synthesis.
///
/// The daemon pre-retrieves context via `QueryService` and stuffs it into
/// the prompt. The sub-agent has no tools — it just reads and synthesizes.
pub struct ClaudeCliInference {
    model: String,
    timeout_secs: u64,
}

impl ClaudeCliInference {
    /// Create a new inference engine with the default model (haiku) and timeout.
    #[must_use]
    pub fn new() -> Self {
        Self {
            model: "haiku".to_string(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }

    /// Override the model used for inference.
    #[must_use]
    pub fn with_model(mut self, model: &str) -> Self {
        self.model = model.to_string();
        self
    }
}

impl Default for ClaudeCliInference {
    fn default() -> Self {
        Self::new()
    }
}

impl Inference for ClaudeCliInference {
    fn infer(
        &self,
        prompt: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<InferenceResponse, InferenceError>> + Send>,
    > {
        let model = self.model.clone();
        let timeout_secs = self.timeout_secs;
        let prompt_owned = prompt.to_string();

        Box::pin(async move {
            // Spawn the child via tokio's Command with `kill_on_drop(true)`
            // so that if the outer timeout fires and this future is dropped,
            // the child receives SIGKILL and its pipes/thread are reclaimed.
            // The previous impl wrapped a blocking wait in `spawn_blocking`
            // under `tokio::time::timeout`, which leaked the process + pipe
            // + blocking-pool thread on every timeout.
            let mut child = tokio::process::Command::new("claude")
                .args([
                    "-p",
                    "--output-format",
                    "json",
                    "--model",
                    &model,
                    // No tools — pure text synthesis from the prompt.
                    "--allowed-tools",
                    "",
                ])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| InferenceError::CliNotFound {
                    reason: e.to_string(),
                })?;

            let output =
                tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async move {
                    // Send the prompt. A broken pipe here means claude died
                    // mid-startup — surface that distinctly instead of
                    // letting it surface later as an unclear non-zero exit.
                    let mut stdin =
                        child
                            .stdin
                            .take()
                            .ok_or_else(|| InferenceError::StdinWriteFailed {
                                reason: "stdin handle was not captured".into(),
                            })?;
                    stdin
                        .write_all(prompt_owned.as_bytes())
                        .await
                        .map_err(|e| InferenceError::StdinWriteFailed {
                            reason: e.to_string(),
                        })?;
                    // Close stdin so claude sees EOF and starts processing.
                    drop(stdin);

                    child
                        .wait_with_output()
                        .await
                        .map_err(|e| InferenceError::CliFailed {
                            exit_code: -1,
                            stderr: format!("wait_with_output: {e}"),
                        })
                })
                .await
                .map_err(|_| InferenceError::Timeout { timeout_secs })??;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                return Err(InferenceError::CliFailed {
                    exit_code: output.status.code().unwrap_or(-1),
                    stderr,
                });
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            parse_claude_output(&stdout, &model)
        })
    }
}

/// Parse the JSON output from `claude -p --output-format json`.
///
/// The output is a JSON array of message objects. We extract the last
/// assistant message's text content as the answer.
fn parse_claude_output(output: &str, model: &str) -> Result<InferenceResponse, InferenceError> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Err(InferenceError::ParseFailed {
            reason: "empty output from claude".to_string(),
        });
    }

    // Try to parse as JSON first.
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(trimmed) {
        // Try "result" field (simple format)
        if let Some(result) = parsed.get("result").and_then(|v| v.as_str()) {
            return Ok(InferenceResponse {
                answer: result.to_string(),
                model: model.to_string(),
            });
        }

        // Try array format: last assistant message content
        if let Some(arr) = parsed.as_array() {
            for msg in arr.iter().rev() {
                if msg.get("type").and_then(|t| t.as_str()) == Some("assistant")
                    && let Some(content) = msg.get("content")
                {
                    if let Some(text) = content.as_str() {
                        return Ok(InferenceResponse {
                            answer: text.to_string(),
                            model: model.to_string(),
                        });
                    }
                    if let Some(blocks) = content.as_array() {
                        let text: String = blocks
                            .iter()
                            .filter_map(|b| {
                                if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                                    b.get("text").and_then(|t| t.as_str())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        if !text.is_empty() {
                            return Ok(InferenceResponse {
                                answer: text,
                                model: model.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    // Fallback: treat entire output as the answer (plain text mode)
    Ok(InferenceResponse {
        answer: trimmed.to_string(),
        model: model.to_string(),
    })
}

/// Mock inference for testing — returns a canned response.
#[cfg(test)]
pub struct MockInference {
    /// The canned answer to return.
    pub response: String,
}

#[cfg(test)]
impl Inference for MockInference {
    fn infer(
        &self,
        _prompt: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<InferenceResponse, InferenceError>> + Send>,
    > {
        let answer = self.response.clone();
        Box::pin(async move {
            Ok(InferenceResponse {
                answer,
                model: "mock".to_string(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_result_format() {
        let json = r#"{"result": "The answer is 42."}"#;
        let resp = parse_claude_output(json, "haiku").unwrap();
        assert_eq!(resp.answer, "The answer is 42.");
        assert_eq!(resp.model, "haiku");
    }

    #[test]
    fn parse_array_format() {
        let json = r#"[{"type":"assistant","content":"Hello world"}]"#;
        let resp = parse_claude_output(json, "sonnet").unwrap();
        assert_eq!(resp.answer, "Hello world");
    }

    #[test]
    fn parse_array_with_content_blocks() {
        let json = r#"[{"type":"assistant","content":[{"type":"text","text":"Part 1"},{"type":"text","text":"Part 2"}]}]"#;
        let resp = parse_claude_output(json, "haiku").unwrap();
        assert_eq!(resp.answer, "Part 1\nPart 2");
    }

    #[test]
    fn parse_plain_text_fallback() {
        let output = "Just a plain text response.";
        let resp = parse_claude_output(output, "haiku").unwrap();
        assert_eq!(resp.answer, "Just a plain text response.");
    }

    #[test]
    fn parse_empty_fails() {
        let result = parse_claude_output("", "haiku");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn mock_inference_returns_canned_response() {
        let mock = MockInference {
            response: "Test answer".to_string(),
        };
        let resp = mock.infer("any prompt").await.unwrap();
        assert_eq!(resp.answer, "Test answer");
        assert_eq!(resp.model, "mock");
    }
}
