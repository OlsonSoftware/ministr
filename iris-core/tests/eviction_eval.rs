//! Eviction strategy evaluation — compares salience-aware eviction (with
//! `MemoryTracker`) against FSRS-only eviction (without salience) on context
//! retention quality.
//!
//! Simulates a multi-turn agent session with a mix of task-relevant and
//! irrelevant content, then measures which strategy retains more of the
//! content the agent actually needs.

use iris_core::session::eviction::EvictionRanker;
use iris_core::session::memory::{AccessRating, MemoryTracker};
use iris_core::session::{EvictionPolicy, Session, SessionId};
use iris_core::types::{ContentId, Resolution};

fn cid(s: &str) -> ContentId {
    ContentId::from(s.to_string())
}

/// Build a session simulating 30 turns of mixed content delivery.
///
/// Task-relevant items (prefixed `task/`) relate to the agent's current work.
/// Irrelevant items (prefixed `noise/`) are background context.
fn build_eval_session() -> Session {
    let mut session = Session::new(
        SessionId::from("eval-eviction".to_string()),
        100_000,
        EvictionPolicy::Fifo,
    );

    // Turns 1-10: initial context loading (mix of relevant and noise)
    for i in 1..=5 {
        session.record_delivery(
            &cid(&format!("task/module_{i}.rs#main_struct")),
            Resolution::Section,
            500,
            i,
            format!("hash-task-{i}"),
        );
        session.record_delivery(
            &cid(&format!("noise/readme_{i}.md#intro")),
            Resolution::Section,
            400,
            i,
            format!("hash-noise-{i}"),
        );
    }

    // Turns 11-20: deeper investigation (more task-relevant content)
    for i in 6..=10 {
        session.record_delivery(
            &cid(&format!("task/impl_{i}.rs#function")),
            Resolution::Claim,
            200,
            i + 5,
            format!("hash-task-impl-{i}"),
        );
        session.record_delivery(
            &cid(&format!("noise/changelog_{i}.md#entry")),
            Resolution::Summary,
            300,
            i + 5,
            format!("hash-noise-cl-{i}"),
        );
    }

    // Turns 21-30: late-session work
    for i in 11..=15 {
        session.record_delivery(
            &cid(&format!("task/test_{i}.rs#test_case")),
            Resolution::Section,
            350,
            i + 10,
            format!("hash-task-test-{i}"),
        );
        session.record_delivery(
            &cid(&format!("noise/license_{i}.md#section")),
            Resolution::Section,
            250,
            i + 10,
            format!("hash-noise-lic-{i}"),
        );
    }

    // Record queries that establish the agent's current task
    session.record_query("task module main struct implementation");
    session.record_query("task impl function test case");

    session
}

/// Build a `MemoryTracker` that reflects repeated access to task-relevant items.
fn build_memory_tracker() -> MemoryTracker {
    let mut tracker = MemoryTracker::new();

    // Task-relevant items accessed multiple times (Good/Easy ratings)
    for i in 1..=5 {
        let id = format!("task/module_{i}.rs#main_struct");
        tracker.record_access(&id, i, AccessRating::Good);
        tracker.record_access(&id, i + 10, AccessRating::Easy); // re-accessed later
    }
    for i in 6..=10 {
        let id = format!("task/impl_{i}.rs#function");
        tracker.record_access(&id, i + 5, AccessRating::Good);
    }
    for i in 11..=15 {
        let id = format!("task/test_{i}.rs#test_case");
        tracker.record_access(&id, i + 10, AccessRating::Good);
    }

    // Noise items accessed once (initial delivery only)
    for i in 1..=5 {
        tracker.record_access(&format!("noise/readme_{i}.md#intro"), i, AccessRating::Good);
    }
    for i in 6..=10 {
        tracker.record_access(
            &format!("noise/changelog_{i}.md#entry"),
            i + 5,
            AccessRating::Good,
        );
    }
    for i in 11..=15 {
        tracker.record_access(
            &format!("noise/license_{i}.md#section"),
            i + 10,
            AccessRating::Good,
        );
    }

    tracker
}

/// Measure what fraction of the top-N eviction candidates are noise (irrelevant).
///
/// Higher = better: the strategy is correctly targeting noise for eviction.
#[allow(clippy::cast_precision_loss)]
fn noise_eviction_rate(
    candidates: &[iris_core::session::eviction::EvictionCandidate],
    n: usize,
) -> f64 {
    let top_n: Vec<_> = candidates.iter().take(n).collect();
    if top_n.is_empty() {
        return 0.0;
    }
    let noise_count = top_n
        .iter()
        .filter(|c| c.content_id.starts_with("noise/"))
        .count();
    noise_count as f64 / top_n.len() as f64
}

