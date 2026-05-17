#![allow(clippy::uninlined_format_args)]
//! Budget tracking hypothesis tests.
//!
//! Tests each hypothesis for why `ministr_usage` shows 0 tokens after `ministr_read`.

use ministr_core::session::{
    AccessMode, DropPolicy, SessionId, SessionRegistry, UsageConfig, UsageTracker, WindowEstimator,
};
use ministr_core::storage::{SqliteStorage, Storage};
use ministr_core::types::{ContentId, Resolution};

// ─── Hypothesis 1: WindowEstimator.record() doesn't update current_tokens ───

#[test]
fn window_estimator_record_updates_tokens() {
    let mut w = WindowEstimator::new(100_000, DropPolicy::Fifo);
    assert_eq!(w.estimated_used(), 0);

    let _ = w.record("sec-1", 500);
    assert_eq!(
        w.estimated_used(),
        500,
        "H1 FAIL: record() didn't update tokens"
    );

    let _ = w.record("sec-2", 300);
    assert_eq!(
        w.estimated_used(),
        800,
        "H1 FAIL: second record() didn't accumulate"
    );

    eprintln!(
        "H1 PASS: WindowEstimator.record() correctly updates tokens: {}",
        w.estimated_used()
    );
}

// ─── Hypothesis 2: UsageTracker.record_tokens() doesn't delegate to window ──

#[test]
fn budget_tracker_record_tokens_updates_status() {
    let config = UsageConfig {
        max_context_tokens: 100_000,
        ..UsageConfig::default()
    };
    let mut tracker = UsageTracker::new(config, DropPolicy::Fifo);

    let status_before = tracker.usage_status();
    assert_eq!(status_before.tokens_used, 0);

    let _ = tracker.record_tokens("sec-1", 500);
    let status_after = tracker.usage_status();
    assert!(
        status_after.tokens_used > 0,
        "H2 FAIL: record_tokens didn't update budget: {}",
        status_after.tokens_used
    );

    eprintln!(
        "H2 PASS: UsageTracker.record_tokens() → tokens_used = {}",
        status_after.tokens_used
    );
}

// ─── Hypothesis 3: SessionEntry budget resets when session is restored ────────

#[tokio::test]
async fn restored_session_budget_reflects_previous_deliveries() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    let budget_config = UsageConfig {
        max_context_tokens: 100_000,
        ..UsageConfig::default()
    };

    // Phase 1: Create a session and deliver content.
    let session_id = SessionId::from("test-budget-session".to_string());
    {
        let mut registry = SessionRegistry::new(budget_config.clone());
        let entry = registry.create_session(
            "test-budget-session",
            Some(budget_config.clone()),
            AccessMode::ReadWrite,
        );

        // Deliver some content.
        let cid = ContentId("sec-1".into());
        entry
            .session
            .record_delivery(&cid, Resolution::Section, 500, 1, "hash1".into());
        let _ = entry.budget.record_tokens("sec-1", 500);

        let status = entry.budget.usage_status();
        assert_eq!(status.tokens_used, 500, "Phase 1: budget should be 500");
        eprintln!("Phase 1: tokens_used = {} (correct)", status.tokens_used);

        // Persist the session.
        storage.save_session(&entry.session).await.unwrap();
    }

    // Phase 2: Restore the session (simulating restart).
    {
        let mut registry = SessionRegistry::new(budget_config.clone());
        let restored = storage.load_session(&session_id).await.unwrap().unwrap();

        let delivered_count = restored.delivered_count();
        eprintln!(
            "Phase 2: restored session has {} delivered items",
            delivered_count
        );
        assert!(
            delivered_count > 0,
            "H3 FAIL: restored session lost deliveries"
        );

        // This is what with_persistence does:
        let entry = registry.create_session(
            "test-budget-session",
            Some(budget_config.clone()),
            AccessMode::ReadWrite,
        );
        entry.session = restored;

        // Check if the budget tracker knows about the restored content.
        let status = entry.budget.usage_status();
        eprintln!(
            "Phase 2: tokens_used = {} (UsageTracker after restore)",
            status.tokens_used
        );

        if status.tokens_used == 0 {
            eprintln!("H3 CONFIRMED: UsageTracker resets to 0 after session restore!");
            eprintln!("  The Session has delivered items but the UsageTracker is fresh.");
            eprintln!("  This is the root cause: with_persistence creates a new UsageTracker");
            eprintln!("  but only restores the Session, not the budget state.");
        }

        // Now try reading "sec-1" again — will the session think it's already delivered?
        let cid = ContentId("sec-1".into());
        let already_delivered = entry.session.is_delivered(&cid);
        let has_changed = entry.session.has_changed(&cid, "hash1");
        eprintln!("Phase 2: is_delivered={already_delivered}, has_changed={has_changed}");

        if already_delivered && !has_changed {
            eprintln!("H3 CONFIRMED (part 2): read() will hit 'already delivered' path,");
            eprintln!("  skipping record_section_delivery entirely → budget stays at 0");
        }
    }
}

