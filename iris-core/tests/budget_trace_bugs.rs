//! Regression-guard tests for bugs found in the budget manager /
//! eviction ranker trace.

use iris_core::session::eviction::EvictionRanker;
use iris_core::session::{EvictionPolicy, Session, SessionId};
use iris_core::types::{ContentId, Resolution};

/// BG1 — Before the fix, `EvictionRanker::compute_base_score` did raw
/// `u32 - u32` on `current_turn - item.turn_delivered` with no guard.
/// `Session::restore` took `current_turn` and `delivered` as independent
/// arguments with zero validation, so an inconsistent persistence record
/// (or a buggy restore path) with `turn_delivered > current_turn` would:
///   * debug builds → `attempt to subtract with overflow` panic
///   * release builds → u32 wrap to ~4 billion → recency factor explodes
///
/// Fix: two-layer defence — `restore` clamps `current_turn` upward to the
/// max delivered turn, and the ranker uses `saturating_sub` on the hot
/// path as belt-and-braces. This test exercises both layers.
#[test]
fn bg1_ranker_handles_current_turn_below_turn_delivered() {
    use iris_core::session::DeliveredItem;
    use std::collections::BTreeMap;

    let cid = "future-item";
    let mut delivered = BTreeMap::new();
    delivered.insert(
        cid.to_string(),
        DeliveredItem {
            content_id: ContentId(cid.to_string()),
            resolution: Resolution::Section,
            token_count: 200,
            turn_delivered: 100, // > current_turn below
            content_hash: "h".into(),
            compression_tier: iris_core::session::CompressionTier::Full,
            compressed_summary: None,
        },
    );

    let session = Session::restore(
        SessionId::from("restored".to_string()),
        100_000,
        EvictionPolicy::Fifo,
        delivered,
        vec![ContentId(cid.to_string())],
        5, // current_turn < turn_delivered
    );

    // Pre-fix: this line panics (debug) or returns nonsense scores (release).
    // Post-fix: returns a single candidate with recency clamped to 0.0.
    let candidates = EvictionRanker::rank(&session, 5, None);

    assert_eq!(candidates.len(), 1);
    let factors = candidates[0].factors.as_ref().expect("factors present");
    assert!(
        (0.0..=1.0).contains(&factors.recency),
        "recency must stay in [0,1]; got {}",
        factors.recency
    );
    assert!(
        candidates[0].score.is_finite(),
        "score must be finite; got {}",
        candidates[0].score
    );
}

/// BG1 supplementary — a "`current_turn` == 0 with `turn_delivered` > 0"
/// case also crosses the underflow line if the early-return `if
/// current_turn == 0` branch is ever removed. This pins the expected
/// semantics: the zero-turn short-circuit returns 0 for recency
/// regardless of the item's claimed turn.
#[test]
fn bg1_zero_current_turn_short_circuit_is_safe() {
    use iris_core::session::DeliveredItem;
    use std::collections::BTreeMap;

    let cid = "future-item";
    let mut delivered = BTreeMap::new();
    delivered.insert(
        cid.to_string(),
        DeliveredItem {
            content_id: ContentId(cid.to_string()),
            resolution: Resolution::Section,
            token_count: 100,
            turn_delivered: 50,
            content_hash: "h".into(),
            compression_tier: iris_core::session::CompressionTier::Full,
            compressed_summary: None,
        },
    );

    let session = Session::restore(
        SessionId::from("restored-zero".to_string()),
        100_000,
        EvictionPolicy::Fifo,
        delivered,
        vec![ContentId(cid.to_string())],
        0,
    );

    let candidates = EvictionRanker::rank(&session, 5, None);
    assert_eq!(candidates.len(), 1);
    let factors = candidates[0].factors.as_ref().unwrap();
    assert!(
        (factors.recency - 0.0).abs() < f64::EPSILON,
        "current_turn=0 must yield recency=0, got {}",
        factors.recency
    );
}
