//! Session state manipulation helpers for the ministr server.
//!
//! These `impl MinistrServer` methods handle recording deliveries in the session
//! shadow, building tool responses with budget and coherence metadata, and
//! background compression of evicted entries.

use serde::Serialize;

use ministr_core::session::{
    AccessMode, CompressionTier, SessionEntry, SessionId, SessionRegistry, UsageStatus,
};
use ministr_core::token::count_tokens;
use ministr_core::types::{ContentId, Resolution};

use super::MinistrServer;
use super::types::{NextAction, ToolResponse};

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
    use ministr_api::{AppendDropFuture, DropEntry, DropsLedger, DropsLedgerError, ListDropsFuture};
    use ministr_core::session::{CoherenceAlert, UsageConfig};
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
