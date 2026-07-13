//! Session state manipulation helpers for the ministr server.
//!
//! These `impl MinistrServer` methods handle recording deliveries in the session
//! shadow, building tool responses with budget and coherence metadata, and
//! background compression of evicted entries.

use serde::Serialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ministr_api::SessionSnapshot;
use ministr_core::session::{
    AccessMode, CompressionTier, SessionEntry, SessionId, SessionRegistry, UsageStatus,
};
use ministr_core::storage::Storage;
use ministr_core::token::count_tokens;
use ministr_core::types::{ContentId, DeliveryIdentity, Resolution};

use super::MinistrServer;
use super::types::{NextAction, ToolResponse};
use crate::task::iso8601_now;

/// Emit a drops-ledger entry per evicted claim id.
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

/// Consult the durable storage for a previously-snapshotted
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

/// Emit a `SessionSnapshot` so the cloud's [`PostgresSessionStorage`]
/// holds enough state to restore the session on the next pod.
///
/// Skipped when no tenant is scoped (stdio / in-process / self-hosted serve);
/// the storage backend is also typically `None` in those modes, so
/// [`SessionRegistry::persist_snapshot`] would collapse to a no-op anyway.
///
/// `opened_at` and `last_seen_at` both carry the current wall-clock. The
/// Postgres UPSERT preserves `opened_at` across re-saves so the FIRST
/// insert captures the actual opening time and later calls only advance
/// `last_seen_at`. A single-corpus session records that corpus directly;
/// mixed-corpus sessions keep `corpus_id = None` and carry every corpus in
/// the structured state payload. `coherence_score` remains the aggregate
/// value expected by existing storage backends.
///
/// [`PostgresSessionStorage`]: ministr_cloud::session_storage::PostgresSessionStorage
fn emit_session_snapshot(reg: &SessionRegistry, session_id: &str, status: &UsageStatus) {
    let Some(tenant_id) = crate::tenant_scope::current() else {
        return;
    };
    emit_session_snapshot_for_tenant(reg, session_id, status, tenant_id);
}

fn emit_session_snapshot_for_tenant(
    reg: &SessionRegistry,
    session_id: &str,
    status: &UsageStatus,
    tenant_id: String,
) {
    let now = iso8601_now();
    let state = reg.snapshot_state(session_id, next_snapshot_revision());
    let mut corpus_ids = state
        .delivered
        .iter()
        .map(|delivery| delivery.identity.corpus_id.clone())
        .collect::<Vec<_>>();
    corpus_ids.sort();
    corpus_ids.dedup();
    let snapshot = SessionSnapshot {
        session_id: session_id.to_owned(),
        tenant_id,
        corpus_id: (corpus_ids.len() == 1).then(|| corpus_ids[0].clone()),
        opened_at: now.clone(),
        last_seen_at: now,
        budget_used: i64::try_from(status.tokens_used).unwrap_or(i64::MAX),
        coherence_score: 0.0,
        state,
    };
    reg.persist_snapshot(snapshot);
}

fn next_snapshot_revision() -> u64 {
    static LAST_REVISION: AtomicU64 = AtomicU64::new(0);
    let wall_clock = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
        });
    let mut previous = LAST_REVISION.load(Ordering::Relaxed);
    loop {
        let next = wall_clock.max(previous.saturating_add(1));
        match LAST_REVISION.compare_exchange_weak(
            previous,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return next,
            Err(actual) => previous = actual,
        }
    }
}

