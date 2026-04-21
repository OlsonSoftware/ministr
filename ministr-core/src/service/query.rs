//! Search, read, and extraction query operations for [`QueryService`].
//!
//! This module contains the core query methods: survey (search), read,
//! extract claims, related claims, and the private helpers for reranking
//! and content resolution.

use std::collections::HashSet;

use tracing::instrument;

use std::collections::HashMap;

use crate::embedding::{DualEmbedder, Reranker};
use crate::search::{MultiResolutionSearch, ScoredResult, SearchConfig};
use crate::storage::Storage;
use crate::types::{ClaimId, ContentId, Resolution, SectionId, SymbolId, VectorId};

use super::{
    ClaimResult, QueryError, QueryService, RelatedClaimResult, SectionDetail, SurveyResult,
    cosine_similarity, is_unresolved_placeholder,
};

/// How strongly Matryoshka full-dim rescoring overrides the prior (RRF-fused
/// dense + sparse + resolution-weighted) score. `0.7` = the new signal
/// dominates but the prior stays in the mix so sparse/lexical contributions
/// aren't erased.
const MATRYOSHKA_BLEND: f32 = 0.7;

/// How strongly cross-encoder reranking overrides the prior composed score.
/// `0.8` — the reranker is our best signal, but we keep some memory of the
/// upstream retrieval stack.
const RERANK_BLEND: f32 = 0.8;

/// Min-max normalize a slice of scores in-place into `[0, 1]`. If every
/// score is identical the range collapses and every entry is set to `0.5`
/// so downstream blends still compose meaningfully.
fn min_max_normalize(scores: &mut [f32]) {
    if scores.is_empty() {
        return;
    }
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for &s in scores.iter() {
        if s < min {
            min = s;
        }
        if s > max {
            max = s;
        }
    }
    let range = max - min;
    if range < f32::EPSILON {
        for s in scores.iter_mut() {
            *s = 0.5;
        }
    } else {
        for s in scores.iter_mut() {
            *s = (*s - min) / range;
        }
    }
}

impl QueryService {
    /// Search the corpus for content relevant to a natural language query.
    ///
    /// Performs multi-resolution vector search and enriches results with
    /// content from storage.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError`] if embedding, search, or storage operations fail.
    #[instrument(skip(self), fields(query_len = query.len(), top_k))]
    pub async fn survey(&self, query: &str, top_k: usize) -> Result<Vec<SurveyResult>, QueryError> {
        let mut searcher = MultiResolutionSearch::new(self.embedder.as_ref(), self.index.as_ref());
        if let (Some(se), Some(si)) = (&self.sparse_embedder, &self.sparse_index) {
            searcher = searcher.with_sparse(se.as_ref(), si.as_ref());
        }
        let sparse_weight = if self.sparse_embedder.is_some() {
            0.3
        } else {
            0.0
        };
        // When reranking is enabled, fetch more candidates so the reranker
        // has a larger pool to re-score before truncation.
        let rerank_top_k = self.reranker.as_ref().map(|_| top_k.max(10) * 3);
        let search_top_k = rerank_top_k.unwrap_or(top_k);
        let config = SearchConfig {
            raw_k: search_top_k.max(10) * 3,
            top_k: search_top_k,
            sparse_weight,
            rerank_top_k,
        };

        let scored = searcher.search(query, config)?;

        // Two-stage Matryoshka rescore: use full-dim vectors to re-rank the
        // coarse truncated-dim results from HNSW.
        let scored = if let Some(dual_emb) = &self.dual_embedder {
            self.rescore_with_full_dim(query, scored, dual_emb.as_ref())
                .await?
        } else {
            scored
        };

        let mut results = Vec::with_capacity(scored.len());
        for sr in scored {
            let content_id = sr.vector_id.content_id().to_string();
            let resolution = sr.resolution;

            let (text, heading_path) = self
                .resolve_content(&sr.vector_id, resolution)
                .await
                .unwrap_or_else(|_| (format!("[content unavailable: {content_id}]"), None));

            // Skip unresolved placeholders (e.g. during indexing)
            if is_unresolved_placeholder(&text) {
                continue;
            }

            results.push(SurveyResult {
                content_id,
                resolution: resolution.to_string(),
                score: sr.score,
                text,
                heading_path,
            });
        }

        // Apply cross-encoder reranking if configured
        if let Some(reranker) = &self.reranker {
            results = Self::rerank_results(query, results, top_k, reranker.as_ref())?;
        }

        Ok(results)
    }

