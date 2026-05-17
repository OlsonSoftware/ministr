//! Multi-tier compression pipeline for context budget management.
//!
//! Manages tier transitions for delivered content as budget pressure changes.
//! Content progresses through tiers: `Full → Abstractive → Extractive → Bookmark → Evicted`.
//!
//! The pipeline recommends tier promotions based on pressure level, access recency,
//! and token weight. Higher-pressure situations trigger more aggressive compression.

use serde::Serialize;

use super::types::{CompressionTier, DeliveredItem, Session};
use super::usage::UsageLevel;

/// A recommended tier promotion for a delivered item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct TierPromotion {
    /// The content ID to promote.
    pub content_id: String,
    /// Current compression tier.
    pub current_tier: CompressionTier,
    /// Recommended new tier.
    pub recommended_tier: CompressionTier,
    /// Estimated tokens freed by this promotion.
    pub estimated_tokens_freed: usize,
}

/// Multi-tier compression pipeline.
///
/// Recommends tier transitions for delivered content based on budget pressure.
/// The pipeline ensures graceful degradation — content is compressed in stages
/// before being fully evicted, preserving at least structural awareness
/// (bookmarks) until pressure forces complete removal.
///
/// # Tier progression rules
///
/// Promotions never move content to a less-compressed tier. Because
/// Abstractive retains ~10% while Extractive retains ~30%, any promotion
/// out of Abstractive skips straight to Bookmark.
///
/// | Pressure   | Current Tier  | Recommended Tier |
/// |------------|---------------|------------------|
/// | Normal     | any           | (no change)      |
/// | Elevated   | Full          | Extractive       |
/// | Elevated   | Abstractive   | Bookmark         |
/// | Elevated   | Extractive    | Bookmark         |
/// | Critical   | Full          | Abstractive      |
/// | Critical   | Abstractive   | Bookmark         |
/// | Critical   | Extractive    | Bookmark         |
/// | Critical   | Bookmark      | Evicted          |
///
/// # Examples
///
/// ```
/// use ministr_core::session::compression::CompressionPipeline;
/// use ministr_core::session::{CompressionTier, UsageLevel};
///
/// let next = CompressionPipeline::next_tier(CompressionTier::Full, UsageLevel::Elevated);
/// assert_eq!(next, Some(CompressionTier::Extractive));
///
/// let next = CompressionPipeline::next_tier(CompressionTier::Full, UsageLevel::Critical);
/// assert_eq!(next, Some(CompressionTier::Abstractive));
///
/// let next = CompressionPipeline::next_tier(CompressionTier::Full, UsageLevel::Normal);
/// assert_eq!(next, None);
/// ```
pub struct CompressionPipeline;

impl CompressionPipeline {
    /// Determine the next tier for content at the given pressure level.
    ///
    /// Returns `None` if no promotion is recommended (either pressure is
    /// normal or the content is already at the terminal tier for this
    /// pressure level).
    #[must_use]
    pub fn next_tier(current: CompressionTier, pressure: UsageLevel) -> Option<CompressionTier> {
        match pressure {
            UsageLevel::Normal => None,
            UsageLevel::Elevated => match current {
                CompressionTier::Full => Some(CompressionTier::Extractive),
                // Abstractive is already more compressed than Extractive
                // (~10% vs ~30% retained). Skip straight to Bookmark rather
                // than decompressing.
                CompressionTier::Abstractive | CompressionTier::Extractive => {
                    Some(CompressionTier::Bookmark)
                }
                CompressionTier::Bookmark | CompressionTier::Evicted => None,
            },
            UsageLevel::Critical => match current {
                CompressionTier::Full => Some(CompressionTier::Abstractive),
                // Same reasoning: Abstractive → Bookmark, not Extractive.
                CompressionTier::Abstractive | CompressionTier::Extractive => {
                    Some(CompressionTier::Bookmark)
                }
                CompressionTier::Bookmark => Some(CompressionTier::Evicted),
                CompressionTier::Evicted => None,
            },
        }
    }

