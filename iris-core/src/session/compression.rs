//! Multi-tier compression pipeline for context budget management.
//!
//! Manages tier transitions for delivered content as budget pressure changes.
//! Content progresses through tiers: `Full → Abstractive → Extractive → Bookmark → Evicted`.
//!
//! The pipeline recommends tier promotions based on pressure level, access recency,
//! and token weight. Higher-pressure situations trigger more aggressive compression.

use serde::Serialize;

use super::budget::PressureLevel;
use super::types::{CompressionTier, DeliveredItem, Session};

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
/// | Pressure   | Current Tier  | Recommended Tier |
/// |------------|---------------|------------------|
/// | Normal     | any           | (no change)      |
/// | Elevated   | Full          | Extractive       |
/// | Elevated   | Extractive    | Bookmark         |
/// | Critical   | Full          | Abstractive      |
/// | Critical   | Abstractive   | Extractive       |
/// | Critical   | Extractive    | Bookmark         |
/// | Critical   | Bookmark      | Evicted          |
///
/// # Examples
///
/// ```
/// use iris_core::session::compression::CompressionPipeline;
/// use iris_core::session::{CompressionTier, PressureLevel};
///
/// let next = CompressionPipeline::next_tier(CompressionTier::Full, PressureLevel::Elevated);
/// assert_eq!(next, Some(CompressionTier::Extractive));
///
/// let next = CompressionPipeline::next_tier(CompressionTier::Full, PressureLevel::Critical);
/// assert_eq!(next, Some(CompressionTier::Abstractive));
///
/// let next = CompressionPipeline::next_tier(CompressionTier::Full, PressureLevel::Normal);
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
    pub fn next_tier(current: CompressionTier, pressure: PressureLevel) -> Option<CompressionTier> {
        match pressure {
            PressureLevel::Normal => None,
            PressureLevel::Elevated => match current {
                CompressionTier::Full | CompressionTier::Abstractive => {
                    Some(CompressionTier::Extractive)
                }
                CompressionTier::Extractive => Some(CompressionTier::Bookmark),
                CompressionTier::Bookmark | CompressionTier::Evicted => None,
            },
            PressureLevel::Critical => match current {
                CompressionTier::Full => Some(CompressionTier::Abstractive),
                CompressionTier::Abstractive => Some(CompressionTier::Extractive),
                CompressionTier::Extractive => Some(CompressionTier::Bookmark),
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
        pressure: PressureLevel,
        recency_protection_turns: u32,
    ) -> Vec<TierPromotion> {
        if pressure == PressureLevel::Normal {
            return Vec::new();
        }

        let current_turn = session.current_turn();
        let mut promotions = Vec::new();

        for item in session.delivered_items() {
            // Skip recently accessed items under elevated pressure
            if pressure == PressureLevel::Elevated {
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
        promotions.sort_by(|a, b| b.estimated_tokens_freed.cmp(&a.estimated_tokens_freed));

        promotions
    }

    /// Estimate tokens freed by moving from current tier to target tier.
    ///
    /// Uses approximate compression ratios:
    /// - Abstractive: ~90% reduction
    /// - Extractive: ~70% reduction
    /// - Bookmark: ~95% reduction (heading stub only)
    /// - Evicted: 100% reduction
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    fn estimate_tokens_freed(item: &DeliveredItem, target: CompressionTier) -> usize {
        let current_tokens = item.token_count;
        let estimated_remaining = match target {
            CompressionTier::Full => current_tokens,
            CompressionTier::Abstractive => (current_tokens as f64 * 0.1) as usize,
            CompressionTier::Extractive => (current_tokens as f64 * 0.3) as usize,
            CompressionTier::Bookmark => 5, // heading stub
            CompressionTier::Evicted => 0,
        };
        current_tokens.saturating_sub(estimated_remaining)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{EvictionPolicy, SessionId};
    use crate::types::{ContentId, Resolution};

    fn make_session() -> Session {
        Session::new(
            SessionId::from("test-compression".to_string()),
            100_000,
            EvictionPolicy::Fifo,
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
                CompressionPipeline::next_tier(tier, PressureLevel::Normal),
                None,
                "no promotion under normal pressure for {tier:?}"
            );
        }
    }

    #[test]
    fn elevated_promotes_full_to_extractive() {
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Full, PressureLevel::Elevated),
            Some(CompressionTier::Extractive)
        );
    }

    #[test]
    fn elevated_promotes_extractive_to_bookmark() {
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Extractive, PressureLevel::Elevated),
            Some(CompressionTier::Bookmark)
        );
    }

    #[test]
    fn elevated_does_not_promote_bookmark() {
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Bookmark, PressureLevel::Elevated),
            None
        );
    }

    #[test]
    fn critical_promotes_full_to_abstractive() {
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Full, PressureLevel::Critical),
            Some(CompressionTier::Abstractive)
        );
    }

    #[test]
    fn critical_promotes_bookmark_to_evicted() {
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Bookmark, PressureLevel::Critical),
            Some(CompressionTier::Evicted)
        );
    }

    #[test]
    fn critical_does_not_promote_evicted() {
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Evicted, PressureLevel::Critical),
            None
        );
    }

    #[test]
    fn elevated_promotes_abstractive_to_extractive() {
        assert_eq!(
            CompressionPipeline::next_tier(CompressionTier::Abstractive, PressureLevel::Elevated),
            Some(CompressionTier::Extractive)
        );
    }

    // --- recommend_promotions tests ---

    #[test]
    fn no_promotions_under_normal_pressure() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 500, 1, "h1".into());

        let promotions =
            CompressionPipeline::recommend_promotions(&session, PressureLevel::Normal, 3);
        assert!(promotions.is_empty());
    }

    #[test]
    fn elevated_promotes_old_items() {
        let mut session = make_session();
        session.record_delivery(&cid("old"), Resolution::Section, 500, 1, "h1".into());
        session.record_delivery(&cid("recent"), Resolution::Section, 500, 10, "h2".into());

        let promotions =
            CompressionPipeline::recommend_promotions(&session, PressureLevel::Elevated, 3);

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
            CompressionPipeline::recommend_promotions(&session, PressureLevel::Critical, 3);

        // Both items should be promoted under critical pressure
        assert_eq!(promotions.len(), 2);
    }

    #[test]
    fn promotions_sorted_by_tokens_freed() {
        let mut session = make_session();
        session.record_delivery(&cid("small"), Resolution::Section, 100, 1, "h1".into());
        session.record_delivery(&cid("large"), Resolution::Section, 2000, 1, "h2".into());

        let promotions =
            CompressionPipeline::recommend_promotions(&session, PressureLevel::Elevated, 0);

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
            CompressionPipeline::recommend_promotions(&session, PressureLevel::Elevated, 0);
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
            CompressionPipeline::recommend_promotions(&session, PressureLevel::Critical, 0);
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
