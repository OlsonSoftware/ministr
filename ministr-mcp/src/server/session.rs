//! Session state manipulation helpers for the ministr server.
//!
//! These `impl MinistrServer` methods handle recording deliveries in the session
//! shadow, building tool responses with budget and coherence metadata, and
//! background compression of evicted entries.

use serde::Serialize;

use ministr_core::session::{
    AccessMode, BudgetStatus, CompressionTier, PressureLevel, SessionEntry, SessionId,
    SessionRegistry,
};
use ministr_core::token::count_tokens;
use ministr_core::types::{ContentId, Resolution};

use super::MinistrServer;
use super::types::ToolResponse;

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
    /// On *first* resolution (i.e. the entry was just created), stamps
    /// the captured `parent_session_id` and `client_name` hints onto
    /// the entry so the tray / `SessionDashboard` can render lineage.
    /// On subsequent calls the stamps are skipped (the entry already
    /// carries them).
    pub(super) fn ensure_session_mut<'a>(
        &self,
        reg: &'a mut SessionRegistry,
    ) -> &'a mut SessionEntry {
        let was_missing = !reg.contains(&self.active_session_id);
        let entry = reg.get_or_create(&self.active_session_id, None, AccessMode::ReadWrite);
        if was_missing {
            if let Some(parent) = self.parent_session_id_hint.as_deref() {
                entry.parent_session_id = Some(SessionId::from(parent.to_string()));
            }
            if let Some(name) = self
                .client_name_hint
                .try_lock()
                .ok()
                .and_then(|g| g.clone())
            {
                entry.client_name = Some(name);
            }
        }
        entry
    }
}

impl MinistrServer {
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
    ) -> BudgetStatus {
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

        let status = entry.budget.budget_status();
        drop(reg);

        // Phase 1: bookmark compression for evicted entries.
        if !evicted_ids.is_empty() {
            let mut heading_paths = Vec::with_capacity(evicted_ids.len());
            for evicted_id in &evicted_ids {
                heading_paths.push(self.service.section_heading_path(evicted_id).await);
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

        // Phase 2: background extractive compression to upgrade bookmarks.
        if !evicted_ids.is_empty() {
            let service = self.service.clone();
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
    /// having to call `ministr_budget` explicitly.
    pub(super) async fn build_response<T: Serialize + rmcp::schemars::JsonSchema>(
        &self,
        data: T,
        budget_status: BudgetStatus,
    ) -> ToolResponse<T> {
        let mut reg = self.registry.lock().await;
        let entry = self.ensure_session_mut(&mut reg);
        let alerts = entry.session.drain_alerts();

        // Compute eviction recommendations when under pressure
        let eviction_recommendations = if budget_status.pressure_level == PressureLevel::Normal {
            Vec::new()
        } else {
            entry
                .budget
                .eviction_candidates(&entry.session, 3, Some(&entry.memory))
        };
        drop(reg);

        let progress = &self.ingestion_progress;
        let indexing = progress.is_running();
        let indexing_message = if indexing {
            let done = progress.files_done();
            let total = progress.files_total();
            Some(format!("Checking {done}/{total} files"))
        } else {
            None
        };

        ToolResponse {
            budget_status,
            coherence_alerts: alerts,
            indexing_in_progress: indexing,
            indexing_message,
            eviction_recommendations,
            result: data,
        }
    }
}
