//! Heuristic claim relationship detection.
//!
//! Detects cross-references and co-occurring entities between claims from
//! different sections. Relationships are detected at ingestion time and
//! stored for fast traversal via the `ministr_related` tool.

use std::collections::{HashMap, HashSet};

use crate::types::{Claim, ClaimRelationship, RelationType};

/// Trait for detecting relationships between claims.
pub trait RelationshipDetector: Send + Sync {
    /// Detect relationships among a set of claims.
    ///
    /// The input is a flat list of all claims across the corpus (or a batch
    /// of newly-ingested claims). Returns directed relationships with confidence.
    fn detect(&self, claims: &[Claim]) -> Vec<ClaimRelationship>;
}

/// Heuristic relationship detector using entity co-occurrence.
///
/// Extracts significant terms (capitalized words, technical terms, numbers)
/// from each claim. Claims from **different** sections that share enough
/// significant terms are linked with a `References` relationship.
///
/// # Examples
///
/// ```
/// use ministr_core::extraction::relationships::{HeuristicRelationshipDetector, RelationshipDetector};
/// use ministr_core::types::{Claim, ClaimId, SectionId};
///
/// let detector = HeuristicRelationshipDetector::new();
/// let claims = vec![
///     Claim {
///         id: ClaimId("c1".into()),
///         text: "The API uses JWT tokens with RS256 signing.".into(),
///         section_id: SectionId("s1".into()),
///     },
///     Claim {
///         id: ClaimId("c2".into()),
///         text: "JWT tokens expire after 24 hours by default.".into(),
///         section_id: SectionId("s2".into()),
///     },
/// ];
/// let rels = detector.detect(&claims);
/// assert!(!rels.is_empty());
/// ```
pub struct HeuristicRelationshipDetector {
    /// Minimum number of shared significant terms to form a relationship.
    min_shared_terms: usize,
}

impl HeuristicRelationshipDetector {
    /// Create a detector with default thresholds.
    #[must_use]
    pub fn new() -> Self {
        Self {
            min_shared_terms: 2,
        }
    }

    /// Create a detector with a custom minimum shared term count.
    #[must_use]
    pub fn with_min_shared_terms(min_shared_terms: usize) -> Self {
        Self { min_shared_terms }
    }
}

impl Default for HeuristicRelationshipDetector {
    fn default() -> Self {
        Self::new()
    }
}

impl RelationshipDetector for HeuristicRelationshipDetector {
    fn detect(&self, claims: &[Claim]) -> Vec<ClaimRelationship> {
        if claims.len() < 2 {
            return Vec::new();
        }

        // Extract significant terms for each claim
        let claim_terms: Vec<HashSet<String>> = claims
            .iter()
            .map(|c| extract_significant_terms(&c.text))
            .collect();

        let mut relationships = Vec::new();
        let mut seen_pairs: HashSet<(String, String)> = HashSet::new();

        for i in 0..claims.len() {
            for j in (i + 1)..claims.len() {
                // Only relate claims from different sections
                if claims[i].section_id == claims[j].section_id {
                    continue;
                }

                let shared: HashSet<_> = claim_terms[i].intersection(&claim_terms[j]).collect();

                if shared.len() >= self.min_shared_terms {
                    let pair_key = (claims[i].id.0.clone(), claims[j].id.0.clone());
                    if seen_pairs.contains(&pair_key) {
                        continue;
                    }
                    seen_pairs.insert(pair_key);

                    let confidence = compute_confidence(
                        shared.len(),
                        claim_terms[i].len(),
                        claim_terms[j].len(),
                    );

                    let relation_type = classify_relationship(&claims[i].text, &claims[j].text);

                    relationships.push(ClaimRelationship {
                        source_claim_id: claims[i].id.clone(),
                        target_claim_id: claims[j].id.clone(),
                        relation_type,
                        confidence,
                    });
                }
            }
        }

        relationships
    }
}

/// Common English stopwords that should not count as significant terms.
const STOPWORDS: &[&str] = &[
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "shall", "should", "may", "might", "can", "could",
    "must", "not", "and", "but", "or", "nor", "for", "yet", "so", "in", "on", "at", "to", "from",
    "by", "of", "with", "as", "into", "through", "during", "before", "after", "above", "below",
    "between", "under", "over", "each", "every", "all", "both", "few", "more", "most", "other",
    "some", "such", "no", "only", "own", "same", "than", "too", "very", "just", "that", "this",
    "these", "those", "it", "its", "they", "them", "their", "we", "our", "you", "your", "he",
    "she", "his", "her", "which", "what", "who", "whom", "when", "where", "how", "why", "if",
    "then", "also", "about", "up", "out", "any", "per",
];

