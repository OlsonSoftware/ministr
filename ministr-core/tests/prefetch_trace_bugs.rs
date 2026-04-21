//! Regression guards for bugs found in the prefetch engine trace.
//!
//! Each test pins behaviour that was broken or silently incorrect before
//! the fixes landed. Keep these here (rather than in the in-module tests)
//! so they're read as post-mortem documentation alongside the fix.

use ministr_core::session::prefetch::{PrefetchEngine, TopicTracker};

/// PE1 regression — recency decay is driven by `advance_turn`, and the
/// fix wires it into `trigger_prefetch`. This test pins the engine's
/// contract: `advance_turn` is the public surface that moves the turn
/// counter, and it reliably advances `cache.current_turn()`.
///
/// In production the call site is in `trigger_prefetch` (ministr-mcp and
/// ministr-daemon) — verified manually because setting up the full MCP
/// server in a unit test is prohibitive.
#[test]
fn pe1_advance_turn_moves_recency_clock() {
    let mut engine = PrefetchEngine::new(10);
    assert_eq!(engine.cache().current_turn(), 0);

    for _ in 0..50 {
        engine.advance_turn();
    }

    assert_eq!(
        engine.cache().current_turn(),
        50,
        "advance_turn must increment the cache turn counter"
    );
}

/// PE2 regression — the tracker must reject embeddings whose dimension
/// differs from the first retained vector. Before the fix, mismatched
/// dimensions silently truncated or zero-padded, producing a topologically
/// wrong topic vector that would then be fed to HNSW.
#[test]
fn pe2_topic_tracker_rejects_mismatched_dim() {
    let mut tracker = TopicTracker::new(4, 0.3);

    // First embedding sets the expected dim.
    tracker.record_access(vec![1.0, 0.0, 0.0]);
    assert_eq!(tracker.len(), 1);

    // A 5-dim embedding arrives (e.g. embedder was swapped). It must be
    // rejected so the running topic vector stays self-consistent.
    tracker.record_access(vec![10.0, 20.0, 30.0, 40.0, 50.0]);
    assert_eq!(
        tracker.len(),
        1,
        "mismatched-dim embedding must not be stored"
    );

    // Same-dim embeddings continue to be accepted.
    tracker.record_access(vec![0.5, 0.5, 0.5]);
    assert_eq!(tracker.len(), 2);

    let topic = tracker.topic_vector().expect("vector produced");
    assert_eq!(topic.len(), 3, "topic dim matches the accepted inputs");
    assert!(topic.iter().all(|v| v.is_finite()));
}

/// PE2 regression — also reject when a LATER vector is shorter than the
/// first. Previously this zero-padded silently.
#[test]
fn pe2_topic_tracker_rejects_shorter_later_vec() {
    let mut tracker = TopicTracker::new(4, 0.3);

    tracker.record_access(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
    tracker.record_access(vec![10.0, 20.0]);

    assert_eq!(
        tracker.len(),
        1,
        "shorter mismatched embedding must be rejected"
    );
}
