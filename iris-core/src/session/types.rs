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
}
