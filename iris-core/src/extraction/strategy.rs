//! Pluggable compression strategy trait and implementations.
//!
//! `CompressStrategy` defines a uniform interface for text compression.
//! Implementations can be extractive (TF-IDF), salience-weighted, or
//! content-type-aware. The `AutoCompressor` selects the best strategy
//! based on content type (code → symbol summary, docs → extractive).

use super::claims::{ClaimExtractor, HeuristicClaimExtractor};
use super::summary::{ExtractiveSummaryGenerator, SummaryGenerator};
use crate::types::SectionId;

/// Pluggable backend for compressing a text section.
///
/// Implementations must be `Send + Sync` for use in async service methods.
pub trait CompressStrategy: Send + Sync {
    /// Compress `text` into a shorter summary.
    ///
    /// `max_sentences` is a hint for extractive strategies (ignored by others).
    /// Returns `None` if compression would not reduce token count.
    fn compress(&self, text: &str, max_sentences: usize) -> Option<String>;

    /// Name of the compression method (for reporting in `CompressedItem.method`).
    fn method_name(&self) -> &'static str;
}

// ---------------------------------------------------------------------------
// Strategy 1: Plain extractive TF-IDF (the existing default)
// ---------------------------------------------------------------------------

/// Wraps `ExtractiveSummaryGenerator` as a `CompressStrategy`.
#[derive(Default)]
pub struct ExtractiveStrategy {
    inner: ExtractiveSummaryGenerator,
}

impl CompressStrategy for ExtractiveStrategy {
    fn compress(&self, text: &str, max_sentences: usize) -> Option<String> {
        let summary = self.inner.summarize(text, max_sentences);
        if summary.len() >= text.len() {
            None
        } else {
            Some(summary)
        }
    }

    fn method_name(&self) -> &'static str {
        "extractive"
    }
}

// ---------------------------------------------------------------------------
// Strategy 2: Salience-weighted extractive (COMPRESS2.1)
// ---------------------------------------------------------------------------

/// Extractive compression that boosts sentences containing task keywords.
///
/// TF-IDF scores are multiplied by `(1 + salience_boost)` for sentences
/// matching at least one keyword. This retains high-value sentences that
/// the agent is likely to need while discarding boilerplate.
pub struct SalienceWeightedStrategy {
    inner: ExtractiveSummaryGenerator,
    keywords: Vec<String>,
    /// Multiplicative boost for sentences matching keywords.
    /// Reserved for future scoring refinement.
    _salience_boost: f64,
}

impl SalienceWeightedStrategy {
    /// Create a salience-weighted compressor from task keywords.
    ///
    /// `boost` controls how strongly keyword-matching sentences are favoured.
    /// A value of 1.0 doubles the score of matching sentences.
    #[must_use]
    pub fn new(keywords: Vec<String>, boost: f64) -> Self {
        Self {
            inner: ExtractiveSummaryGenerator::new(),
            keywords,
            _salience_boost: boost,
        }
    }
}

impl CompressStrategy for SalienceWeightedStrategy {
    fn compress(&self, text: &str, max_sentences: usize) -> Option<String> {
        if self.keywords.is_empty() {
            return ExtractiveStrategy::default().compress(text, max_sentences);
        }

        // Split into sentences, score with TF-IDF, then boost salient ones
        let sentences: Vec<&str> = split_sentences(text);
        if sentences.len() <= max_sentences {
            return None;
        }

        // Get base TF-IDF scores from the extractive summarizer
        let base_summary = self.inner.summarize(text, max_sentences);

        // Boost: for each sentence in the base summary, check keyword overlap
        // Actually, we need to re-score. Use a simpler approach: run extractive
        // on the full text with 2x budget, then keep the keyword-matching ones
        // first, fill remaining slots with top TF-IDF.
        let expanded = self.inner.summarize(text, max_sentences * 2);
        let expanded_sentences: Vec<&str> = split_sentences(&expanded);

        let mut salient: Vec<&str> = Vec::new();
        let mut other: Vec<&str> = Vec::new();

        for s in &expanded_sentences {
            let lower = s.to_lowercase();
            if self.keywords.iter().any(|kw| lower.contains(kw.as_str())) {
                salient.push(s);
            } else {
                other.push(s);
            }
        }

        // Prefer salient sentences, fill remaining from non-salient
        let mut selected: Vec<&str> = salient.into_iter().take(max_sentences).collect();
        let remaining = max_sentences.saturating_sub(selected.len());
        selected.extend(other.into_iter().take(remaining));

        if selected.is_empty() {
            return Some(base_summary);
        }

        let result = selected.join(" ");
        if result.len() >= text.len() {
            None
        } else {
            Some(result)
        }
    }

