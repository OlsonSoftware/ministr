//! Federated session registry for multi-agent context sharing.
//!
//! The [`SessionRegistry`] manages multiple named agent sessions that share a
//! single indexed corpus. Each session has its own [`Session`] shadow,
//! [`BudgetTracker`], and [`AccessMode`], enabling independent budget tracking
//! and isolation policies across concurrent agents.
//!
//! # Architecture
//!
//! The registry sits between the MCP server and individual sessions. Each MCP
//! connection identifies its active session by name, and the registry routes
//! operations to the correct session entry. Cross-session coherence is
//! propagated by [`invalidate_all`](SessionRegistry::invalidate_all), which
//! marks changed content as stale in every session.
//!
//! # Examples
//!
//! ```
//! use ministr_core::session::{
//!     AccessMode, BudgetConfig, EvictionPolicy, SessionRegistry,
//! };
//!
//! let config = BudgetConfig::default();
//! let mut registry = SessionRegistry::new(config);
//!
//! // Create two sessions sharing the same corpus
//! registry.get_or_create("agent-1", None, AccessMode::ReadWrite);
//! registry.get_or_create("agent-2", None, AccessMode::ReadOnly);
//!
//! assert_eq!(registry.session_count(), 2);
//! ```

use std::collections::HashMap;

use super::budget::{BudgetConfig, BudgetTracker};
use super::types::{AccessMode, Session, SessionId};

/// A single session entry in the registry, bundling session state with its
/// budget tracker and access mode.
pub struct SessionEntry {
    /// The session shadow tracking delivered content and access patterns.
    pub session: Session,
    /// Independent budget tracker for this session's context window.
    pub budget: BudgetTracker,
    /// Access mode controlling what operations this session can perform.
    pub access_mode: AccessMode,
    /// FSRS-based memory tracker for importance-aware eviction.
    pub memory: super::memory::MemoryTracker,
    /// Parent session id when this session was created on behalf of a
    /// subagent (e.g. Claude Code's Task tool spawning a sub-claude).
    /// Populated from `MINISTR_PARENT_SESSION_ID` at startup or from
    /// the MCP client's metadata on `initialize`. `None` for top-level
    /// sessions.
    pub parent_session_id: Option<SessionId>,
    /// MCP `clientInfo.name` captured at initialize. Helps the tray /
    /// SessionDashboard tell e.g. `claude-code` from `claude-subagent`
    /// from `mcp-inspector` apart. `None` until the handshake completes.
    pub client_name: Option<String>,
}

/// Registry managing multiple named sessions that share a single corpus.
///
/// Each session has independent budget tracking and access policies.
/// Cross-session coherence is propagated via [`invalidate_all`](Self::invalidate_all).
pub struct SessionRegistry {
    /// Map of session ID string to session entry.
    sessions: HashMap<String, SessionEntry>,
    /// Default budget configuration for new sessions.
    default_budget_config: BudgetConfig,
}

impl SessionRegistry {
    /// Create a new empty registry with the given default budget configuration.
    #[must_use]
    pub fn new(default_budget_config: BudgetConfig) -> Self {
        Self {
            sessions: HashMap::new(),
            default_budget_config,
        }
    }

    /// Create a new session with the given ID, budget config, and access mode.
    ///
    /// Returns a mutable reference to the new session entry. If a session with
    /// the same ID already exists, it is replaced.
    ///
    /// # Panics
    ///
    /// This method does not panic under normal use; the internal `expect` is
    /// guarded by a preceding insert.
    pub fn create_session(
        &mut self,
        id: &str,
        budget_config: Option<BudgetConfig>,
        access_mode: AccessMode,
    ) -> &mut SessionEntry {
        let config = budget_config.unwrap_or_else(|| self.default_budget_config.clone());
        let policy = config.eviction_policy;
        let session = Session::new(
            SessionId::from(id.to_string()),
            config.max_context_tokens,
            policy,
        );
        let budget = BudgetTracker::new(config, policy);
        self.sessions.insert(
            id.to_string(),
            SessionEntry {
                session,
                budget,
                access_mode,
                memory: super::memory::MemoryTracker::new(),
                parent_session_id: None,
                client_name: None,
            },
        );
        self.sessions.get_mut(id).expect("just inserted")
    }

