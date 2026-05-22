//! Session state manipulation helpers for the ministr server.
//!
//! These `impl MinistrServer` methods handle recording deliveries in the session
//! shadow, building tool responses with budget and coherence metadata, and
//! background compression of evicted entries.

use serde::Serialize;

use ministr_api::SessionSnapshot;
use ministr_core::session::{
    AccessMode, CompressionTier, SessionEntry, SessionId, SessionRegistry, UsageStatus,
};
use ministr_core::token::count_tokens;
use ministr_core::types::{ContentId, Resolution};

use super::MinistrServer;
use super::types::{NextAction, ToolResponse};
use crate::task::iso8601_now;

/// F6.1-d-c — emit a drops-ledger entry per evicted claim id.
///
/// Skipped when no tenant is scoped (stdio / in-process / self-hosted serve);
/// the ledger backend is also typically `None` in those modes, so
/// [`SessionRegistry::record_drops`] would collapse to a no-op anyway, but
/// gating on the tenant lets the call site stay unconditional.
fn emit_section_drops(reg: &SessionRegistry, session_id: &str, evicted_ids: &[String]) {
    if !evicted_ids.is_empty()
        && let Some(tenant_id) = crate::tenant_scope::current()
    {
        reg.record_drops(&tenant_id, session_id, evicted_ids);
    }
}

/// F6.1-f — consult the durable storage for a previously-snapshotted
/// session, hydrating an in-memory shell when found.
///
/// Skipped when no tenant is scoped (stdio / in-process / self-hosted serve)
/// because [`SessionRegistry::try_restore`] needs a `tenant_id` to look up
/// the snapshot by its `(tenant_id, session_id)` PK. The registry's
/// `try_restore` is itself idempotent — already-in-memory sessions short-
/// circuit there — so this helper can be invoked unconditionally before
/// `ensure_session_mut`. Failures inside `try_restore` are logged at warn
/// level and collapse to `None` (caller falls through to fresh creation).
async fn try_restore_session(reg: &mut SessionRegistry, session_id: &str) {
    let Some(tenant_id) = crate::tenant_scope::current() else {
        return;
    };
    let _ = reg
        .try_restore(session_id, &tenant_id, None, AccessMode::ReadWrite)
        .await;
}

/// F6.1-e — emit a `SessionSnapshot` so the cloud's [`PostgresSessionStorage`]
/// holds enough state to restore the session on the next pod.
///
/// Skipped when no tenant is scoped (stdio / in-process / self-hosted serve);
/// the storage backend is also typically `None` in those modes, so
/// [`SessionRegistry::persist_snapshot`] would collapse to a no-op anyway.
///
/// `opened_at` and `last_seen_at` both carry the current wall-clock. The
/// Postgres UPSERT preserves `opened_at` across re-saves (per F6.1-a's
/// `save` contract) so the FIRST insert captures the actual opening time
/// and later calls only advance `last_seen_at`. `corpus_id` is intentionally
/// `None` for v0 — `record_section_delivery` doesn't carry the bound corpus;
/// threading it through is a follow-up. `coherence_score` is 0.0 as a
/// placeholder per F6.1-c-followup's "coherence-score restore deferred"
/// note; the field is non-optional in the snapshot schema but no consumer
/// reads it yet.
///
/// [`PostgresSessionStorage`]: ministr_cloud::session_storage::PostgresSessionStorage
fn emit_session_snapshot(reg: &SessionRegistry, session_id: &str, status: &UsageStatus) {
    let Some(tenant_id) = crate::tenant_scope::current() else {
        return;
    };
    let now = iso8601_now();
    let snapshot = SessionSnapshot {
        session_id: session_id.to_owned(),
        tenant_id,
        corpus_id: None,
        opened_at: now.clone(),
        last_seen_at: now,
        budget_used: i64::try_from(status.tokens_used).unwrap_or(i64::MAX),
        coherence_score: 0.0,
    };
    reg.persist_snapshot(snapshot);
}