// ─── Hypothesis 3 FIX: replay delivered items into budget on restore ─────────

#[tokio::test]
async fn budget_replayed_after_restore() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    let budget_config = UsageConfig {
        max_context_tokens: 100_000,
        ..UsageConfig::default()
    };

    // Phase 1: Create session, deliver, persist.
    {
        let mut registry = SessionRegistry::new(budget_config.clone());
        let entry = registry.create_session(
            "replay-test",
            Some(budget_config.clone()),
            AccessMode::ReadWrite,
        );
        let cid = ContentId("sec-1".into());
        entry
            .session
            .record_delivery(&cid, Resolution::Section, 500, 1, "hash1".into());
        let _ = entry.budget.record_tokens("sec-1", 500);
        storage.save_session(&entry.session).await.unwrap();
    }

    // Phase 2: Restore and REPLAY delivered items into budget.
    {
        let session_id = SessionId::from("replay-test".to_string());
        let mut registry = SessionRegistry::new(budget_config.clone());
        let restored = storage.load_session(&session_id).await.unwrap().unwrap();

        let entry = registry.create_session(
            "replay-test",
            Some(budget_config.clone()),
            AccessMode::ReadWrite,
        );

        // THE FIX: replay delivered items into budget tracker.
        for item in restored.delivered_items() {
            let _ = entry
                .budget
                .record_tokens(item.content_id.as_ref(), item.token_count);
        }
        entry.session = restored;

        let status = entry.budget.usage_status();
        eprintln!("H3 FIX: tokens_used after replay = {}", status.tokens_used);
        assert_eq!(
            status.tokens_used, 500,
            "budget should reflect replayed deliveries"
        );
        eprintln!("H3 FIX PASS: budget correctly restored to 500 tokens");
    }
}

// ─── Hypothesis 4: Session.is_delivered returns true for content from prev session ──

#[tokio::test]
async fn is_delivered_true_after_restore() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    let session_id = SessionId::from("test-dedup".to_string());
    let budget_config = UsageConfig::default();

    // Create session, deliver, persist.
    {
        let mut registry = SessionRegistry::new(budget_config.clone());
        let entry = registry.create_session(
            "test-dedup",
            Some(budget_config.clone()),
            AccessMode::ReadWrite,
        );
        let cid = ContentId("sec-x".into());
        entry
            .session
            .record_delivery(&cid, Resolution::Section, 100, 1, "hashX".into());
        storage.save_session(&entry.session).await.unwrap();
    }

    // Restore and check.
    {
        let restored = storage.load_session(&session_id).await.unwrap().unwrap();
        let cid = ContentId("sec-x".into());
        let is_delivered = restored.is_delivered(&cid);
        let has_changed = restored.has_changed(&cid, "hashX");

        eprintln!("H4: is_delivered={is_delivered}, has_changed={has_changed}");

        if is_delivered && !has_changed {
            eprintln!(
                "H4 CONFIRMED: restored session marks content as 'already delivered, unchanged'"
            );
            eprintln!("  → ministr_read skips record_section_delivery → budget stays 0");
        } else if !is_delivered {
            eprintln!("H4 REJECTED: restored session does NOT mark content as delivered");
        } else {
            eprintln!("H4 PARTIAL: delivered but changed — will re-deliver (budget updates)");
        }
    }
}
