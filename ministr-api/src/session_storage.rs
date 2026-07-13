//! Durable agent-session checkpoint seam.
//!
//! open-core boundary for snapshotting agent-session state to
//! durable storage so a pod restart or cross-pod load-balance doesn't
//! reset a session. The trait lives in `ministr-api` (MIT) so
//! `ministr-core`'s `SessionRegistry` can hold an
//! `Option<Arc<dyn SessionStorage>>` without depending on the closed
//! `ministr-cloud` crate. The cloud ships a `PostgresSessionStorage`
//! that writes to the `agent_sessions` table; self-hosted serve leaves
//! the field `None` and sessions remain in-memory.
//!
//! # What's persisted vs. what's recomputed
//!
//! v1 stores the load-bearing aggregate fields plus a versioned `state`
//! payload:
//! - `budget_used` — cumulative tokens; a resumed session can't
//!   accidentally re-burn the budget the prior pod already consumed.
//! - `coherence_score` — the live coherence number; resuming with the
//!   warm score avoids the cold-start visual artifact.
//! - `last_seen_at` — for the eventual stale-session prune cron.
//!
//! - exact `(corpus_id, content_id, resolution)` deliveries, including
//!   compression tier and summaries;
//! - the bounded access trajectory and exact dropped identities; and
//! - cumulative token-economics counters.
//!
//! Old rows do not have `state`; serde defaults them to an empty v0 payload.
//! Postgres implementations should store `state` in a `JSONB NOT NULL DEFAULT
//! '{}'` column and reject an UPSERT whose `revision` is older than the stored
//! revision. That makes fire-and-forget checkpoints monotonic even when two
//! writes complete out of order.
//!
//! # Open / save / load semantics
//!
//! - `save` is fire-and-forget (returns a future the caller awaits,
//!   but the future logs + swallows errors rather than propagating
//!   them — a checkpoint failure must not break the user's tool call).
//! - `load` returns `Option<SessionSnapshot>` so a fresh pod can ask
//!   "do you remember this session?" without distinguishing "no
//!   session" from "storage error" at the call site.
//! - `touch` updates only `last_seen_at` — cheaper than a full save
//!   for the hot path where the session is unchanged.
//! - `delete` removes a session row on explicit close.

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

use crate::metadata::DeliveryIdentity;

/// Current durable session-state payload version.
pub const SESSION_STATE_VERSION: u16 = 1;

/// One exact delivery persisted inside [`SessionSnapshot::state`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionDeliverySnapshot {
    /// Corpus/content/transport-resolution identity used for deduplication.
    pub identity: DeliveryIdentity,
    /// Coarse index resolution (`section`, `summary`, or `claim`).
    pub index_resolution: String,
    /// Tokens still represented in the agent context after compression.
    pub token_count: usize,
    /// Interaction turn when this representation was delivered.
    pub turn_delivered: u32,
    /// Content hash used for delta detection.
    pub content_hash: String,
    /// Compression tier (`full`, `extractive`, `abstractive`, or `bookmark`).
    pub compression_tier: String,
    /// Persisted compressed text when the tier carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compressed_summary: Option<String>,
}

/// Cumulative token-economics counters persisted across pod restarts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SessionMetricsSnapshot {
    pub total_deliveries: u64,
    pub cumulative_tokens_delivered: u64,
    pub total_evictions: u64,
    pub cumulative_tokens_evicted: u64,
    pub total_compressions: u64,
    pub cumulative_tokens_compressed: u64,
    pub delta_updates: u64,
    pub dedup_hits: u64,
    pub cumulative_tokens_deduplicated: u64,
    pub cumulative_bytes_deduplicated: u64,
}

/// Versioned state stored as one JSON value by durable session backends.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct SessionStateSnapshot {
    /// Payload schema version. Zero identifies a legacy/defaulted row.
    pub version: u16,
    /// Monotonic epoch-microsecond revision used to reject stale UPSERTs.
    pub revision: u64,
    /// Exact deliveries that are still resident in the agent context.
    pub delivered: Vec<SessionDeliverySnapshot>,
    /// Bounded access order used by prefetch and restore ordering.
    pub trajectory: Vec<DeliveryIdentity>,
    /// Exact historical deliveries no longer resident. These tombstones make
    /// a newer drop checkpoint authoritative over an older in-flight save.
    pub dropped_identities: Vec<DeliveryIdentity>,
    /// Current interaction turn.
    pub current_turn: u32,
    /// Cumulative token-economics counters.
    pub metrics: SessionMetricsSnapshot,
}

