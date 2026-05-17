//! Regression guards for session-registry bugs found via trace.

use ministr_core::session::memory::AccessRating;
use ministr_core::session::{
    AccessMode, DropPolicy, Session, SessionId, SessionRegistry, UsageConfig,
};
use ministr_core::types::{ContentId, Resolution};

/// SR1 regression — `Session::new`'s `eviction_policy` parameter is no
/// longer dead. The session now stores the policy and exposes it via
/// `Session::eviction_policy()`.
#[test]
fn sr1_session_new_stores_eviction_policy() {
    let budget = 1000;
    let s_fifo = Session::new(SessionId::from("a".to_string()), budget, DropPolicy::Fifo);
    let s_lru = Session::new(SessionId::from("a".to_string()), budget, DropPolicy::Lru);
    let s_fsrs = Session::new(SessionId::from("a".to_string()), budget, DropPolicy::Fsrs);

    assert_eq!(s_fifo.eviction_policy(), DropPolicy::Fifo);
    assert_eq!(s_lru.eviction_policy(), DropPolicy::Lru);
    assert_eq!(s_fsrs.eviction_policy(), DropPolicy::Fsrs);
}

/// SR2 regression — `UsageConfig` now carries an `eviction_policy`
/// field, and `SessionRegistry::create_session` threads it through to
/// the `UsageTracker`. A session created with an FSRS-configured
/// `UsageConfig` actually uses FSRS eviction (evicts the lowest-
/// retrievability entry), not FIFO. Before the fix the registry
/// hardcoded FIFO; the entire FSRS code path was unreachable through
/// the public API.
#[test]
fn sr2_fsrs_config_actually_uses_fsrs_eviction() {
    let config = UsageConfig {
        max_context_tokens: 100,
        pressure_threshold: 0.8,
        critical_threshold: 0.95,
        eviction_policy: DropPolicy::Fsrs,
    };
    let mut registry = SessionRegistry::new(config);
    registry.create_session("agent", None, AccessMode::ReadWrite);
    let entry = registry.get_session_mut("agent").unwrap();

    // Build a clear retrievability differential by aging "second" far
    // into the past while keeping "first" fresh. "second" was accessed
    // 100 turns ago; "first" was accessed at the current turn.
    entry.memory.record_access("second", 0, AccessRating::Good);
    entry.memory.record_access("first", 100, AccessRating::Good);

    // Deliver both (memory already recorded so record_tokens_with_memory
    // sees accurate scores at eviction time).
    entry.session.record_delivery(
        &ContentId::from("first".to_string()),
        Resolution::Section,
        40,
        100,
        "h1".into(),
    );
    let _ = entry
        .budget
        .record_tokens_with_memory("first", 40, &entry.memory, 100);

    entry.session.record_delivery(
        &ContentId::from("second".to_string()),
        Resolution::Section,
        40,
        100,
        "h2".into(),
    );
    let _ = entry
        .budget
        .record_tokens_with_memory("second", 40, &entry.memory, 100);

    // "third" just accessed at current turn — retrievability ≈ 1.0.
    entry.memory.record_access("third", 100, AccessRating::Good);

    // Push over capacity at turn 100.
    //   first:  last_access=100, t=0  → R ≈ 1.0
    //   second: last_access=0,   t=100 → R ≈ (1 + 100/9)^-1 ≈ 0.08
    //   third:  last_access=100, t=0  → R ≈ 1.0
    // FSRS picks "second" (lowest R). FIFO would pick "first" (oldest).
    let evicted = entry
        .budget
        .record_tokens_with_memory("third", 40, &entry.memory, 100);

    assert_eq!(
        evicted,
        vec!["second".to_string()],
        "FSRS-configured session must evict by lowest retrievability, \
         not FIFO order; got {evicted:?}"
    );

    // Session also reflects the configured policy.
    assert_eq!(entry.session.eviction_policy(), DropPolicy::Fsrs);
}

/// SR2 regression — default config still uses FIFO (backward compat).
#[test]
fn sr2_default_config_is_still_fifo() {
    let mut registry = SessionRegistry::new(UsageConfig::default());
    registry.create_session("agent", None, AccessMode::ReadWrite);
    let entry = registry.get_session("agent").unwrap();
    assert_eq!(entry.session.eviction_policy(), DropPolicy::Fifo);
}