    /// Get a reference to a session entry by ID.
    #[must_use]
    pub fn get_session(&self, id: &str) -> Option<&SessionEntry> {
        self.sessions.get(id)
    }

    /// Get a mutable reference to a session entry by ID.
    pub fn get_session_mut(&mut self, id: &str) -> Option<&mut SessionEntry> {
        self.sessions.get_mut(id)
    }

    /// Get or create a session entry.
    ///
    /// If a session with the given ID exists, returns a mutable reference to it.
    /// Otherwise, creates a new session with the given (or default) budget config
    /// and access mode.
    ///
    /// # Panics
    ///
    /// This method does not panic under normal use; the internal `expect` is
    /// guarded by a preceding existence check.
    pub fn get_or_create(
        &mut self,
        id: &str,
        budget_config: Option<BudgetConfig>,
        access_mode: AccessMode,
    ) -> &mut SessionEntry {
        if self.sessions.contains_key(id) {
            return self.sessions.get_mut(id).expect("just checked");
        }
        self.create_session(id, budget_config, access_mode)
    }

    /// Remove a session from the registry.
    ///
    /// Returns the removed session entry, or `None` if no session with the
    /// given ID existed.
    pub fn remove_session(&mut self, id: &str) -> Option<SessionEntry> {
        self.sessions.remove(id)
    }