impl MinistrServer {
    /// Resolve the active session entry, bootstrapping it lazily if missing.
    ///
    /// Tool handlers used to call
    /// `reg.get_session_mut(&self.active_session_id).expect("active session exists")`,
    /// which assumed the session was eagerly registered at server
    /// construction. After [`Self::fork_for_new_session`] (which runs
    /// inside the sync rmcp factory closure and so cannot lock the
    /// async-mutex'd registry), the session id exists on the server but
    /// no entry has been inserted yet. This helper bridges that gap by
    /// using `get_or_create`.
    ///
    /// Stamps the captured `parent_session_id` / `client_name` hints
    /// onto the entry whenever the entry's corresponding field is still
    /// empty — *not* only on first resolution. The `initialize`
    /// handshake (which sets `client_name_hint`) and the first tool
    /// call can race; gating on creation alone meant a name set after
    /// the entry existed would never be stamped. The hint→entry copy
    /// is per-field idempotent, so re-checking on every resolution is
    /// cheap and self-healing.
    pub(super) fn ensure_session_mut<'a>(
        &self,
        reg: &'a mut SessionRegistry,
    ) -> &'a mut SessionEntry {
        let entry = reg.get_or_create(&self.active_session_id, None, AccessMode::ReadWrite);
        if entry.parent_session_id.is_none()
            && let Some(parent) = self.parent_session_id_hint.as_deref()
        {
            entry.parent_session_id = Some(SessionId::from(parent.to_string()));
        }
        if entry.client_name.is_none()
            && let Some(name) = self.client_name_hint.lock().ok().and_then(|g| g.clone())
        {
            entry.client_name = Some(name);
        }
        // F6.2-e-followup — stamp tenant_id from the request's
        // tenant_scope task-local. `None` on self-hosted / stdio /
        // in-process tests (no scope mounted); `Some(subject)` on
        // every cloud request after `validate_token_middleware` and
        // `scope_tenant`. Mirrors the parent / client_name stamping
        // shape so the hint is captured on every resolution rather
        // than only on first-create.
        //
        // F-Test-3b finding (2026-05-21): on the cloud `/mcp` path,
        // `tenant_scope::current()` ALWAYS returns `None` here — the
        // scope_tenant middleware DOES wrap the rmcp `StreamableHttpService`
        // (`cmd_serve_http` line ~433), but rmcp's internal request
        // dispatcher spawns the tool handler in a task that doesn't
        // inherit tokio task-locals. So every tool-call session is left
        // unstamped, and the F6.2-e-followup-ii filters route it to the
        // "self-hosted, admit nothing for scoped callers" arm of
        // `admit_session_for_scope`. Tracked as F-Test-3b-blocker in
        // ROADMAP discovered findings.
        if entry.tenant_id.is_none()
            && let Some(subject) = crate::tenant_scope::current()
        {
            entry.tenant_id = Some(subject);
        }
        entry
    }

    /// Record a section delivery in the session shadow and budget tracker.
    ///
    /// When the delivery causes window eviction, applies bookmark compression
    /// to evicted entries synchronously and spawns background extractive
    /// compression to upgrade bookmarks into summaries.
    ///
    /// Returns the budget status snapshot after recording.
    pub(super) async fn record_section_delivery(
        &self,
        section_id: &str,
        text: &str,
        content_hash: String,
    ) -> UsageStatus {
        let token_count = count_tokens(text);
        let content_id = ContentId(section_id.to_string());
        let mut reg = self.registry.lock().await;
        // F6.1-f — hydrate from durable storage on first access this pod.
        try_restore_session(&mut reg, &self.active_session_id).await;
        let entry = self.ensure_session_mut(&mut reg);
        let turn = entry.session.current_turn() + 1;
        entry.session.record_delivery(
            &content_id,
            Resolution::Section,
            token_count,
            turn,
            content_hash,
        );
        let evicted_ids = entry.budget.record_tokens(section_id, token_count);

        let status = entry.budget.usage_status();

        // F6.1-d-c — persist eviction events to the drops ledger before
        // releasing the registry lock.
        emit_section_drops(&reg, &self.active_session_id, &evicted_ids);

        // F6.1-e — checkpoint the session snapshot (budget_used + timestamps)
        // so a fresh pod can lazy-restore it via SessionRegistry::try_restore.
        emit_session_snapshot(&reg, &self.active_session_id, &status);

        drop(reg);

        // Phase 1: bookmark compression for evicted entries.
        // `section_heading_path` reads from storage and is only available
        // in local-engine mode. In daemon-forward mode we skip heading-path
        // enrichment — the daemon owns the section delivery state and
        // doesn't need the proxy to bookmark it.
        if !evicted_ids.is_empty()
            && let Some(ref service) = self.service
        {
            let mut heading_paths = Vec::with_capacity(evicted_ids.len());
            for evicted_id in &evicted_ids {
                heading_paths.push(service.section_heading_path(evicted_id).await);
            }
            let mut reg = self.registry.lock().await;
            if let Some(entry) = reg.get_session_mut(&self.active_session_id) {
                for (evicted_id, heading_path) in evicted_ids.iter().zip(&heading_paths) {
                    let evicted_cid = ContentId(evicted_id.clone());
                    entry.session.mask_to_bookmark(&evicted_cid, heading_path);
                }
            }
            drop(reg);
        }

        self.persist_session().await;

        // Phase 2: background extractive compression — only when running
        // local. The daemon's compression is reachable via the backend
        // trait but isn't useful here because the session shadow lives in
        // this process.
        if !evicted_ids.is_empty()
            && let Some(service) = self.service.clone()
        {
            let registry = self.registry.clone();
            let session_id = self.active_session_id.clone();
            tokio::spawn(async move {
                if let Ok(compressed) = service.compress_content(&evicted_ids).await {
                    let mut reg = registry.lock().await;
                    if let Some(entry) = reg.get_session_mut(&session_id) {
                        for item in compressed {
                            let cid = ContentId(item.original_id.clone());
                            entry.session.set_compressed_summary(
                                &cid,
                                item.summary,
                                CompressionTier::Extractive,
                                item.compressed_tokens,
                            );
                        }
                    }
                }
            });
        }

        status
    }

    /// Build a tool response with budget status and any pending coherence alerts.
    ///
    /// When budget pressure is elevated or critical, proactively includes
    /// eviction recommendations so the agent can free context tokens without
    /// having to call `ministr_usage` explicitly.
    pub(super) async fn build_response<T: Serialize + rmcp::schemars::JsonSchema>(
        &self,
        data: T,
        usage_status: UsageStatus,
    ) -> ToolResponse<T> {
        self.build_response_with(data, usage_status, Vec::new())
            .await
    }

    /// Build a tool response, appending per-handler next-action hints after
    /// the global pressure- and coherence-driven ones.
    ///
    /// Use this when a specific tool can suggest a concrete follow-up
    /// (e.g. `ministr_survey` recommending `ministr_read` on the top hit).
    /// The `extra_next_actions` are appended last so urgent global signals
    /// (compress under pressure, re-read changed sections) appear first.
    pub(super) async fn build_response_with<T: Serialize + rmcp::schemars::JsonSchema>(
        &self,
        data: T,
        usage_status: UsageStatus,
        extra_next_actions: Vec<NextAction>,
    ) -> ToolResponse<T> {
        let mut reg = self.registry.lock().await;
        // F6.1-f — hydrate from durable storage on first access this pod.
        try_restore_session(&mut reg, &self.active_session_id).await;
        let entry = self.ensure_session_mut(&mut reg);
        let alerts = entry.session.drain_alerts();
        drop(reg);

        // Budget pressure is tracked internally (UsageTracker keeps
        // recording for compression/dedup) but never surfaced to the
        // agent — the injected numbers were making agents wrongly think
        // they were out of context. So no eviction recommendations are
        // computed or sent, regardless of pressure level.
        let drop_suggestions = Vec::new();

        let progress = &self.ingestion_progress;
        let indexing = progress.is_running();
        let indexing_message = if indexing {
            let done = progress.files_done();
            let total = progress.files_total();
            Some(format!("Checking {done}/{total} files"))
        } else {
            None
        };

        let next_actions = build_next_actions(&alerts, extra_next_actions);

        ToolResponse {
            usage_status,
            coherence_alerts: alerts,
            indexing_in_progress: indexing,
            indexing_message,
            drop_suggestions,
            next_actions,
            result: data,
        }
    }
}

