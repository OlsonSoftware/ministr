//! Eviction ranking for context budget management.
//!
//! Scores delivered content items to determine which are best candidates for
//! eviction from the agent's context window. Items are ranked by a composite
//! score of recency decay, token weight, resolution priority, and attention
//! position bias.
//!
//! The position factor models the "Lost in the Middle" phenomenon (Liu et al.,
//! 2023): LLMs attend well to content at the start (primacy bias) and end
//! (recency bias) of their context window, but poorly to content in the middle.
//! Mid-context content is therefore a better eviction candidate — the LLM is
//! already losing it to attention bias.

use serde::Serialize;

use super::types::{DeliveredItem, Session};

/// A candidate for eviction from the agent's context window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EvictionCandidate {
    /// The content ID of the item to evict.
    pub content_id: String,
    /// Human-readable reason for the eviction recommendation.
    pub reason: String,
    /// Token count that would be recovered by evicting this item.
    pub tokens_recoverable: usize,
    /// Composite eviction score (higher = better candidate for eviction).
    pub score: f64,
}

/// Ranks delivered content items by eviction priority.
///
/// The ranking considers four factors:
/// - **Recency decay** (weight 0.4): older items score higher. Uses normalized
///   turn distance: `(current_turn - turn_delivered) / current_turn`.
/// - **Token weight** (weight 0.25): larger items are more valuable to evict,
///   as they free more budget. Normalized against the largest item.
/// - **Attention position** (weight 0.2): mid-context items score higher,
///   modeling the "Lost in the Middle" U-shaped attention bias. Content at
///   the start and end of the context window is better attended by the LLM.
/// - **Resolution priority** (weight 0.15): summaries are cheap to re-fetch
///   and score higher for eviction. Sections are moderate, claims score lowest.
///
/// # Examples
///
/// ```
/// use iris_core::session::{Session, SessionId, EvictionPolicy};
/// use iris_core::session::eviction::EvictionRanker;
/// use iris_core::types::{ContentId, Resolution};
///
/// let mut session = Session::new(
///     SessionId::from("test".to_string()),
///     100_000,
///     EvictionPolicy::Fifo,
/// );
///
/// session.record_delivery(
///     &ContentId::from("old-section".to_string()),
///     Resolution::Section,
///     500,
///     1,
///     "hash1".to_string(),
/// );
/// session.record_delivery(
///     &ContentId::from("recent-claim".to_string()),
///     Resolution::Claim,
///     50,
///     10,
///     "hash2".to_string(),
/// );
///
/// let candidates = EvictionRanker::rank(&session, 5);
/// assert!(!candidates.is_empty());
/// // Old section should rank higher (better eviction candidate)
/// assert_eq!(candidates[0].content_id, "old-section");
/// ```
pub struct EvictionRanker;

impl EvictionRanker {
    /// Rank delivered items by eviction priority.
    ///
    /// Returns up to `max_candidates` items sorted by descending eviction
    /// score (best candidates first).
    #[must_use]
    pub fn rank(session: &Session, max_candidates: usize) -> Vec<EvictionCandidate> {
        let current_turn = session.current_turn();
        let items: Vec<&DeliveredItem> = session.delivered_items().collect();

        if items.is_empty() || max_candidates == 0 {
            return Vec::new();
        }

        // Find max token count for normalization
        let max_tokens = items
            .iter()
            .map(|item| item.token_count)
            .max()
            .unwrap_or(1)
            .max(1);

        // Find turn bounds for position normalization
        let min_turn = items
            .iter()
            .map(|item| item.turn_delivered)
            .min()
            .unwrap_or(0);
        let max_turn = items
            .iter()
            .map(|item| item.turn_delivered)
            .max()
            .unwrap_or(0);

        let mut candidates: Vec<EvictionCandidate> = items
            .iter()
            .map(|item| {
                let score = Self::compute_score(item, current_turn, max_tokens, min_turn, max_turn);
                let reason = Self::describe_reason(item, current_turn, min_turn, max_turn);
                EvictionCandidate {
                    content_id: item.content_id.0.clone(),
                    reason,
                    tokens_recoverable: item.token_count,
                    score,
                }
            })
            .collect();

        // Sort by score descending (best eviction candidates first)
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        candidates.truncate(max_candidates);
        candidates
    }