/// Extract significant terms from a claim text.
///
/// Significant terms include:
/// - Capitalized words (likely named entities or technical terms)
/// - Words containing digits (version numbers, quantities)
/// - Longer lowercase words that aren't stopwords (technical vocabulary)
fn extract_significant_terms(text: &str) -> HashSet<String> {
    let mut terms = HashSet::new();

    for word in text.split_whitespace() {
        // Strip punctuation
        let clean: String = word
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .collect();

        if clean.is_empty() || clean.len() < 2 {
            continue;
        }

        let lower = clean.to_lowercase();

        // Skip stopwords
        if STOPWORDS.contains(&lower.as_str()) {
            continue;
        }

        // Capitalized word (not at sentence start — heuristic: len > 1 and starts with uppercase)
        let first_char = clean.chars().next();
        let has_digit = clean.chars().any(|c| c.is_ascii_digit());

        if has_digit {
            // Numbers/versions are always significant
            terms.insert(lower);
        } else if matches!(first_char, Some(c) if c.is_uppercase()) {
            // Capitalized words (named entities, technical terms)
            terms.insert(lower);
        } else if clean.len() >= 4 {
            // Longer words that aren't stopwords are likely technical terms
            terms.insert(lower);
        }
    }

    terms
}

/// Compute confidence score based on term overlap.
///
/// Uses Jaccard-like coefficient: `shared / min(|a|, |b|)` to favor
/// smaller claims that are highly specific.
fn compute_confidence(shared: usize, terms_a: usize, terms_b: usize) -> f32 {
    let min_size = terms_a.min(terms_b);
    if min_size == 0 {
        return 0.0;
    }
    #[allow(clippy::cast_precision_loss)]
    let raw = (shared as f32) / (min_size as f32);
    // Clamp to [0.0, 1.0]
    raw.min(1.0)
}

/// Negation words used for contradiction detection.
const NEGATION_WORDS: &[&str] = &[
    "not",
    "no",
    "never",
    "neither",
    "nor",
    "cannot",
    "doesn't",
    "don't",
    "didn't",
    "isn't",
    "aren't",
    "wasn't",
    "weren't",
    "won't",
    "wouldn't",
    "shouldn't",
    "couldn't",
    "without",
    "disable",
    "disabled",
    "removed",
    "deprecated",
];

/// Version/update indicators.
const UPDATE_INDICATORS: &[&str] = &[
    "updated",
    "changed",
    "modified",
    "replaced",
    "superseded",
    "upgraded",
    "migrated",
    "new",
    "previously",
    "formerly",
    "now",
    "instead",
];

/// Classify the relationship type between two claims based on content heuristics.
fn classify_relationship(text_a: &str, text_b: &str) -> RelationType {
    let lower_a = text_a.to_lowercase();
    let lower_b = text_b.to_lowercase();

    // Check for contradiction: one claim negates what the other asserts
    let a_has_negation = NEGATION_WORDS.iter().any(|w| {
        lower_a
            .split_whitespace()
            .any(|word| word.trim_matches(|c: char| !c.is_alphanumeric()) == *w)
    });
    let b_has_negation = NEGATION_WORDS.iter().any(|w| {
        lower_b
            .split_whitespace()
            .any(|word| word.trim_matches(|c: char| !c.is_alphanumeric()) == *w)
    });

    if a_has_negation != b_has_negation {
        return RelationType::Contradicts;
    }

    // Check for update relationship
    let has_update_indicator = UPDATE_INDICATORS
        .iter()
        .any(|w| lower_a.contains(w) || lower_b.contains(w));

    if has_update_indicator {
        return RelationType::Updates;
    }

    // Default: references (co-occurrence of shared terms)
    RelationType::References
}

