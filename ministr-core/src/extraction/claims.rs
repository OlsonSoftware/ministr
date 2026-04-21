//! Heuristic claim extraction from section text.
//!
//! Extracts atomic factual statements (claims) from prose by splitting on
//! sentence boundaries and filtering for sentences that contain assertions:
//! named entities, numeric values, technical terms, or definitive verbs.

use crate::types::{Claim, ClaimId, SectionId};

/// Common abbreviations that should not cause sentence splits.
const ABBREVIATIONS: &[&str] = &[
    "mr.", "mrs.", "ms.", "dr.", "prof.", "sr.", "jr.", "vs.", "etc.", "inc.", "ltd.", "co.",
    "corp.", "dept.", "est.", "approx.", "vol.", "no.", "fig.", "eq.", "ref.", "e.g.", "i.e.",
    "viz.", "al.", "p.", "pp.", "ch.", "sec.", "gen.", "gov.", "sgt.", "cpl.", "pvt.", "capt.",
    "lt.", "col.", "maj.", "cmdr.", "adm.", "rev.", "st.",
];

/// Assertion patterns: copula, modal, auxiliary, and common technical verbs
/// surrounded by spaces to match as whole words in context.
const ASSERTION_PATTERNS: &[&str] = &[
    " is ",
    " are ",
    " was ",
    " were ",
    " has ",
    " have ",
    " had ",
    " can ",
    " will ",
    " must ",
    " shall ",
    " should ",
    " provides ",
    " supports ",
    " uses ",
    " requires ",
    " enables ",
    " implements ",
    " contains ",
    " includes ",
    " returns ",
    " accepts ",
    " stores ",
    " handles ",
    " manages ",
    " runs ",
    " executes ",
    " generates ",
    " defaults ",
    " configures ",
    " limits ",
    " ensures ",
    " validates ",
];

/// Assertive verbs used for word-level matching in claim detection.
const ASSERTIVE_VERBS: &[&str] = &[
    "is",
    "are",
    "was",
    "were",
    "uses",
    "requires",
    "must",
    "should",
    "provides",
    "supports",
    "enables",
    "implements",
    "contains",
    "includes",
    "allows",
    "defines",
    "specifies",
    "returns",
    "accepts",
    "produces",
    "generates",
    "creates",
    "stores",
    "handles",
    "manages",
    "runs",
    "executes",
    "processes",
    "performs",
    "operates",
    "defaults",
    "configures",
    "sets",
    "limits",
    "restricts",
    "enforces",
    "validates",
    "ensures",
];

/// Common English words that appear capitalized mid-sentence but are not named entities.
const COMMON_CAPITALIZED: &[&str] = &[
    "The", "A", "An", "In", "On", "At", "By", "For", "To", "Of", "And", "Or", "But", "With",
    "From", "Is", "It", "As", "If", "Not", "No", "This", "That", "These", "Those",
];

/// Trait for extracting claims from section text.
pub trait ClaimExtractor: Send + Sync {
    /// Extract atomic claims from the given section text.
    fn extract(&self, text: &str, section_id: &SectionId) -> Vec<Claim>;
}

/// Heuristic claim extractor using sentence splitting and assertion detection.
///
/// The extraction pipeline:
/// 1. Split text into sentences at `.` `!` `?` boundaries (handling abbreviations)
/// 2. Filter out sentences that are too short or too long
/// 3. Score each sentence for "claim-ness" (contains numbers, named entities, assertions)
/// 4. Return sentences that pass the score threshold as claims
///
/// # Examples
///
/// ```
/// use ministr_core::extraction::claims::{HeuristicClaimExtractor, ClaimExtractor};
/// use ministr_core::types::SectionId;
///
/// let extractor = HeuristicClaimExtractor::new();
/// let claims = extractor.extract(
///     "The API uses JWT tokens with RS256 signing. Rate limits are 100 requests per minute.",
///     &SectionId("test#section".into()),
/// );
/// assert!(!claims.is_empty());
/// ```
pub struct HeuristicClaimExtractor {
    /// Minimum number of words for a sentence to be considered a claim.
    min_words: usize,
    /// Maximum number of words for a sentence to be considered a claim.
    max_words: usize,
}