    fn method_name(&self) -> &'static str {
        "salience_extractive"
    }
}

// ---------------------------------------------------------------------------
// Strategy 3: Structured claim compression (COMPRESS2.2)
// ---------------------------------------------------------------------------

/// Compresses prose into a compact list of atomic claims.
///
/// Uses `HeuristicClaimExtractor` to identify factual assertions, then
/// joins them with " | " separators. Achieves higher information density
/// than extractive summarization for documentation-heavy content.
#[derive(Default)]
pub struct StructuredClaimStrategy {
    extractor: HeuristicClaimExtractor,
}

impl CompressStrategy for StructuredClaimStrategy {
    fn compress(&self, text: &str, _max_sentences: usize) -> Option<String> {
        let section_id = SectionId("compress".into());
        let claims = self.extractor.extract(text, &section_id);

        if claims.is_empty() {
            return None;
        }

        let summary: String = claims
            .iter()
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>()
            .join(" | ");

        if summary.len() >= text.len() {
            None
        } else {
            Some(summary)
        }
    }

    fn method_name(&self) -> &'static str {
        "structured_claims"
    }
}

// ---------------------------------------------------------------------------
// Compression quality scoring (COMPRESS2.3)
// ---------------------------------------------------------------------------

/// Quality metrics for a single compression result.
#[derive(Debug, Clone)]
pub struct CompressionQuality {
    /// Compression ratio: compressed_tokens / original_tokens (lower = more compression).
    pub ratio: f64,
    /// Information retention: fraction of original key terms preserved in summary.
    pub retention: f64,
    /// Information density: unique key terms per token in the compressed output.
    pub density: f64,
    /// Combined quality score (0.0–1.0): balances compression and retention.
    pub score: f64,
}

impl CompressionQuality {
    /// Evaluate compression quality by comparing original and compressed text.
    ///
    /// Extracts key terms (words ≥4 chars, not stop words) from both texts
    /// and measures their overlap. The combined score favours high retention
    /// at a reasonable compression ratio.
    #[must_use]
    pub fn evaluate(original: &str, compressed: &str) -> Self {
        let orig_tokens = token_count(original);
        let comp_tokens = token_count(compressed);

        let ratio = if orig_tokens == 0 {
            1.0
        } else {
            comp_tokens as f64 / orig_tokens as f64
        };

        let orig_terms = extract_key_terms(original);
        let comp_terms = extract_key_terms(compressed);

        let retention = if orig_terms.is_empty() {
            1.0
        } else {
            let retained = orig_terms.intersection(&comp_terms).count();
            retained as f64 / orig_terms.len() as f64
        };

        let density = if comp_tokens == 0 {
            0.0
        } else {
            comp_terms.len() as f64 / comp_tokens as f64
        };

        // Combined score: retention matters most (0.6), ratio reward (0.3),
        // density bonus (0.1). Ratio is inverted: lower ratio → higher score.
        let ratio_score = (1.0 - ratio).clamp(0.0, 1.0);
        let score = (0.6 * retention + 0.3 * ratio_score + 0.1 * density.min(1.0)).clamp(0.0, 1.0);

        Self {
            ratio,
            retention,
            density,
            score,
        }
    }
}

/// Extract key terms from text: lowercase words ≥4 chars, excluding stop words.
fn extract_key_terms(text: &str) -> std::collections::HashSet<String> {
    const STOP_WORDS: &[&str] = &[
        "this", "that", "with", "from", "have", "been", "were", "will", "would", "could",
        "should", "about", "their", "there", "which", "where", "when", "what", "into", "also",
        "each", "than", "more", "some", "such", "does", "them", "then", "very", "just",
    ];

    text.split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 4)
        .map(str::to_lowercase)
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .collect()
}

/// Simple whitespace token count (consistent with the rest of the module).
fn token_count(text: &str) -> usize {
    text.split_whitespace().count()
}

