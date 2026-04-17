//! Regression guards for compression-pipeline bugs found via trace.

use iris_core::session::compression::CompressionPipeline;
use iris_core::session::{CompressionTier, PressureLevel};
use iris_core::session::{EvictionPolicy, Session, SessionId};
use iris_core::types::{ContentId, Resolution};

/// CH1 regression — Abstractive must NEVER promote to Extractive.
///
/// The enum ordering (Full < Abstractive < Extractive < Bookmark < Evicted)
/// disagrees with retention ratios (Full 100% > Extractive ~30% >
/// Abstractive ~10% > Bookmark ~0%). Before the fix, `next_tier` recommended
/// Abstractive → Extractive under both Elevated and Critical pressure,
/// which is actually a *decompression* — it triples the active text
/// from ~10% back to ~30% of the original. `recommend_promotions` would
/// then report positive `estimated_tokens_freed` for what is really an
/// addition to the context budget.
///
/// Fix: `next_tier` routes Abstractive straight to Bookmark under any
/// non-Normal pressure. `estimate_tokens_freed` now derives its numbers
/// from `CompressionTier::retention_ratio()` and refuses to report
/// "savings" for a decompression.
#[test]
fn ch1_abstractive_skips_extractive_and_goes_to_bookmark() {
    assert_eq!(
        CompressionPipeline::next_tier(CompressionTier::Abstractive, PressureLevel::Elevated),
        Some(CompressionTier::Bookmark),
    );
    assert_eq!(
        CompressionPipeline::next_tier(CompressionTier::Abstractive, PressureLevel::Critical),
        Some(CompressionTier::Bookmark),
    );
}

/// CH1 regression — `recommend_promotions` for an Abstractive item must
/// now surface the Bookmark transition (the only downstream tier that
/// actually saves tokens) and report a non-negative, believable
/// `estimated_tokens_freed`.
#[test]
fn ch1_recommend_promotions_reports_real_savings() {
    let mut session = Session::new(
        SessionId::from("ch1-regression".to_string()),
        100_000,
        EvictionPolicy::Fifo,
    );

    session.record_delivery(
        &ContentId::from("doc".to_string()),
        Resolution::Section,
        1000,
        1,
        "h".into(),
    );
    session.set_compression_tier(
        &ContentId::from("doc".to_string()),
        CompressionTier::Abstractive,
        100, // 10% retained after LLM summary
    );

    let promotions =
        CompressionPipeline::recommend_promotions(&session, PressureLevel::Critical, 0);

    let promo = promotions
        .iter()
        .find(|p| p.content_id == "doc")
        .expect("one promotion for the one Abstractive item");

    assert_eq!(promo.recommended_tier, CompressionTier::Bookmark);
    // Bookmark is a fixed 5-token heading stub. From 100 tokens, that's 95 freed.
    assert_eq!(promo.estimated_tokens_freed, 95);
}

/// CH1 regression — `CompressionTier::retention_ratio` is the canonical
/// source of truth for "how compressed is this tier", which is
/// monotonically non-increasing across the intended pipeline path
/// Full → Abstractive → Bookmark → Evicted (skipping Extractive when
/// starting from Abstractive). The enum's `PartialOrd` derive is NOT
/// monotonic here, which is why every comparator that cares about
/// compression strength must use this helper.
#[test]
fn ch1_retention_ratio_is_monotonic_along_pipeline_path() {
    // The actual pipeline path an item could take under sustained pressure:
    let path = [
        CompressionTier::Full,
        CompressionTier::Abstractive,
        CompressionTier::Bookmark,
        CompressionTier::Evicted,
    ];
    for window in path.windows(2) {
        let (a, b) = (window[0], window[1]);
        assert!(
            a.retention_ratio() >= b.retention_ratio(),
            "retention must not grow along the pipeline: \
             {a:?}({}) → {b:?}({})",
            a.retention_ratio(),
            b.retention_ratio()
        );
    }

    // And the enum ordering DOES contradict retention at the
    // Abstractive ↔ Extractive boundary — document that tension so
    // nobody re-introduces enum-comparison-as-compression-check later.
    assert!(CompressionTier::Abstractive < CompressionTier::Extractive);
    assert!(
        CompressionTier::Abstractive.retention_ratio()
            < CompressionTier::Extractive.retention_ratio()
    );
}
