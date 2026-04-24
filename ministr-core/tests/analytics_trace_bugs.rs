//! Regression guards for bugs found in the Analytics trace.

use ministr_core::analytics::Analytics;
use ministr_core::session::{AccessMode, BudgetConfig, SessionRegistry};
use ministr_core::storage::{SqliteStorage, Storage};
use ministr_core::types::{ContentId, Resolution, SectionId};

fn sid(s: &str) -> SectionId {
    SectionId(s.to_string())
}

fn cid(s: &str) -> ContentId {
    ContentId(s.to_string())
}

/// AN1 regression — flushing co-access patterns repeatedly on a
/// growing trajectory must NOT inflate pair counts. Each pair should
/// be counted exactly once per session regardless of how many times
/// `record_co_access_incremental` runs.
///
/// Before the fix, calling `record_co_accesses(&full_trajectory)`
/// from `persist_session` after every tool call re-counted every
/// pair already present. The new `record_co_access_incremental` API
/// only generates pairs involving items NOT yet flushed, guaranteeing
/// one-increment-per-pair semantics.
#[tokio::test]
async fn an1_incremental_flush_counts_each_pair_once_per_session() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let analytics = Analytics::new(storage);

    // Simulate the Session-driven incremental flush: walk the
    // trajectory [a, b, c, d] one item at a time.
    //
    // After "a": new=[a], prev=[] → no pairs.
    // After "b": new=[b], prev=[a] → pair (a,b).
    // After "c": new=[c], prev=[a,b] → pairs (a,c), (b,c).
    // After "d": new=[d], prev=[a,b,c] → pairs (a,d), (b,d), (c,d).
    // Total: 6 pairs, each recorded ONCE.
    let trajectory = [sid("a"), sid("b"), sid("c"), sid("d")];
    let mut flushed: Vec<SectionId> = Vec::new();
    for i in 0..trajectory.len() {
        let new = &trajectory[i..=i];
        analytics
            .record_co_access_incremental(new, &flushed)
            .await
            .unwrap();
        flushed.push(trajectory[i].clone());
    }

    // Every pair should have co_count = 1.
    let co_a = analytics.co_accessed_with(&sid("a"), 10).await.unwrap();
    let ab = co_a
        .iter()
        .find(|r| r.section_id == sid("b"))
        .expect("(a,b) pair present");
    assert_eq!(
        ab.co_count, 1,
        "(a,b) must be counted once per session, not inflated by \
         repeated flushes; got {}",
        ab.co_count
    );

    let ac = co_a.iter().find(|r| r.section_id == sid("c")).unwrap();
    let ad = co_a.iter().find(|r| r.section_id == sid("d")).unwrap();
    assert_eq!(ac.co_count, 1);
    assert_eq!(ad.co_count, 1);
}

/// AN1 regression — the `Session::unflushed_co_access_items` +
/// `mark_co_access_flushed` pair correctly identifies new items in a
/// growing trajectory across multiple "persist" cycles. Exercises the
/// Session-level state machine that drives the Analytics incremental
/// API.
#[tokio::test]
async fn an1_session_tracks_unflushed_items_correctly() {
    let mut registry = SessionRegistry::new(BudgetConfig::default());
    let entry = registry.create_session("agent", None, AccessMode::ReadWrite);

    entry
        .session
        .record_delivery(&cid("a"), Resolution::Section, 100, 1, "h1".into());
    entry
        .session
        .record_delivery(&cid("b"), Resolution::Section, 100, 2, "h2".into());

    // First flush sees a+b as new.
    let (new, prev) = entry.session.unflushed_co_access_items();
    assert_eq!(new.len(), 2);
    assert!(prev.is_empty());
    let ids: Vec<String> = new.iter().map(|c| c.0.clone()).collect();
    assert!(ids.contains(&"a".to_string()));
    assert!(ids.contains(&"b".to_string()));
    entry.session.mark_co_access_flushed(new);

    // Re-accessing "a" (already flushed) must not show up as new.
    entry
        .session
        .record_delivery(&cid("a"), Resolution::Section, 100, 3, "h1".into());
    let (new, prev) = entry.session.unflushed_co_access_items();
    assert!(
        new.is_empty(),
        "re-read of already-flushed section must not appear as new; got {new:?}"
    );
    assert_eq!(prev.len(), 2);

    // Adding a genuinely new section marks it as new on next flush.
    entry
        .session
        .record_delivery(&cid("c"), Resolution::Section, 100, 4, "h3".into());
    let (new, prev) = entry.session.unflushed_co_access_items();
    assert_eq!(new.len(), 1);
    assert_eq!(new[0], cid("c"));
    assert_eq!(prev.len(), 2);
}

/// AN2 regression — `Storage::record_co_accesses` must skip self-pairs
/// even when the caller's input contains duplicate IDs. Prevents
/// `(x, x)` rows from corrupting the co-access table.
#[tokio::test]
async fn an2_storage_skips_self_pairs_on_duplicate_input() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    // Input contains 'a' twice.
    storage
        .record_co_accesses(&[sid("a"), sid("b"), sid("a")])
        .await
        .unwrap();

    let co_a = storage.get_co_accessed(&sid("a"), 10).await.unwrap();
    assert!(
        co_a.iter().all(|r| r.section_id != sid("a")),
        "no self-pair (a,a) must exist; got {:?}",
        co_a.iter()
            .map(|r| (r.section_id.0.as_str(), r.co_count))
            .collect::<Vec<_>>()
    );
}

/// AN2 regression — `Storage::record_co_access_pairs` also skips
/// self-pairs when callers supply (x, x) directly.
#[tokio::test]
async fn an2_storage_pairs_api_skips_self_pairs() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    storage
        .record_co_access_pairs(&[
            (sid("a"), sid("a")), // self-pair — must be skipped
            (sid("a"), sid("b")), // real pair
        ])
        .await
        .unwrap();

    let co_a = storage.get_co_accessed(&sid("a"), 10).await.unwrap();
    assert!(co_a.iter().all(|r| r.section_id != sid("a")));
    assert_eq!(co_a.iter().filter(|r| r.section_id == sid("b")).count(), 1);
}