/// Build a term-to-claims inverted index for efficient relationship detection.
///
/// This is useful for large corpora where O(n²) pairwise comparison is too slow.
/// Returns a map from each significant term to the list of claim indices that contain it.
#[must_use]
pub fn build_term_index(claims: &[Claim]) -> HashMap<String, Vec<usize>> {
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, claim) in claims.iter().enumerate() {
        for term in extract_significant_terms(&claim.text) {
            index.entry(term).or_default().push(i);
        }
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ClaimId, SectionId};

    fn make_claim(id: &str, text: &str, section: &str) -> Claim {
        Claim {
            id: ClaimId(id.into()),
            text: text.into(),
            section_id: SectionId(section.into()),
        }
    }

    // --- extract_significant_terms ---

    #[test]
    fn extracts_capitalized_words() {
        let terms = extract_significant_terms("The API uses JWT tokens with RS256 signing.");
        assert!(terms.contains("api"));
        assert!(terms.contains("jwt"));
        assert!(terms.contains("rs256"));
    }

    #[test]
    fn extracts_numeric_terms() {
        let terms = extract_significant_terms("Rate limits are 100 requests per minute.");
        assert!(terms.contains("100"));
        assert!(terms.contains("rate"));
    }

    #[test]
    fn filters_stopwords() {
        let terms = extract_significant_terms("The system is very good and also fast.");
        assert!(!terms.contains("the"));
        assert!(!terms.contains("is"));
        assert!(!terms.contains("and"));
        assert!(!terms.contains("very"));
    }

    #[test]
    fn empty_text_returns_empty() {
        let terms = extract_significant_terms("");
        assert!(terms.is_empty());
    }

    // --- compute_confidence ---

    #[test]
    fn confidence_full_overlap() {
        let conf = compute_confidence(3, 3, 5);
        assert!((conf - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn confidence_partial_overlap() {
        let conf = compute_confidence(2, 4, 4);
        assert!((conf - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn confidence_zero_terms() {
        let conf = compute_confidence(0, 0, 0);
        assert!((conf - 0.0).abs() < f32::EPSILON);
    }

    // --- classify_relationship ---

    #[test]
    fn classifies_contradiction() {
        let rt = classify_relationship(
            "The API uses JWT tokens.",
            "The API does not use JWT tokens.",
        );
        assert_eq!(rt, RelationType::Contradicts);
    }

    #[test]
    fn classifies_update() {
        let rt = classify_relationship(
            "The rate limit was previously 50 requests per minute.",
            "The rate limit is now 100 requests per minute.",
        );
        assert_eq!(rt, RelationType::Updates);
    }

    #[test]
    fn classifies_references_by_default() {
        let rt = classify_relationship(
            "The API uses JWT tokens with RS256.",
            "JWT tokens expire after 24 hours.",
        );
        assert_eq!(rt, RelationType::References);
    }

    // --- full detector ---

    #[test]
    fn detects_cross_section_relationships() {
        let detector = HeuristicRelationshipDetector::new();
        let claims = vec![
            make_claim("c1", "The API uses JWT tokens with RS256 signing.", "s1"),
            make_claim("c2", "JWT tokens expire after 24 hours by default.", "s2"),
        ];

        let rels = detector.detect(&claims);
        assert!(
            !rels.is_empty(),
            "should detect relationship between claims sharing JWT/tokens"
        );
        assert_eq!(rels[0].source_claim_id.0, "c1");
        assert_eq!(rels[0].target_claim_id.0, "c2");
    }

    #[test]
    fn no_relationship_within_same_section() {
        let detector = HeuristicRelationshipDetector::new();
        let claims = vec![
            make_claim("c1", "The API uses JWT tokens.", "s1"),
            make_claim("c2", "JWT tokens expire after 24 hours.", "s1"),
        ];

        let rels = detector.detect(&claims);
        assert!(
            rels.is_empty(),
            "should not relate claims within the same section"
        );
    }

    #[test]
    fn no_relationship_for_unrelated_claims() {
        let detector = HeuristicRelationshipDetector::new();
        let claims = vec![
            make_claim("c1", "The API uses JWT tokens with RS256.", "s1"),
            make_claim("c2", "PostgreSQL stores user data encrypted.", "s2"),
        ];

        let rels = detector.detect(&claims);
        assert!(rels.is_empty(), "should not relate unrelated claims");
    }

    #[test]
    fn single_claim_returns_empty() {
        let detector = HeuristicRelationshipDetector::new();
        let claims = vec![make_claim("c1", "The API uses JWT tokens.", "s1")];
        assert!(detector.detect(&claims).is_empty());
    }

    #[test]
    fn empty_claims_returns_empty() {
        let detector = HeuristicRelationshipDetector::new();
        assert!(detector.detect(&[]).is_empty());
    }

    #[test]
    fn custom_min_shared_terms() {
        let detector = HeuristicRelationshipDetector::with_min_shared_terms(3);
        let claims = vec![
            make_claim("c1", "The API uses JWT tokens.", "s1"),
            make_claim("c2", "JWT tokens expire after 24 hours.", "s2"),
        ];

        // With min_shared_terms=3, 2 shared terms shouldn't be enough
        let rels = detector.detect(&claims);
        assert!(rels.is_empty());
    }

    // --- build_term_index ---

    #[test]
    fn term_index_groups_claims_by_term() {
        let claims = vec![
            make_claim("c1", "The API uses JWT tokens.", "s1"),
            make_claim("c2", "JWT tokens expire after 24 hours.", "s2"),
            make_claim("c3", "Redis caches user data.", "s3"),
        ];

        let index = build_term_index(&claims);
        assert_eq!(index.get("jwt").map(Vec::len), Some(2));
        assert_eq!(index.get("tokens").map(Vec::len), Some(2));
        assert_eq!(index.get("redis").map(Vec::len), Some(1));
    }
}
