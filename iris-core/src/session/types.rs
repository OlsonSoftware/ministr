//! Core session types for tracking delivered content and agent context state.
//!
//! A [`Session`] represents a single agent interaction session. It tracks which
//! content has been delivered to the agent, at what resolution and token cost,
//! and maintains a trajectory of content access for prefetch prediction.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fmt;
use std::time::Instant;

/// Maximum number of trajectory entries to retain per session.
///
/// The trajectory is used by the prefetch engine for sequential access prediction.
/// Only recent history matters for this, so we cap it to bound memory usage.
const MAX_TRAJECTORY_LEN: usize = 1000;

/// Maximum number of recent queries to retain for task-awareness.
///
/// Used by the salience scorer to infer current task context.
/// Kept small since only the most recent queries reflect the agent's focus.
const MAX_RECENT_QUERIES: usize = 10;

use serde::{Deserialize, Serialize};

use crate::types::{ContentId, Resolution};

/// Cumulative session metrics for token economics tracking.
///
/// Tracks running totals of deliveries, evictions, compressions, and delta
/// updates across the session lifetime. These counters increase monotonically
/// and are never reset — they represent the full history of the session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMetrics {
    /// Total number of content deliveries (including re-deliveries).
    pub total_deliveries: u64,
    /// Cumulative tokens delivered to the agent across all deliveries.
    pub cumulative_tokens_delivered: u64,
    /// Total number of explicit evictions (via `iris_evicted`).
    pub total_evictions: u64,
    /// Cumulative tokens freed by explicit evictions.
    pub cumulative_tokens_evicted: u64,
    /// Total number of compression tier transitions.
    pub total_compressions: u64,
    /// Cumulative tokens freed by compression operations.
    pub cumulative_tokens_compressed: u64,
    /// Number of delta updates (content changed since last delivery).
    pub delta_updates: u64,
    /// Number of deduplicated requests (content already delivered, same hash).
    pub dedup_hits: u64,
}

impl SessionMetrics {
    /// Net token savings: evicted + compressed.
    #[must_use]
    pub fn total_tokens_saved(&self) -> u64 {
        self.cumulative_tokens_evicted + self.cumulative_tokens_compressed
    }

    /// Compression ratio: saved / delivered. Returns 0.0 if nothing delivered.
    #[must_use]
    pub fn compression_ratio(&self) -> f64 {
        if self.cumulative_tokens_delivered == 0 {
            return 0.0;
        }
        self.total_tokens_saved() as f64 / self.cumulative_tokens_delivered as f64
    }
}

/// A coherence alert generated when underlying content changes.
///
/// Contains lists of changed sections and stale content IDs that the agent
/// should be notified about. Alerts are queued in the session and drained
/// by the transport layer on the next tool response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CoherenceAlert {
    /// Section IDs that were re-indexed due to file changes.
    pub changed_sections: Vec<String>,
    /// Content IDs in the session shadow that are now stale.
    pub stale_content_ids: Vec<String>,
}

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

/// Access mode for a session within a federated session registry.
///
/// Controls what operations are permitted for a session. Read-only sessions
/// can query, survey, and read content but cannot trigger mutations like
/// fetching web content, cloning repositories, or explicitly evicting items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessMode {
    /// Full access — the session can read, write, fetch, clone, and evict.
    ReadWrite,
    /// Read-only access — the session can query and read but not mutate.
    ReadOnly,
}

/// Eviction policy that models how the agent's context window discards old content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvictionPolicy {
    /// First-in, first-out: oldest delivered content is evicted first.
    Fifo,
    /// Least recently used: content not re-accessed is evicted first.
    Lru,
    /// FSRS-based: evict content with lowest predicted recall probability.
    /// Requires retrievability scores passed to the window estimator.
    Fsrs,
}

