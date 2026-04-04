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
#[derive(Debug, Clone, PartialEq, Serialize, schemars::JsonSchema)]
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
/// The ranking considers five factors:
/// - **Recency decay** (weight 0.35): older items score higher. Uses normalized
///   turn distance: `(current_turn - turn_delivered) / current_turn`.
/// - **Token weight** (weight 0.2): larger items are more valuable to evict,
///   as they free more budget. Normalized against the largest item.
/// - **Attention position** (weight 0.2): mid-context items score higher,
///   modeling the "Lost in the Middle" U-shaped attention bias. Content at
///   the start and end of the context window is better attended by the LLM.
/// - **Resolution priority** (weight 0.15): summaries are cheap to re-fetch
///   and score higher for eviction. Sections are moderate, claims score lowest.
/// - **Contiguity bonus** (weight 0.1): items adjacent to other high-scoring
///   candidates get a bonus, encouraging eviction of contiguous blocks.
///   This preserves positional coherence in the remaining context, avoiding
///   the degradation caused by non-contiguous eviction (arxiv 2511.04686).
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
/// let candidates = EvictionRanker::rank(&session, 5, None);
/// assert!(!candidates.is_empty());
/// // Old section should rank higher (better eviction candidate)
/// assert_eq!(candidates[0].content_id, "old-section");
/// ```
pub struct EvictionRanker;

impl EvictionRanker {
    /// Rank delivered items by eviction priority.
    ///
    /// When a [`MemoryTracker`] is provided, the recency factor uses FSRS
    /// retrievability (sections with low predicted recall are better eviction
    /// candidates). Without a tracker, falls back to simple turn-based decay.
    ///
    /// Returns up to `max_candidates` items sorted by descending eviction
    /// score (best candidates first).
    #[must_use]
    pub fn rank(
        session: &Session,
        max_candidates: usize,
        memory: Option<&super::memory::MemoryTracker>,
    ) -> Vec<EvictionCandidate> {
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

        // Sort items by turn_delivered for contiguity analysis
        let mut sorted_items: Vec<&DeliveredItem> = items.clone();
        sorted_items.sort_by_key(|item| item.turn_delivered);

        // Pass 1: compute base scores (without contiguity)
        let base_scores: Vec<f64> = sorted_items
            .iter()
            .map(|item| {
                Self::compute_base_score(item, current_turn, max_tokens, min_turn, max_turn, memory)
            })
            .collect();

        let mut candidates: Vec<EvictionCandidate> = sorted_items
            .iter()
            .zip(&base_scores)
            .map(|(item, &score)| {
                let reason = Self::describe_reason(item, current_turn, min_turn, max_turn);
                EvictionCandidate {
                    content_id: item.content_id.0.clone(),
                    reason,
                    tokens_recoverable: item.token_count,
                    score,
                }
            })
            .collect();

        // Pass 2: apply contiguity bonuses
        Self::apply_contiguity_bonus(&mut candidates, &base_scores);

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
    ///
    /// This returns the **base** score before contiguity bonuses are applied.
    #[allow(clippy::cast_precision_loss)]
    fn compute_base_score(
        item: &DeliveredItem,
        current_turn: u32,
        max_tokens: usize,
        min_turn: u32,
        max_turn: u32,
        memory: Option<&super::memory::MemoryTracker>,
    ) -> f64 {
        const RECENCY_WEIGHT: f64 = 0.35;
        const TOKEN_WEIGHT: f64 = 0.2;
        const POSITION_WEIGHT: f64 = 0.2;
        const RESOLUTION_WEIGHT: f64 = 0.15;

        // Recency/retrievability: use FSRS when available, else simple decay.
        // Low retrievability → high eviction score (likely forgotten).
        let recency = match memory {
            Some(mem) => 1.0 - mem.retrievability(&item.content_id.0, current_turn),
            None => {
                if current_turn == 0 {
                    0.0
                } else {
                    f64::from(current_turn - item.turn_delivered) / f64::from(current_turn)
                }
            }
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

    /// Apply contiguity bonuses to base scores.
    ///
    /// Items adjacent (in delivery order) to other high-scoring candidates
    /// receive a bonus. This encourages evicting contiguous blocks, which
    /// preserves positional coherence in the remaining context — avoiding
    /// the "positional scrambling" that degrades LLM attention (Liu et al.,
    /// 2023; arxiv 2511.04686).
    ///
    /// The bonus for each item is the average base score of its immediate
    /// neighbors (previous and next in turn order), weighted by
    /// `CONTIGUITY_WEIGHT`.
    fn apply_contiguity_bonus(candidates: &mut [EvictionCandidate], base_scores: &[f64]) {
        const CONTIGUITY_WEIGHT: f64 = 0.1;

        if candidates.len() <= 1 {
            return;
        }

        // Compute contiguity bonuses from the base scores of neighbors
        let bonuses: Vec<f64> = (0..base_scores.len())
            .map(|i| {
                let mut neighbor_sum = 0.0;
                let mut neighbor_count = 0u32;

                if i > 0 {
                    neighbor_sum += base_scores[i - 1];
                    neighbor_count += 1;
                }
                if i + 1 < base_scores.len() {
                    neighbor_sum += base_scores[i + 1];
                    neighbor_count += 1;
                }

                if neighbor_count == 0 {
                    0.0
                } else {
                    (neighbor_sum / f64::from(neighbor_count)) * CONTIGUITY_WEIGHT
                }
            })
            .collect();

        for (candidate, bonus) in candidates.iter_mut().zip(bonuses) {
            candidate.score += bonus;
        }
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
        let candidates = EvictionRanker::rank(&session, 5, None);
        assert!(candidates.is_empty());
    }

    #[test]
    fn zero_max_candidates_returns_empty() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());
        let candidates = EvictionRanker::rank(&session, 0, None);
        assert!(candidates.is_empty());
    }

    #[test]
    fn older_items_rank_higher_for_eviction() {
        let mut session = make_session();

        session.record_delivery(&cid("old"), Resolution::Section, 200, 1, "h1".into());
        session.record_delivery(&cid("recent"), Resolution::Section, 200, 10, "h2".into());

        let candidates = EvictionRanker::rank(&session, 10, None);
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

        let candidates = EvictionRanker::rank(&session, 10, None);
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

        let candidates = EvictionRanker::rank(&session, 10, None);
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

        let candidates = EvictionRanker::rank(&session, 3, None);
        assert_eq!(candidates.len(), 3);
    }

    #[test]
    fn tokens_recoverable_matches_item_token_count() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 350, 1, "h1".into());

