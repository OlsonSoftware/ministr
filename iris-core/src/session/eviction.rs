//! Eviction ranking for context budget management.
//!
//! Scores delivered content items to determine which are best candidates for
//! eviction from the agent's context window. Items are ranked by a composite
//! score of recency decay, token weight, and resolution priority.

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
/// The ranking considers three factors:
/// - **Recency decay** (weight 0.5): older items score higher. Uses normalized
///   turn distance: `(current_turn - turn_delivered) / current_turn`.
/// - **Token weight** (weight 0.3): larger items are more valuable to evict,
///   as they free more budget. Normalized against the largest item.
/// - **Resolution priority** (weight 0.2): summaries are cheap to re-fetch
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

        let mut candidates: Vec<EvictionCandidate> = items
            .iter()
            .map(|item| {
                let score = Self::compute_score(item, current_turn, max_tokens);
                let reason = Self::describe_reason(item, current_turn);
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
    #[allow(clippy::cast_precision_loss)]
    fn compute_score(item: &DeliveredItem, current_turn: u32, max_tokens: usize) -> f64 {
        const RECENCY_WEIGHT: f64 = 0.5;
        const TOKEN_WEIGHT: f64 = 0.3;
        const RESOLUTION_WEIGHT: f64 = 0.2;

        // Recency decay: older items score higher
        let recency = if current_turn == 0 {
            0.0
        } else {
            f64::from(current_turn - item.turn_delivered) / f64::from(current_turn)
        };

        // Token weight: larger items are more valuable to evict
        let token_score = item.token_count as f64 / max_tokens as f64;

        // Resolution priority: summaries easiest to re-fetch, claims hardest
        let resolution_score = Self::resolution_score(item);

        recency * RECENCY_WEIGHT + token_score * TOKEN_WEIGHT + resolution_score * RESOLUTION_WEIGHT
    }

    /// Score by resolution — higher means easier to re-fetch.
    fn resolution_score(item: &DeliveredItem) -> f64 {
        use crate::types::Resolution;
        match item.resolution {
            Resolution::Summary => 1.0,
            Resolution::Section => 0.5,
            Resolution::Claim => 0.2,
        }
    }

    /// Generate a human-readable reason for the eviction recommendation.
    fn describe_reason(item: &DeliveredItem, current_turn: u32) -> String {
        let age = current_turn.saturating_sub(item.turn_delivered);
        let resolution = match item.resolution {
            crate::types::Resolution::Summary => "summary",
            crate::types::Resolution::Section => "section",
            crate::types::Resolution::Claim => "claim",
        };

        if age > 5 {
            format!(
                "Delivered {age} turns ago ({resolution}, {} tokens) — likely stale",
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
}
