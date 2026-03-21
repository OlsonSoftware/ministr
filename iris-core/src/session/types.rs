//! Core session types for tracking delivered content and agent context state.
//!
//! A [`Session`] represents a single agent interaction session. It tracks which
//! content has been delivered to the agent, at what resolution and token cost,
//! and maintains a trajectory of content access for prefetch prediction.

use std::collections::BTreeMap;
use std::fmt;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::types::{ContentId, Resolution};

/// Unique identifier for an agent session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl AsRef<str> for SessionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Eviction policy that models how the agent's context window discards old content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvictionPolicy {
    /// First-in, first-out: oldest delivered content is evicted first.
    Fifo,
    /// Least recently used: content not re-accessed is evicted first.
    Lru,
}

/// Record of a single content delivery to the agent.
///
/// Tracks what was delivered, at what resolution, the token cost, and when
/// (which turn) it was delivered. The content hash enables delta detection
/// when the underlying content changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveredItem {
    /// The content that was delivered.
    pub content_id: ContentId,
    /// Resolution level at which it was delivered.
    pub resolution: Resolution,
    /// Token count of the delivered content.
    pub token_count: usize,
    /// The interaction turn when this content was delivered.
    pub turn_delivered: u32,
    /// SHA-256 hash of the delivered content for change detection.
    pub content_hash: String,
}

/// An agent interaction session tracking delivered content and access patterns.
///
/// The session shadow maintains a record of everything delivered to the agent,
/// enabling deduplication, delta updates, and budget-aware responses.
///
/// # Examples
///
/// ```
/// use iris_core::session::{Session, SessionId, EvictionPolicy};
/// use iris_core::types::{ContentId, Resolution};
///
/// let mut session = Session::new(
///     SessionId::from("sess-1".to_string()),
///     100_000,
///     EvictionPolicy::Fifo,
/// );
///
/// session.record_delivery(
///     &ContentId::from("doc-api".to_string()),
///     Resolution::Section,
///     250,
///     1,
///     "abc123".to_string(),
/// );
///
/// assert!(session.is_delivered(&ContentId::from("doc-api".to_string())));
/// assert_eq!(session.total_delivered_tokens(), 250);
/// ```
pub struct Session {
    /// Unique session identifier.
    pub id: SessionId,
    /// When this session was created.
    created_at: Instant,
    /// Maximum context budget in tokens for this session.
    pub agent_context_budget: usize,
    /// Map of delivered content, keyed by `ContentId`.
    delivered: BTreeMap<String, DeliveredItem>,
    /// Ordered trajectory of content accesses (content IDs in access order).
    trajectory: Vec<ContentId>,
    /// Current interaction turn counter.
    current_turn: u32,
}

impl Session {
    /// Create a new session with the given budget and eviction policy.
    #[must_use]
    pub fn new(
        id: SessionId,
        agent_context_budget: usize,
        _eviction_policy: EvictionPolicy,
    ) -> Self {
        Self {
            id,
            created_at: Instant::now(),
            agent_context_budget,
            delivered: BTreeMap::new(),
            trajectory: Vec::new(),
            current_turn: 0,
        }
    }

    /// Restore a session from persisted state.
    ///
    /// Used for crash recovery — reconstructs a `Session` from data loaded
    /// from `SQLite`. The `created_at` timestamp is reset to `Instant::now()`
    /// since `Instant` is not serializable.
    #[must_use]
    pub fn restore(
        id: SessionId,
        agent_context_budget: usize,
        delivered: BTreeMap<String, DeliveredItem>,
        trajectory: Vec<ContentId>,
        current_turn: u32,
    ) -> Self {
        Self {
            id,
            created_at: Instant::now(),
            agent_context_budget,
            delivered,
            trajectory,
            current_turn,
        }
    }

