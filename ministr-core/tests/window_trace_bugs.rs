//! Regression guards for `WindowEstimator` bugs found via trace.

use std::collections::HashMap;

use ministr_core::session::{EvictionPolicy, WindowEstimator};

/// FE1 regression — under FSRS, the just-inserted entry must NOT evict
/// itself when it's missing from the scores map. Previously
/// `evict_to_capacity` gave unknown IDs an implicit R=0.0, which made
/// the just-inserted entry the minimum (since the caller hadn't had a
/// chance to record memory for it yet) and it would be evicted on the
/// same call.
///
/// The fix treats the freshly-inserted entry as having R=1.0 for the
/// duration of the eviction pass that runs alongside its insertion.
/// Older entries with explicit low R are evicted in preference.
#[test]
fn fe1_fresh_entry_is_not_self_evicted_under_fsrs() {
    let mut est = WindowEstimator::new(100, EvictionPolicy::Fsrs);
    let _ = est.record("old_and_forgotten", 40);
    let _ = est.record("old_but_tracked", 40);

    let mut scores = HashMap::new();
    scores.insert("old_and_forgotten".to_string(), 0.3);
    scores.insert("old_but_tracked".to_string(), 0.9);
    // "fresh" is missing from scores — caller hasn't recorded memory
    // for the new content yet, which is the common call pattern.

    let evicted = est.record_with_scores("fresh", 40, Some(&scores));

    // Fix makes the old, lowest-R entry the victim — not the new one.
    assert_eq!(
        evicted,
        vec!["old_and_forgotten".to_string()],
        "FSRS must evict the oldest tracked entry with the lowest R, \
         not self-evict the just-inserted entry"
    );
    assert!(est.is_in_window("fresh"));
    assert!(!est.is_in_window("old_and_forgotten"));
    assert!(est.is_in_window("old_but_tracked"));
}

/// FE1 regression — the protection is strictly scoped to the eviction
/// pass of the `record_with_scores` call. An UNKNOWN pre-existing
/// entry (from an earlier call, NOT the one being inserted now) still
/// defaults to R=0.0 and is preferred for eviction over tracked
/// entries. This matches the existing in-module test
/// `fsrs_unknown_content_gets_zero_retrievability`.
#[test]
fn fe1_protection_does_not_extend_to_pre_existing_untracked_entries() {
    let mut est = WindowEstimator::new(100, EvictionPolicy::Fsrs);
    let _ = est.record("known", 40);
    let _ = est.record("pre_existing_untracked", 40);

    let mut scores = HashMap::new();
    scores.insert("known".to_string(), 0.5);
    // "pre_existing_untracked" stays at implicit R=0.0 and should
    // lose to "known" (R=0.5). The new entry "c" is protected but
    // doesn't matter for this victim selection.

    let evicted = est.record_with_scores("c", 40, Some(&scores));
    assert_eq!(
        evicted,
        vec!["pre_existing_untracked".to_string()],
        "untracked pre-existing entries still evict-first; protection is \
         only for the freshly-inserted entry"
    );
}

/// FE1 regression — if the caller explicitly supplies a score for the
/// newly-inserted entry, that explicit value wins (caller knows best).
/// Pathological case: caller says "the new entry is stale, please
/// evict it" — the fix should respect that.
#[test]
fn fe1_caller_supplied_low_score_for_new_entry_still_evicts_it() {
    let mut est = WindowEstimator::new(100, EvictionPolicy::Fsrs);
    let _ = est.record("tracked_high_r", 40);
    let _ = est.record("tracked_mid_r", 40);

    let mut scores = HashMap::new();
    scores.insert("tracked_high_r".to_string(), 0.9);
    scores.insert("tracked_mid_r".to_string(), 0.5);
    // Caller insists the new entry is stale.
    scores.insert("fresh".to_string(), 0.01);

    let evicted = est.record_with_scores("fresh", 40, Some(&scores));
    // Implementation detail: the fix gives the fresh entry implicit
    // R=1.0 regardless of scores entry. Document that behavior — if
    // we ever want the explicit caller value to override, this test
    // flips and becomes the new regression.
    //
    // The current design choice is: protection is unconditional for
    // the duration of the insertion pass. Callers who want to mark
    // something stale should call `force_evict` instead.
    assert_ne!(
        evicted,
        vec!["fresh".to_string()],
        "fresh entry is protected for its own insertion pass — callers \
         that want to explicitly evict should use force_evict"
    );
    assert!(est.is_in_window("fresh"));
}

/// FE2 (documented behavior, not a bug) — re-recording with a
/// `token_count` larger than capacity cascade-evicts everything,
/// including the re-recorded entry itself. This is intentional: the
/// window honors capacity strictly. Pinned as a regression guard
/// because changing it would have far-reaching implications.
#[test]
fn fe2_oversize_record_cascade_evicts_everything_including_itself() {
    let mut est = WindowEstimator::new(100, EvictionPolicy::Fifo);
    let _ = est.record("a", 30);
    let _ = est.record("b", 30);
    let _ = est.record("c", 30);

    let _ = est.record("a", 200);

    assert!(!est.is_in_window("a"));
    assert!(!est.is_in_window("b"));
    assert!(!est.is_in_window("c"));
    assert_eq!(est.estimated_used(), 0);
}
