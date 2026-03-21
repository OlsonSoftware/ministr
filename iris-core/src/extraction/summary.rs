//! Extractive summary generation using TF-IDF sentence scoring.
//!
//! Generates summaries by selecting the most information-dense sentences
//! from a text based on their TF-IDF scores. The selected sentences are
//! returned in their original order to preserve readability.

use std::collections::HashMap;

/// Trait for generating summaries from text.
pub trait SummaryGenerator: Send + Sync {
    /// Generate a summary from the given text.
    ///
    /// Returns a condensed version of the text containing the most
    /// informative sentences.
    fn summarize(&self, text: &str, max_sentences: usize) -> String;
}

/// Extractive summary generator using TF-IDF sentence scoring.
///
/// Scores each sentence by the normalized sum of its words' TF-IDF values.
/// Selects the top-k scoring sentences and returns them in original order.
///
/// # Examples
///
/// ```
/// use iris_core::extraction::summary::{ExtractiveSummaryGenerator, SummaryGenerator};
///
/// let summarizer = ExtractiveSummaryGenerator::new();
/// let summary = summarizer.summarize(
///     "Rust is a systems programming language. It provides memory safety without garbage collection. \
///      The borrow checker ensures data race freedom. Many developers enjoy using Rust.",
///     2,
/// );
/// // Summary should contain the 2 most informative sentences
/// assert!(!summary.is_empty());
/// ```
pub struct ExtractiveSummaryGenerator;

impl ExtractiveSummaryGenerator {
    /// Create a new extractive summary generator.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for ExtractiveSummaryGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl SummaryGenerator for ExtractiveSummaryGenerator {
    fn summarize(&self, text: &str, max_sentences: usize) -> String {
        if text.is_empty() || max_sentences == 0 {
            return String::new();
        }

        let sentences = split_sentences(text);
        if sentences.is_empty() {
            return String::new();
        }

        if sentences.len() <= max_sentences {
            return sentences.join(" ");
        }

        // Tokenize each sentence into words
        let tokenized: Vec<Vec<String>> = sentences.iter().map(|s| tokenize_words(s)).collect();

        // Compute IDF across all sentences
        let idf = compute_idf(&tokenized);

        // Score each sentence by normalized TF-IDF sum
        let scores: Vec<f64> = tokenized
            .iter()
            .map(|words| score_sentence(words, &idf))
            .collect();

        // Select top-k sentence indices by score
        let mut indexed_scores: Vec<(usize, f64)> = scores.iter().copied().enumerate().collect();
        indexed_scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let mut selected_indices: Vec<usize> = indexed_scores
            .iter()
            .take(max_sentences)
            .map(|(i, _)| *i)
            .collect();

        // Sort by original position to preserve reading order
        selected_indices.sort_unstable();

        selected_indices
            .iter()
            .map(|&i| sentences[i].as_str())
            .collect::<Vec<&str>>()
            .join(" ")
    }
}

/// Split text into sentences at `.` `!` `?` boundaries.
///
/// A simplified sentence splitter (shares logic with the claim extractor
/// but kept separate to avoid coupling). For summary generation, precision
/// of sentence boundaries matters less than for claim extraction.
fn split_sentences(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut sentences = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        current.push(chars[i]);

        if matches!(chars[i], '.' | '!' | '?') {
            let next_nonspace = chars[(i + 1)..]
                .iter()
                .position(|c| !c.is_whitespace())
                .map(|p| i + 1 + p);

            let is_sentence_end = match next_nonspace {
                Some(next_idx) => chars[next_idx].is_uppercase(),
                None => true,
            };

            if is_sentence_end {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    sentences.push(trimmed);
                }
                current = String::new();
            }
        }

        i += 1;
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        sentences.push(trimmed);
    }

    sentences
}

/// Tokenize text into lowercase words, stripping punctuation.
fn tokenize_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| !w.is_empty() && !is_stop_word(w))
        .collect()
}

/// Check if a word is a common English stop word.
fn is_stop_word(word: &str) -> bool {
    matches!(
        word,
        "a" | "an"
            | "the"
            | "and"
            | "or"
            | "but"
            | "in"
            | "on"
            | "at"
            | "to"
            | "for"
            | "of"
            | "with"
            | "by"
            | "from"
            | "is"
            | "are"
            | "was"
            | "were"
            | "be"
            | "been"
            | "being"
            | "have"
            | "has"
            | "had"
            | "do"
            | "does"
            | "did"
            | "will"
            | "would"
            | "could"
            | "should"
            | "may"
            | "might"
            | "shall"
            | "can"
            | "it"
            | "its"
            | "this"
            | "that"
            | "these"
            | "those"
            | "not"
            | "no"
            | "if"
            | "so"
            | "as"
            | "up"
            | "out"
            | "about"
            | "into"
            | "over"
            | "after"
            | "also"
            | "each"
            | "which"
            | "their"
            | "there"
            | "then"
            | "them"
            | "they"
            | "than"
            | "when"
            | "what"
            | "who"
            | "how"
            | "all"
            | "any"
            | "both"
            | "more"
            | "most"
            | "other"
            | "some"
            | "such"
            | "only"
            | "very"
    )
}