    /// Record that content was delivered to the agent.
    ///
    /// Updates the delivered map, appends to the trajectory, and advances
    /// the turn counter if this is a new turn.
    pub fn record_delivery(
        &mut self,
        content_id: &ContentId,
        resolution: Resolution,
        token_count: usize,
        turn: u32,
        content_hash: String,
    ) {
        self.current_turn = self.current_turn.max(turn);
        self.trajectory.push(content_id.clone());

        let item = DeliveredItem {
            content_id: content_id.clone(),
            resolution,
            token_count,
            turn_delivered: turn,
            content_hash,
        };

        self.delivered.insert(content_id.0.clone(), item);
    }

    /// Check whether content has already been delivered in this session.
    #[must_use]
    pub fn is_delivered(&self, content_id: &ContentId) -> bool {
        self.delivered.contains_key(&content_id.0)
    }

    /// Get the delivered item record for a content ID, if it exists.
    #[must_use]
    pub fn get_delivered(&self, content_id: &ContentId) -> Option<&DeliveredItem> {
        self.delivered.get(&content_id.0)
    }

    /// Check whether the content has changed since it was last delivered.
    ///
    /// Returns `true` if the content was previously delivered with a different
    /// hash, indicating that a delta update should be sent.
    #[must_use]
    pub fn has_changed(&self, content_id: &ContentId, current_hash: &str) -> bool {
        self.delivered
            .get(&content_id.0)
            .is_some_and(|item| item.content_hash != current_hash)
    }

    /// Total tokens delivered across all content in this session.
    #[must_use]
    pub fn total_delivered_tokens(&self) -> usize {
        self.delivered.values().map(|item| item.token_count).sum()
    }

    /// Number of distinct content items delivered.
    #[must_use]
    pub fn delivered_count(&self) -> usize {
        self.delivered.len()
    }

    /// The ordered trajectory of content accesses (may contain duplicates).
    #[must_use]
    pub fn trajectory(&self) -> &[ContentId] {
        &self.trajectory
    }

    /// The current interaction turn.
    #[must_use]
    pub fn current_turn(&self) -> u32 {
        self.current_turn
    }

    /// How long this session has been active.
    #[must_use]
    pub fn elapsed(&self) -> std::time::Duration {
        self.created_at.elapsed()
    }

    /// Iterator over all delivered items.
    pub fn delivered_items(&self) -> impl Iterator<Item = &DeliveredItem> {
        self.delivered.values()
    }

    /// Remove a delivered item from the session shadow.
    ///
    /// Used when the agent explicitly signals it has evicted content from
    /// its context window (via `iris_evicted`). Returns the removed item
    /// if it existed.
    pub fn remove_delivered(&mut self, content_id: &ContentId) -> Option<DeliveredItem> {
        self.delivered.remove(&content_id.0)
    }

