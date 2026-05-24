//! Prefetch, analytics, and session persistence helpers for the ministr server.
//!
//! These `impl MinistrServer` methods run after read operations to proactively
//! warm the prefetch cache and record access patterns for cross-session
//! analytics.

use tracing::warn;

use ministr_core::analytics::Analytics;
use ministr_core::storage::Storage;
use ministr_core::types::{SectionId, VectorId};

use super::MinistrServer;

impl MinistrServer {
    /// Trigger all prefetch strategies after a read operation.
    ///
    /// Runs four strategies in sequence:
    /// 1. **Sequential** — next section + parent document summary
    /// 2. **Structural** — sibling sections from the same document
    /// 3. **Topical** — sections nearest to the running topic vector
    /// 4. **Cross-session** — frequently co-accessed sections from analytics
    #[allow(clippy::too_many_lines)]
    pub(super) async fn trigger_prefetch(&self, section_id: &str) {
        if let Some(ref storage) = self.storage {
            let sid = SectionId(section_id.to_string());

            // --- Sequential prefetch ---
            let next_section = storage.get_next_section(&sid).await.unwrap_or(None);

            let claims_count = if let Some(ref next) = next_section {
                storage.list_claims(&next.id).await.map(|c| c.len()).ok()
            } else {
                None
            };

            let doc_record = storage.get_document_for_section(&sid).await.ok().flatten();

            let mut prefetch = self.prefetch.lock().await;
            prefetch.advance_turn();
            prefetch.prefetch_sequential(next_section, claims_count);

            // --- Structural prefetch (sibling sections) ---
            if let Some(ref doc) = doc_record
                && let Ok(all_sections) = storage.list_sections(&doc.id).await
            {
                let current_pos = all_sections.iter().position(|s| s.id.0 == section_id);
                if let Some(pos) = current_pos {
                    let start = pos.saturating_sub(2);
                    let end = (pos + 3).min(all_sections.len());
                    let siblings: Vec<_> = all_sections[start..end]
                        .iter()
                        .filter(|s| s.id.0 != section_id)
                        .cloned()
                        .collect();

                    let mut claims_counts = std::collections::HashMap::new();
                    for s in &siblings {
                        if let Ok(claims) = storage.list_claims(&s.id).await {
                            claims_counts.insert(s.id.0.clone(), claims.len());
                        }
                    }

                    prefetch.prefetch_structural(siblings, &claims_counts);
                }
            }

            // --- Topical prefetch (similarity to running topic) ---
            // Embedder + vector index are local-only. In daemon-forward mode
            // (`self.service` is `None`) topical prefetch is skipped — the
            // daemon already maintains its own prefetch state server-side.
            let Some(ref service) = self.service else {
                return;
            };
            if let Ok(Some(section)) = storage.get_section(&sid).await {
                if let Ok(embeddings) = service.embedder().embed(&[&section.text])
                    && let Some(embedding) = embeddings.into_iter().next()
                {
                    prefetch.record_topic_access(embedding);
                }

                if let Some(topic_vec) = prefetch.topic_vector()
                    && let Ok(results) = service.index().search_knn(&topic_vec, 5)
                {
                    let mut candidates = Vec::new();
                    for result in results {
                        let vid = VectorId::parse(&result.id);
                        if let Some(vid) = vid
                            && vid.resolution() == ministr_core::types::Resolution::Section
                        {
                            let cid = vid.content_id();
                            if cid == section_id {
                                continue;
                            }
                            let candidate_sid = SectionId(cid.to_string());
                            if let Ok(Some(s)) = storage.get_section(&candidate_sid).await {
                                candidates.push(s);
                            }
                        }
                    }

                    let mut claims_counts = std::collections::HashMap::new();
                    for s in &candidates {
                        if let Ok(claims) = storage.list_claims(&s.id).await {
                            claims_counts.insert(s.id.0.clone(), claims.len());
                        }
                    }

                    prefetch.prefetch_topical(candidates, &claims_counts);
                }
            }

            // --- Cross-session prefetch (frequently co-accessed sections) ---
            if let Some(ref analytics) = self.analytics {
                let sid_ref = SectionId(section_id.to_string());
                if let Ok(co_accessed) = analytics
                    .co_accessed_with(&sid_ref, Analytics::default_co_access_limit())
                    .await
                {
                    let mut candidates = Vec::new();
                    for co in co_accessed {
                        if prefetch.cache().peek(&co.section_id.0).is_some() {
                            continue;
                        }
                        if let Ok(Some(s)) = storage.get_section(&co.section_id).await {
                            candidates.push(s);
                        }
                    }

                    if !candidates.is_empty() {
                        let mut claims_counts = std::collections::HashMap::new();
                        for s in &candidates {
                            if let Ok(claims) = storage.list_claims(&s.id).await {
                                claims_counts.insert(s.id.0.clone(), claims.len());
                            }
                        }
                        prefetch.prefetch_cross_session(candidates, &claims_counts);
                    }
                }
            }
        }
    }

    /// Record a section access in cross-session analytics.
    pub(super) async fn record_analytics_access(&self, section_id: &str) {
        if let Some(ref analytics) = self.analytics {
            let sid = SectionId(section_id.to_string());
            if let Err(e) = analytics.record_access(&sid).await {
                warn!(error = %e, "failed to record analytics access");
            }
        }
    }

    /// Persist the current session state to storage, if persistence is enabled.
    ///
    /// Also incrementally flushes co-access patterns: only pairs that
    /// involve sections newly added to the trajectory since the last
    /// flush are recorded. This prevents the O(N³) inflation that
    /// would happen if the entire trajectory were re-recorded on every
    /// tool call.
    pub(super) async fn persist_session(&self) {
        if let Some(ref storage) = self.storage {
            let mut reg = self.registry.lock().await;
            let Some(entry) = reg.get_session_mut(&self.effective_session_id()) else {
                return;
            };
            if let Err(e) = storage.save_session(&entry.session).await {
                warn!(error = %e, "failed to persist session");
            }

            // Incremental co-access flush.
            if let Some(ref analytics) = self.analytics {
                let (new_items, already_flushed) = entry.session.unflushed_co_access_items();
                let fresh_ids: Vec<SectionId> =
                    new_items.iter().map(|c| SectionId(c.0.clone())).collect();
                let known_ids: Vec<SectionId> = already_flushed
                    .iter()
                    .map(|c| SectionId(c.0.clone()))
                    .collect();
                // Mark BEFORE drop so the session state is updated
                // atomically with the flush decision.
                entry.session.mark_co_access_flushed(new_items);
                drop(reg);
                if !fresh_ids.is_empty()
                    && let Err(e) = analytics
                        .record_co_access_incremental(&fresh_ids, &known_ids)
                        .await
                {
                    warn!(error = %e, "failed to record co-access patterns");
                }
            }
        }
    }
}