impl SessionStateSnapshot {
    fn is_empty(&self) -> bool {
        self.version == 0
            && self.revision == 0
            && self.delivered.is_empty()
            && self.trajectory.is_empty()
            && self.dropped_identities.is_empty()
            && self.current_turn == 0
            && self.metrics == SessionMetricsSnapshot::default()
    }
}

/// Errors a [`SessionStorage`] implementation can surface.
#[derive(Debug, thiserror::Error)]
pub enum SessionStorageError {
    /// Storage layer rejected the call (network, schema drift, etc.).
    /// Callers typically log + continue — a checkpoint hiccup should
    /// not fail the enclosing tool call.
    #[error("session storage: {0}")]
    Storage(String),
}

/// One persisted snapshot of an agent session.
///
/// Field order mirrors the columns in `0008_agent_sessions.sql`. The
/// snapshot is the contract between `ministr-core`'s `SessionRegistry`
/// and the cloud's Postgres backend; future field additions go on the
/// end so older crates can deserialise newer rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SessionSnapshot {
    /// Session id as the agent presented it (free-form string).
    pub session_id: String,
    /// Tenant UUID string. The PK is `(tenant_id, session_id)` so two
    /// tenants can use the same id without colliding.
    pub tenant_id: String,
    /// Bound corpus id, if any. Sessions opened before a corpus is
    /// chosen leave this `None`.
    pub corpus_id: Option<String>,
    /// ISO-8601 UTC timestamp the session was first opened.
    pub opened_at: String,
    /// ISO-8601 UTC timestamp the session was most recently touched.
    pub last_seen_at: String,
    /// Cumulative tokens this session has consumed.
    pub budget_used: i64,
    /// Cross-session coherence score, in `[0.0, 1.0]`.
    pub coherence_score: f64,
    /// Corpus-aware delivery/compression/drop state. Missing on legacy rows.
    #[serde(default, skip_serializing_if = "SessionStateSnapshot::is_empty")]
    pub state: SessionStateSnapshot,
}

impl SessionSnapshot {
    /// Whether this checkpoint may replace `current` for the same primary key.
    ///
    /// v1 revisions are authoritative over legacy rows. Two legacy rows fall
    /// back to their ISO timestamp, preserving compatibility during rollout.
    #[must_use]
    pub fn supersedes(&self, current: &Self) -> bool {
        match (self.state.revision, current.state.revision) {
            (0, 0) => self.last_seen_at >= current.last_seen_at,
            (0, _) => false,
            (_, 0) => true,
            (incoming, stored) => incoming >= stored,
        }
    }
}

/// Returned future shape for [`SessionStorage::save`].
pub type SaveSessionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), SessionStorageError>> + Send + 'a>>;

/// Returned future shape for [`SessionStorage::load`].
pub type LoadSessionFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Option<SessionSnapshot>, SessionStorageError>> + Send + 'a>>;

/// Returned future shape for [`SessionStorage::touch`] and [`SessionStorage::delete`].
pub type SessionMutFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), SessionStorageError>> + Send + 'a>>;

/// Snapshot agent-session state to durable storage.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn SessionStorage>` inside `ministr-core`'s `SessionRegistry`.
///
/// # Failure posture
///
/// Storage errors are surfaced via `Result` so the registry can choose
/// its own posture (today: log + continue; future: prometheus counter).
/// A storage outage must never break a live tool call — the in-memory
/// `SessionEntry` is the source of truth for the active request, and the
/// snapshot is a best-effort durability layer.
pub trait SessionStorage: Send + Sync + std::fmt::Debug {
    /// Write the full snapshot, upserting on `(tenant_id, session_id)`.
    /// Implementations must persist [`SessionSnapshot::state`] and only
    /// replace an existing row when [`SessionSnapshot::supersedes`] returns
    /// true, so an older in-flight checkpoint cannot resurrect dropped data.
    fn save<'a>(&'a self, snapshot: &'a SessionSnapshot) -> SaveSessionFuture<'a>;