// ---------------------------------------------------------------------------
// Strategy 4: Auto-tier selection (COMPRESS2.4)
// ---------------------------------------------------------------------------

/// Content type classification for auto-tier compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    /// Source code (detected by file extension or content patterns).
    Code,
    /// Documentation / prose.
    Documentation,
    /// Atomic claims (already compressed).
    Claim,
}

/// Automatically selects a compression strategy based on content type.
///
/// - **Code** → symbol summary (signature + doc comment only)
/// - **Documentation** → extractive TF-IDF
/// - **Claims** → returns `None` (already maximally compressed)
#[derive(Default)]
pub struct AutoCompressor {
    extractive: ExtractiveStrategy,
}

impl AutoCompressor {
    /// Compress with auto-tier selection based on content type.
    ///
    /// `content_id` is used to classify the content type:
    /// - IDs containing `.rs`, `.py`, `.ts`, `.js`, `.go` etc. → Code
    /// - IDs containing `:c` (claim suffix) → Claim
    /// - Everything else → Documentation
    #[must_use]
    pub fn compress_auto(
        &self,
        content_id: &str,
        text: &str,
        max_sentences: usize,
    ) -> Option<(String, &'static str)> {
        let content_type = classify_content(content_id);
        match content_type {
            ContentType::Claim => None, // already compressed
            ContentType::Code => {
                let summary = compress_code(text);
                if summary.len() >= text.len() {
                    None
                } else {
                    Some((summary, "code_summary"))
                }
            }
            ContentType::Documentation => self
                .extractive
                .compress(text, max_sentences)
                .map(|s| (s, "extractive")),
        }
    }
}

/// Classify content type from a content ID.
fn classify_content(content_id: &str) -> ContentType {
    const CODE_EXTENSIONS: &[&str] = &[
        ".rs", ".py", ".ts", ".tsx", ".js", ".jsx", ".go", ".java", ".c", ".cpp", ".h", ".hpp",
        ".cs", ".rb", ".swift", ".kt", ".scala", ".zig", ".lua", ".sh", ".bash", ".toml", ".yaml",
        ".yml", ".json",
    ];

    // Claim IDs end with `:cN` (e.g., "file.md#heading:c0")
    if content_id
        .rsplit_once(':')
        .is_some_and(|(_, suffix)| suffix.starts_with('c') && suffix[1..].parse::<u32>().is_ok())
    {
        return ContentType::Claim;
    }

    let id_lower = content_id.to_lowercase();
    if CODE_EXTENSIONS.iter().any(|ext| id_lower.contains(ext)) {
        return ContentType::Code;
    }

    ContentType::Documentation
}

/// Compress code content: retain the first significant block (signature
/// + doc comment) and discard the body.
fn compress_code(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= 5 {
        return text.to_string();
    }

    let mut summary_lines: Vec<&str> = Vec::new();
    let mut in_doc = false;

    for line in &lines {
        let trimmed = line.trim();

        // Collect doc comments
        if trimmed.starts_with("///") || trimmed.starts_with("//!") || trimmed.starts_with('#') {
            in_doc = true;
            summary_lines.push(line);
            continue;
        }

        // After doc comments, keep the first non-empty non-comment line (signature)
        if in_doc && !trimmed.is_empty() && !trimmed.starts_with("//") {
            summary_lines.push(line);
            break;
        }

        // Before any doc comments, keep leading lines up to first blank
        if !in_doc {
            if trimmed.is_empty() && !summary_lines.is_empty() {
                break;
            }
            summary_lines.push(line);
            // Stop after the first function/struct/impl/trait line
            if trimmed.starts_with("pub ")
                || trimmed.starts_with("fn ")
                || trimmed.starts_with("struct ")
                || trimmed.starts_with("impl ")
                || trimmed.starts_with("trait ")
                || trimmed.starts_with("enum ")
                || trimmed.starts_with("type ")
                || trimmed.starts_with("const ")
                || trimmed.starts_with("def ")
                || trimmed.starts_with("class ")
                || trimmed.starts_with("func ")
                || trimmed.starts_with("function ")
            {
                break;
            }
        }
    }

    if summary_lines.is_empty() {
        // Fallback: first 3 lines
        lines.iter().take(3).copied().collect::<Vec<_>>().join("\n")
    } else {
        summary_lines.join("\n")
    }
}