impl HeuristicClaimExtractor {
    /// Create an extractor with default thresholds.
    #[must_use]
    pub fn new() -> Self {
        Self {
            min_words: 4,
            max_words: 60,
        }
    }

    /// Create an extractor with custom word count thresholds.
    #[must_use]
    pub fn with_thresholds(min_words: usize, max_words: usize) -> Self {
        Self {
            min_words,
            max_words,
        }
    }
}

impl Default for HeuristicClaimExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaimExtractor for HeuristicClaimExtractor {
    fn extract(&self, text: &str, section_id: &SectionId) -> Vec<Claim> {
        let sentences = split_sentences(text);

        sentences
            .into_iter()
            .filter(|s| {
                let word_count = s.split_whitespace().count();
                word_count >= self.min_words && word_count <= self.max_words
            })
            .filter(|s| is_assertion(s))
            .enumerate()
            .map(|(i, sentence)| {
                let claim_id = format!("{}:c{i}", section_id.0);
                Claim {
                    id: ClaimId(claim_id),
                    text: sentence,
                    section_id: section_id.clone(),
                }
            })
            .collect()
    }
}

/// Split text into sentences, handling common abbreviations.
///
/// Splits on sentence-ending punctuation (`.` `!` `?`) followed by whitespace
/// and an uppercase letter or end-of-string, while avoiding splits on common
/// abbreviations like "e.g.", "i.e.", "Dr.", "Mr.", etc.
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
            let is_abbrev = is_abbreviation(&current);

            if !is_abbrev {
                let next_nonspace = chars[(i + 1)..]
                    .iter()
                    .position(|c| !c.is_whitespace())
                    .map(|p| i + 1 + p);

                let is_sentence_end = match next_nonspace {
                    Some(next_idx) => chars[next_idx].is_uppercase() || chars[next_idx] == '"',
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
        }

        i += 1;
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        sentences.push(trimmed);
    }

    sentences
}

/// Check if the current buffer ends with a known abbreviation (whole word).
fn is_abbreviation(buffer: &str) -> bool {
    let lower = buffer.to_lowercase();
    ABBREVIATIONS.iter().any(|abbr| {
        if lower.len() < abbr.len() {
            return false;
        }
        if !lower.ends_with(abbr) {
            return false;
        }
        if lower.len() == abbr.len() {
            return true;
        }
        let before = lower.as_bytes()[lower.len() - abbr.len() - 1];
        before == b' ' || before == b'\n' || before == b'\t'
    })
}

/// Determine if a sentence is likely an assertion (a factual claim).
///
/// Scores the sentence on multiple heuristics and requires at least 2
/// signals to classify as a claim.
fn is_assertion(sentence: &str) -> bool {
    if sentence.ends_with('?') {
        return false;
    }

    let words: Vec<&str> = sentence.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }

    let mut score: u32 = 0;
    let lower = sentence.to_lowercase();

    score += score_numeric(&words);
    score += score_assertive_verbs(&lower);
    score += score_named_entities(&words);
    score += score_assertion_patterns(&lower);

    score >= 2
}

/// +2 if any word contains a digit.
fn score_numeric(words: &[&str]) -> u32 {
    if words.iter().any(|w| w.chars().any(|c| c.is_ascii_digit())) {
        2
    } else {
        0
    }
}

/// +1 if any word matches an assertive verb.
fn score_assertive_verbs(lower: &str) -> u32 {
    u32::from(ASSERTIVE_VERBS.iter().any(|v| {
        lower
            .split_whitespace()
            .any(|w| w.trim_matches(|c: char| !c.is_alphanumeric()) == *v)
    }))
}

/// +1 if there's a capitalized word mid-sentence that isn't a common word.
fn score_named_entities(words: &[&str]) -> u32 {
    let has_named_entity = words.iter().skip(1).any(|w| {
        let first = w.chars().next();
        matches!(first, Some(c) if c.is_uppercase())
            && w.len() > 1
            && !COMMON_CAPITALIZED.contains(w)
    });
    u32::from(has_named_entity)
}

