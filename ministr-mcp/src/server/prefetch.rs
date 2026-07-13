//! Prefetch, analytics, and session persistence helpers for the ministr server.
//!
//! These `impl MinistrServer` methods run after read operations to proactively
//! warm the prefetch cache and record access patterns for cross-session
//! analytics.

use tracing::warn;

use ministr_core::analytics::Analytics;
use ministr_core::service::DefinitionOptions;
use ministr_core::session::prefetch::PrefetchStrategy;
use ministr_core::storage::Storage;
use ministr_core::types::{PRIMARY_CORPUS_ID, SectionId, VectorId};

use super::MinistrServer;

impl MinistrServer {
    /// Warm likely follow-up reads through the routed backend. This covers
    /// daemon-forwarded, linked-project, and cross-corpus surveys without
    /// requiring direct access to their storage.
    pub(super) async fn warm_survey_sections(
        &self,
        tenant_subject: Option<&str>,
        results: &[ministr_core::service::SurveyResult],
    ) {
        let mut candidates = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for result in results {
            let locator = result
                .text_metadata
                .continuation
                .as_ref()
                .unwrap_or(&result.locator);
            if !locator.identity.resolution.starts_with("section")
                || !seen.insert(locator.identity.clone())
            {
                continue;
            }
            candidates.push((
                locator.identity.clone(),
                locator
                    .project
                    .clone()
                    .or_else(|| locator.source_corpus.clone()),
            ));
            if candidates.len() >= super::helpers::MAX_INTENT_PREFETCH_SURVEY {
                break;
            }
        }

        for (identity, project) in candidates {
            let key = identity.storage_key();
            let cache_warm = self.prefetch.lock().await.cache().peek(&key).is_some();
            let metadata_warm = self
                .prefetch_section_metadata
                .lock()
                .await
                .contains_key(&key);
            if cache_warm && metadata_warm {
                continue;
            }
            let Ok(response) = self
                .backend
                .read_section(tenant_subject, project.as_deref(), &identity.content_id)
                .await
            else {
                continue;
            };
            let metadata = response.metadata;
            let detail = response.data;
            self.prefetch.lock().await.prefetch_section_detail(
                &identity.corpus_id,
                detail,
                PrefetchStrategy::AgentPlan,
            );
            self.prefetch_section_metadata
                .lock()
                .await
                .insert(key, metadata);
        }
    }

    /// Warm the normal bounded definition after a navigation result predicts
    /// that it will be requested next. This deliberately runs through the
    /// selected backend, so local, linked-project, and daemon-forward modes
    /// populate the same proxy cache under the true corpus identity.
    pub(super) async fn warm_symbol_definition(
        &self,
        tenant_subject: Option<&str>,
        project: Option<&str>,
        symbol_id: &str,
        strategy: PrefetchStrategy,
    ) {
        let Ok(corpus_id) = self.backend.routed_corpus_id(project) else {
            return;
        };
        if self
            .prefetch
            .lock()
            .await
            .has_definition(&corpus_id, symbol_id)
        {
            return;
        }
        let Ok(response) = self
            .backend
            .definition(
                tenant_subject,
                project,
                symbol_id,
                DefinitionOptions::default(),
            )
            .await
        else {
            return;
        };
        let metadata = response.metadata;
        let mut definition = response.data;
        definition.locator.identity.corpus_id.clone_from(&corpus_id);
        definition.locator.project = project.map(str::to_owned);
        self.prefetch
            .lock()
            .await
            .prefetch_definition(&corpus_id, definition, strategy);
        let key = ministr_core::types::DeliveryIdentity::new(
            &corpus_id,
            symbol_id,
            "symbol_definition_default",
        )
        .storage_key();
        self.prefetch_definition_metadata
            .lock()
            .await
            .insert(key, metadata);
    }

    /// Consume a normal bounded definition from the route-aware warm cache.
    pub(super) async fn try_prefetched_definition(
        &self,
        project: Option<&str>,
        symbol_id: &str,
        options: DefinitionOptions,
    ) -> Option<crate::backend::BackendResponse<ministr_core::service::SymbolDefinition>> {
        if options != DefinitionOptions::default() {
            return None;
        }
        let corpus_id = self.backend.routed_corpus_id(project).ok()?;
        let definition = self
            .prefetch
            .lock()
            .await
            .try_serve_definition(&corpus_id, symbol_id)?;
        let key = ministr_core::types::DeliveryIdentity::new(
            &corpus_id,
            symbol_id,
            "symbol_definition_default",
        )
        .storage_key();
        let metadata = self
            .prefetch_definition_metadata
            .lock()
            .await
            .get(&key)
            .cloned()
            .unwrap_or_default();
        Some(crate::backend::BackendResponse {
            data: definition,
            metadata,
        })
    }

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
            prefetch.prefetch_sequential_for(PRIMARY_CORPUS_ID, next_section, claims_count);

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

                    prefetch.prefetch_sections_for(
                        PRIMARY_CORPUS_ID,
                        PrefetchStrategy::Structural,
                        siblings,
                        &claims_counts,
                        3,
                    );
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

                    prefetch.prefetch_sections_for(
                        PRIMARY_CORPUS_ID,
                        PrefetchStrategy::Topical,
                        candidates,
                        &claims_counts,
                        usize::MAX,
                    );
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
                        let identity = ministr_core::types::DeliveryIdentity::new(
                            PRIMARY_CORPUS_ID,
                            co.section_id.0.clone(),
                            "section_full",
                        );
                        if prefetch.cache().peek(&identity.storage_key()).is_some() {
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
                        prefetch.prefetch_sections_for(
                            PRIMARY_CORPUS_ID,
                            PrefetchStrategy::CrossSession,
                            candidates,
                            &claims_counts,
                            usize::MAX,
                        );
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
                let fresh_ids: Vec<SectionId> = new_items
                    .iter()
                    .map(|identity| SectionId(identity.content_id.clone()))
                    .collect();
                let known_ids: Vec<SectionId> = already_flushed
                    .iter()
                    .map(|identity| SectionId(identity.content_id.clone()))
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