impl MinistrServer {
    #[allow(clippy::too_many_lines)] // one metadata snapshot keeps progress and freshness fields mutually consistent
    async fn local_idle_query_metadata(
        &self,
    ) -> (
        ministr_api::metadata::ResponseStatus,
        ministr_api::metadata::Completeness,
        Option<ministr_api::metadata::ResponseError>,
    ) {
        use ministr_api::metadata::{
            Completeness, CompletenessState, ResponseError, ResponseStatus,
        };

        let Some(storage) = self.storage.as_ref() else {
            return (ResponseStatus::Ok, Completeness::default(), None);
        };
        let (sections, symbols, records) = tokio::join!(
            storage.section_count(),
            storage.symbol_count(),
            storage.list_file_hashes()
        );
        let (Ok(sections), Ok(symbols), Ok(mut records)) = (sections, symbols, records) else {
            return (
                ResponseStatus::Error,
                Completeness {
                    completeness: CompletenessState::Unavailable,
                    indexed_items: 0,
                    estimated_total_items: None,
                    affected_capabilities: vec!["search".to_string(), "navigation".to_string()],
                    index_generation: None,
                    absence_is_conclusive: false,
                    retry_guidance: Some(
                        "Repair or reopen the local corpus, then retry.".to_string(),
                    ),
                },
                Some(ResponseError {
                    error_code: "local_index_unavailable".to_string(),
                    retryable: true,
                    message: "Local index metadata could not be read.".to_string(),
                    corpus_id: Some(ministr_core::types::PRIMARY_CORPUS_ID.to_string()),
                    backend: Some("local".to_string()),
                }),
            );
        };
        let indexed_items = sections.saturating_add(symbols);
        records.sort_by(|a, b| a.path.cmp(&b.path));
        let generation = blake3::hash(
            &serde_json::to_vec(
                &records
                    .iter()
                    .map(|record| (&record.path, &record.content_hash))
                    .collect::<Vec<_>>(),
            )
            .unwrap_or_default(),
        )
        .to_hex()
        .to_string();

        let stale = if records.is_empty() {
            indexed_items > 0
        } else {
            let Ok(root) = std::env::current_dir() else {
                return (
                    ResponseStatus::Partial,
                    Completeness {
                        completeness: CompletenessState::Stale,
                        indexed_items,
                        estimated_total_items: Some(indexed_items),
                        affected_capabilities: vec!["search".to_string(), "navigation".to_string()],
                        index_generation: Some(generation),
                        absence_is_conclusive: false,
                        retry_guidance: Some(
                            "Retry after the local working directory is available.".to_string(),
                        ),
                    },
                    None,
                );
            };
            let freshness_records = records.clone();
            tokio::task::spawn_blocking(move || {
                ministr_core::freshness::compute_freshness(&[root], &freshness_records, &[])
            })
            .await
            .ok()
            .and_then(Result::ok)
            .is_none_or(|files| {
                files
                    .iter()
                    .any(|file| file.state != ministr_core::freshness::FreshnessState::Current)
            })
        };
        let completeness = Completeness {
            completeness: if stale {
                CompletenessState::Stale
            } else {
                CompletenessState::Complete
            },
            indexed_items,
            estimated_total_items: Some(indexed_items),
            affected_capabilities: if stale {
                vec!["search".to_string(), "navigation".to_string()]
            } else {
                Vec::new()
            },
            index_generation: Some(format!("local:{generation}")),
            absence_is_conclusive: !stale,
            retry_guidance: stale.then(|| {
                "Refresh the local corpus before treating absence as conclusive.".to_string()
            }),
        };
        (
            if stale {
                ResponseStatus::Partial
            } else {
                ResponseStatus::Ok
            },
            completeness,
            None,
        )
    }

    /// Hydrate the active session before any exclusion/dedup decision.
    pub(super) async fn restore_active_session(&self) {
        let mut registry = self.registry.lock().await;
        try_restore_session(&mut registry, &self.effective_session_id()).await;
    }