    /// Like [`survey`], but filters out results whose content ID is in
    /// `exclude_ids` before truncating to `top_k`.
    ///
    /// This ensures the 3x over-fetch buffer compensates for already-delivered
    /// content rather than being wasted by premature truncation.
    ///
    /// Returns `(results, deduplicated_count)` where `deduplicated_count` is
    /// the number of candidates that were skipped due to exclusion.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError`] if embedding, search, or storage operations fail.
    #[instrument(skip(self, exclude_ids), fields(query_len = query.len(), top_k, exclude_count = exclude_ids.len()))]
    pub async fn survey_excluding(
        &self,
        query: &str,
        top_k: usize,
        exclude_ids: &HashSet<String>,
    ) -> Result<(Vec<SurveyResult>, usize), QueryError> {
        let mut searcher = MultiResolutionSearch::new(self.embedder.as_ref(), self.index.as_ref());
        if let (Some(se), Some(si)) = (&self.sparse_embedder, &self.sparse_index) {
            searcher = searcher.with_sparse(se.as_ref(), si.as_ref());
        }
        let sparse_weight = if self.sparse_embedder.is_some() {
            0.3
        } else {
            0.0
        };
        // Fetch the full raw_k candidates without truncation so we can
        // filter out excluded IDs before selecting the final top_k.
        let fetch_k = top_k.max(10) * 3;
        let config = SearchConfig {
            raw_k: fetch_k,
            top_k: fetch_k,
            sparse_weight,
            rerank_top_k: None,
        };

        let scored = searcher.search(query, config)?;

        // Two-stage Matryoshka rescore (same as in survey).
        let scored = if let Some(dual_emb) = &self.dual_embedder {
            self.rescore_with_full_dim(query, scored, dual_emb.as_ref())
                .await?
        } else {
            scored
        };

        let mut results = Vec::new();
        let mut deduplicated_count = 0;

        // When reranking, collect more candidates so the reranker has a
        // larger pool; otherwise stop at top_k.
        let collect_k = if self.reranker.is_some() {
            fetch_k
        } else {
            top_k
        };

        for sr in scored {
            let content_id = sr.vector_id.content_id().to_string();

            if exclude_ids.contains(&content_id) {
                deduplicated_count += 1;
                continue;
            }

            let resolution = sr.resolution;
            let (text, heading_path) = self
                .resolve_content(&sr.vector_id, resolution)
                .await
                .unwrap_or_else(|_| (format!("[content unavailable: {content_id}]"), None));

            // Skip unresolved placeholders (e.g. during indexing)
            if is_unresolved_placeholder(&text) {
                continue;
            }

            results.push(SurveyResult {
                content_id,
                resolution: resolution.to_string(),
                score: sr.score,
                text,
                heading_path,
            });

            if results.len() >= collect_k {
                break;
            }
        }

        // Apply cross-encoder reranking if configured
        if let Some(reranker) = &self.reranker {
            results = Self::rerank_results(query, results, top_k, reranker.as_ref())?;
        } else {
            results.truncate(top_k);
        }

        Ok((results, deduplicated_count))
    }

    /// Read the full text of a section by its hierarchical ID.
    ///
    /// Returns the section content with heading path and the count of
    /// claims available for extraction.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::SectionNotFound`] if no section exists with the
    /// given ID, or [`QueryError::Storage`] on database errors.
    #[instrument(skip(self))]
    pub async fn read_section(&self, section_id: &str) -> Result<SectionDetail, QueryError> {
        let sid = SectionId(section_id.to_string());

        let section =
            self.storage
                .get_section(&sid)
                .await?
                .ok_or_else(|| QueryError::SectionNotFound {
                    id: section_id.to_string(),
                })?;

        let claims = self.storage.list_claims(&sid).await?;

        Ok(SectionDetail {
            section_id: section_id.to_string(),
            heading_path: section.heading_path,
            text: section.text,
            summary: section.summary,
            claims_available: claims.len(),
        })
    }