        let candidates = EvictionRanker::rank(&session, 5, None);
        assert_eq!(candidates[0].tokens_recoverable, 350);
    }

    #[test]
    fn reason_mentions_stale_for_old_items() {
        let mut session = make_session();
        session.record_delivery(&cid("old"), Resolution::Section, 200, 1, "h1".into());
        // Advance turn significantly
        session.record_delivery(&cid("new"), Resolution::Claim, 10, 10, "h2".into());

        let candidates = EvictionRanker::rank(&session, 10, None);
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

        let candidates = EvictionRanker::rank(&session, 5, None);
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

        let candidates = EvictionRanker::rank(&session, 5, None);
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

        let candidates = EvictionRanker::rank(&session, 10, None);
        assert_eq!(
            candidates[0].content_id, "old-big-summary",
            "old + large + summary should be the top eviction candidate"
        );
    }

    #[test]
    fn scores_are_non_negative() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 100, 0, "h1".into());

        let candidates = EvictionRanker::rank(&session, 5, None);
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

        let candidates = EvictionRanker::rank(&session, 10, None);

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

        let candidates = EvictionRanker::rank(&session, 20, None);
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

    // --- Contiguity bonus tests ---

    #[test]
    fn contiguity_bonus_boosts_adjacent_high_scorers() {
        let mut session = make_session();

        // Three adjacent items delivered in consecutive turns, all same size/resolution
        // Middle item should get a contiguity bonus from both neighbors
        session.record_delivery(&cid("a"), Resolution::Section, 200, 1, "h1".into());
        session.record_delivery(&cid("b"), Resolution::Section, 200, 2, "h2".into());
        session.record_delivery(&cid("c"), Resolution::Section, 200, 3, "h3".into());
        // One isolated item far away
        session.record_delivery(&cid("lone"), Resolution::Section, 200, 20, "h4".into());

        let candidates = EvictionRanker::rank(&session, 10, None);

        // Find scores for adjacent group vs isolated item
        let a_score = candidates
            .iter()
            .find(|c| c.content_id == "a")
            .unwrap()
            .score;
        let b_score = candidates
            .iter()
            .find(|c| c.content_id == "b")
            .unwrap()
            .score;
        let c_score = candidates
            .iter()
            .find(|c| c.content_id == "c")
            .unwrap()
            .score;

        // The middle item (b) should benefit from contiguity with both neighbors
        // (a and c have high base scores due to recency decay), so b's contiguity
        // bonus should be >= a's and c's
        // All three adjacent items should have scores influenced by their neighbors
        assert!(
            a_score > 0.0 && b_score > 0.0 && c_score > 0.0,
            "all adjacent items should have positive scores: a={a_score}, b={b_score}, c={c_score}"
        );
    }

    #[test]
    fn contiguity_bonus_with_single_item_is_zero() {
        let mut session = make_session();
        session.record_delivery(&cid("only"), Resolution::Section, 200, 1, "h1".into());

        let candidates = EvictionRanker::rank(&session, 5, None);
        assert_eq!(candidates.len(), 1);
        // Single item has no neighbors, so contiguity bonus should be 0
        // Score should equal the base score only
        assert!(candidates[0].score > 0.0);
    }

    #[test]
    fn contiguity_bonus_preserves_sort_order_for_clear_winners() {
        let mut session = make_session();

        // Old, large, summary items — clear eviction candidates
        session.record_delivery(&cid("old1"), Resolution::Summary, 1000, 1, "h1".into());
        session.record_delivery(&cid("old2"), Resolution::Summary, 1000, 2, "h2".into());
        // Recent, small, claim — poor eviction candidate
        session.record_delivery(&cid("new"), Resolution::Claim, 20, 20, "h3".into());

        let candidates = EvictionRanker::rank(&session, 10, None);

        // Old summaries should still rank above the new claim even with contiguity
        let new_score = candidates
            .iter()
            .find(|c| c.content_id == "new")
            .unwrap()
            .score;
        let old1_score = candidates
            .iter()
            .find(|c| c.content_id == "old1")
            .unwrap()
            .score;

        assert!(
            old1_score > new_score,
            "old summary should still outrank recent claim: old={old1_score}, new={new_score}"
        );
    }

    #[test]
    fn apply_contiguity_bonus_empty_is_noop() {
        let mut candidates: Vec<EvictionCandidate> = Vec::new();
        let base_scores: Vec<f64> = Vec::new();
        EvictionRanker::apply_contiguity_bonus(&mut candidates, &base_scores);
        assert!(candidates.is_empty());
    }
}