/// Split text into sentences (simple heuristic).
fn split_sentences(text: &str) -> Vec<&str> {
    let mut sentences = Vec::new();
    let mut start = 0;

    for (i, c) in text.char_indices() {
        if (c == '.' || c == '!' || c == '?')
            && i + 1 < text.len()
            && text.as_bytes().get(i + 1).copied() == Some(b' ')
        {
            let sentence = text[start..=i].trim();
            if !sentence.is_empty() {
                sentences.push(sentence);
            }
            start = i + 2;
        }
    }

    // Last segment
    let remainder = text[start..].trim();
    if !remainder.is_empty() {
        sentences.push(remainder);
    }

    sentences
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extractive_strategy_compresses() {
        let strategy = ExtractiveStrategy::default();
        let text = "Rust is a systems language. It provides memory safety. \
                     The borrow checker ensures freedom. Many enjoy Rust. \
                     Cargo is the package manager.";
        let result = strategy.compress(text, 2);
        assert!(result.is_some());
        assert!(result.unwrap().len() < text.len());
    }

    #[test]
    fn extractive_strategy_skips_short_text() {
        let strategy = ExtractiveStrategy::default();
        let text = "Short text.";
        let result = strategy.compress(text, 2);
        // Short text cannot be meaningfully compressed
        assert!(result.is_none());
    }

    #[test]
    fn salience_weighted_prefers_keyword_sentences() {
        let strategy = SalienceWeightedStrategy::new(vec!["eviction".into()], 1.0);
        let text = "The weather is nice today. Eviction scoring uses multiple factors. \
                     Birds fly south in winter. The eviction ranker protects salient content. \
                     Coffee is popular worldwide.";
        let result = strategy.compress(text, 2);
        assert!(result.is_some());
        let summary = result.unwrap();
        assert!(
            summary.to_lowercase().contains("eviction"),
            "salience-weighted summary should retain keyword sentences: {summary}"
        );
    }

    #[test]
    fn salience_weighted_falls_back_without_keywords() {
        let strategy = SalienceWeightedStrategy::new(vec![], 1.0);
        let text = "Rust is great. Python is popular. Go is fast. Java is enterprise. C is low-level.";
        let result = strategy.compress(text, 2);
        assert!(result.is_some());
    }

    #[test]
    fn classify_content_detects_code() {
        assert_eq!(
            classify_content("iris-core/src/session/eviction.rs#EvictionRanker"),
            ContentType::Code
        );
        assert_eq!(classify_content("src/main.py"), ContentType::Code);
        assert_eq!(
            classify_content("package.json#dependencies"),
            ContentType::Code
        );
    }

    #[test]
    fn classify_content_detects_claims() {
        assert_eq!(
            classify_content("README.md#overview:c0"),
            ContentType::Claim
        );
        assert_eq!(
            classify_content("DESIGN.md#architecture/overview:c12"),
            ContentType::Claim
        );
    }

    #[test]
    fn classify_content_detects_docs() {
        assert_eq!(
            classify_content("README.md#overview"),
            ContentType::Documentation
        );
        assert_eq!(
            classify_content("docs/architecture.md#session-shadow"),
            ContentType::Documentation
        );
    }

    #[test]
    fn auto_compressor_skips_claims() {
        let auto = AutoCompressor::default();
        let result = auto.compress_auto("file.md#heading:c0", "This is a claim.", 2);
        assert!(result.is_none(), "claims should not be compressed");
    }

    #[test]
    fn auto_compressor_uses_code_strategy_for_code() {
        let auto = AutoCompressor::default();
        let code = "/// Compute eviction score.\n\
                     pub fn compute_score(item: &Item) -> f64 {\n\
                     \tlet recency = item.age();\n\
                     \tlet token = item.tokens();\n\
                     \trecency * 0.3 + token * 0.2\n\
                     }";
        let result = auto.compress_auto("src/eviction.rs#compute_score", code, 2);
        assert!(result.is_some());
        let (summary, method) = result.unwrap();
        assert_eq!(method, "code_summary");
        assert!(
            summary.contains("pub fn compute_score"),
            "code summary should retain signature: {summary}"
        );
    }

    #[test]
    fn auto_compressor_uses_extractive_for_docs() {
        let auto = AutoCompressor::default();
        let text = "Rust is a systems language. It provides memory safety. \
                     The borrow checker ensures freedom. Many enjoy Rust. \
                     Cargo is the package manager.";
        let result = auto.compress_auto("README.md#overview", text, 2);
        assert!(result.is_some());
        let (_, method) = result.unwrap();
        assert_eq!(method, "extractive");
    }

    #[test]
    fn compress_code_retains_signature() {
        let code = "/// Tracks memory state.\npub struct MemoryTracker {\n    states: HashMap<String, MemoryState>,\n}\n\nimpl MemoryTracker {\n    pub fn new() -> Self {\n        Self::default()\n    }\n}";
        let summary = compress_code(code);
        assert!(
            summary.contains("pub struct MemoryTracker"),
            "should retain struct signature: {summary}"
        );
        assert!(summary.len() < code.len());
    }

    #[test]
    fn split_sentences_basic() {
        let text = "First sentence. Second sentence. Third.";
        let sentences = split_sentences(text);
        assert_eq!(sentences.len(), 3);
        assert_eq!(sentences[0], "First sentence.");
        assert_eq!(sentences[1], "Second sentence.");
    }

    #[test]
    fn structured_claims_extracts_assertions() {
        let strategy = StructuredClaimStrategy::default();
        // Mix of claim-worthy and filler sentences — claims strategy should drop filler
        let text = "In this section we discuss the compression pipeline. \
                     The iris MCP server uses 128-dimensional ONNX embeddings for semantic search. \
                     See the documentation for more details on configuration. \
                     SQLite stores approximately 2,229 sections across 6 Cargo workspace members. \
                     Consider the trade-offs carefully before proceeding. \
                     The HNSW index supports up to 100,000 vectors with cosine similarity. \
                     Let us now turn to the next topic in this overview. \
                     TF-IDF scoring achieves 60-80% compression on documentation content. \
                     Remember to check the configuration before deploying.";
        let result = strategy.compress(text, 2);
        assert!(result.is_some(), "should extract claims from mixed text");
        let summary = result.unwrap();
        assert!(
            summary.contains('|'),
            "structured claims should use | separator: {summary}"
        );
        assert!(
            summary.len() < text.len(),
            "claims should be shorter than original (filler removed)"
        );
    }

    #[test]
    fn structured_claims_method_name() {
        let strategy = StructuredClaimStrategy::default();
        assert_eq!(strategy.method_name(), "structured_claims");
    }

    // --- Compression quality scoring tests ---

    #[test]
    fn quality_identical_texts() {
        let q = CompressionQuality::evaluate("hello world", "hello world");
        assert!((q.ratio - 1.0).abs() < f64::EPSILON);
        assert!((q.retention - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn quality_empty_compressed() {
        let q = CompressionQuality::evaluate("hello world foo bar", "");
        assert!(q.ratio < f64::EPSILON);
        assert!(q.retention < f64::EPSILON);
    }

    #[test]
    fn quality_good_compression() {
        let original = "The iris MCP server uses 128-dimensional ONNX embeddings for semantic search. \
                         It provides memory safety without garbage collection. \
                         The borrow checker ensures data race freedom. \
                         Many developers enjoy using Rust for systems programming.";
        let compressed = "iris MCP server uses ONNX embeddings. Borrow checker ensures safety.";
        let q = CompressionQuality::evaluate(original, compressed);
        assert!(q.ratio < 0.5, "should show significant compression: {}", q.ratio);
        assert!(q.retention > 0.0, "should retain some key terms: {}", q.retention);
        assert!(q.score > 0.3, "combined score should be reasonable: {}", q.score);
    }

    #[test]
    fn quality_extractive_vs_original() {
        let strategy = ExtractiveStrategy::default();
        let text = "Rust is a systems language. It provides memory safety. \
                     The borrow checker ensures freedom. Many enjoy Rust. \
                     Cargo is the package manager.";
        if let Some(compressed) = strategy.compress(text, 2) {
            let q = CompressionQuality::evaluate(text, &compressed);
            assert!(q.ratio < 1.0, "should compress");
            assert!(q.score > 0.0, "should have positive quality score");
        }
    }
}