    /// Recommend tier promotions for all delivered items in a session.
    ///
    /// Scans delivered items and returns promotions for any that should
    /// move to a higher compression tier given the current pressure level.
    /// Items are sorted by estimated tokens freed (largest first) to
    /// prioritize high-impact promotions.
    ///
    /// Respects access recency: recently accessed items (within the last
    /// `recency_protection_turns` turns) are skipped unless pressure is
    /// critical.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn recommend_promotions(
        session: &Session,
        pressure: UsageLevel,
        recency_protection_turns: u32,
    ) -> Vec<TierPromotion> {
        if pressure == UsageLevel::Normal {
            return Vec::new();
        }

        let current_turn = session.current_turn();
        let mut promotions = Vec::new();

        for item in session.delivered_items() {
            // Skip recently accessed items under elevated pressure
            if pressure == UsageLevel::Elevated {
                let age = current_turn.saturating_sub(item.turn_delivered);
                if age < recency_protection_turns {
                    continue;
                }
            }

            if let Some(next) = Self::next_tier(item.compression_tier, pressure) {
                let estimated_freed = Self::estimate_tokens_freed(item, next);
                promotions.push(TierPromotion {
                    content_id: item.content_id.0.clone(),
                    current_tier: item.compression_tier,
                    recommended_tier: next,
                    estimated_tokens_freed: estimated_freed,
                });
            }
        }

        // Sort by tokens freed descending (most impactful first)
        promotions.sort_by_key(|p| std::cmp::Reverse(p.estimated_tokens_freed));

        promotions
    }

    /// Estimate tokens freed by moving from current tier to target tier.
    ///
    /// Retention ratios come from [`CompressionTier::retention_ratio`],
    /// the canonical source of truth. If the target retains MORE than the
    /// current tier (i.e. a decompression), this returns 0 — there are no
    /// "freed" tokens in that direction. Callers upstream of this function
    /// (e.g. `next_tier`) are responsible for not recommending such
    /// transitions in the first place.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn estimate_tokens_freed(item: &DeliveredItem, target: CompressionTier) -> usize {
        let current_tokens = item.token_count;
        let estimated_remaining = match target {
            CompressionTier::Full => current_tokens,
            CompressionTier::Bookmark => 5, // heading stub
            CompressionTier::Evicted => 0,
            other => {
                // Derive remaining tokens from the canonical retention ratio.
                // Current token_count already reflects the current tier's
                // compression; scale by the ratio of target/current to get
                // the expected post-transition size.
                let current_ratio = item.compression_tier.retention_ratio();
                let target_ratio = other.retention_ratio();
                if current_ratio <= 0.0 {
                    current_tokens
                } else {
                    let scale = (target_ratio / current_ratio).min(1.0);
                    (current_tokens as f64 * scale) as usize
                }
            }
        };
        current_tokens.saturating_sub(estimated_remaining)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{DropPolicy, SessionId};
    use crate::types::{ContentId, Resolution};

    fn make_session() -> Session {
        Session::new(
            SessionId::from("test-compression".to_string()),
            100_000,
            DropPolicy::Fifo,
        )
    }

    fn cid(s: &str) -> ContentId {
        ContentId::from(s.to_string())
    }

    // --- next_tier tests ---

    #[test]
    fn normal_pressure_never_promotes() {
        for tier in [
            CompressionTier::Full,
            CompressionTier::Abstractive,
            CompressionTier::Extractive,
            CompressionTier::Bookmark,
            CompressionTier::Evicted,
        ] {
            assert_eq!(
                CompressionPipeline::next_tier(tier, UsageLevel::Normal),
                None,
                "no promotion under normal pressure for {tier:?}"
            );
        }
    }

    #[test]
    fn elevated_promotes_full_to_extractive() {
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Full, UsageLevel::Elevated),
            Some(CompressionTier::Extractive)
        );
    }

    #[test]
    fn elevated_promotes_extractive_to_bookmark() {
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Extractive, UsageLevel::Elevated),
            Some(CompressionTier::Bookmark)
        );
    }

    #[test]
    fn elevated_does_not_promote_bookmark() {
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Bookmark, UsageLevel::Elevated),
            None
        );
    }

    #[test]
    fn critical_promotes_full_to_abstractive() {
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Full, UsageLevel::Critical),
            Some(CompressionTier::Abstractive)
        );
    }

    #[test]
    fn critical_promotes_bookmark_to_evicted() {
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Bookmark, UsageLevel::Critical),
            Some(CompressionTier::Evicted)
        );
    }

    #[test]
    fn critical_does_not_promote_evicted() {
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Evicted, UsageLevel::Critical),
            None
        );
    }

    #[test]
    fn elevated_promotes_abstractive_to_bookmark() {
        // Abstractive (~10% retained) must never move to Extractive (~30%).
        // Under Elevated pressure it skips straight to Bookmark.
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Abstractive, UsageLevel::Elevated),
            Some(CompressionTier::Bookmark)
        );
    }

    #[test]
    fn critical_promotes_abstractive_to_bookmark() {
        // Same rule under Critical: never decompress Abstractive.
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Abstractive, UsageLevel::Critical),
            Some(CompressionTier::Bookmark)
        );
    }

    // --- recommend_promotions tests ---

    #[test]
    fn no_promotions_under_normal_pressure() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 500, 1, "h1".into());

        let promotions = CompressionPipeline::recommend_promotions(&session, UsageLevel::Normal, 3);
        assert!(promotions.is_empty());
    }

    #[test]
    fn elevated_promotes_old_items() {
        let mut session = make_session();
        session.record_delivery(&cid("old"), Resolution::Section, 500, 1, "h1".into());
        session.record_delivery(&cid("recent"), Resolution::Section, 500, 10, "h2".into());

        let promotions =
            CompressionPipeline::recommend_promotions(&session, UsageLevel::Elevated, 3);

        // Only old item should be promoted (recent is within recency protection)
        assert_eq!(promotions.len(), 1);
        assert_eq!(promotions[0].content_id, "old");
        assert_eq!(promotions[0].recommended_tier, CompressionTier::Extractive);
    }

    #[test]
    fn critical_promotes_all_items() {
        let mut session = make_session();
        session.record_delivery(&cid("old"), Resolution::Section, 500, 1, "h1".into());
        session.record_delivery(&cid("recent"), Resolution::Section, 500, 10, "h2".into());

        let promotions =
            CompressionPipeline::recommend_promotions(&session, UsageLevel::Critical, 3);

        // Both items should be promoted under critical pressure
        assert_eq!(promotions.len(), 2);
    }

    #[test]
    fn promotions_sorted_by_tokens_freed() {
        let mut session = make_session();
        session.record_delivery(&cid("small"), Resolution::Section, 100, 1, "h1".into());
        session.record_delivery(&cid("large"), Resolution::Section, 2000, 1, "h2".into());

        let promotions =
            CompressionPipeline::recommend_promotions(&session, UsageLevel::Elevated, 0);

        assert_eq!(promotions.len(), 2);
        assert_eq!(
            promotions[0].content_id, "large",
            "larger item should be first (more tokens freed)"
        );
        assert!(promotions[0].estimated_tokens_freed > promotions[1].estimated_tokens_freed);
    }

    #[test]
    fn already_bookmarked_not_promoted_under_elevated() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 500, 1, "h1".into());
        session.mask_to_bookmark(&cid("s1"), &["Chapter 1".into()]);

        let promotions =
            CompressionPipeline::recommend_promotions(&session, UsageLevel::Elevated, 0);
        assert!(
            promotions.is_empty(),
            "bookmarked items should not be promoted under elevated pressure"
        );
    }

    #[test]
    fn bookmarked_promoted_to_evicted_under_critical() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 500, 1, "h1".into());
        session.mask_to_bookmark(&cid("s1"), &["Chapter 1".into()]);

        let promotions =
            CompressionPipeline::recommend_promotions(&session, UsageLevel::Critical, 0);
        assert_eq!(promotions.len(), 1);
        assert_eq!(promotions[0].recommended_tier, CompressionTier::Evicted);
    }

    // --- estimate_tokens_freed tests ---

    #[test]
    fn estimate_freed_for_bookmark() {
        let item = DeliveredItem {
            content_id: ContentId("test".into()),
            resolution: Resolution::Section,
            token_count: 1000,
            turn_delivered: 1,
            content_hash: "h".into(),
            compression_tier: CompressionTier::Full,
            compressed_summary: None,
        };

        let freed = CompressionPipeline::estimate_tokens_freed(&item, CompressionTier::Bookmark);
        assert_eq!(freed, 995); // 1000 - 5 (bookmark stub)
    }

    #[test]
    fn estimate_freed_for_evicted() {
        let item = DeliveredItem {
            content_id: ContentId("test".into()),
            resolution: Resolution::Section,
            token_count: 1000,
            turn_delivered: 1,
            content_hash: "h".into(),
            compression_tier: CompressionTier::Bookmark,
            compressed_summary: None,
        };

        let freed = CompressionPipeline::estimate_tokens_freed(&item, CompressionTier::Evicted);
        assert_eq!(freed, 1000);
    }

    // --- TierPromotion serde ---

    #[test]
    fn tier_promotion_serializes() {
        let promo = TierPromotion {
            content_id: "test".into(),
            current_tier: CompressionTier::Full,
            recommended_tier: CompressionTier::Extractive,
            estimated_tokens_freed: 700,
        };
        let json = serde_json::to_string(&promo).unwrap();
        assert!(json.contains("extractive"));
        assert!(json.contains("700"));
    }
}