/// Compression tier for delivered content in the multi-tier eviction pipeline.
///
/// Content progresses through tiers as budget pressure increases:
/// `Full → Abstractive → Extractive → Bookmark → Evicted`.
///
/// Higher tiers use fewer tokens but retain less information. The pipeline
/// ensures graceful degradation — content is compressed before being fully
/// evicted, preserving at least structural awareness (bookmarks) until
/// budget pressure forces complete removal.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum CompressionTier {
    /// Full original text — no compression applied.
    Full,
    /// LLM-generated abstractive summary (~90%+ token reduction).
    Abstractive,
    /// TF-IDF extractive summary (~60–80% token reduction).
    Extractive,
    /// Heading-only bookmark (~95%+ reduction). Re-fetchable via `iris_read`.
    Bookmark,
    /// Fully evicted from context — only metadata remains in the session shadow.
    Evicted,
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
    /// Current compression tier in the multi-tier eviction pipeline.
    pub compression_tier: CompressionTier,
    /// Compressed summary text, populated by automatic eviction compression.
    /// Present when tier is Extractive or Abstractive after background compression.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub compressed_summary: Option<String>,
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
    /// Capped at [`MAX_TRAJECTORY_LEN`] to bound memory; oldest entries are dropped.
    trajectory: VecDeque<ContentId>,
    /// Current interaction turn counter.
    current_turn: u32,
    /// Content IDs that have been marked stale due to underlying file changes.
    stale: HashSet<String>,
    /// Pending coherence alerts waiting to be delivered to the agent.
    pending_alerts: VecDeque<CoherenceAlert>,
    /// Recent search queries issued by the agent (sliding window).
    /// Used for task-awareness in salience-based eviction scoring.
    recent_queries: VecDeque<String>,
    /// Cumulative token economics metrics.
    metrics: SessionMetrics,
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
            trajectory: VecDeque::new(),
            current_turn: 0,
            stale: HashSet::new(),
            pending_alerts: VecDeque::new(),
            recent_queries: VecDeque::new(),
            metrics: SessionMetrics::default(),
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
        // Convert to VecDeque, keeping only the most recent entries.
        let skip = trajectory.len().saturating_sub(MAX_TRAJECTORY_LEN);
        let trajectory: VecDeque<ContentId> = trajectory.into_iter().skip(skip).collect();
        Self {
            id,
            created_at: Instant::now(),
            agent_context_budget,
            delivered,
            trajectory,
            current_turn,
            stale: HashSet::new(),
            pending_alerts: VecDeque::new(),
            recent_queries: VecDeque::new(),
            metrics: SessionMetrics::default(),
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
        self.trajectory.push_back(content_id.clone());
        if self.trajectory.len() > MAX_TRAJECTORY_LEN {
            self.trajectory.pop_front();
        }

        self.metrics.total_deliveries += 1;
        self.metrics.cumulative_tokens_delivered += token_count as u64;

        let item = DeliveredItem {
            content_id: content_id.clone(),
            resolution,
            token_count,
            turn_delivered: turn,
            content_hash,
            compression_tier: CompressionTier::Full,
            compressed_summary: None,
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

    /// Returns the set of all delivered content ID strings.
    ///
    /// Used by the service layer to exclude already-delivered content
    /// from search results before truncation.
    #[must_use]
    pub fn delivered_ids(&self) -> HashSet<String> {
        self.delivered.keys().cloned().collect()
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
    ///
    /// Capped at the most recent [`MAX_TRAJECTORY_LEN`] entries.
    #[must_use]
    pub fn trajectory(&self) -> &VecDeque<ContentId> {
        &self.trajectory
    }

    /// The current interaction turn.
    #[must_use]
    pub fn current_turn(&self) -> u32 {
        self.current_turn
    }

    /// Advance the turn counter by one and return the new value.
    ///
    /// Called once per tool call the agent makes against this session.
    /// Independent of [`Session::record_delivery`] so non-delivery tool
    /// calls (e.g. `iris_survey`, `iris_symbols`) still register as
    /// progress on the session's live-turn stream.
    pub fn tick(&mut self) -> u32 {
        self.current_turn = self.current_turn.saturating_add(1);
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
        self.stale.remove(&content_id.0);
        let item = self.delivered.remove(&content_id.0)?;
        self.metrics.total_evictions += 1;
        self.metrics.cumulative_tokens_evicted += item.token_count as u64;
        Some(item)
    }

    /// Mask a delivered item to bookmark tier.
    ///
    /// Converts the item to a heading-only bookmark, keeping the content ID
    /// in the session shadow but reducing its token count to a small stub.
    /// The content remains re-fetchable via `iris_read`.
    ///
    /// Returns the number of tokens freed, or `None` if the content was not
    /// found or was already at bookmark tier or higher.
    pub fn mask_to_bookmark(
        &mut self,
        content_id: &ContentId,
        heading_path: &[String],
    ) -> Option<usize> {
        let item = self.delivered.get_mut(&content_id.0)?;

        // Don't re-mask items already at bookmark or evicted tier
        if item.compression_tier >= CompressionTier::Bookmark {
            return None;
        }

        let original_tokens = item.token_count;
        let bookmark = Self::bookmark_text(heading_path);
        let bookmark_tokens = bookmark.split_whitespace().count().max(1);

        item.compression_tier = CompressionTier::Bookmark;
        item.token_count = bookmark_tokens;

        let freed = original_tokens.saturating_sub(bookmark_tokens);
        if freed > 0 {
            self.metrics.total_compressions += 1;
            self.metrics.cumulative_tokens_compressed += freed as u64;
        }
        Some(freed)
    }

    /// Set the compression tier and token count for a delivered item.
    ///
    /// Used by the compression pipeline to track tier transitions. Returns
    /// the number of tokens freed, or `None` if the content was not found.
    pub fn set_compression_tier(
        &mut self,
        content_id: &ContentId,
        tier: CompressionTier,
        new_token_count: usize,
    ) -> Option<usize> {
        let item = self.delivered.get_mut(&content_id.0)?;
        let original_tokens = item.token_count;
        item.compression_tier = tier;
        item.token_count = new_token_count;
        let freed = original_tokens.saturating_sub(new_token_count);
        if freed > 0 {
            self.metrics.total_compressions += 1;
            self.metrics.cumulative_tokens_compressed += freed as u64;
        }
        Some(freed)
    }

    /// Update a delivered item with a compressed summary and new tier.
    ///
    /// Used by automatic eviction compression: after an entry is bookmarked,
    /// background compression produces a summary that upgrades the tier from
    /// `Bookmark` to `Extractive` or `Abstractive`.
    ///
    /// Returns `Some(tokens_freed)` if the item exists, `None` otherwise.
    pub fn set_compressed_summary(
        &mut self,
        content_id: &ContentId,
        summary: String,
        tier: CompressionTier,
        new_token_count: usize,
    ) -> Option<usize> {
        let item = self.delivered.get_mut(&content_id.0)?;
        let original_tokens = item.token_count;
        item.compression_tier = tier;
        item.token_count = new_token_count;
        item.compressed_summary = Some(summary);
        let freed = original_tokens.saturating_sub(new_token_count);
        if freed > 0 {
            self.metrics.total_compressions += 1;
            self.metrics.cumulative_tokens_compressed += freed as u64;
        }
        Some(freed)
    }

    /// Generate bookmark text from a heading path.
    ///
    /// Produces a compact heading-only stub like `"[bookmark: Chapter 3 > Auth > Tokens]"`
    /// that preserves structural awareness at minimal token cost.
    #[must_use]
    pub fn bookmark_text(heading_path: &[String]) -> String {
        if heading_path.is_empty() {
            return "[bookmark]".to_string();
        }
        format!("[bookmark: {}]", heading_path.join(" > "))
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

    // --- Coherence / stale tracking ---

    /// Mark a delivered content item as stale due to underlying file changes.
    ///
    /// Returns `true` if the content was delivered and is now marked stale.
    /// Returns `false` if the content was not in the session shadow.
    #[must_use]
    pub fn mark_stale(&mut self, content_id: &ContentId) -> bool {
        if self.delivered.contains_key(&content_id.0) {
            self.stale.insert(content_id.0.clone());
            true
        } else {
            false
        }
    }

    /// Check whether a delivered content item has been marked stale.
    #[must_use]
    pub fn is_stale(&self, content_id: &ContentId) -> bool {
        self.stale.contains(&content_id.0)
    }

    /// Get all content IDs currently marked as stale.
    #[must_use]
    pub fn stale_content_ids(&self) -> Vec<String> {
        self.stale.iter().cloned().collect()
    }

    /// Clear the stale mark for a content item (e.g. after re-delivery).
    pub fn clear_stale(&mut self, content_id: &ContentId) {
        self.stale.remove(&content_id.0);
    }

    /// Invalidate all delivered items that reference the given section IDs.
    ///
    /// Marks matching delivered items as stale and enqueues a coherence alert.
    /// Returns the number of items that were invalidated.
    #[must_use]
    pub fn invalidate_sections(&mut self, changed_section_ids: &[String]) -> usize {
        let mut stale_ids = Vec::new();

        for section_id in changed_section_ids {
            let cid = ContentId(section_id.clone());
            if self.mark_stale(&cid) {
                stale_ids.push(section_id.clone());
            }
        }

        let count = stale_ids.len();

        if !stale_ids.is_empty() {
            self.pending_alerts.push_back(CoherenceAlert {
                changed_sections: changed_section_ids.to_vec(),
                stale_content_ids: stale_ids,
            });
        }

        count
    }

    /// Drain all pending coherence alerts.
    ///
    /// Returns the alerts and removes them from the queue. The transport
    /// layer should call this on each tool response to deliver pending
    /// alerts to the agent.
    #[must_use]
    pub fn drain_alerts(&mut self) -> Vec<CoherenceAlert> {
        self.pending_alerts.drain(..).collect()
    }

    /// Check if there are pending coherence alerts.
    #[must_use]
    pub fn has_pending_alerts(&self) -> bool {
        !self.pending_alerts.is_empty()
    }

    // --- Metrics ---

    /// Cumulative token economics metrics for this session.
    #[must_use]
    pub fn metrics(&self) -> &SessionMetrics {
        &self.metrics
    }

    /// Record a dedup hit (agent requested content already delivered, same hash).
    pub fn record_dedup_hit(&mut self) {
        self.metrics.dedup_hits += 1;
    }

    /// Record a delta update (content changed since last delivery).
    pub fn record_delta_update(&mut self) {
        self.metrics.delta_updates += 1;
    }

    /// Record a search query issued by the agent for task-awareness.
    ///
    /// Maintains a sliding window of recent queries (capped at
    /// [`MAX_RECENT_QUERIES`]) used by the salience scorer to infer
    /// what the agent is currently working on.
    pub fn record_query(&mut self, query: &str) {
        if self.recent_queries.len() >= MAX_RECENT_QUERIES {
            self.recent_queries.pop_front();
        }
        self.recent_queries.push_back(query.to_string());
    }

    /// Recent search queries (most recent last).
    ///
    /// Used by [`EvictionRanker`] for task-aware salience scoring.
    #[must_use]
    pub fn recent_queries(&self) -> &VecDeque<String> {
        &self.recent_queries
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
            compression_tier: CompressionTier::Full,
            compressed_summary: None,
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

    // --- Stale tracking / coherence tests ---

    #[test]
    fn mark_stale_delivered_item() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());

        assert!(session.mark_stale(&cid("s1")));
        assert!(session.is_stale(&cid("s1")));
    }

    #[test]
    fn mark_stale_undelivered_returns_false() {
        let mut session = make_session();
        assert!(!session.mark_stale(&cid("unknown")));
        assert!(!session.is_stale(&cid("unknown")));
    }

    #[test]
    fn stale_content_ids_lists_all_stale() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());
        session.record_delivery(&cid("s2"), Resolution::Section, 100, 1, "h2".into());
        session.record_delivery(&cid("s3"), Resolution::Section, 150, 1, "h3".into());

        let _ = session.mark_stale(&cid("s1"));
        let _ = session.mark_stale(&cid("s3"));

        let stale = session.stale_content_ids();
        assert_eq!(stale.len(), 2);
        assert!(stale.contains(&"s1".to_string()));
        assert!(stale.contains(&"s3".to_string()));
    }

    #[test]
    fn clear_stale_removes_mark() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());

        let _ = session.mark_stale(&cid("s1"));
        assert!(session.is_stale(&cid("s1")));

        session.clear_stale(&cid("s1"));
        assert!(!session.is_stale(&cid("s1")));
    }

    #[test]
    fn invalidate_sections_marks_delivered_stale_and_enqueues_alert() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());
        session.record_delivery(&cid("s2"), Resolution::Section, 100, 1, "h2".into());

        let count = session.invalidate_sections(&["s1".into(), "s3".into()]);
        // s1 was delivered (stale), s3 was not
        assert_eq!(count, 1);
        assert!(session.is_stale(&cid("s1")));
        assert!(!session.is_stale(&cid("s3")));

        assert!(session.has_pending_alerts());
        let alerts = session.drain_alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(
            alerts[0].changed_sections,
            vec!["s1".to_string(), "s3".to_string()]
        );
        assert_eq!(alerts[0].stale_content_ids, vec!["s1".to_string()]);
    }

    #[test]
    fn invalidate_sections_no_overlap_produces_no_alert() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());

        let count = session.invalidate_sections(&["s99".into()]);
        assert_eq!(count, 0);
        assert!(!session.has_pending_alerts());
    }

    #[test]
    fn drain_alerts_empties_queue() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());

        let _ = session.invalidate_sections(&["s1".into()]);
        assert!(session.has_pending_alerts());

        let alerts = session.drain_alerts();
        assert_eq!(alerts.len(), 1);
        assert!(!session.has_pending_alerts());

        // Second drain returns empty
        let alerts = session.drain_alerts();
        assert!(alerts.is_empty());
    }

    #[test]
    fn multiple_invalidations_queue_multiple_alerts() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());
        session.record_delivery(&cid("s2"), Resolution::Section, 100, 1, "h2".into());

        let _ = session.invalidate_sections(&["s1".into()]);
        let _ = session.invalidate_sections(&["s2".into()]);

        let alerts = session.drain_alerts();
        assert_eq!(alerts.len(), 2);
    }

    #[test]
    fn coherence_alert_serde_roundtrip() {
        let alert = CoherenceAlert {
            changed_sections: vec!["s1".into(), "s2".into()],
            stale_content_ids: vec!["s1".into()],
        };
        let json = serde_json::to_string(&alert).unwrap();
        let back: CoherenceAlert = serde_json::from_str(&json).unwrap();
        assert_eq!(back, alert);
    }

    #[test]
    fn re_delivery_clears_stale() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());
        let _ = session.mark_stale(&cid("s1"));
        assert!(session.is_stale(&cid("s1")));

        // Re-deliver with updated content
        session.record_delivery(&cid("s1"), Resolution::Section, 210, 2, "h2".into());
        // Stale should be cleared after re-delivery
        session.clear_stale(&cid("s1"));
        assert!(!session.is_stale(&cid("s1")));
    }

    // --- CompressionTier tests ---

    #[test]
    fn compression_tier_ordering() {
        assert!(CompressionTier::Full < CompressionTier::Abstractive);
        assert!(CompressionTier::Abstractive < CompressionTier::Extractive);
        assert!(CompressionTier::Extractive < CompressionTier::Bookmark);
        assert!(CompressionTier::Bookmark < CompressionTier::Evicted);
    }

    #[test]
    fn compression_tier_serde_roundtrip() {
        for tier in [
            CompressionTier::Full,
            CompressionTier::Abstractive,
            CompressionTier::Extractive,
            CompressionTier::Bookmark,
            CompressionTier::Evicted,
        ] {
            let json = serde_json::to_string(&tier).unwrap();
            let back: CompressionTier = serde_json::from_str(&json).unwrap();
            assert_eq!(back, tier, "roundtrip failed for {tier:?}");
        }
    }

    #[test]
    fn new_delivery_has_full_tier() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 200, 1, "h1".into());

        let item = session.get_delivered(&cid("s1")).unwrap();
        assert_eq!(item.compression_tier, CompressionTier::Full);
    }

    // --- Masking / bookmark tests ---

    #[test]
    fn mask_to_bookmark_reduces_tokens() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 500, 1, "h1".into());

        let freed = session.mask_to_bookmark(&cid("s1"), &["Chapter 1".into(), "Auth".into()]);
        assert!(freed.is_some());
        let freed = freed.unwrap();
        assert!(freed > 490, "should free most tokens: freed {freed}");

        let item = session.get_delivered(&cid("s1")).unwrap();
        assert_eq!(item.compression_tier, CompressionTier::Bookmark);
        assert!(
            item.token_count < 10,
            "bookmark should have minimal tokens: {}",
            item.token_count
        );
    }

    #[test]
    fn mask_to_bookmark_nonexistent_returns_none() {
        let mut session = make_session();
        let result = session.mask_to_bookmark(&cid("unknown"), &["Heading".into()]);
        assert!(result.is_none());
    }

    #[test]
    fn mask_already_bookmarked_returns_none() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 500, 1, "h1".into());

        // First mask succeeds
        assert!(
            session
                .mask_to_bookmark(&cid("s1"), &["H1".into()])
                .is_some()
        );
        // Second mask returns None (already bookmarked)
        assert!(
            session
                .mask_to_bookmark(&cid("s1"), &["H1".into()])
                .is_none()
        );
    }

    #[test]
    fn bookmark_text_with_heading_path() {
        let text = Session::bookmark_text(&["Chapter 3".into(), "Auth".into(), "Tokens".into()]);
        assert_eq!(text, "[bookmark: Chapter 3 > Auth > Tokens]");
    }

    #[test]
    fn bookmark_text_empty_heading() {
        let text = Session::bookmark_text(&[]);
        assert_eq!(text, "[bookmark]");
    }

    #[test]
    fn bookmark_text_single_heading() {
        let text = Session::bookmark_text(&["Introduction".into()]);
        assert_eq!(text, "[bookmark: Introduction]");
    }

    // --- set_compression_tier tests ---

    #[test]
    fn set_compression_tier_updates_item() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 1000, 1, "h1".into());

        let freed = session.set_compression_tier(&cid("s1"), CompressionTier::Extractive, 300);
        assert_eq!(freed, Some(700));

        let item = session.get_delivered(&cid("s1")).unwrap();
        assert_eq!(item.compression_tier, CompressionTier::Extractive);
        assert_eq!(item.token_count, 300);
    }

    #[test]
    fn set_compression_tier_nonexistent_returns_none() {
        let mut session = make_session();
        let result = session.set_compression_tier(&cid("nope"), CompressionTier::Bookmark, 5);
        assert!(result.is_none());
    }

    // --- set_compressed_summary tests ---

    #[test]
    fn set_compressed_summary_stores_and_updates_tier() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 1000, 1, "h1".into());

        let freed = session.set_compressed_summary(
            &cid("s1"),
            "A summary of s1.".into(),
            CompressionTier::Extractive,
            300,
        );
        assert_eq!(freed, Some(700));

        let item = session.get_delivered(&cid("s1")).unwrap();
        assert_eq!(item.compression_tier, CompressionTier::Extractive);
        assert_eq!(item.token_count, 300);
        assert_eq!(item.compressed_summary.as_deref(), Some("A summary of s1."));
    }

    #[test]
    fn set_compressed_summary_nonexistent_returns_none() {
        let mut session = make_session();
        let result = session.set_compressed_summary(
            &cid("nope"),
            "summary".into(),
            CompressionTier::Extractive,
            100,
        );
        assert!(result.is_none());
    }

    // --- Full pipeline integration test ---

    #[test]
    fn full_tier_progression_through_masking() {
        let mut session = make_session();
        session.record_delivery(&cid("doc"), Resolution::Section, 2000, 1, "h1".into());

        // Start at Full
        assert_eq!(
            session.get_delivered(&cid("doc")).unwrap().compression_tier,
            CompressionTier::Full
        );

        // Simulate extractive compression
        let freed = session.set_compression_tier(&cid("doc"), CompressionTier::Extractive, 600);
        assert_eq!(freed, Some(1400));

        // Mask to bookmark
        let freed = session.mask_to_bookmark(&cid("doc"), &["API".into(), "Rate Limits".into()]);
        assert!(freed.is_some());

        let item = session.get_delivered(&cid("doc")).unwrap();
        assert_eq!(item.compression_tier, CompressionTier::Bookmark);
        assert!(item.token_count < 10);

        // Simulate eviction
        let freed = session.set_compression_tier(&cid("doc"), CompressionTier::Evicted, 0);
        assert!(freed.is_some());

        let item = session.get_delivered(&cid("doc")).unwrap();
        assert_eq!(item.compression_tier, CompressionTier::Evicted);
        assert_eq!(item.token_count, 0);
    }

    // --- SessionMetrics tests ---

    #[test]
    fn metrics_initial_state_is_zero() {
        let session = make_session();
        let m = session.metrics();
        assert_eq!(m.total_deliveries, 0);
        assert_eq!(m.cumulative_tokens_delivered, 0);
        assert_eq!(m.total_evictions, 0);
        assert_eq!(m.cumulative_tokens_evicted, 0);
        assert_eq!(m.total_compressions, 0);
        assert_eq!(m.cumulative_tokens_compressed, 0);
        assert_eq!(m.delta_updates, 0);
        assert_eq!(m.dedup_hits, 0);
        assert_eq!(m.total_tokens_saved(), 0);
        assert!((m.compression_ratio() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn metrics_track_deliveries() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 100, 1, "h1".into());
        session.record_delivery(&cid("s2"), Resolution::Section, 200, 2, "h2".into());

        let m = session.metrics();
        assert_eq!(m.total_deliveries, 2);
        assert_eq!(m.cumulative_tokens_delivered, 300);
    }

    #[test]
    fn metrics_track_evictions() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 100, 1, "h1".into());
        session.record_delivery(&cid("s2"), Resolution::Section, 200, 2, "h2".into());

        session.remove_delivered(&cid("s1"));

        let m = session.metrics();
        assert_eq!(m.total_evictions, 1);
        assert_eq!(m.cumulative_tokens_evicted, 100);
        assert_eq!(m.total_tokens_saved(), 100);
    }

    #[test]
    fn metrics_track_compression() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 500, 1, "h1".into());

        let freed = session.set_compression_tier(&cid("s1"), CompressionTier::Extractive, 150);
        assert_eq!(freed, Some(350));

        let m = session.metrics();
        assert_eq!(m.total_compressions, 1);
        assert_eq!(m.cumulative_tokens_compressed, 350);
        assert_eq!(m.total_tokens_saved(), 350);
    }

    #[test]
    fn metrics_track_bookmark_compression() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 500, 1, "h1".into());

        let freed = session.mask_to_bookmark(&cid("s1"), &["Chapter".into(), "Auth".into()]);
        assert!(freed.is_some());
        let freed = freed.unwrap();
        assert!(freed > 490); // bookmark is tiny

        let m = session.metrics();
        assert_eq!(m.total_compressions, 1);
        assert_eq!(m.cumulative_tokens_compressed, freed as u64);
    }

    #[test]
    fn metrics_track_compressed_summary() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 800, 1, "h1".into());

        let freed = session.set_compressed_summary(
            &cid("s1"),
            "Summary text".into(),
            CompressionTier::Abstractive,
            80,
        );
        assert_eq!(freed, Some(720));

        let m = session.metrics();
        assert_eq!(m.total_compressions, 1);
        assert_eq!(m.cumulative_tokens_compressed, 720);
    }

    #[test]
    fn metrics_compression_ratio() {
        let mut session = make_session();
        session.record_delivery(&cid("s1"), Resolution::Section, 1000, 1, "h1".into());
        session.remove_delivered(&cid("s1"));

        let m = session.metrics();
        // delivered=1000, evicted=1000, ratio=1.0
        assert!((m.compression_ratio() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn metrics_dedup_and_delta() {
        let mut session = make_session();
        session.record_dedup_hit();
        session.record_dedup_hit();
        session.record_delta_update();

        let m = session.metrics();
        assert_eq!(m.dedup_hits, 2);
        assert_eq!(m.delta_updates, 1);
    }

    #[test]
    fn metrics_cumulative_across_operations() {
        let mut session = make_session();

        // Deliver 3 items
        session.record_delivery(&cid("s1"), Resolution::Section, 100, 1, "h1".into());
        session.record_delivery(&cid("s2"), Resolution::Section, 200, 1, "h2".into());
        session.record_delivery(&cid("s3"), Resolution::Section, 300, 2, "h3".into());

        // Compress one
        session.set_compression_tier(&cid("s2"), CompressionTier::Extractive, 60);

        // Evict one
        session.remove_delivered(&cid("s1"));

        // Record misc
        session.record_dedup_hit();
        session.record_delta_update();

        let m = session.metrics();
        assert_eq!(m.total_deliveries, 3);
        assert_eq!(m.cumulative_tokens_delivered, 600);
        assert_eq!(m.total_evictions, 1);
        assert_eq!(m.cumulative_tokens_evicted, 100);
        assert_eq!(m.total_compressions, 1);
        assert_eq!(m.cumulative_tokens_compressed, 140);
        assert_eq!(m.total_tokens_saved(), 240);
        assert_eq!(m.dedup_hits, 1);
        assert_eq!(m.delta_updates, 1);
        // compression_ratio = 240/600 = 0.4
        assert!((m.compression_ratio() - 0.4).abs() < 0.001);
    }
}