    /// Load a snapshot by `(tenant_id, session_id)`. Returns `Ok(None)`
    /// when no row exists — distinguishable from a storage error so
    /// the registry can hydrate a fresh in-memory `SessionEntry` vs.
    /// fail closed on backend issues.
    fn load<'a>(&'a self, tenant_id: &'a str, session_id: &'a str) -> LoadSessionFuture<'a>;

    /// Touch `last_seen_at` for an existing row. Cheaper than a full
    /// `save` when the budget / coherence haven't changed — useful for
    /// keep-alive on long-lived sessions whose state is mostly stable.
    fn touch<'a>(&'a self, tenant_id: &'a str, session_id: &'a str) -> SessionMutFuture<'a>;

    /// Remove a session row. Idempotent — deleting a non-existent
    /// session is `Ok(())`.
    fn delete<'a>(&'a self, tenant_id: &'a str, session_id: &'a str) -> SessionMutFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct StubStorage {
        rows: Mutex<Vec<SessionSnapshot>>,
    }

    impl SessionStorage for StubStorage {
        fn save<'a>(&'a self, snapshot: &'a SessionSnapshot) -> SaveSessionFuture<'a> {
            let owned = snapshot.clone();
            Box::pin(async move {
                let mut rows = self.rows.lock().unwrap();
                if let Some(current) = rows.iter_mut().find(|row| {
                    row.tenant_id == owned.tenant_id && row.session_id == owned.session_id
                }) {
                    if owned.supersedes(current) {
                        *current = owned;
                    }
                } else {
                    rows.push(owned);
                }
                Ok(())
            })
        }

        fn load<'a>(&'a self, tenant_id: &'a str, session_id: &'a str) -> LoadSessionFuture<'a> {
            Box::pin(async move {
                let rows = self.rows.lock().unwrap();
                Ok(rows
                    .iter()
                    .find(|r| r.tenant_id == tenant_id && r.session_id == session_id)
                    .cloned())
            })
        }

        fn touch<'a>(&'a self, tenant_id: &'a str, session_id: &'a str) -> SessionMutFuture<'a> {
            Box::pin(async move {
                let mut rows = self.rows.lock().unwrap();
                if let Some(row) = rows
                    .iter_mut()
                    .find(|r| r.tenant_id == tenant_id && r.session_id == session_id)
                {
                    row.last_seen_at = "2026-05-21T00:00:00Z".into();
                }
                Ok(())
            })
        }

        fn delete<'a>(&'a self, tenant_id: &'a str, session_id: &'a str) -> SessionMutFuture<'a> {
            Box::pin(async move {
                let mut rows = self.rows.lock().unwrap();
                rows.retain(|r| !(r.tenant_id == tenant_id && r.session_id == session_id));
                Ok(())
            })
        }
    }

    fn fixture() -> SessionSnapshot {
        SessionSnapshot {
            session_id: "sess-1".into(),
            tenant_id: "tenant-uuid".into(),
            corpus_id: Some("corpus-a".into()),
            opened_at: "2026-05-21T00:00:00Z".into(),
            last_seen_at: "2026-05-21T00:00:00Z".into(),
            budget_used: 1024,
            coherence_score: 0.87,
            state: SessionStateSnapshot::default(),
        }
    }

    #[test]
    fn legacy_wire_snapshot_defaults_state() {
        let legacy = serde_json::json!({
            "session_id": "sess-legacy",
            "tenant_id": "tenant-uuid",
            "corpus_id": null,
            "opened_at": "2026-05-21T00:00:00Z",
            "last_seen_at": "2026-05-21T00:00:00Z",
            "budget_used": 12,
            "coherence_score": 0.5
        });
        let restored: SessionSnapshot = serde_json::from_value(legacy).unwrap();
        assert_eq!(restored.state, SessionStateSnapshot::default());
    }

    #[test]
    fn state_wire_round_trip_keeps_colliding_content_ids_distinct() {
        let mut snapshot = fixture();
        snapshot.state = SessionStateSnapshot {
            version: SESSION_STATE_VERSION,
            revision: 42,
            delivered: ["left", "right"]
                .into_iter()
                .map(|corpus_id| SessionDeliverySnapshot {
                    identity: DeliveryIdentity {
                        corpus_id: corpus_id.into(),
                        content_id: "same-id".into(),
                        resolution: "section_excerpt".into(),
                    },
                    index_resolution: "section".into(),
                    token_count: 10,
                    turn_delivered: 1,
                    content_hash: corpus_id.into(),
                    compression_tier: "full".into(),
                    compressed_summary: None,
                })
                .collect(),
            ..SessionStateSnapshot::default()
        };
        let encoded = serde_json::to_vec(&snapshot).unwrap();
        let decoded: SessionSnapshot = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, snapshot);
        assert_ne!(
            decoded.state.delivered[0].identity,
            decoded.state.delivered[1].identity
        );
    }

    #[tokio::test]
    async fn save_then_load_round_trips() {
        let stub = StubStorage::default();
        let storage: Arc<dyn SessionStorage> = Arc::new(stub);
        let snap = fixture();
        storage.save(&snap).await.unwrap();
        let loaded = storage
            .load(&snap.tenant_id, &snap.session_id)
            .await
            .unwrap();
        assert_eq!(loaded, Some(snap));
    }

    #[tokio::test]
    async fn load_returns_none_for_unknown_session() {
        let storage: Arc<dyn SessionStorage> = Arc::new(StubStorage::default());
        let loaded = storage.load("nobody", "nothing").await.unwrap();
        assert_eq!(loaded, None);
    }

    #[tokio::test]
    async fn save_upserts_on_tenant_session_pk() {
        let storage: Arc<dyn SessionStorage> = Arc::new(StubStorage::default());
        let mut snap = fixture();
        storage.save(&snap).await.unwrap();
        snap.budget_used = 4096;
        storage.save(&snap).await.unwrap();
        let loaded = storage
            .load(&snap.tenant_id, &snap.session_id)
            .await
            .unwrap();
        assert_eq!(loaded.unwrap().budget_used, 4096);
    }

    #[tokio::test]
    async fn stale_revision_cannot_overwrite_newer_drop_state() {
        let storage: Arc<dyn SessionStorage> = Arc::new(StubStorage::default());
        let mut newer = fixture();
        newer.state.version = SESSION_STATE_VERSION;
        newer.state.revision = 11;
        newer.state.dropped_identities.push(DeliveryIdentity {
            corpus_id: "corpus-a".into(),
            content_id: "dropped".into(),
            resolution: "section_excerpt".into(),
        });
        let mut stale = fixture();
        stale.state.version = SESSION_STATE_VERSION;
        stale.state.revision = 10;
        stale.state.delivered.push(SessionDeliverySnapshot {
            identity: DeliveryIdentity {
                corpus_id: "corpus-a".into(),
                content_id: "dropped".into(),
                resolution: "section_excerpt".into(),
            },
            index_resolution: "section".into(),
            token_count: 20,
            turn_delivered: 1,
            content_hash: "old".into(),
            compression_tier: "full".into(),
            compressed_summary: None,
        });

        storage.save(&newer).await.unwrap();
        storage.save(&stale).await.unwrap();
        let loaded = storage
            .load(&newer.tenant_id, &newer.session_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.state.revision, 11);
        assert!(loaded.state.delivered.is_empty());
        assert_eq!(loaded.state.dropped_identities.len(), 1);
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let storage: Arc<dyn SessionStorage> = Arc::new(StubStorage::default());
        // Deleting a non-existent session should still return Ok.
        storage.delete("nobody", "nothing").await.unwrap();
        let snap = fixture();
        storage.save(&snap).await.unwrap();
        storage
            .delete(&snap.tenant_id, &snap.session_id)
            .await
            .unwrap();
        // Second delete on the now-gone row is still Ok.
        storage
            .delete(&snap.tenant_id, &snap.session_id)
            .await
            .unwrap();
        assert_eq!(
            storage
                .load(&snap.tenant_id, &snap.session_id)
                .await
                .unwrap(),
            None
        );
    }
}