    /// Look up the heading path for a section, returning an empty vec if not found.
    ///
    /// Used by the eviction cascade to generate meaningful bookmark text
    /// without loading the full section content.
    pub async fn section_heading_path(&self, section_id: &str) -> Vec<String> {
        let sid = SectionId(section_id.to_string());
        self.storage
            .get_section(&sid)
            .await
            .ok()
            .flatten()
            .map_or_else(Vec::new, |s| s.heading_path)
    }

    /// Extract atomic claims from a section, optionally filtered by query relevance.
    ///
    /// When a query is provided, claims are scored by cosine similarity to the
    /// query embedding and returned in descending relevance order. Without a
    /// query, all claims are returned in document order.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::SectionNotFound`] if the section does not exist,
    /// or [`QueryError`] on embedding/storage failures.
    #[instrument(skip(self))]
    pub async fn extract_claims(
        &self,
        section_id: &str,
        query: Option<&str>,
    ) -> Result<Vec<ClaimResult>, QueryError> {
        let sid = SectionId(section_id.to_string());

        // Try section lookup first
        let section_exists = self.storage.get_section(&sid).await?.is_some();

        let claims = if section_exists {
            self.storage.list_claims(&sid).await?
        } else if section_id.starts_with("sym-") {
            // Fall back to generating claims from symbol doc comments
            self.extract_symbol_claims(section_id).await?
        } else {
            return Err(QueryError::SectionNotFound {
                id: section_id.to_string(),
            });
        };

        if claims.is_empty() {
            return Ok(Vec::new());
        }

        match query {
            Some(q) if !q.is_empty() => {
                // Embed query and all claim texts, compute cosine similarity
                let claim_texts: Vec<&str> = claims.iter().map(|c| c.text.as_str()).collect();
                let mut all_texts = vec![q];
                all_texts.extend(claim_texts.iter());

                let embeddings = self.embedder.embed(&all_texts)?;
                let query_vec = &embeddings[0];

                let mut scored: Vec<ClaimResult> = claims
                    .iter()
                    .enumerate()
                    .map(|(i, claim)| {
                        let claim_vec = &embeddings[i + 1];
                        let similarity = cosine_similarity(query_vec, claim_vec);
                        ClaimResult {
                            claim_id: claim.id.to_string(),
                            text: claim.text.clone(),
                            relevance: Some(similarity),
                        }
                    })
                    .collect();

                // Sort by relevance descending
                scored.sort_by(|a, b| {
                    b.relevance
                        .unwrap_or(0.0)
                        .partial_cmp(&a.relevance.unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });

                Ok(scored)
            }
            _ => {
                // No query — return all claims in document order
                Ok(claims
                    .into_iter()
                    .map(|c| ClaimResult {
                        claim_id: c.id.to_string(),
                        text: c.text,
                        relevance: None,
                    })
                    .collect())
            }
        }
    }

    /// Find claims related to the given claim via the relationship index.
    ///
    /// Returns claims that reference, contradict, depend on, or update the
    /// given claim. Optionally filtered by relation type.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::ClaimNotFound`] if the claim does not exist,
    /// or [`QueryError::Storage`] on database errors.
    #[instrument(skip(self))]
    pub async fn related_claims(
        &self,
        claim_id: &str,
        relation_types: Option<&[crate::types::RelationType]>,
    ) -> Result<Vec<RelatedClaimResult>, QueryError> {
        let cid = ClaimId(claim_id.to_string());

        // Verify claim exists
        self.storage
            .get_claim(&cid)
            .await?
            .ok_or_else(|| QueryError::ClaimNotFound {
                id: claim_id.to_string(),
            })?;

        let related = self
            .storage
            .get_related_claims(&cid, relation_types)
            .await?;

        Ok(related
            .into_iter()
            .map(|r| RelatedClaimResult {
                claim_id: r.claim_id.0,
                text: r.text,
                relation_type: r.relation_type.to_string(),
                source_section: r.section_id.0,
                confidence: r.confidence,
            })
            .collect())
    }

    /// Rerank survey results using a cross-encoder model, composed with the
    /// prior composed score from upstream retrieval.
    ///
    /// 1. Ask the reranker for cross-encoder scores over `(query, text)` pairs.
    /// 2. **Blend** each result's new rerank score with its normalized prior
    ///    score using [`RERANK_BLEND`]. Both signals are min-max normalized
    ///    across the candidate set first so the blend is scale-aware.
    /// 3. Re-sort by the composed score and truncate to `top_k`.
    ///
    /// Preserving the prior keeps upstream RRF + Matryoshka contributions in
    /// the final ranking instead of letting the cross-encoder fully overwrite
    /// them.
    pub(super) fn rerank_results(
        query: &str,
        results: Vec<SurveyResult>,
        top_k: usize,
        model: &dyn Reranker,
    ) -> Result<Vec<SurveyResult>, QueryError> {
        if results.is_empty() {
            return Ok(results);
        }

        // Snapshot and normalize priors across the result set.
        let mut priors: Vec<f32> = results.iter().map(|r| r.score).collect();
        min_max_normalize(&mut priors);

        // Compute reranker scores (index-aligned to `results` input order).
        let texts: Vec<&str> = results.iter().map(|r| r.text.as_str()).collect();
        let scores = model.rerank(query, &texts)?;

        // Build an index-aligned rerank score vector (None for any result the
        // reranker didn't return a score for, which shouldn't happen but we
        // handle it defensively).
        let mut rerank_by_index: Vec<Option<f32>> = vec![None; results.len()];
        for rs in &scores {
            if let Some(slot) = rerank_by_index.get_mut(rs.index) {
                *slot = Some(rs.score);
            }
        }

        // Normalize the rerank scores across the subset that has them.
        let mut rerank_values: Vec<f32> = rerank_by_index.iter().filter_map(|&s| s).collect();
        min_max_normalize(&mut rerank_values);
        let mut rerank_iter = rerank_values.into_iter();
        let rerank_norm: Vec<Option<f32>> = rerank_by_index
            .iter()
            .map(|s| s.map(|_| rerank_iter.next().unwrap_or(0.5)))
            .collect();

        // Compose: blend rerank + prior into a single composed score per result.
        let mut composed: Vec<SurveyResult> = results
            .into_iter()
            .enumerate()
            .map(|(i, mut r)| {
                r.score = match rerank_norm[i] {
                    Some(rs) => RERANK_BLEND * rs + (1.0 - RERANK_BLEND) * priors[i],
                    None => priors[i],
                };
                r
            })
            .collect();

        // Sort descending and truncate.
        composed.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        composed.truncate(top_k);
        Ok(composed)
    }

    /// Resolve a vector ID to its content text and optional heading path.
    pub(super) async fn resolve_content(
        &self,
        vector_id: &VectorId,
        resolution: Resolution,
    ) -> Result<(String, Option<Vec<String>>), QueryError> {
        let content_id = vector_id.content_id();

        match resolution {
            Resolution::Summary => {
                if vector_id.is_doc_summary() {
                    // Document summary — look up document record
                    let doc_id = ContentId(content_id.to_string());
                    if let Some(doc) = self.storage.get_document(&doc_id).await? {
                        let text = doc
                            .summary
                            .unwrap_or_else(|| format!("[no summary for document: {}]", doc.title));
                        Ok((text, None))
                    } else {
                        Ok((format!("[document not found: {content_id}]"), None))
                    }
                } else {
                    // Section summary — look up section record
                    let sid = SectionId(content_id.to_string());
                    if let Some(section) = self.storage.get_section(&sid).await? {
                        let text = section
                            .summary
                            .unwrap_or_else(|| format!("[no summary for section: {content_id}]"));
                        Ok((text, Some(section.heading_path)))
                    } else {
                        Ok((format!("[section not found: {content_id}]"), None))
                    }
                }
            }
            Resolution::Section => {
                let sid = SectionId(content_id.to_string());
                if let Some(section) = self.storage.get_section(&sid).await? {
                    Ok((section.text, Some(section.heading_path)))
                } else {
                    Ok((format!("[section not found: {content_id}]"), None))
                }
            }
            Resolution::Claim => {
                let cid = ClaimId(content_id.to_string());
                if let Some(claim) = self.storage.get_claim(&cid).await? {
                    Ok((claim.text, None))
                } else {
                    Ok((format!("[claim not found: {content_id}]"), None))
                }
            }
            Resolution::SymbolStub => {
                let sid = SymbolId(content_id.to_string());
                if let Some(sym) = self.storage.get_symbol(&sid).await? {
                    let text = match &sym.doc_comment {
                        Some(doc) => format!("{}\n{doc}", sym.signature),
                        None => sym.signature.clone(),
                    };
                    let heading = vec![sym.file_path.clone(), format!("{} {}", sym.kind, sym.name)];
                    Ok((text, Some(heading)))
                } else {
                    Ok((format!("[symbol not found: {content_id}]"), None))
                }
            }
            Resolution::SymbolFull => {
                let sid = SymbolId(content_id.to_string());
                if let Some(sym) = self.storage.get_symbol(&sid).await? {
                    // Read source context around the symbol
                    let text = format!(
                        "// {}:{}-{}\n{}",
                        sym.file_path, sym.line_start, sym.line_end, sym.signature
                    );
                    let heading = vec![sym.file_path.clone(), format!("{} {}", sym.kind, sym.name)];
                    Ok((text, Some(heading)))
                } else {
                    Ok((format!("[symbol not found: {content_id}]"), None))
                }
            }
        }
    }

    /// Rescore coarse HNSW candidates using full-dimension cosine similarity,
    /// composed with the prior RRF-fused score.
    ///
    /// 1. Embed the query at full dimension using the dual embedder.
    /// 2. Retrieve stored full-dim vectors from SQLite for the top candidates.
    /// 3. Compute cosine similarity between full-dim query and each candidate.
    /// 4. **Blend** the new cosine with the prior score — both normalized to
    ///    `[0, 1]` across the candidate set — using [`MATRYOSHKA_BLEND`].
    ///    This preserves the sparse/lexical contribution from RRF instead of
    ///    overwriting it with a pure dense signal.
    /// 5. Re-sort by the composed score.
    ///
    /// Candidates without stored full-dim vectors keep their normalized prior
    /// so the whole set stays on one scale after this stage.
    async fn rescore_with_full_dim(
        &self,
        query: &str,
        mut candidates: Vec<ScoredResult>,
        dual_embedder: &dyn DualEmbedder,
    ) -> Result<Vec<ScoredResult>, QueryError> {
        if candidates.is_empty() || self.matryoshka_rerank_depth == 0 {
            return Ok(candidates);
        }

        // Limit to the rerank depth.
        candidates.truncate(self.matryoshka_rerank_depth);

        // Snapshot prior scores and normalize across the candidate set so
        // the blend below combines comparable scales.
        let mut priors: Vec<f32> = candidates.iter().map(|c| c.score).collect();
        min_max_normalize(&mut priors);

        // Get full-dim query vector (single inference).
        let dual = dual_embedder
            .embed_dual(&[query])
            .map_err(QueryError::Index)?;
        let full_query = &dual.full[0];

        // Fetch stored full-dim vectors for all candidate IDs.
        let candidate_ids: Vec<&str> = candidates.iter().map(|c| c.vector_id.as_str()).collect();
        let stored = self
            .storage
            .get_full_dim_vectors(&candidate_ids)
            .await
            .map_err(QueryError::Storage)?;

        // Build a lookup map for fast access.
        let stored_map: HashMap<&str, &[f32]> = stored
            .iter()
            .map(|(id, vec)| (id.as_str(), vec.as_slice()))
            .collect();

        // Compute Matryoshka cosine for each candidate (None when no full-dim
        // vector is available).
        let matryoshka_scores: Vec<Option<f32>> = candidates
            .iter()
            .map(|c| {
                stored_map
                    .get(c.vector_id.as_str())
                    .map(|full_vec| cosine_similarity(full_query, full_vec))
            })
            .collect();

        // Normalize Matryoshka scores across the subset that has them so the
        // blend combines comparable ranges.
        let mut matryoshka_values: Vec<f32> = matryoshka_scores.iter().filter_map(|&s| s).collect();
        min_max_normalize(&mut matryoshka_values);
        let mut matryoshka_iter = matryoshka_values.into_iter();
        let matryoshka_norm: Vec<Option<f32>> = matryoshka_scores
            .iter()
            .map(|s| s.map(|_| matryoshka_iter.next().unwrap_or(0.5)))
            .collect();

        // Compose: if Matryoshka produced a score, blend; otherwise keep the
        // normalized prior so every entry ends up on the [0, 1] scale.
        for (i, candidate) in candidates.iter_mut().enumerate() {
            candidate.score = match matryoshka_norm[i] {
                Some(m) => MATRYOSHKA_BLEND * m + (1.0 - MATRYOSHKA_BLEND) * priors[i],
                None => priors[i],
            };
        }

        // Re-sort by composed values (descending).
        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(candidates)
    }
}