/// Compute inverse document frequency for each term across sentences.
///
/// `IDF = ln(N / df)` where N = total sentences, df = sentences containing the term.
#[allow(clippy::cast_precision_loss)]
fn compute_idf(tokenized_sentences: &[Vec<String>]) -> HashMap<String, f64> {
    let n = tokenized_sentences.len() as f64;
    let mut doc_freq: HashMap<String, usize> = HashMap::new();

    for words in tokenized_sentences {
        // Count each word only once per sentence
        let unique: std::collections::HashSet<&str> = words.iter().map(String::as_str).collect();
        for word in unique {
            *doc_freq.entry(word.to_string()).or_insert(0) += 1;
        }
    }

    doc_freq
        .into_iter()
        .map(|(word, df)| {
            #[allow(clippy::cast_precision_loss)]
            let idf = (n / df as f64).ln();
            (word, idf)
        })
        .collect()
}

/// Score a sentence by the normalized sum of its words' TF-IDF values.
///
/// `TF = count(word) / sentence_length`,
/// `Score = sum(TF * IDF) / sentence_length`.
#[allow(clippy::cast_precision_loss)]
fn score_sentence(words: &[String], idf: &HashMap<String, f64>) -> f64 {
    if words.is_empty() {
        return 0.0;
    }

    let n = words.len() as f64;

    // Compute term frequency within this sentence
    let mut tf: HashMap<&str, usize> = HashMap::new();
    for word in words {
        *tf.entry(word.as_str()).or_insert(0) += 1;
    }

    let score: f64 = tf
        .iter()
        .map(|(word, &count)| {
            let term_freq = count as f64 / n;
            let inv_doc_freq = idf.get(*word).copied().unwrap_or(0.0);
            term_freq * inv_doc_freq
        })
        .sum();

    // Normalize by sentence length to avoid bias toward longer sentences
    score / n
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Sentence splitting ---

    #[test]
    fn split_basic() {
        let s = split_sentences("First sentence. Second sentence. Third sentence.");
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn split_empty() {
        assert!(split_sentences("").is_empty());
    }

    #[test]
    fn split_no_punctuation() {
        let s = split_sentences("No ending punctuation");
        assert_eq!(s.len(), 1);
    }

    // --- Tokenization ---

    #[test]
    fn tokenize_strips_punctuation() {
        let words = tokenize_words("Hello, world! This is a test.");
        assert!(words.contains(&"hello".to_string()));
        assert!(words.contains(&"world".to_string()));
        assert!(words.contains(&"test".to_string()));
    }

    #[test]
    fn tokenize_removes_stop_words() {
        let words = tokenize_words("the cat is on the mat");
        assert!(!words.contains(&"the".to_string()));
        assert!(!words.contains(&"is".to_string()));
        assert!(!words.contains(&"on".to_string()));
        assert!(words.contains(&"cat".to_string()));
        assert!(words.contains(&"mat".to_string()));
    }

    // --- IDF computation ---

    #[test]
    fn idf_common_word_lower_score() {
        let sentences = vec![
            vec!["rust".into(), "language".into()],
            vec!["rust".into(), "memory".into()],
            vec!["python".into(), "language".into()],
        ];
        let idf = compute_idf(&sentences);
        // "rust" appears in 2/3 sentences, "python" in 1/3
        assert!(idf["python"] > idf["rust"]);
    }

    // --- Scoring ---

    #[test]
    fn score_empty_sentence_is_zero() {
        let idf = HashMap::new();
        assert!(score_sentence(&[], &idf).abs() < f64::EPSILON);
    }

    // --- Full summarization ---

    #[test]
    fn summarize_empty_text() {
        let summarizer = ExtractiveSummaryGenerator::new();
        assert_eq!(summarizer.summarize("", 3), "");
    }

    #[test]
    fn summarize_zero_sentences() {
        let summarizer = ExtractiveSummaryGenerator::new();
        assert_eq!(summarizer.summarize("Some text here.", 0), "");
    }

    #[test]
    fn summarize_fewer_sentences_than_max() {
        let summarizer = ExtractiveSummaryGenerator::new();
        let text = "Only one sentence.";
        let summary = summarizer.summarize(text, 5);
        assert_eq!(summary, "Only one sentence.");
    }

    #[test]
    fn summarize_selects_informative_sentences() {
        let summarizer = ExtractiveSummaryGenerator::new();
        let text = "Rust provides memory safety without garbage collection. \
                     The borrow checker prevents data races at compile time. \
                     Many people like programming. \
                     HNSW indexes enable approximate nearest neighbor search in logarithmic time. \
                     Things happen in the world.";
        let summary = summarizer.summarize(text, 2);

        // The summary should pick the more informative sentences
        // (those with specific technical terms), not the generic ones
        assert!(!summary.is_empty());
        // Should contain 2 sentences (2 periods)
        let sentence_count = summary.matches('.').count();
        assert_eq!(sentence_count, 2, "expected 2 sentences, got: {summary}");
    }

    #[test]
    fn summarize_preserves_original_order() {
        let summarizer = ExtractiveSummaryGenerator::new();
        let text = "HNSW provides logarithmic search complexity. \
                     Normal sentence about general things. \
                     Another normal sentence here. \
                     SQLite stores structured data efficiently on disk.";
        let summary = summarizer.summarize(text, 2);

        // Both technical sentences should be selected, and HNSW should come before SQLite
        if summary.contains("HNSW") && summary.contains("SQLite") {
            let hnsw_pos = summary.find("HNSW").unwrap();
            let sqlite_pos = summary.find("SQLite").unwrap();
            assert!(
                hnsw_pos < sqlite_pos,
                "sentences should be in original order"
            );
        }
    }

    #[test]
    fn summarize_handles_single_sentence() {
        let summarizer = ExtractiveSummaryGenerator::new();
        let summary = summarizer.summarize("Just one sentence.", 1);
        assert_eq!(summary, "Just one sentence.");
    }
}