/// +1 if the sentence contains a known assertion pattern.
fn score_assertion_patterns(lower: &str) -> u32 {
    u32::from(ASSERTION_PATTERNS.iter().any(|p| lower.contains(p)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_claims(text: &str) -> Vec<Claim> {
        let extractor = HeuristicClaimExtractor::new();
        extractor.extract(text, &SectionId("test#section".into()))
    }

    // --- Sentence splitting ---

    #[test]
    fn split_simple_sentences() {
        let sentences = split_sentences("Hello world. This is a test. Done.");
        assert_eq!(sentences.len(), 3);
        assert_eq!(sentences[0], "Hello world.");
        assert_eq!(sentences[1], "This is a test.");
        assert_eq!(sentences[2], "Done.");
    }

    #[test]
    fn split_handles_abbreviations() {
        let sentences = split_sentences("Dr. Smith works at Inc. Corp. He is a professor.");
        assert!(!sentences.is_empty());
        assert!(sentences[0].contains("Dr. Smith"));
    }

    #[test]
    fn split_handles_eg_ie() {
        let sentences = split_sentences("Use a format e.g. JSON or XML. The parser handles both.");
        assert_eq!(sentences.len(), 2);
        assert!(sentences[0].contains("e.g."));
    }

    #[test]
    fn split_empty_text() {
        let sentences = split_sentences("");
        assert!(sentences.is_empty());
    }

    #[test]
    fn split_no_sentence_ending() {
        let sentences = split_sentences("No ending punctuation here");
        assert_eq!(sentences.len(), 1);
        assert_eq!(sentences[0], "No ending punctuation here");
    }

    #[test]
    fn split_question_and_exclamation() {
        let sentences = split_sentences("Is this a question? Yes! It is.");
        assert_eq!(sentences.len(), 3);
    }

    // --- Assertion detection ---

    #[test]
    fn assertion_with_number() {
        assert!(is_assertion(
            "Rate limits are set to 100 requests per minute."
        ));
    }

    #[test]
    fn assertion_with_named_entity() {
        assert!(is_assertion("The API uses JWT tokens with RS256 signing."));
    }

    #[test]
    fn assertion_with_verb() {
        assert!(is_assertion(
            "The system provides automatic failover for all services."
        ));
    }

    #[test]
    fn question_is_not_assertion() {
        assert!(!is_assertion("What is the rate limit?"));
    }

    #[test]
    fn short_fragment_not_assertion() {
        assert!(!is_assertion("See above."));
    }

    // --- Full extraction ---

    #[test]
    fn extract_claims_from_technical_text() {
        let text = "The auth service uses JWT tokens with RS256 signing. \
                     Rate limits are set to 100 requests per minute per API key. \
                     See the documentation for more details.";
        let claims = extract_claims(text);
        assert!(
            claims.len() >= 2,
            "expected at least 2 claims, got {}",
            claims.len()
        );
    }

    #[test]
    fn claim_ids_are_sequential() {
        let text = "The API returns JSON responses. The server runs on port 8080. \
                     Redis is used for caching with a 5 minute TTL.";
        let claims = extract_claims(text);
        for (i, claim) in claims.iter().enumerate() {
            assert!(
                claim.id.0.ends_with(&format!(":c{i}")),
                "claim id {} should end with :c{i}",
                claim.id
            );
        }
    }

    #[test]
    fn claim_section_id_preserved() {
        let claims = extract_claims("The system provides 99.9% uptime guarantee.");
        for claim in &claims {
            assert_eq!(claim.section_id.0, "test#section");
        }
    }

    #[test]
    fn empty_text_no_claims() {
        let claims = extract_claims("");
        assert!(claims.is_empty());
    }

    #[test]
    fn questions_filtered_out() {
        let text = "What is the rate limit? How does auth work? \
                     The rate limit is 100 requests per minute.";
        let claims = extract_claims(text);
        for claim in &claims {
            assert!(!claim.text.ends_with('?'));
        }
    }

    #[test]
    fn custom_thresholds() {
        let extractor = HeuristicClaimExtractor::with_thresholds(2, 10);
        let claims = extractor.extract(
            "Port is 8080. The authentication service uses JWT tokens with RS256 signing algorithm for all requests.",
            &SectionId("test#s".into()),
        );
        for claim in &claims {
            assert!(claim.text.split_whitespace().count() <= 10);
        }
    }

    // --- Precision / recall on a hand-labeled corpus ---

    /// A labeled sentence: `(text, expected_is_claim)`.
    ///
    /// True positives: factual, verifiable statements with technical content.
    /// True negatives: questions, fragments, vague prose, instructions.
    const LABELED_CORPUS: &[(&str, bool)] = &[
        // ---- True claims (should be extracted) ----
        ("The API uses JWT tokens with RS256 signing.", true),
        ("Rate limits are set to 100 requests per minute.", true),
        ("The server runs on port 8080 by default.", true),
        ("Redis is used for caching with a 5 minute TTL.", true),
        (
            "The database supports up to 10,000 concurrent connections.",
            true,
        ),
        ("PostgreSQL stores all user data in encrypted form.", true),
        (
            "The system provides automatic failover for all services.",
            true,
        ),
        ("TLS 1.3 is required for all external connections.", true),
        (
            "The response payload contains a JSON object with 3 fields.",
            true,
        ),
        (
            "Each worker thread handles approximately 500 requests per second.",
            true,
        ),
        (
            "The deployment pipeline runs 4 stages before production.",
            true,
        ),
        (
            "Kubernetes manages container orchestration across 12 nodes.",
            true,
        ),
        ("The OAuth2 flow requires a client ID and secret.", true),
        (
            "GraphQL queries are validated against the schema at compile time.",
            true,
        ),
        (
            "The HNSW index stores vectors in a memory-mapped file.",
            true,
        ),
        (
            "Batch ingestion processes up to 1000 documents per minute.",
            true,
        ),
        (
            "The circuit breaker opens after 5 consecutive failures.",
            true,
        ),
        (
            "Prometheus collects metrics every 15 seconds from all endpoints.",
            true,
        ),
        (
            "The retry policy uses exponential backoff with a maximum of 30 seconds.",
            true,
        ),
        ("SQLite uses WAL mode for concurrent read access.", true),
        // ---- Non-claims (should not be extracted) ----
        ("What is the rate limit?", false),
        ("How does authentication work?", false),
        ("See above.", false),
        ("For more details, refer to the documentation.", false),
        ("Note the following.", false),
        ("In this section we discuss the approach.", false),
        ("Consider the trade-offs carefully.", false),
        ("The following example shows how to do this.", false),
        ("Let us now turn to the next topic.", false),
        ("As mentioned earlier, there are several options.", false),
        ("First, set up the environment.", false),
        ("Click the button to proceed.", false),
        ("Open the terminal and run the command.", false),
        ("Remember to save your work frequently.", false),
        ("Think about what you want to achieve.", false),
        ("Why did the test fail?", false),
        ("Where can I find the logs?", false),
        ("Summary of the chapter.", false),
        ("Next steps and recommendations.", false),
        ("Table of contents below.", false),
    ];

    /// Compute precision and recall against the labeled corpus.
    ///
    /// The corpus is joined into a single text block and run through the extractor.
    /// Each labeled sentence is matched against the extracted claims to determine
    /// true positives, false positives, and false negatives.
    fn compute_precision_recall() -> (f64, f64) {
        let full_text: String = LABELED_CORPUS
            .iter()
            .map(|(s, _)| *s)
            .collect::<Vec<_>>()
            .join(" ");

        let claims = extract_claims(&full_text);
        let extracted_texts: Vec<&str> = claims.iter().map(|c| c.text.as_str()).collect();

        let mut true_positives: u32 = 0;
        let mut false_positives: u32 = 0;
        let mut false_negatives: u32 = 0;

        // Check each extracted claim against labels
        for extracted in &extracted_texts {
            let is_labeled_claim = LABELED_CORPUS.iter().any(|(s, label)| {
                *label && extracted.trim_end_matches('.') == s.trim_end_matches('.')
            });
            if is_labeled_claim {
                true_positives += 1;
            } else {
                // Extracted something not labeled as a claim (or not in corpus)
                false_positives += 1;
            }
        }

        // Check each labeled claim to see if it was extracted
        for (sentence, is_claim) in LABELED_CORPUS {
            if *is_claim {
                let was_extracted = extracted_texts
                    .iter()
                    .any(|e| e.trim_end_matches('.') == sentence.trim_end_matches('.'));
                if !was_extracted {
                    false_negatives += 1;
                }
            }
        }

        let precision = if true_positives + false_positives == 0 {
            0.0
        } else {
            f64::from(true_positives) / f64::from(true_positives + false_positives)
        };

        let recall = if true_positives + false_negatives == 0 {
            0.0
        } else {
            f64::from(true_positives) / f64::from(true_positives + false_negatives)
        };

        (precision, recall)
    }

    #[test]
    fn claim_extraction_precision() {
        let (precision, _recall) = compute_precision_recall();
        assert!(
            precision >= 0.70,
            "precision {precision:.2} is below the 0.70 threshold"
        );
    }

    #[test]
    fn claim_extraction_recall() {
        let (_precision, recall) = compute_precision_recall();
        assert!(
            recall >= 0.70,
            "recall {recall:.2} is below the 0.70 threshold"
        );
    }

    #[test]
    fn claim_extraction_f1() {
        let (precision, recall) = compute_precision_recall();
        let f1 = if precision + recall == 0.0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        };
        assert!(
            f1 >= 0.70,
            "F1 score {f1:.2} (P={precision:.2}, R={recall:.2}) is below the 0.70 threshold"
        );
    }

    /// Edge-case corpus: abbreviations, code references, mixed content.
    const EDGE_CASE_CORPUS: &[(&str, bool)] = &[
        // Claims with abbreviations — should still be extracted
        (
            "Dr. Smith's API uses RSA-256 encryption for all tokens.",
            true,
        ),
        ("The server e.g. nginx handles 10,000 connections.", true),
        // Technical claims with version numbers
        ("Python 3.12 introduces the new buffer protocol.", true),
        ("The v2.0 API returns HTTP 429 when rate-limited.", true),
        // Non-claims: imperatives, fragments, vague
        ("Install the package via pip.", false),
        ("Run the tests before merging.", false),
        ("See also the appendix.", false),
        ("Overview of changes.", false),
    ];

    #[test]
    fn edge_case_precision_recall() {
        let full_text: String = EDGE_CASE_CORPUS
            .iter()
            .map(|(s, _)| *s)
            .collect::<Vec<_>>()
            .join(" ");

        let claims = extract_claims(&full_text);
        let extracted_texts: Vec<&str> = claims.iter().map(|c| c.text.as_str()).collect();

        let mut true_positives: u32 = 0;
        let mut false_negatives: u32 = 0;

        for (sentence, is_claim) in EDGE_CASE_CORPUS {
            if *is_claim {
                let was_extracted = extracted_texts
                    .iter()
                    .any(|e| e.trim_end_matches('.') == sentence.trim_end_matches('.'));
                if was_extracted {
                    true_positives += 1;
                } else {
                    false_negatives += 1;
                }
            }
        }

        let expected_claims =
            u32::try_from(EDGE_CASE_CORPUS.iter().filter(|(_, c)| *c).count()).unwrap();
        let recall = f64::from(true_positives) / f64::from(expected_claims);
        assert!(
            recall >= 0.50,
            "edge-case recall {recall:.2} ({true_positives}/{expected_claims}) — \
             missed: {false_negatives} claims"
        );

        // No non-claim should be extracted
        for (sentence, is_claim) in EDGE_CASE_CORPUS {
            if !*is_claim {
                let was_extracted = extracted_texts
                    .iter()
                    .any(|e| e.trim_end_matches('.') == sentence.trim_end_matches('.'));
                assert!(
                    !was_extracted,
                    "non-claim was incorrectly extracted: {sentence}"
                );
            }
        }
    }
}