    /// Resolve the active session entry, bootstrapping it lazily if missing.
    ///
    /// Tool handlers used to call
    /// `reg.get_session_mut(&self.effective_session_id()).expect("active session exists")`,
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
        let entry = reg.get_or_create(&self.effective_session_id(), None, AccessMode::ReadWrite);
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
        // Stamp tenant_id from the request's tenant_scope task-local.
        // `None` on self-hosted / stdio / in-process tests (no scope
        // mounted); `Some(subject)` on every cloud request after
        // `validate_token_middleware` and `scope_tenant`.
        //
        // On the cloud `/mcp` path, the task-local set by the outer
        // `scope_tenant` middleware doesn't survive rmcp's internal
        // request dispatcher (its spawn loses tokio task-locals).
        // `current_tenant_subject` walks the task-local first, then
        // falls back to `self.tenant_id_hint`, which is captured at
        // `initialize` time from `context.extensions`'s
        // `axum::http::request::Parts` (the Parts extension path
        // survives the spawn boundary). Mirrors the parent /
        // client_name hint pattern.
        if entry.tenant_id.is_none()
            && let Some(subject) = self.current_tenant_subject()
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
        identity: &DeliveryIdentity,
        text: &str,
        content_hash: String,
    ) -> UsageStatus {
        let token_count = count_tokens(text);
        let mut reg = self.registry.lock().await;
        // Hydrate from durable storage on first access this pod.
        try_restore_session(&mut reg, &self.effective_session_id()).await;
        let entry = self.ensure_session_mut(&mut reg);
        let turn = entry.session.current_turn() + 1;
        entry.session.record_delivery_identity(
            identity,
            Resolution::Section,
            token_count,
            turn,
            content_hash,
        );
        let evicted_ids = entry
            .budget
            .record_tokens(&identity.storage_key(), token_count);
        let evicted_identities: Vec<DeliveryIdentity> = evicted_ids
            .iter()
            .map(|key| DeliveryIdentity::from_storage_key(key, "section_full"))
            .collect();
        let evicted_content_ids: Vec<String> = evicted_identities
            .iter()
            .map(|evicted| evicted.content_id.clone())
            .collect();

        let status = entry.budget.usage_status();

        // Persist eviction events to the drops ledger before releasing
        // the registry lock.
        emit_section_drops(&reg, &self.effective_session_id(), &evicted_content_ids);

        drop(reg);

        // Phase 1: bookmark compression for evicted entries.
        // `section_heading_path` reads from storage and is only available
        // in local-engine mode. In daemon-forward mode we skip heading-path
        // enrichment — the daemon owns the section delivery state and
        // doesn't need the proxy to bookmark it.
        if !evicted_identities.is_empty()
            && let Some(ref service) = self.service
        {
            let mut heading_paths = Vec::with_capacity(evicted_identities.len());
            for evicted in &evicted_identities {
                if evicted.corpus_id == ministr_core::types::PRIMARY_CORPUS_ID {
                    heading_paths.push(service.section_heading_path(&evicted.content_id).await);
                } else {
                    heading_paths.push(Vec::new());
                }
            }
            let mut reg = self.registry.lock().await;
            if let Some(entry) = reg.get_session_mut(&self.effective_session_id()) {
                for (evicted, heading_path) in evicted_identities.iter().zip(&heading_paths) {
                    entry
                        .session
                        .mask_identity_to_bookmark(evicted, heading_path);
                }
            }
            drop(reg);
        }

        self.persist_session().await;

        // Phase 2: background extractive compression — only when running
        // local. The daemon's compression is reachable via the backend
        // trait but isn't useful here because the session shadow lives in
        // this process.
        let local_evicted: Vec<String> = evicted_identities
            .iter()
            .filter(|evicted| evicted.corpus_id == ministr_core::types::PRIMARY_CORPUS_ID)
            .map(|evicted| evicted.content_id.clone())
            .collect();
        if !local_evicted.is_empty()
            && let Some(service) = self.service.clone()
        {
            let registry = self.registry.clone();
            let session_id = self.active_session_id.clone();
            tokio::spawn(async move {
                if let Ok(compressed) = service.compress_content(&local_evicted).await {
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
        // Hydrate from durable storage on first access this pod.
        try_restore_session(&mut reg, &self.effective_session_id()).await;
        let entry = self.ensure_session_mut(&mut reg);
        let alerts = entry.session.drain_alerts();
        let persisted_status = entry.budget.usage_status();
        // One response-boundary checkpoint owns cloud persistence for every
        // delivery, drop, and compression path. This avoids partial handlers
        // racing separate fire-and-forget saves for the same session.
        emit_session_snapshot(&reg, &self.effective_session_id(), &persisted_status);
        drop(reg);

        let mut stale_identities: Vec<DeliveryIdentity> = alerts
            .iter()
            .flat_map(|alert| alert.stale_identities.iter().cloned())
            .collect();
        for identity in stale_identities.clone() {
            if identity.resolution.starts_with("symbol") {
                stale_identities.push(DeliveryIdentity::new(
                    &identity.corpus_id,
                    &identity.content_id,
                    "symbol_definition_default",
                ));
            }
        }
        if !stale_identities.is_empty() {
            self.prefetch
                .lock()
                .await
                .invalidate_identities(&stale_identities);
            let stale_keys: HashSet<String> = stale_identities
                .iter()
                .map(DeliveryIdentity::storage_key)
                .collect();
            self.prefetch_definition_metadata
                .lock()
                .await
                .retain(|key, _| !stale_keys.contains(key));
            self.prefetch_section_metadata
                .lock()
                .await
                .retain(|key, _| !stale_keys.contains(key));
        }

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

        let (status, completeness, error) = if indexing {
            (
                ministr_api::metadata::ResponseStatus::Partial,
                ministr_api::metadata::Completeness {
                    completeness: ministr_api::metadata::CompletenessState::Partial,
                    indexed_items: progress.files_done(),
                    estimated_total_items: Some(progress.files_total()),
                    affected_capabilities: vec!["search".to_string(), "navigation".to_string()],
                    index_generation: None,
                    absence_is_conclusive: false,
                    retry_guidance: Some(
                        "Indexing is active; retry negative queries after completion.".to_string(),
                    ),
                },
                None,
            )
        } else {
            self.local_idle_query_metadata().await
        };

        ToolResponse {
            status,
            completeness,
            corpora: Vec::new(),
            error,
            usage_status,
            coherence_alerts: alerts,
            indexing_in_progress: indexing,
            indexing_message,
            drop_suggestions,
            next_actions,
            result: Some(data),
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
        if !alert.stale_identities.is_empty() {
            for identity in &alert.stale_identities {
                let (action, argument, content_id) = if identity.resolution.starts_with("symbol") {
                    (
                        "ministr_definition",
                        "symbol_id",
                        identity.content_id.clone(),
                    )
                } else if identity.resolution.starts_with("claim") {
                    (
                        "ministr_read",
                        "section_id",
                        ministr_core::types::parent_section_id(&identity.content_id)
                            .unwrap_or(&identity.content_id)
                            .to_string(),
                    )
                } else {
                    ("ministr_read", "section_id", identity.content_id.clone())
                };
                let mut args = serde_json::Value::Object(serde_json::Map::from_iter([(
                    argument.to_string(),
                    serde_json::Value::String(content_id),
                )]));
                if identity.corpus_id != ministr_core::types::PRIMARY_CORPUS_ID {
                    args["project"] = serde_json::Value::String(identity.corpus_id.clone());
                    args["source_corpus"] = serde_json::Value::String(identity.corpus_id.clone());
                }
                actions.push(NextAction {
                    action: action.to_string(),
                    args,
                    reason: "Indexed content changed since last delivery; fetch the routed delta"
                        .to_string(),
                });
            }
            continue;
        }
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
        LoadSessionFuture, SaveSessionFuture, SessionMutFuture, SessionStorage,
        SessionStorageError,
    };
    use ministr_core::session::{CoherenceAlert, UsageConfig, UsageLevel};
    use std::sync::{Arc, Mutex as StdMutex};

    /// Test-only ledger that records every entry it receives.
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

    /// When a tenant is scoped and evictions are non-empty,
    /// the wiring helper fires one ledger entry per evicted claim id.
    #[tokio::test]
    async fn emit_section_drops_fires_when_tenant_scoped() {
        let stub = Arc::new(StubLedger::default());
        let registry = SessionRegistry::new(UsageConfig::default())
            .with_drops_ledger(Arc::clone(&stub) as Arc<dyn DropsLedger>);
        let evicted: Vec<String> = vec!["docs/a.md#x".into(), "docs/b.md#y".into()];

        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), None, async {
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

    /// Without a tenant scope (stdio / self-hosted), the wiring skips the
    /// ledger call. Mirrors the production no-op for those modes.
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

    /// Test-only `SessionStorage` that captures every save.
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

        fn load<'a>(&'a self, tenant_id: &'a str, session_id: &'a str) -> LoadSessionFuture<'a> {
            // Return the most-recently-saved snapshot matching the
            // `(tenant_id, session_id)` PK so round-trip tests can
            // pre-seed via `save`.
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

        fn touch<'a>(&'a self, _tenant_id: &'a str, _session_id: &'a str) -> SessionMutFuture<'a> {
            Box::pin(async { Ok(()) })
        }

        fn delete<'a>(&'a self, _tenant_id: &'a str, _session_id: &'a str) -> SessionMutFuture<'a> {
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

    /// When a tenant is scoped and storage is wired, the snapshot helper
    /// fires one save carrying the live `tokens_used`.
    #[tokio::test]
    async fn emit_session_snapshot_fires_when_tenant_scoped() {
        let stub = Arc::new(StubStorage::default());
        let mut registry = SessionRegistry::new(UsageConfig::default())
            .with_storage(Arc::clone(&stub) as Arc<dyn SessionStorage>);
        let identity = DeliveryIdentity::new("corpus-a", "shared-id", "section_excerpt");
        registry
            .create_session("agent-session-1", None, AccessMode::ReadWrite)
            .session
            .record_delivery_identity(&identity, Resolution::Section, 18, 1, "content-hash".into());
        let status = fixture_status(5_000);

        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), None, async {
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
        assert_eq!(snap.corpus_id.as_deref(), Some("corpus-a"));
        assert_eq!(snap.budget_used, 5_000);
        assert_eq!(snap.state.version, ministr_api::SESSION_STATE_VERSION);
        assert_eq!(snap.state.delivered.len(), 1);
        assert_eq!(snap.state.delivered[0].identity.corpus_id, "corpus-a");
        assert_eq!(snap.state.delivered[0].identity.content_id, "shared-id");
        assert_eq!(
            snap.state.delivered[0].identity.resolution,
            "section_excerpt"
        );
        assert!(!snap.opened_at.is_empty());
        assert_eq!(snap.opened_at, snap.last_seen_at);
    }

    /// Without a tenant scope, the snapshot helper short-circuits before
    /// building a snapshot or touching storage.
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

    /// When no storage backend is wired, the registry's `persist_snapshot`
    /// collapses to a no-op even with a scoped tenant.
    #[tokio::test]
    async fn emit_session_snapshot_is_noop_without_storage() {
        let registry = SessionRegistry::new(UsageConfig::default());
        // No `with_storage` call — registry.storage stays `None`.
        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), None, async {
            emit_session_snapshot(&registry, "agent-session-1", &fixture_status(123));
        })
        .await;
        // No assertion target — the point is the call doesn't panic.
    }

    /// When a tenant is scoped and storage has a matching snapshot,
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
            state: ministr_api::SessionStateSnapshot::default(),
        };
        stub.saves
            .lock()
            .expect("stub storage mutex never poisoned")
            .push(snapshot);

        // Sanity: registry has no in-memory shadow yet.
        assert!(registry.get_session("agent-session-1").is_none());

        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), None, async {
            try_restore_session(&mut registry, "agent-session-1").await;
        })
        .await;

        assert!(
            registry.get_session("agent-session-1").is_some(),
            "shell should be materialised after a storage hit",
        );
    }

    /// Without a tenant scope, `try_restore` is impossible (no PK lookup
    /// key) and the helper short-circuits without touching storage.
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

    /// When the session already exists in-memory, `try_restore`
    /// short-circuits (per its own contract) and the helper is effectively
    /// a no-op.
    #[tokio::test]
    async fn try_restore_session_is_noop_when_already_in_memory() {
        let stub = Arc::new(StubStorage::default());
        let mut registry = SessionRegistry::new(UsageConfig::default())
            .with_storage(Arc::clone(&stub) as Arc<dyn SessionStorage>);

        // Bootstrap the session in-memory before any restore attempt.
        registry.create_session("agent-session-1", None, AccessMode::ReadWrite);

        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), None, async {
            try_restore_session(&mut registry, "agent-session-1").await;
        })
        .await;

        // Stub records no `load` calls in `saves`; the strongest signal is
        // that the session remains a fresh-bootstrapped shell rather than
        // anything snapshot-derived — `try_restore` would have failed to
        // overwrite an existing entry anyway, but we want the no-op shape.
        assert!(registry.get_session("agent-session-1").is_some());
    }

    /// Without a storage backend wired, `try_restore` falls through to
    /// its `None` branch. Helper must not panic and must not leave a
    /// stray entry.
    #[tokio::test]
    async fn try_restore_session_is_noop_when_no_storage() {
        let mut registry = SessionRegistry::new(UsageConfig::default());
        // No `with_storage` — `registry.storage` stays `None`.

        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), None, async {
            try_restore_session(&mut registry, "agent-session-1").await;
        })
        .await;

        assert!(registry.get_session("agent-session-1").is_none());
    }

    /// Empty eviction list is a no-op even when scoped.
    #[tokio::test]
    async fn emit_section_drops_skips_when_no_evictions() {
        let stub = Arc::new(StubLedger::default());
        let registry = SessionRegistry::new(UsageConfig::default())
            .with_drops_ledger(Arc::clone(&stub) as Arc<dyn DropsLedger>);

        crate::tenant_scope::scope_for_test(Some("tenant-x".into()), None, async {
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
            stale_identities: vec![],
        }];
        let actions = build_next_actions(&alerts, Vec::new());

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action, "ministr_read");
        assert_eq!(actions[0].args["section_id"], "docs/a.md#x");
        assert_eq!(actions[1].args["section_id"], "docs/b.md#y");
    }

    #[test]
    fn coherence_actions_preserve_corpus_route_from_exact_identity() {
        let alerts = vec![CoherenceAlert {
            changed_sections: vec!["docs/a.md#x".into()],
            stale_content_ids: vec!["docs/a.md#x".into()],
            stale_identities: vec![ministr_core::types::DeliveryIdentity::new(
                "linked-corpus",
                "docs/a.md#x",
                "section_full",
            )],
        }];
        let actions = build_next_actions(&alerts, Vec::new());
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].args["section_id"], "docs/a.md#x");
        assert_eq!(actions[0].args["project"], "linked-corpus");
        assert_eq!(actions[0].args["source_corpus"], "linked-corpus");
    }

    #[test]
    fn coherence_actions_are_resolution_aware_and_executable() {
        let alerts = vec![CoherenceAlert {
            changed_sections: vec!["claim-1".into(), "symbol-1".into()],
            stale_content_ids: vec!["claim-1".into(), "symbol-1".into()],
            stale_identities: vec![
                ministr_core::types::DeliveryIdentity::new(
                    "linked-corpus",
                    "docs/a.md#x:c0",
                    "claim_excerpt",
                ),
                ministr_core::types::DeliveryIdentity::new(
                    "linked-corpus",
                    "sym-src/a.rs::run",
                    "symbol_full",
                ),
            ],
        }];
        let actions = build_next_actions(&alerts, Vec::new());

        assert_eq!(actions[0].action, "ministr_read");
        assert_eq!(actions[0].args["section_id"], "docs/a.md#x");
        assert_eq!(actions[0].args["project"], "linked-corpus");
        assert_eq!(actions[1].action, "ministr_definition");
        assert_eq!(actions[1].args["symbol_id"], "sym-src/a.rs::run");
        assert_eq!(actions[1].args["source_corpus"], "linked-corpus");
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
            stale_identities: vec![],
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
            stale_identities: vec![],
        }];
        let actions = build_next_actions(&alerts, extras);

        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action, "ministr_read");
        assert_eq!(actions[1].action, "ministr_definition");
    }
}