    /// Detect whether a re-request indicates the agent lost this content.
    ///
    /// Returns `true` if the content was previously delivered with the same
    /// hash (unchanged), meaning the agent is re-requesting content we
    /// thought it still had. This is an implicit eviction signal — the
    /// agent's context window dropped the content before our estimator
    /// predicted it would.
    #[must_use]
    pub fn is_re_request(&self, content_id: &ContentId, current_hash: &str) -> bool {
        self.delivered
            .get(&content_id.0)
            .is_some_and(|item| item.content_hash == current_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session() -> Session {
        Session::new(
            SessionId::from("test-session".to_string()),
            100_000,
            EvictionPolicy::Fifo,
        )
    }

    fn cid(s: &str) -> ContentId {
        ContentId::from(s.to_string())
    }

    #[test]
    fn session_creation() {
        let session = make_session();
        assert_eq!(session.id.0, "test-session");
        assert_eq!(session.agent_context_budget, 100_000);
        assert_eq!(session.delivered_count(), 0);
        assert_eq!(session.total_delivered_tokens(), 0);
        assert_eq!(session.current_turn(), 0);
        assert!(session.trajectory().is_empty());
    }

    #[test]
    fn record_and_check_delivery() {
        let mut session = make_session();

        session.record_delivery(&cid("doc-api"), Resolution::Section, 250, 1, "hash1".into());

        assert!(session.is_delivered(&cid("doc-api")));
        assert!(!session.is_delivered(&cid("doc-other")));
        assert_eq!(session.delivered_count(), 1);
        assert_eq!(session.total_delivered_tokens(), 250);
        assert_eq!(session.current_turn(), 1);
    }

    #[test]
    fn get_delivered_item() {
        let mut session = make_session();

        session.record_delivery(&cid("s1"), Resolution::Claim, 50, 1, "h1".into());

        let item = session.get_delivered(&cid("s1")).unwrap();
        assert_eq!(item.resolution, Resolution::Claim);
        assert_eq!(item.token_count, 50);
        assert_eq!(item.turn_delivered, 1);
        assert_eq!(item.content_hash, "h1");

        assert!(session.get_delivered(&cid("nonexistent")).is_none());
    }

    #[test]
    fn has_changed_detects_hash_mismatch() {
        let mut session = make_session();

        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "original".into());

        assert!(!session.has_changed(&cid("s1"), "original"));
        assert!(session.has_changed(&cid("s1"), "updated"));
        // Non-delivered content: has_changed returns false (nothing to compare)
        assert!(!session.has_changed(&cid("unknown"), "anything"));
    }

    #[test]
    fn multiple_deliveries_accumulate_tokens() {
        let mut session = make_session();

        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());
        session.record_delivery(&cid("s2"), Resolution::Summary, 100, 1, "h2".into());
        session.record_delivery(&cid("c1"), Resolution::Claim, 30, 2, "h3".into());

        assert_eq!(session.delivered_count(), 3);
        assert_eq!(session.total_delivered_tokens(), 330);
        assert_eq!(session.current_turn(), 2);
    }

    #[test]
    fn re_delivery_updates_existing_record() {
        let mut session = make_session();

        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "v1".into());
        session.record_delivery(&cid("s1"), Resolution::Section, 180, 2, "v2".into());

        // Should update, not duplicate
        assert_eq!(session.delivered_count(), 1);
        assert_eq!(session.total_delivered_tokens(), 180);