    /// List all session IDs in the registry.
    #[must_use]
    pub fn session_ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }

    /// Number of sessions in the registry.
    #[must_use]
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Propagate coherence invalidation to all sessions.
    ///
    /// Calls [`Session::invalidate_sections`] on every session in the registry.
    /// Returns the total number of items invalidated across all sessions.
    pub fn invalidate_all(&mut self, changed_section_ids: &[String]) -> usize {
        let mut total = 0;
        for entry in self.sessions.values_mut() {
            total += entry.session.invalidate_sections(changed_section_ids);
        }
        total
    }

    /// Check if a session exists in the registry.
    #[must_use]
    pub fn contains(&self, id: &str) -> bool {
        self.sessions.contains_key(id)
    }

    /// Get the default budget configuration.
    #[must_use]
    pub fn default_budget_config(&self) -> &BudgetConfig {
        &self.default_budget_config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::PressureLevel;
    use crate::types::{ContentId, Resolution};

    fn default_registry() -> SessionRegistry {
        SessionRegistry::new(BudgetConfig::default())
    }

    fn small_registry() -> SessionRegistry {
        SessionRegistry::new(BudgetConfig {
            max_context_tokens: 1000,
            pressure_threshold: 0.8,
            critical_threshold: 0.95,
            ..BudgetConfig::default()
        })
    }

    // ── Lifecycle tests ──────────────────────────────────────────────

    #[test]
    fn new_registry_is_empty() {
        let registry = default_registry();
        assert_eq!(registry.session_count(), 0);
        assert!(registry.session_ids().is_empty());
    }

    #[test]
    fn create_session_adds_entry() {
        let mut registry = default_registry();
        registry.create_session("agent-1", None, AccessMode::ReadWrite);

        assert_eq!(registry.session_count(), 1);
        assert!(registry.contains("agent-1"));
        assert!(!registry.contains("agent-2"));
    }

    #[test]
    fn create_session_with_custom_budget() {
        let mut registry = default_registry();
        let custom = BudgetConfig {
            max_context_tokens: 5000,
            pressure_threshold: 0.7,
            critical_threshold: 0.9,
            ..BudgetConfig::default()
        };
        registry.create_session("agent-1", Some(custom), AccessMode::ReadWrite);

        let entry = registry.get_session("agent-1").unwrap();
        assert_eq!(entry.session.agent_context_budget, 5000);
        assert_eq!(entry.budget.config().max_context_tokens, 5000);
    }

    #[test]
    fn create_session_replaces_existing() {
        let mut registry = small_registry();
        let entry = registry.create_session("agent-1", None, AccessMode::ReadWrite);
        entry.session.record_delivery(
            &ContentId::from("s1".to_string()),
            Resolution::Section,
            100,
            1,
            "h1".into(),
        );
        assert_eq!(
            registry
                .get_session("agent-1")
                .unwrap()
                .session
                .delivered_count(),
            1
        );

        // Replace with new session
        registry.create_session("agent-1", None, AccessMode::ReadOnly);
        let entry = registry.get_session("agent-1").unwrap();
        assert_eq!(entry.session.delivered_count(), 0);
        assert_eq!(entry.access_mode, AccessMode::ReadOnly);
    }

    #[test]
    fn get_session_returns_none_for_missing() {
        let registry = default_registry();
        assert!(registry.get_session("nonexistent").is_none());
    }

    #[test]
    fn get_session_mut_allows_mutation() {
        let mut registry = small_registry();
        registry.create_session("agent-1", None, AccessMode::ReadWrite);

        let entry = registry.get_session_mut("agent-1").unwrap();
        entry.session.record_delivery(
            &ContentId::from("s1".to_string()),
            Resolution::Section,
            100,
            1,
            "h1".into(),
        );
        let _ = entry.budget.record_tokens("s1", 100);

        let entry = registry.get_session("agent-1").unwrap();
        assert_eq!(entry.session.delivered_count(), 1);
        assert_eq!(entry.budget.budget_status().tokens_used, 100);
    }

    #[test]
    fn get_or_create_returns_existing() {
        let mut registry = small_registry();
        let entry = registry.create_session("agent-1", None, AccessMode::ReadWrite);
        entry.session.record_delivery(
            &ContentId::from("s1".to_string()),
            Resolution::Section,
            100,
            1,
            "h1".into(),
        );

        // get_or_create should return existing, not replace
        let entry = registry.get_or_create("agent-1", None, AccessMode::ReadOnly);
        assert_eq!(entry.session.delivered_count(), 1);
        assert_eq!(entry.access_mode, AccessMode::ReadWrite); // unchanged
    }

    #[test]
    fn get_or_create_creates_new() {
        let mut registry = small_registry();
        registry.get_or_create("agent-1", None, AccessMode::ReadOnly);

        assert_eq!(registry.session_count(), 1);
        let entry = registry.get_session("agent-1").unwrap();
        assert_eq!(entry.access_mode, AccessMode::ReadOnly);
    }

    #[test]
    fn remove_session_returns_entry() {
        let mut registry = small_registry();
        registry.create_session("agent-1", None, AccessMode::ReadWrite);

        let removed = registry.remove_session("agent-1");
        assert!(removed.is_some());
        assert_eq!(registry.session_count(), 0);
        assert!(!registry.contains("agent-1"));
    }

    #[test]
    fn remove_nonexistent_returns_none() {
        let mut registry = default_registry();
        assert!(registry.remove_session("ghost").is_none());
    }

    #[test]
    fn session_ids_lists_all() {
        let mut registry = default_registry();
        registry.create_session("alpha", None, AccessMode::ReadWrite);
        registry.create_session("bravo", None, AccessMode::ReadOnly);
        registry.create_session("charlie", None, AccessMode::ReadWrite);

        let mut ids = registry.session_ids();
        ids.sort();
        assert_eq!(ids, vec!["alpha", "bravo", "charlie"]);
    }

    // ── Budget independence tests ────────────────────────────────────

    #[test]
    fn budgets_are_independent() {
        let mut registry = small_registry();
        registry.create_session("agent-1", None, AccessMode::ReadWrite);
        registry.create_session("agent-2", None, AccessMode::ReadWrite);

        // Record tokens only in agent-1
        let entry1 = registry.get_session_mut("agent-1").unwrap();
        let _ = entry1.budget.record_tokens("s1", 900);
        assert_eq!(entry1.budget.pressure_level(), PressureLevel::Elevated);

        // agent-2 should still be at normal
        let entry2 = registry.get_session("agent-2").unwrap();
        assert_eq!(entry2.budget.pressure_level(), PressureLevel::Normal);
        assert_eq!(entry2.budget.budget_status().tokens_used, 0);
    }

    #[test]
    fn sessions_have_independent_deliveries() {
        let mut registry = small_registry();
        registry.create_session("agent-1", None, AccessMode::ReadWrite);
        registry.create_session("agent-2", None, AccessMode::ReadWrite);

        // Deliver to agent-1 only
        let entry1 = registry.get_session_mut("agent-1").unwrap();
        entry1.session.record_delivery(
            &ContentId::from("s1".to_string()),
            Resolution::Section,
            200,
            1,
            "h1".into(),
        );

        assert!(
            registry
                .get_session("agent-1")
                .unwrap()
                .session
                .is_delivered(&ContentId::from("s1".to_string()))
        );
        assert!(
            !registry
                .get_session("agent-2")
                .unwrap()
                .session
                .is_delivered(&ContentId::from("s1".to_string()))
        );
    }

    // ── Coherence propagation tests ──────────────────────────────────

    #[test]
    fn invalidate_all_propagates_to_all_sessions() {
        let mut registry = small_registry();
        registry.create_session("agent-1", None, AccessMode::ReadWrite);
        registry.create_session("agent-2", None, AccessMode::ReadWrite);

        // Both sessions have s1 delivered
        for id in &["agent-1", "agent-2"] {
            let entry = registry.get_session_mut(id).unwrap();
            entry.session.record_delivery(
                &ContentId::from("s1".to_string()),
                Resolution::Section,
                200,
                1,
                "h1".into(),
            );
        }

        let total = registry.invalidate_all(&["s1".to_string()]);
        assert_eq!(total, 2);

        for id in &["agent-1", "agent-2"] {
            let entry = registry.get_session(id).unwrap();
            assert!(entry.session.is_stale(&ContentId::from("s1".to_string())));
            assert!(entry.session.has_pending_alerts());
        }
    }

    #[test]
    fn invalidate_all_only_affects_delivered_content() {
        let mut registry = small_registry();
        registry.create_session("agent-1", None, AccessMode::ReadWrite);
        registry.create_session("agent-2", None, AccessMode::ReadWrite);

        // Only agent-1 has s1
        let entry1 = registry.get_session_mut("agent-1").unwrap();
        entry1.session.record_delivery(
            &ContentId::from("s1".to_string()),
            Resolution::Section,
            200,
            1,
            "h1".into(),
        );

        let total = registry.invalidate_all(&["s1".to_string()]);
        assert_eq!(total, 1); // only agent-1 had s1

        assert!(
            registry
                .get_session("agent-1")
                .unwrap()
                .session
                .is_stale(&ContentId::from("s1".to_string()))
        );
        assert!(
            !registry
                .get_session("agent-2")
                .unwrap()
                .session
                .is_stale(&ContentId::from("s1".to_string()))
        );
    }

    #[test]
    fn invalidate_all_empty_sections_is_noop() {
        let mut registry = small_registry();
        registry.create_session("agent-1", None, AccessMode::ReadWrite);

        let total = registry.invalidate_all(&[]);
        assert_eq!(total, 0);
    }

    #[test]
    fn invalidate_all_empty_registry_is_zero() {
        let mut registry = default_registry();
        let total = registry.invalidate_all(&["s1".to_string()]);
        assert_eq!(total, 0);
    }

    // ── Access mode tests ────────────────────────────────────────────

    #[test]
    fn access_mode_is_stored_correctly() {
        let mut registry = default_registry();
        registry.create_session("rw", None, AccessMode::ReadWrite);
        registry.create_session("ro", None, AccessMode::ReadOnly);

        assert_eq!(
            registry.get_session("rw").unwrap().access_mode,
            AccessMode::ReadWrite
        );
        assert_eq!(
            registry.get_session("ro").unwrap().access_mode,
            AccessMode::ReadOnly
        );
    }

    #[test]
    fn access_mode_serde_roundtrip() {
        for mode in [AccessMode::ReadWrite, AccessMode::ReadOnly] {
            let json = serde_json::to_string(&mode).unwrap();
            let back: AccessMode = serde_json::from_str(&json).unwrap();
            assert_eq!(back, mode);
        }
    }

    // ── Multiple sessions with different budgets ─────────────────────

    #[test]
    fn sessions_with_different_budget_configs() {
        let mut registry = default_registry();

        let small_budget = BudgetConfig {
            max_context_tokens: 500,
            pressure_threshold: 0.8,
            critical_threshold: 0.95,
            ..BudgetConfig::default()
        };
        let large_budget = BudgetConfig {
            max_context_tokens: 100_000,
            pressure_threshold: 0.8,
            critical_threshold: 0.95,
            ..BudgetConfig::default()
        };

        registry.create_session("small", Some(small_budget), AccessMode::ReadWrite);
        registry.create_session("large", Some(large_budget), AccessMode::ReadWrite);

        // Record same amount to both
        for id in &["small", "large"] {
            let entry = registry.get_session_mut(id).unwrap();
            let _ = entry.budget.record_tokens("s1", 450);
        }

        // small should be elevated, large should be normal
        assert_eq!(
            registry
                .get_session("small")
                .unwrap()
                .budget
                .pressure_level(),
            PressureLevel::Elevated
        );
        assert_eq!(
            registry
                .get_session("large")
                .unwrap()
                .budget
                .pressure_level(),
            PressureLevel::Normal
        );
    }

    // ── Default budget config ────────────────────────────────────────

    #[test]
    fn default_budget_config_accessor() {
        let config = BudgetConfig {
            max_context_tokens: 42_000,
            pressure_threshold: 0.75,
            critical_threshold: 0.9,
            ..BudgetConfig::default()
        };
        let registry = SessionRegistry::new(config.clone());
        assert_eq!(registry.default_budget_config().max_context_tokens, 42_000);
    }

    // ── Concurrent multi-session tests ───────────────────────────────

    #[tokio::test]
    async fn concurrent_sessions_independent_deliveries() {
        use std::sync::Arc;
        use tokio::sync::Mutex;

        let registry = Arc::new(Mutex::new(small_registry()));

        // Create two sessions
        {
            let mut reg = registry.lock().await;
            reg.create_session("agent-1", None, AccessMode::ReadWrite);
            reg.create_session("agent-2", None, AccessMode::ReadWrite);
        }

        // Spawn two tasks that deliver to different sessions concurrently
        let reg1 = Arc::clone(&registry);
        let t1 = tokio::spawn(async move {
            for i in 0u32..50 {
                let mut reg = reg1.lock().await;
                let entry = reg.get_session_mut("agent-1").unwrap();
                entry.session.record_delivery(
                    &ContentId::from(format!("s1-{i}")),
                    Resolution::Section,
                    10,
                    i / 10,
                    format!("h1-{i}"),
                );
                let _ = entry.budget.record_tokens(&format!("s1-{i}"), 10);
            }
        });

        let reg2 = Arc::clone(&registry);
        let t2 = tokio::spawn(async move {
            for i in 0u32..50 {
                let mut reg = reg2.lock().await;
                let entry = reg.get_session_mut("agent-2").unwrap();
                entry.session.record_delivery(
                    &ContentId::from(format!("s2-{i}")),
                    Resolution::Section,
                    10,
                    i / 10,
                    format!("h2-{i}"),
                );
                let _ = entry.budget.record_tokens(&format!("s2-{i}"), 10);
            }
        });

        t1.await.unwrap();
        t2.await.unwrap();

        let reg = registry.lock().await;
        let e1 = reg.get_session("agent-1").unwrap();
        let e2 = reg.get_session("agent-2").unwrap();

        assert_eq!(e1.session.delivered_count(), 50);
        assert_eq!(e2.session.delivered_count(), 50);

        // Verify no cross-contamination
        assert!(
            !e1.session
                .is_delivered(&ContentId::from("s2-0".to_string()))
        );
        assert!(
            !e2.session
                .is_delivered(&ContentId::from("s1-0".to_string()))
        );
    }

    #[tokio::test]
    async fn concurrent_coherence_propagation() {
        use std::sync::Arc;
        use tokio::sync::Mutex;

        let registry = Arc::new(Mutex::new(small_registry()));

        // Create 5 sessions, each with s1 delivered
        {
            let mut reg = registry.lock().await;
            for i in 0..5 {
                let entry = reg.create_session(&format!("agent-{i}"), None, AccessMode::ReadWrite);
                entry.session.record_delivery(
                    &ContentId::from("shared-doc".to_string()),
                    Resolution::Section,
                    200,
                    1,
                    "original-hash".into(),
                );
            }
        }

        // Simulate coherence: invalidate the shared doc
        let total = {
            let mut reg = registry.lock().await;
            reg.invalidate_all(&["shared-doc".to_string()])
        };

        assert_eq!(
            total, 5,
            "all 5 sessions should have the shared doc invalidated"
        );

        // Verify each session has the stale mark and pending alert
        let reg = registry.lock().await;
        for i in 0..5 {
            let entry = reg.get_session(&format!("agent-{i}")).unwrap();
            assert!(
                entry
                    .session
                    .is_stale(&ContentId::from("shared-doc".to_string())),
                "agent-{i} should have shared-doc marked stale"
            );
            assert!(
                entry.session.has_pending_alerts(),
                "agent-{i} should have pending coherence alerts"
            );
        }
    }

    #[tokio::test]
    async fn concurrent_session_creation_and_removal() {
        use std::sync::Arc;
        use tokio::sync::Mutex;

        let registry = Arc::new(Mutex::new(default_registry()));

        // Spawn tasks that create sessions concurrently
        let mut handles = Vec::new();
        for i in 0..10 {
            let reg = Arc::clone(&registry);
            handles.push(tokio::spawn(async move {
                let mut r = reg.lock().await;
                r.create_session(
                    &format!("agent-{i}"),
                    None,
                    if i % 2 == 0 {
                        AccessMode::ReadWrite
                    } else {
                        AccessMode::ReadOnly
                    },
                );
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let reg = registry.lock().await;
        assert_eq!(reg.session_count(), 10);

        // Verify access modes
        for i in 0..10 {
            let entry = reg.get_session(&format!("agent-{i}")).unwrap();
            let expected = if i % 2 == 0 {
                AccessMode::ReadWrite
            } else {
                AccessMode::ReadOnly
            };
            assert_eq!(entry.access_mode, expected);
        }
        drop(reg);

        // Remove half the sessions concurrently
        let mut handles = Vec::new();
        for i in (0..10).step_by(2) {
            let reg = Arc::clone(&registry);
            handles.push(tokio::spawn(async move {
                let mut r = reg.lock().await;
                r.remove_session(&format!("agent-{i}"));
            }));
        }
        for h in handles {
            h.await.unwrap();
        }

        let reg = registry.lock().await;
        assert_eq!(reg.session_count(), 5);
        // Only odd-numbered sessions remain
        for i in 0..10 {
            if i % 2 == 0 {
                assert!(!reg.contains(&format!("agent-{i}")));
            } else {
                assert!(reg.contains(&format!("agent-{i}")));
            }
        }
    }

    #[tokio::test]
    async fn concurrent_delivery_and_coherence() {
        use std::sync::Arc;
        use tokio::sync::Mutex;

        let registry = Arc::new(Mutex::new(small_registry()));

        {
            let mut reg = registry.lock().await;
            reg.create_session("writer", None, AccessMode::ReadWrite);
            reg.create_session("reader", None, AccessMode::ReadOnly);
        }

        // Writer delivers content
        {
            let mut reg = registry.lock().await;
            let writer = reg.get_session_mut("writer").unwrap();
            writer.session.record_delivery(
                &ContentId::from("doc-a".to_string()),
                Resolution::Section,
                100,
                1,
                "v1".into(),
            );
            // Reader also has the same content delivered
            let reader = reg.get_session_mut("reader").unwrap();
            reader.session.record_delivery(
                &ContentId::from("doc-a".to_string()),
                Resolution::Section,
                100,
                1,
                "v1".into(),
            );
        }

        // Coherence invalidation
        let invalidated = {
            let mut reg = registry.lock().await;
            reg.invalidate_all(&["doc-a".to_string()])
        };
        assert_eq!(invalidated, 2);

        // Both sessions see stale content
        let reg = registry.lock().await;
        assert!(
            reg.get_session("writer")
                .unwrap()
                .session
                .is_stale(&ContentId::from("doc-a".to_string()))
        );
        assert!(
            reg.get_session("reader")
                .unwrap()
                .session
                .is_stale(&ContentId::from("doc-a".to_string()))
        );
    }
}
