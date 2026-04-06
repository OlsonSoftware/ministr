//! Hybrid code+documentation embedder.
//!
//! [`HybridEmbedder`] wraps two embedding models — one tuned for source code,
//! one for natural-language documentation — and routes each input to the
//! appropriate backend based on a lightweight heuristic.

use std::sync::Arc;

use crate::embedding::Embedder;
use crate::error::IndexError;

/// Code-indicator tokens used by [`looks_like_code`] to classify input text.
const CODE_MARKERS: &[&str] = &[
    "fn ", "def ", "class ", "pub ", "import ", "#include", "struct ", "enum ", "impl ", "trait ",
    "module ", "package ", "func ", "var ", "let ", "const ", "async ", "await ", "return ",
    "if (", "for (", "while (", "match ", "->", "=>", "::", "self.", "this.",
];

/// Characters whose density suggests source code rather than prose.
const CODE_CHARS: &[char] = &['{', '}', '(', ')', ';', '<', '>'];

/// Minimum fraction of lines containing [`CODE_CHARS`] to classify as code.
const CODE_CHAR_THRESHOLD: f64 = 0.3;

/// Heuristic: returns `true` if `text` looks like source code.
///
/// Checks for keyword markers and punctuation density typical of
/// programming languages.
fn looks_like_code(text: &str) -> bool {
    // Fast path: check for keyword markers.
    for marker in CODE_MARKERS {
        if text.contains(marker) {
            return true;
        }
    }

    // Slow path: check punctuation density across lines.
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return false;
    }

    let code_lines = lines
        .iter()
        .filter(|line| {
            let trimmed = line.trim();
            CODE_CHARS.iter().any(|&c| trimmed.contains(c))
        })
        .count();

    #[allow(clippy::cast_precision_loss)]
    let ratio = code_lines as f64 / lines.len() as f64;
    ratio >= CODE_CHAR_THRESHOLD
}

/// A hybrid embedder that routes inputs to a code-specialized or
/// documentation-specialized model based on content heuristics.
///
/// Both inner embedders must produce vectors of the same dimensionality
/// so that results are comparable in a shared vector index.
pub struct HybridEmbedder {
    code: Arc<dyn Embedder>,
    docs: Arc<dyn Embedder>,
    dim: usize,
}

impl HybridEmbedder {
    /// Create a new hybrid embedder from a code model and a docs model.
    ///
    /// # Errors
    ///
    /// Returns [`IndexError::EmbeddingFailed`] if the two embedders have
    /// different dimensionalities.
    pub fn new(code: Arc<dyn Embedder>, docs: Arc<dyn Embedder>) -> Result<Self, IndexError> {
        let code_dim = code.dimension();
        let docs_dim = docs.dimension();
        if code_dim != docs_dim {
            return Err(IndexError::EmbeddingFailed {
                reason: format!(
                    "code embedder dimension ({code_dim}) != docs embedder dimension ({docs_dim})"
                ),
            });
        }
        Ok(Self {
            code,
            docs,
            dim: code_dim,
        })
    }

    /// Returns `true` if the given text is classified as source code.
    #[must_use]
    pub fn is_code(text: &str) -> bool {
        looks_like_code(text)
    }
}

impl Embedder for HybridEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Partition inputs by type, preserving original indices.
        let mut code_indices: Vec<usize> = Vec::new();
        let mut code_texts: Vec<&str> = Vec::new();
        let mut docs_indices: Vec<usize> = Vec::new();
        let mut docs_texts: Vec<&str> = Vec::new();

        for (i, &text) in texts.iter().enumerate() {
            if looks_like_code(text) {
                code_indices.push(i);
                code_texts.push(text);
            } else {
                docs_indices.push(i);
                docs_texts.push(text);
            }
        }

        // Embed each partition with its specialized model.
        let code_vecs = if code_texts.is_empty() {
            Vec::new()
        } else {
            self.code.embed(&code_texts)?
        };

        let docs_vecs = if docs_texts.is_empty() {
            Vec::new()
        } else {
            self.docs.embed(&docs_texts)?
        };

        // Reassemble results in original order.
        let mut results = vec![Vec::new(); texts.len()];
        for (slot, vec) in code_indices.into_iter().zip(code_vecs) {
            results[slot] = vec;
        }
        for (slot, vec) in docs_indices.into_iter().zip(docs_vecs) {
            results[slot] = vec;
        }

        Ok(results)
    }

    fn dimension(&self) -> usize {
        self.dim
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_function_is_code() {
        assert!(looks_like_code(
            "pub fn main() {\n    println!(\"hello\");\n}"
        ));
    }

    #[test]
    fn python_def_is_code() {
        assert!(looks_like_code(
            "def greet(name):\n    return f\"hi {name}\""
        ));
    }

    #[test]
    fn plain_prose_is_not_code() {
        assert!(!looks_like_code(
            "This is a paragraph about embedding models and how they work."
        ));
    }

    #[test]
    fn empty_string_is_not_code() {
        assert!(!looks_like_code(""));
    }

    #[test]
    fn brace_heavy_text_is_code() {
        let text = "{\n  \"key\": \"value\",\n  \"nested\": {\n    \"a\": 1\n  }\n}";
        assert!(looks_like_code(text));
    }
}