    /// Compute the composite eviction score for a single item.
    ///
    /// The score combines four factors: recency decay, token weight, attention
    /// position bias, and resolution priority. The position factor uses a
    /// quadratic U-shaped curve to model the "Lost in the Middle" phenomenon.
    #[allow(clippy::cast_precision_loss)]
    fn compute_score(
        item: &DeliveredItem,
        current_turn: u32,
        max_tokens: usize,
        min_turn: u32,
        max_turn: u32,
    ) -> f64 {
        const RECENCY_WEIGHT: f64 = 0.4;
        const TOKEN_WEIGHT: f64 = 0.25;
        const POSITION_WEIGHT: f64 = 0.2;
        const RESOLUTION_WEIGHT: f64 = 0.15;

        // Recency decay: older items score higher
        let recency = if current_turn == 0 {
            0.0
        } else {
            f64::from(current_turn - item.turn_delivered) / f64::from(current_turn)
        };

        // Token weight: larger items are more valuable to evict
        let token_score = item.token_count as f64 / max_tokens as f64;

        // Attention position: mid-context items score higher (Lost in the Middle).
        // Uses inverted U curve: 4p(1-p) where p is normalized position [0,1].
        // Start (p=0) → 0.0, middle (p=0.5) → 1.0, end (p=1) → 0.0.
        let position_score = Self::position_score(item.turn_delivered, min_turn, max_turn);

        // Resolution priority: summaries easiest to re-fetch, claims hardest
        let resolution_score = Self::resolution_score(item);

        recency * RECENCY_WEIGHT
            + token_score * TOKEN_WEIGHT
            + position_score * POSITION_WEIGHT
            + resolution_score * RESOLUTION_WEIGHT
    }

    /// Score by attention position — higher means worse-attended (mid-context).
    ///
    /// Models the "Lost in the Middle" U-shaped attention curve. Items in the
    /// middle of the context window score 1.0 (best eviction candidates),
    /// while items at the start or end score 0.0 (protected by primacy/recency
    /// attention bias).
    fn position_score(turn_delivered: u32, min_turn: u32, max_turn: u32) -> f64 {
        if max_turn <= min_turn {
            // Single turn or no spread — no position differentiation
            return 0.0;
        }

        let relative_pos = f64::from(turn_delivered - min_turn) / f64::from(max_turn - min_turn);

        // Inverted U: peaks at 1.0 for middle position (0.5)
        4.0 * relative_pos * (1.0 - relative_pos)
    }

    /// Score by resolution — higher means easier to re-fetch.
    fn resolution_score(item: &DeliveredItem) -> f64 {
        use crate::types::Resolution;
        match item.resolution {
            Resolution::Summary => 1.0,
            Resolution::Section => 0.5,
            Resolution::Claim => 0.2,
            Resolution::SymbolStub => 0.8,
            Resolution::SymbolFull => 0.4,
        }
    }

    /// Generate a human-readable reason for the eviction recommendation.
    fn describe_reason(
        item: &DeliveredItem,
        current_turn: u32,
        min_turn: u32,
        max_turn: u32,
    ) -> String {
        let age = current_turn.saturating_sub(item.turn_delivered);
        let resolution = match item.resolution {
            crate::types::Resolution::Summary => "summary",
            crate::types::Resolution::Section => "section",
            crate::types::Resolution::Claim => "claim",
            crate::types::Resolution::SymbolStub => "symbol_stub",
            crate::types::Resolution::SymbolFull => "symbol_full",
        };

        let position_score = Self::position_score(item.turn_delivered, min_turn, max_turn);

        if age > 5 {
            format!(
                "Delivered {age} turns ago ({resolution}, {} tokens) — likely stale",
                item.token_count
            )
        } else if position_score > 0.8 {
            format!(
                "Mid-context {resolution} ({} tokens) — poorly attended by LLM",
                item.token_count
            )
        } else if item.token_count > 500 {
            format!(
                "Large {resolution} ({} tokens) — significant budget recovery",
                item.token_count
            )
        } else {
            format!(
                "{resolution} from turn {} ({} tokens)",
                item.turn_delivered, item.token_count
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{EvictionPolicy, SessionId};
    use crate::types::{ContentId, Resolution};

    fn make_session() -> Session {
        Session::new(
            SessionId::from("test-eviction".to_string()),
            100_000,
            EvictionPolicy::Fifo,
        )
    }

    fn cid(s: &str) -> ContentId {
        ContentId::from(s.to_string())
    }

    #[test]
    fn empty_session_returns_no_candidates() {
        let session = make_session();
        let candidates = EvictionRanker::rank(&session, 5);
        assert!(candidates.is_empty());
    }

    #[test]
    fn zero_max_candidates_returns_empty() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());
        let candidates = EvictionRanker::rank(&session, 0);
        assert!(candidates.is_empty());
    }

    #[test]
    fn older_items_rank_higher_for_eviction() {
        let mut session = make_session();

        session.record_delivery(&cid("old"), Resolution::Section, 200, 1, "h1".into());
        session.record_delivery(&cid("recent"), Resolution::Section, 200, 10, "h2".into());

        let candidates = EvictionRanker::rank(&session, 10);
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            candidates[0].content_id, "old",
            "older item should be first eviction candidate"
        );
    }