/// Measure what fraction of task-relevant items appear in the eviction list.
///
/// Lower = better: the strategy is protecting task-relevant content.
#[allow(clippy::cast_precision_loss)]
fn task_false_eviction_rate(
    candidates: &[iris_core::session::eviction::EvictionCandidate],
    n: usize,
) -> f64 {
    let top_n: Vec<_> = candidates.iter().take(n).collect();
    if top_n.is_empty() {
        return 0.0;
    }
    let task_count = top_n
        .iter()
        .filter(|c| c.content_id.starts_with("task/"))
        .count();
    // 15 total task items
    task_count as f64 / 15.0
}

#[test]
fn salience_aware_eviction_retains_more_task_relevant_items() {
    let session = build_eval_session();
    let memory = build_memory_tracker();

    // Evict half the items (15 out of 30)
    let evict_count = 15;

    // Strategy A: salience-aware (with MemoryTracker)
    let salience_candidates = EvictionRanker::rank(&session, evict_count, Some(&memory));

    // Strategy B: FSRS-only (no MemoryTracker — falls back to turn-based decay)
    let fsrs_only_candidates = EvictionRanker::rank(&session, evict_count, None);

    // Both should return candidates
    assert_eq!(salience_candidates.len(), evict_count);
    assert_eq!(fsrs_only_candidates.len(), evict_count);

    // Salience-aware should target more noise for eviction
    let salience_noise_rate = noise_eviction_rate(&salience_candidates, evict_count);
    let fsrs_noise_rate = noise_eviction_rate(&fsrs_only_candidates, evict_count);

    assert!(
        salience_noise_rate >= fsrs_noise_rate,
        "salience-aware should evict at least as much noise: \
         salience={salience_noise_rate:.2}, fsrs_only={fsrs_noise_rate:.2}"
    );

    // Salience-aware should have fewer false evictions of task items
    let salience_false_rate = task_false_eviction_rate(&salience_candidates, evict_count);
    let fsrs_false_rate = task_false_eviction_rate(&fsrs_only_candidates, evict_count);

    assert!(
        salience_false_rate <= fsrs_false_rate,
        "salience-aware should protect more task items: \
         salience_false={salience_false_rate:.2}, fsrs_false={fsrs_false_rate:.2}"
    );
}

#[test]
fn salience_aware_eviction_prioritizes_noise_in_top_candidates() {
    let session = build_eval_session();
    let memory = build_memory_tracker();

    // Look at just the top 5 eviction candidates
    let candidates = EvictionRanker::rank(&session, 5, Some(&memory));

    let noise_in_top5 = candidates
        .iter()
        .filter(|c| c.content_id.starts_with("noise/"))
        .count();

    // At least 3 of the top 5 should be noise when salience is active
    assert!(
        noise_in_top5 >= 3,
        "top 5 eviction candidates should be mostly noise: got {noise_in_top5}/5 noise items"
    );
}

#[test]
fn memory_tracker_boosts_retrievability_for_frequently_accessed() {
    let memory = build_memory_tracker();

    // Task items accessed multiple times should have higher retrievability
    let task_r = memory.retrievability("task/module_1.rs#main_struct", 25);
    let noise_r = memory.retrievability("noise/readme_1.md#intro", 25);

    assert!(
        task_r > noise_r,
        "frequently-accessed task item should have higher retrievability: \
         task={task_r:.3}, noise={noise_r:.3}"
    );
}

#[test]
fn salience_adjusted_retrievability_further_protects_salient_items() {
    let memory = build_memory_tracker();

    let base_r = memory.retrievability("task/module_1.rs#main_struct", 25);
    let salience_r =
        memory.salience_adjusted_retrievability("task/module_1.rs#main_struct", 25, 0.8);

    assert!(
        salience_r > base_r,
        "salience adjustment should boost retrievability: base={base_r:.3}, adjusted={salience_r:.3}"
    );
}

#[test]
fn eval_token_efficiency_salience_aware_frees_more_noise_tokens() {
    let session = build_eval_session();
    let memory = build_memory_tracker();

    let evict_count = 10;
    let salience_candidates = EvictionRanker::rank(&session, evict_count, Some(&memory));
    let fsrs_candidates = EvictionRanker::rank(&session, evict_count, None);

    // Sum tokens from noise items in each strategy's eviction list
    let salience_noise_tokens: usize = salience_candidates
        .iter()
        .filter(|c| c.content_id.starts_with("noise/"))
        .map(|c| c.tokens_recoverable)
        .sum();
    let fsrs_noise_tokens: usize = fsrs_candidates
        .iter()
        .filter(|c| c.content_id.starts_with("noise/"))
        .map(|c| c.tokens_recoverable)
        .sum();

    assert!(
        salience_noise_tokens >= fsrs_noise_tokens,
        "salience-aware should recover at least as many noise tokens: \
         salience={salience_noise_tokens}, fsrs={fsrs_noise_tokens}"
    );
}