        let item = session.get_delivered(&cid("s1")).unwrap();
        assert_eq!(item.content_hash, "v2");
        assert_eq!(item.turn_delivered, 2);
    }

    #[test]
    fn trajectory_records_access_order_with_duplicates() {
        let mut session = make_session();

        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());
        session.record_delivery(&cid("s2"), Resolution::Section, 150, 1, "h2".into());
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 2, "h1".into());

        let traj = session.trajectory();
        assert_eq!(traj.len(), 3);
        assert_eq!(traj[0], cid("s1"));
        assert_eq!(traj[1], cid("s2"));
        assert_eq!(traj[2], cid("s1"));
    }

    #[test]
    fn delivered_items_iterator() {
        let mut session = make_session();

        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());
        session.record_delivery(&cid("s2"), Resolution::Claim, 30, 1, "h2".into());

        let items: Vec<_> = session.delivered_items().collect();
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn session_id_display_and_conversion() {
        let id = SessionId::from("sess-42".to_string());
        assert_eq!(id.to_string(), "sess-42");
        assert_eq!(id.as_ref(), "sess-42");
    }

    #[test]
    fn eviction_policy_serde_roundtrip() {
        let fifo = EvictionPolicy::Fifo;
        let json = serde_json::to_string(&fifo).unwrap();
        assert_eq!(json, r#""fifo""#);
        let back: EvictionPolicy = serde_json::from_str(&json).unwrap();
        assert_eq!(back, EvictionPolicy::Fifo);

        let lru = EvictionPolicy::Lru;
        let json = serde_json::to_string(&lru).unwrap();
        assert_eq!(json, r#""lru""#);
    }

    #[test]
    fn elapsed_is_non_negative() {
        let session = make_session();
        // elapsed should work without panicking
        let _elapsed = session.elapsed();
    }

    // --- Exhaustive deduplication tests ---

    #[test]
    fn re_delivery_at_different_resolution_updates_record() {
        let mut session = make_session();

        // First deliver as section
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());
        assert_eq!(
            session.get_delivered(&cid("s1")).unwrap().resolution,
            Resolution::Section
        );

        // Re-deliver as claim (e.g. after extract)
        session.record_delivery(&cid("s1"), Resolution::Claim, 30, 2, "h2".into());

        // Should update resolution and token count
        assert_eq!(session.delivered_count(), 1);
        let item = session.get_delivered(&cid("s1")).unwrap();
        assert_eq!(item.resolution, Resolution::Claim);
        assert_eq!(item.token_count, 30);
        assert_eq!(session.total_delivered_tokens(), 30);
    }

    #[test]
    fn has_changed_after_re_delivery_with_new_hash() {
        let mut session = make_session();

        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "v1".into());
        assert!(!session.has_changed(&cid("s1"), "v1"));

        // Re-deliver with updated hash
        session.record_delivery(&cid("s1"), Resolution::Section, 210, 2, "v2".into());
        assert!(
            !session.has_changed(&cid("s1"), "v2"),
            "should match new hash"
        );
        assert!(
            session.has_changed(&cid("s1"), "v1"),
            "old hash should show change"
        );
        assert!(
            session.has_changed(&cid("s1"), "v3"),
            "unknown hash should show change"
        );
    }

    #[test]
    fn large_trajectory_accumulation() {
        let mut session = make_session();

        for i in 0u32..200 {
            let id = format!("section-{i}");
            session.record_delivery(&cid(&id), Resolution::Section, 10, i / 10, format!("h{i}"));
        }

        assert_eq!(session.delivered_count(), 200);
        assert_eq!(session.total_delivered_tokens(), 2000);
        assert_eq!(session.trajectory().len(), 200);
        assert_eq!(session.current_turn(), 19);
    }

    #[test]
    fn large_trajectory_with_re_deliveries() {
        let mut session = make_session();

        // Deliver 50 items, then re-deliver half of them
        for i in 0..50 {
            session.record_delivery(
                &cid(&format!("s{i}")),
                Resolution::Section,
                100,
                1,
                format!("h{i}"),
            );
        }
        for i in 0..25 {
            session.record_delivery(
                &cid(&format!("s{i}")),
                Resolution::Section,
                80,
                2,
                format!("h{i}-v2"),
            );
        }

        // 50 unique items, but 25 were re-delivered with lower token count
        assert_eq!(session.delivered_count(), 50);
        // 25 items at 80 tokens + 25 items at 100 tokens
        assert_eq!(session.total_delivered_tokens(), 25 * 80 + 25 * 100);
        // Trajectory has all 75 entries (50 + 25 re-deliveries)
        assert_eq!(session.trajectory().len(), 75);
    }

    #[test]
    fn concurrent_deliveries_same_turn() {
        let mut session = make_session();

        // Multiple items delivered in the same turn (e.g. survey results)
        session.record_delivery(&cid("s1"), Resolution::Summary, 50, 1, "h1".into());
        session.record_delivery(&cid("s2"), Resolution::Section, 200, 1, "h2".into());
        session.record_delivery(&cid("s3"), Resolution::Claim, 20, 1, "h3".into());
        session.record_delivery(&cid("s4"), Resolution::Summary, 60, 1, "h4".into());

        assert_eq!(session.delivered_count(), 4);
        assert_eq!(session.total_delivered_tokens(), 330);
        assert_eq!(session.current_turn(), 1);

        // All are independently tracked
        assert!(session.is_delivered(&cid("s1")));
        assert!(session.is_delivered(&cid("s2")));
        assert!(session.is_delivered(&cid("s3")));
        assert!(session.is_delivered(&cid("s4")));
    }

    #[test]
    fn session_with_zero_budget() {
        let session = Session::new(
            SessionId::from("zero-budget".to_string()),
            0,
            EvictionPolicy::Fifo,
        );
        assert_eq!(session.agent_context_budget, 0);
        assert_eq!(session.delivered_count(), 0);
        assert_eq!(session.total_delivered_tokens(), 0);
    }

    #[test]
    fn delivered_items_ordered_by_content_id() {
        let mut session = make_session();

        // BTreeMap keys are sorted, so delivered items should be in key order
        session.record_delivery(&cid("charlie"), Resolution::Section, 100, 1, "h1".into());
        session.record_delivery(&cid("alpha"), Resolution::Section, 100, 1, "h2".into());
        session.record_delivery(&cid("bravo"), Resolution::Section, 100, 1, "h3".into());

        let ids: Vec<&str> = session
            .delivered_items()
            .map(|item| item.content_id.0.as_str())
            .collect();
        assert_eq!(ids, vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn delivered_item_serde_roundtrip() {
        let item = DeliveredItem {
            content_id: ContentId("test-id".into()),
            resolution: Resolution::Section,
            token_count: 250,
            turn_delivered: 3,
            content_hash: "abc123".into(),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: DeliveredItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content_id, item.content_id);
        assert_eq!(back.resolution, item.resolution);
        assert_eq!(back.token_count, item.token_count);
        assert_eq!(back.turn_delivered, item.turn_delivered);
        assert_eq!(back.content_hash, item.content_hash);
    }

    #[test]
    fn turn_counter_takes_max_not_sequential() {
        let mut session = make_session();

        // Deliver at turn 5, then turn 3 — current_turn should stay at 5
        session.record_delivery(&cid("s1"), Resolution::Section, 100, 5, "h1".into());
        assert_eq!(session.current_turn(), 5);

        session.record_delivery(&cid("s2"), Resolution::Section, 100, 3, "h2".into());
        assert_eq!(session.current_turn(), 5, "turn should not decrease");

        session.record_delivery(&cid("s3"), Resolution::Section, 100, 7, "h3".into());
        assert_eq!(session.current_turn(), 7, "turn should advance to 7");
    }

    // --- remove_delivered tests ---

    #[test]
    fn remove_delivered_returns_item_and_removes_it() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());

        let removed = session.remove_delivered(&cid("s1"));
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().token_count, 200);
        assert!(!session.is_delivered(&cid("s1")));
        assert_eq!(session.delivered_count(), 0);
        assert_eq!(session.total_delivered_tokens(), 0);
    }

    #[test]
    fn remove_delivered_nonexistent_returns_none() {
        let mut session = make_session();
        assert!(session.remove_delivered(&cid("nonexistent")).is_none());
    }

    #[test]
    fn remove_delivered_does_not_affect_trajectory() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());
        session.record_delivery(&cid("s2"), Resolution::Section, 100, 1, "h2".into());

        session.remove_delivered(&cid("s1"));

        // Trajectory preserves history even after removal
        assert_eq!(session.trajectory().len(), 2);
        assert!(!session.is_delivered(&cid("s1")));
        assert!(session.is_delivered(&cid("s2")));
    }

    // --- is_re_request tests ---

    #[test]
    fn is_re_request_detects_unchanged_re_request() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "hash1".into());

        // Same hash = re-request (agent lost it but content unchanged)
        assert!(session.is_re_request(&cid("s1"), "hash1"));
    }

    #[test]
    fn is_re_request_false_when_content_changed() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "hash1".into());

        // Different hash = content changed, not a re-request signal
        assert!(!session.is_re_request(&cid("s1"), "hash2"));
    }

    #[test]
    fn is_re_request_false_for_undelivered_content() {
        let session = make_session();
        assert!(!session.is_re_request(&cid("unknown"), "anything"));
    }

    #[test]
    fn session_id_equality_and_hash() {
        use std::collections::HashSet;

        let id1 = SessionId::from("sess-1".to_string());
        let id2 = SessionId::from("sess-1".to_string());
        let id3 = SessionId::from("sess-2".to_string());

        assert_eq!(id1, id2);
        assert_ne!(id1, id3);

        let mut set = HashSet::new();
        set.insert(id1.clone());
        assert!(set.contains(&id2));
        assert!(!set.contains(&id3));
    }
}