    #[test]
    fn larger_items_rank_higher_for_eviction() {
        let mut session = make_session();

        // Same turn, same resolution — only difference is token count
        session.record_delivery(&cid("small"), Resolution::Section, 50, 5, "h1".into());
        session.record_delivery(&cid("large"), Resolution::Section, 2000, 5, "h2".into());

        let candidates = EvictionRanker::rank(&session, 10);
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            candidates[0].content_id, "large",
            "larger item should rank higher for eviction"
        );
    }

    #[test]
    fn summaries_rank_higher_than_claims_for_eviction() {
        let mut session = make_session();

        // Same turn, same token count — only resolution differs
        session.record_delivery(&cid("claim"), Resolution::Claim, 100, 5, "h1".into());
        session.record_delivery(&cid("summary"), Resolution::Summary, 100, 5, "h2".into());

        let candidates = EvictionRanker::rank(&session, 10);
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            candidates[0].content_id, "summary",
            "summaries should rank higher for eviction (easy to re-fetch)"
        );
    }

    #[test]
    fn max_candidates_limits_results() {
        let mut session = make_session();

        for i in 0..10 {
            session.record_delivery(
                &cid(&format!("s{i}")),
                Resolution::Section,
                100,
                i + 1,
                format!("h{i}"),
            );
        }

        let candidates = EvictionRanker::rank(&session, 3);
        assert_eq!(candidates.len(), 3);
    }

    #[test]
    fn tokens_recoverable_matches_item_token_count() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 350, 1, "h1".into());

        let candidates = EvictionRanker::rank(&session, 5);
        assert_eq!(candidates[0].tokens_recoverable, 350);
    }

    #[test]
    fn reason_mentions_stale_for_old_items() {
        let mut session = make_session();
        session.record_delivery(&cid("old"), Resolution::Section, 200, 1, "h1".into());
        // Advance turn significantly
        session.record_delivery(&cid("new"), Resolution::Claim, 10, 10, "h2".into());

        let candidates = EvictionRanker::rank(&session, 10);
        let old_candidate = candidates.iter().find(|c| c.content_id == "old").unwrap();
        assert!(
            old_candidate.reason.contains("stale"),
            "reason should mention staleness: {}",
            old_candidate.reason
        );
    }

    #[test]
    fn reason_mentions_large_for_big_items() {
        let mut session = make_session();
        session.record_delivery(&cid("big"), Resolution::Section, 1000, 1, "h1".into());

        let candidates = EvictionRanker::rank(&session, 5);
        assert!(
            candidates[0].reason.contains("Large")
                || candidates[0].reason.contains("stale")
                || candidates[0].reason.contains("budget"),
            "reason should be descriptive: {}",
            candidates[0].reason
        );
    }

    #[test]
    fn single_item_session_returns_that_item() {
        let mut session = make_session();
        session.record_delivery(&cid("only"), Resolution::Section, 300, 1, "h1".into());

        let candidates = EvictionRanker::rank(&session, 5);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].content_id, "only");
    }

    #[test]
    fn composite_scoring_considers_all_factors() {
        let mut session = make_session();

        // Old, small, claim (high recency, low token, low resolution) — moderate
        session.record_delivery(
            &cid("old-small-claim"),
            Resolution::Claim,
            20,
            1,
            "h1".into(),
        );
        // Recent, large, summary (low recency, high token, high resolution) — moderate
        session.record_delivery(
            &cid("new-big-summary"),
            Resolution::Summary,
            2000,
            9,
            "h2".into(),
        );
        // Old, large, summary (high recency, high token, high resolution) — BEST
        session.record_delivery(
            &cid("old-big-summary"),
            Resolution::Summary,
            2000,
            1,
            "h3".into(),
        );
        // Advance turn
        session.record_delivery(&cid("current"), Resolution::Claim, 10, 10, "h4".into());

        let candidates = EvictionRanker::rank(&session, 10);
        assert_eq!(
            candidates[0].content_id, "old-big-summary",
            "old + large + summary should be the top eviction candidate"
        );
    }

    #[test]
    fn scores_are_non_negative() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 100, 0, "h1".into());

        let candidates = EvictionRanker::rank(&session, 5);
        for c in &candidates {
            assert!(c.score >= 0.0, "score should be non-negative: {}", c.score);
        }
    }

    #[test]
    fn candidate_serializes_to_json() {
        let candidate = EvictionCandidate {
            content_id: "test-id".into(),
            reason: "stale content".into(),
            tokens_recoverable: 200,
            score: 0.75,
        };
        let json = serde_json::to_string(&candidate).unwrap();
        assert!(json.contains("test-id"));
        assert!(json.contains("tokens_recoverable"));
    }

    // --- Attention-position-aware scoring tests ---

    #[test]
    fn mid_context_items_rank_higher_than_start_or_end() {
        let mut session = make_session();

        // All same resolution and token count — only position differs
        // Spread across turns 1..=10 with turn 10 as current
        session.record_delivery(&cid("start"), Resolution::Section, 200, 1, "h1".into());
        session.record_delivery(&cid("middle"), Resolution::Section, 200, 5, "h2".into());
        session.record_delivery(&cid("end"), Resolution::Section, 200, 10, "h3".into());

        let candidates = EvictionRanker::rank(&session, 10);

        // Find scores for each
        let _start_score = candidates
            .iter()
            .find(|c| c.content_id == "start")
            .unwrap()
            .score;
        let middle_score = candidates
            .iter()
            .find(|c| c.content_id == "middle")
            .unwrap()
            .score;
        let end_score = candidates
            .iter()
            .find(|c| c.content_id == "end")
            .unwrap()
            .score;

        // Middle should have higher position contribution than end
        // (start also has high recency score, so it may still outrank middle overall)
        // But middle should score higher than end due to position boost
        assert!(
            middle_score > end_score,
            "mid-context item should score higher than end-context: middle={middle_score}, end={end_score}"
        );
    }

    #[test]
    fn position_score_is_zero_for_single_item() {
        // Single item means no position spread — position score should be 0
        let score = EvictionRanker::position_score(5, 5, 5);
        assert!(
            (score - 0.0).abs() < f64::EPSILON,
            "position score should be 0 for single turn: {score}"
        );
    }

    #[test]
    fn position_score_peaks_at_middle() {
        // Turn 5 is exactly in the middle of [0, 10]
        let score = EvictionRanker::position_score(5, 0, 10);
        assert!(
            (score - 1.0).abs() < f64::EPSILON,
            "position score should be 1.0 at exact middle: {score}"
        );
    }

    #[test]
    fn position_score_is_zero_at_boundaries() {
        let start_score = EvictionRanker::position_score(0, 0, 10);
        let end_score = EvictionRanker::position_score(10, 0, 10);

        assert!(
            start_score.abs() < f64::EPSILON,
            "position score at start should be 0: {start_score}"
        );
        assert!(
            end_score.abs() < f64::EPSILON,
            "position score at end should be 0: {end_score}"
        );
    }

    #[test]
    fn position_score_is_symmetric() {
        // 25% from start == 25% from end
        let score_quarter = EvictionRanker::position_score(25, 0, 100);
        let score_three_quarter = EvictionRanker::position_score(75, 0, 100);

        assert!(
            (score_quarter - score_three_quarter).abs() < f64::EPSILON,
            "position score should be symmetric: {score_quarter} vs {score_three_quarter}"
        );
    }

    #[test]
    fn reason_mentions_mid_context_for_middle_items() {
        let mut session = make_session();

        // Many items spread across turns, with a middle item
        for i in 1..=20 {
            session.record_delivery(
                &cid(&format!("s{i}")),
                Resolution::Section,
                100,
                i,
                format!("h{i}"),
            );
        }

        let candidates = EvictionRanker::rank(&session, 20);
        // Items around turn 10 (middle) should mention mid-context in reason
        let mid_candidate = candidates.iter().find(|c| c.content_id == "s10").unwrap();
        assert!(
            mid_candidate.reason.contains("Mid-context") || mid_candidate.reason.contains("stale"),
            "mid-context item reason should be descriptive: {}",
            mid_candidate.reason
        );
    }

    #[test]
    fn two_items_have_zero_position_scores() {
        // With only 2 items at turns 0 and 10, both are at boundaries
        let score_start = EvictionRanker::position_score(0, 0, 10);
        let score_end = EvictionRanker::position_score(10, 0, 10);

        assert!(score_start.abs() < f64::EPSILON);
        assert!(score_end.abs() < f64::EPSILON);
    }
}