/// Synthesize the prioritized next-action list for a tool response.
///
/// Order: coherence-driven (re-read each changed section), then any
/// per-handler hints supplied by the caller. Pure function — easy to
/// unit-test.
///
/// Budget pressure used to contribute compress/evict entries here; it no
/// longer does. Those nudges made agents think they were running out of
/// context. Pressure is still tracked internally for compression/dedup,
/// it's just not turned into agent-facing instructions.
fn build_next_actions(
    coherence_alerts: &[ministr_core::session::CoherenceAlert],
    extra: Vec<NextAction>,
) -> Vec<NextAction> {
    let mut actions = Vec::new();

    // Coherence-driven: re-read changed sections so the agent gets a delta.
    for alert in coherence_alerts {
        for section_id in &alert.changed_sections {
            actions.push(NextAction {
                action: "ministr_read".to_string(),
                args: serde_json::json!({ "section_id": section_id }),
                reason: "Section changed since last delivery; re-read to get the delta".to_string(),
            });
        }
    }

    // Per-handler hints (e.g. "read the top survey hit").
    actions.extend(extra);

    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use ministr_api::{
        AppendDropFuture, DropEntry, DropsLedger, DropsLedgerError, ListDropsFuture,
        LoadSessionFuture, SaveSessionFuture, SessionMutFuture, SessionStorage, SessionStorageError,
    };
    use ministr_core::session::{CoherenceAlert, UsageConfig, UsageLevel};
    use std::sync::{Arc, Mutex as StdMutex};

    /// F6.1-d-c — test-only ledger that records every entry it receives.
    #[derive(Debug, Default)]
    struct StubLedger {
        entries: StdMutex<Vec<DropEntry>>,
    }

    impl DropsLedger for StubLedger {
        fn append<'a>(&'a self, entry: &'a DropEntry) -> AppendDropFuture<'a> {
            let owned = entry.clone();
            Box::pin(async move {
                self.entries
                    .lock()
                    .expect("stub ledger mutex never poisoned")
                    .push(owned);
                Ok::<(), DropsLedgerError>(())
            })
        }

        fn list_for_session<'a>(
            &'a self,
            _tenant_id: &'a str,
            _session_id: &'a str,
        ) -> ListDropsFuture<'a> {
            Box::pin(async move { Ok(Vec::new()) })
        }
    }

    /// F6.1-d-c — when a tenant is scoped and evictions are non-empty,
    /// the wiring helper fires one ledger entry per evicted claim id.
    #[tokio::test]
    async fn emit_section_drops_fires_when_tenant_scoped() {
        let stub = Arc::new(StubLedger::default());
        let registry = SessionRegistry::new(UsageConfig::default())
            .with_drops_ledger(Arc::clone(&stub) as Arc<dyn DropsLedger>);
        let evicted: Vec<String> = vec!["docs/a.md#x".into(), "docs/b.md#y".into()];

        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), async {
            emit_section_drops(&registry, "agent-session-1", &evicted);
        })
        .await;

        // record_drops spawns one task per id; let them run.
        for _ in 0..16 {
            if stub
                .entries
                .lock()
                .expect("stub ledger mutex never poisoned")
                .len()
                >= 2
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        let entries = stub.entries.lock().unwrap();
        assert_eq!(entries.len(), 2, "one ledger entry per evicted claim id");
        assert!(
            entries
                .iter()
                .all(|e| e.tenant_id == "tenant-x" && e.session_id == "agent-session-1"),
        );
        let claim_ids: Vec<&str> = entries.iter().map(|e| e.claim_id.as_str()).collect();
        assert!(claim_ids.contains(&"docs/a.md#x"));
        assert!(claim_ids.contains(&"docs/b.md#y"));
    }

    /// F6.1-d-c — without a tenant scope (stdio / self-hosted), the wiring
    /// skips the ledger call. Mirrors the production no-op for those modes.
    #[tokio::test]
    async fn emit_section_drops_skips_when_no_tenant_scope() {
        let stub = Arc::new(StubLedger::default());
        let registry = SessionRegistry::new(UsageConfig::default())
            .with_drops_ledger(Arc::clone(&stub) as Arc<dyn DropsLedger>);
        let evicted: Vec<String> = vec!["docs/a.md#x".into()];

        // No `scope_for_test` wrapper — current() returns None.
        emit_section_drops(&registry, "agent-session-1", &evicted);

        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        assert!(
            stub.entries.lock().unwrap().is_empty(),
            "no entries should be appended outside a tenant scope",
        );
    }

    /// F6.1-e — test-only `SessionStorage` that captures every save.
    #[derive(Debug, Default)]
    struct StubStorage {
        saves: StdMutex<Vec<SessionSnapshot>>,
    }

    impl SessionStorage for StubStorage {
        fn save<'a>(&'a self, snapshot: &'a SessionSnapshot) -> SaveSessionFuture<'a> {
            let owned = snapshot.clone();
            Box::pin(async move {
                self.saves
                    .lock()
                    .expect("stub storage mutex never poisoned")
                    .push(owned);
                Ok::<(), SessionStorageError>(())
            })
        }

        fn load<'a>(
            &'a self,
            tenant_id: &'a str,
            session_id: &'a str,
        ) -> LoadSessionFuture<'a> {
            // Mirrors the registry.rs F6.1-c StubStorage: return the
            // most-recently-saved snapshot matching the `(tenant_id,
            // session_id)` PK so round-trip tests can pre-seed via `save`.
            Box::pin(async move {
                let saves = self
                    .saves
                    .lock()
                    .expect("stub storage mutex never poisoned");
                Ok(saves
                    .iter()
                    .rfind(|s| s.tenant_id == tenant_id && s.session_id == session_id)
                    .cloned())
            })
        }

        fn touch<'a>(
            &'a self,
            _tenant_id: &'a str,
            _session_id: &'a str,
        ) -> SessionMutFuture<'a> {
            Box::pin(async { Ok(()) })
        }

        fn delete<'a>(
            &'a self,
            _tenant_id: &'a str,
            _session_id: &'a str,
        ) -> SessionMutFuture<'a> {
            Box::pin(async { Ok(()) })
        }
    }

    fn fixture_status(tokens_used: usize) -> UsageStatus {
        let max = 200_000usize;
        UsageStatus {
            tokens_used,
            tokens_remaining: max.saturating_sub(tokens_used),
            level: UsageLevel::Normal,
            // Not load-bearing for these tests; the assertions read
            // `tokens_used` instead.
            utilization: 0.0,
        }
    }

    /// F6.1-e — when a tenant is scoped and storage is wired, the snapshot
    /// helper fires one save carrying the live `tokens_used`.
    #[tokio::test]
    async fn emit_session_snapshot_fires_when_tenant_scoped() {
        let stub = Arc::new(StubStorage::default());
        let registry = SessionRegistry::new(UsageConfig::default())
            .with_storage(Arc::clone(&stub) as Arc<dyn SessionStorage>);
        let status = fixture_status(5_000);

        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), async {
            emit_session_snapshot(&registry, "agent-session-1", &status);
        })
        .await;

        // persist_snapshot spawns a single task; let it run.
        for _ in 0..16 {
            if !stub
                .saves
                .lock()
                .expect("stub storage mutex never poisoned")
                .is_empty()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        let snapshots = stub.saves.lock().unwrap();
        assert_eq!(snapshots.len(), 1, "one snapshot per persist call");
        let snap = &snapshots[0];
        assert_eq!(snap.session_id, "agent-session-1");
        assert_eq!(snap.tenant_id, "tenant-x");
        assert_eq!(snap.corpus_id, None);
        assert_eq!(snap.budget_used, 5_000);
        assert!(!snap.opened_at.is_empty());
        assert_eq!(snap.opened_at, snap.last_seen_at);
    }

    /// F6.1-e — without a tenant scope, the snapshot helper short-circuits
    /// before building a snapshot or touching storage.
    #[tokio::test]
    async fn emit_session_snapshot_skips_when_no_tenant_scope() {
        let stub = Arc::new(StubStorage::default());
        let registry = SessionRegistry::new(UsageConfig::default())
            .with_storage(Arc::clone(&stub) as Arc<dyn SessionStorage>);

        emit_session_snapshot(&registry, "agent-session-1", &fixture_status(5_000));

        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        assert!(
            stub.saves.lock().unwrap().is_empty(),
            "no saves should be issued outside a tenant scope",
        );
    }

    /// F6.1-e — when no storage backend is wired, the registry's
    /// `persist_snapshot` collapses to a no-op even with a scoped tenant.
    #[tokio::test]
    async fn emit_session_snapshot_is_noop_without_storage() {
        let registry = SessionRegistry::new(UsageConfig::default());
        // No `with_storage` call — registry.storage stays `None`.
        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), async {
            emit_session_snapshot(&registry, "agent-session-1", &fixture_status(123));
        })
        .await;
        // No assertion target — the point is the call doesn't panic.
    }

    /// F6.1-f — when a tenant is scoped and storage has a matching snapshot,
    /// `try_restore_session` materialises the in-memory shell.
    #[tokio::test]
    async fn try_restore_session_hydrates_when_storage_hits() {
        let stub = Arc::new(StubStorage::default());
        let mut registry = SessionRegistry::new(UsageConfig::default())
            .with_storage(Arc::clone(&stub) as Arc<dyn SessionStorage>);

        // Pre-seed the stub with a snapshot the helper should find.
        let snapshot = SessionSnapshot {
            session_id: "agent-session-1".into(),
            tenant_id: "tenant-x".into(),
            corpus_id: None,
            opened_at: "2026-05-21T00:00:00Z".into(),
            last_seen_at: "2026-05-21T00:00:00Z".into(),
            budget_used: 1_234,
            coherence_score: 0.0,
        };
        stub.saves
            .lock()
            .expect("stub storage mutex never poisoned")
            .push(snapshot);

        // Sanity: registry has no in-memory shadow yet.
        assert!(registry.get_session("agent-session-1").is_none());

        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), async {
            try_restore_session(&mut registry, "agent-session-1").await;
        })
        .await;

        assert!(
            registry.get_session("agent-session-1").is_some(),
            "shell should be materialised after a storage hit",
        );
    }

    /// F6.1-f — without a tenant scope, `try_restore` is impossible (no PK
    /// lookup key) and the helper short-circuits without touching storage.
    #[tokio::test]
    async fn try_restore_session_skips_when_no_tenant_scope() {
        let stub = Arc::new(StubStorage::default());
        let mut registry = SessionRegistry::new(UsageConfig::default())
            .with_storage(Arc::clone(&stub) as Arc<dyn SessionStorage>);

        // No scope wrapper — current() returns None.
        try_restore_session(&mut registry, "agent-session-1").await;
        assert!(
            registry.get_session("agent-session-1").is_none(),
            "no scope ⇒ no restore ⇒ no shadow created",
        );
    }

    /// F6.1-f — when the session already exists in-memory, `try_restore`
    /// short-circuits (per its own contract) and the helper is effectively
    /// a no-op.
    #[tokio::test]
    async fn try_restore_session_is_noop_when_already_in_memory() {
        let stub = Arc::new(StubStorage::default());
        let mut registry = SessionRegistry::new(UsageConfig::default())
            .with_storage(Arc::clone(&stub) as Arc<dyn SessionStorage>);

        // Bootstrap the session in-memory before any restore attempt.
        registry.create_session("agent-session-1", None, AccessMode::ReadWrite);

        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), async {
            try_restore_session(&mut registry, "agent-session-1").await;
        })
        .await;

        // Stub records no `load` calls in `saves`; the strongest signal is
        // that the session remains a fresh-bootstrapped shell rather than
        // anything snapshot-derived — `try_restore` would have failed to
        // overwrite an existing entry anyway, but we want the no-op shape.
        assert!(registry.get_session("agent-session-1").is_some());
    }

    /// F6.1-f — without a storage backend wired, `try_restore` falls
    /// through to its `None` branch (per F6.1-c contract). Helper must
    /// not panic and must not leave a stray entry.
    #[tokio::test]
    async fn try_restore_session_is_noop_when_no_storage() {
        let mut registry = SessionRegistry::new(UsageConfig::default());
        // No `with_storage` — `registry.storage` stays `None`.

        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), async {
            try_restore_session(&mut registry, "agent-session-1").await;
        })
        .await;

        assert!(registry.get_session("agent-session-1").is_none());
    }

    /// F6.1-d-c — empty eviction list is a no-op even when scoped.
    #[tokio::test]
    async fn emit_section_drops_skips_when_no_evictions() {
        let stub = Arc::new(StubLedger::default());
        let registry = SessionRegistry::new(UsageConfig::default())
            .with_drops_ledger(Arc::clone(&stub) as Arc<dyn DropsLedger>);

        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), async {
            emit_section_drops(&registry, "agent-session-1", &[]);
        })
        .await;

        for _ in 0..4 {
            tokio::task::yield_now().await;
        }
        assert!(stub.entries.lock().unwrap().is_empty());
    }

    #[test]
    fn no_actions_with_no_alerts_or_extras() {
        let actions = build_next_actions(&[], Vec::new());
        assert!(actions.is_empty());
    }

    #[test]
    fn coherence_alerts_emit_one_read_per_changed_section() {
        let alerts = vec![CoherenceAlert {
            changed_sections: vec!["docs/a.md#x".into(), "docs/b.md#y".into()],
            stale_content_ids: vec![],
        }];
        let actions = build_next_actions(&alerts, Vec::new());

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action, "ministr_read");
        assert_eq!(actions[0].args["section_id"], "docs/a.md#x");
        assert_eq!(actions[1].args["section_id"], "docs/b.md#y");
    }

    /// Regression guard for the budget-hint removal: even with coherence
    /// activity in play, no compress/evict pressure nudges are emitted.
    /// `build_next_actions` no longer even accepts a pressure argument,
    /// so this asserts the only actions are the coherence re-reads.
    #[test]
    fn no_compress_or_evict_actions_are_ever_emitted() {
        let alerts = vec![CoherenceAlert {
            changed_sections: vec!["docs/a.md#x".into()],
            stale_content_ids: vec![],
        }];
        let actions = build_next_actions(&alerts, Vec::new());

        assert!(
            actions
                .iter()
                .all(|a| a.action != "ministr_compress" && a.action != "ministr_dropped"),
            "budget pressure must not inject compress/evict next-actions",
        );
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].action, "ministr_read");
    }

    #[test]
    fn extras_are_appended_after_coherence() {
        let extras = vec![NextAction {
            action: "ministr_definition".to_string(),
            args: serde_json::json!({ "symbol_id": "sym-1" }),
            reason: "single match".to_string(),
        }];
        let alerts = vec![CoherenceAlert {
            changed_sections: vec!["docs/a.md#x".into()],
            stale_content_ids: vec![],
        }];
        let actions = build_next_actions(&alerts, extras);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action, "ministr_read");
        assert_eq!(actions[1].action, "ministr_definition");
    }
}
