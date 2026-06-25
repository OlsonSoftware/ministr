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

/// Decay applied to a 1-hop ref-graph neighbour's inherited score, so an
/// expanded neighbour ranks *below* the hit that pulled it in. The reranker
/// can lift a genuinely relevant neighbour back up; on its own,
/// expansion never displaces a primary hit (RepoGraph / LocAgent pattern).
const GRAPH_EXPAND_DECAY: f32 = 0.5;

/// Graph-augmented retrieval (RepoGraph / LocAgent, 2026 SWE-bench SOTA):
/// expand a result set by walking ministr's existing symbol ref-graph.
///
/// For each primary hit (in rank order) the `neighbour_lookup` returns its
/// 1-hop ref-graph neighbours (callers / callees / implementors) as candidate
/// [`SurveyResult`]s. Any neighbour whose `content_id` is not already present is
/// added with a decayed score (`source.score * GRAPH_EXPAND_DECAY`); existing
/// hits keep their higher score, and a neighbour pulled by an earlier (higher)
/// hit is not lowered by a later one. At most `max_expand` neighbours are added,
/// then the set is re-sorted descending so neighbours slot below their source.
///
/// Pure + storage-agnostic (the lookup is injected) so it is deterministic and
/// testable without a model or the DB. The storage-backed survey wiring is the
/// rq-graph-wire follow-up.
#[allow(dead_code)] // wired into survey() by rq-graph-wire
pub(super) fn graph_expand_results<F>(
    hits: &[SurveyResult],
    max_expand: usize,
    mut neighbour_lookup: F,
) -> Vec<SurveyResult>
where
    F: FnMut(&str) -> Vec<SurveyResult>,
{
    if hits.is_empty() || max_expand == 0 {
        return hits.to_vec();
    }

    let mut by_id: HashMap<String, SurveyResult> = HashMap::with_capacity(hits.len() + max_expand);
    let mut order: Vec<String> = Vec::with_capacity(hits.len() + max_expand);
    for h in hits {
        if by_id.insert(h.content_id.clone(), h.clone()).is_none() {
            order.push(h.content_id.clone());
        }
    }

    let mut added = 0usize;
    'hits: for h in hits {
        if added >= max_expand {
            break;
        }
        for mut neighbour in neighbour_lookup(&h.content_id) {
            if added >= max_expand {
                break 'hits;
            }
            // Never overwrite a primary hit or an already-expanded neighbour
            // (the first, highest-scoring source to reach it wins).
            if by_id.contains_key(&neighbour.content_id) {
                continue;
            }
            neighbour.score = h.score * GRAPH_EXPAND_DECAY;
            order.push(neighbour.content_id.clone());
            by_id.insert(neighbour.content_id.clone(), neighbour);
            added += 1;
        }
    }

    let mut out: Vec<SurveyResult> = order
        .into_iter()
        .filter_map(|id| by_id.remove(&id))
        .collect();
    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    out
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
        // The per-corpus configured weight (with_sparse); <= 0 or no
        // components attached behaves dense-only.
        let sparse_weight = if self.sparse_embedder.is_some() {
            self.sparse_weight.max(0.0)
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
                source_corpus: None,
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
        // The per-corpus configured weight (with_sparse); <= 0 or no
        // components attached behaves dense-only.
        let sparse_weight = if self.sparse_embedder.is_some() {
            self.sparse_weight.max(0.0)
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
                source_corpus: None,
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

#[cfg(test)]
mod tests {
    use super::{
        GRAPH_EXPAND_DECAY, QueryService, RERANK_BLEND, SurveyResult, graph_expand_results,
    };
    use crate::embedding::{RerankScore, Reranker};
    use crate::error::IndexError;

    /// Cross-encoder that returns a caller-supplied score per input document
    /// (index-aligned). No model, fully deterministic.
    struct FixedReranker {
        scores: Vec<f32>,
    }

    impl Reranker for FixedReranker {
        fn rerank(&self, _query: &str, documents: &[&str]) -> Result<Vec<RerankScore>, IndexError> {
            Ok(documents
                .iter()
                .enumerate()
                .map(|(index, _)| RerankScore {
                    index,
                    score: self.scores[index],
                })
                .collect())
        }
    }

    /// Cross-encoder that only scores the first document (exercises the
    /// defensive "reranker returned fewer scores than results" path).
    struct PartialReranker;

    impl Reranker for PartialReranker {
        fn rerank(
            &self,
            _query: &str,
            _documents: &[&str],
        ) -> Result<Vec<RerankScore>, IndexError> {
            Ok(vec![RerankScore {
                index: 0,
                score: 1.0,
            }])
        }
    }

    fn sr(content_id: &str, score: f32) -> SurveyResult {
        SurveyResult {
            content_id: content_id.to_string(),
            resolution: "section".to_string(),
            score,
            text: format!("text for {content_id}"),
            heading_path: None,
            source_corpus: None,
        }
    }

    #[test]
    fn rerank_promotes_high_cross_encoder_score_over_prior() {
        // Priors say A is best, C is worst. The cross-encoder INVERTS that:
        // C is the most relevant. With RERANK_BLEND=0.8 the cross-encoder
        // dominates, so C should be promoted to #1 and A demoted.
        let results = vec![sr("A", 0.9), sr("B", 0.5), sr("C", 0.1)];
        let reranker = FixedReranker {
            scores: vec![0.0, 0.5, 1.0],
        };

        let out = QueryService::rerank_results("query", results, 3, &reranker).unwrap();

        assert_eq!(out.len(), 3);
        assert_eq!(
            out[0].content_id, "C",
            "cross-encoder winner should rank #1"
        );
        assert_eq!(out[1].content_id, "B");
        assert_eq!(out[2].content_id, "A", "prior winner should be demoted");

        // The blend keeps the prior in the mix: C's composed score is
        // RERANK_BLEND*1.0 + (1-RERANK_BLEND)*0.0, not a bare 1.0.
        let expected_c = RERANK_BLEND * 1.0 + (1.0 - RERANK_BLEND) * 0.0;
        assert!((out[0].score - expected_c).abs() < 1e-6);
    }

    #[test]
    fn rerank_truncates_to_top_k() {
        let results = vec![sr("A", 0.9), sr("B", 0.5), sr("C", 0.1)];
        let reranker = FixedReranker {
            scores: vec![0.0, 0.5, 1.0],
        };

        let out = QueryService::rerank_results("query", results, 2, &reranker).unwrap();

        assert_eq!(out.len(), 2, "should truncate to top_k after reranking");
        assert_eq!(out[0].content_id, "C");
        assert_eq!(out[1].content_id, "B");
    }

    #[test]
    fn rerank_empty_results_is_noop() {
        let reranker = FixedReranker { scores: vec![] };
        let out = QueryService::rerank_results("query", Vec::new(), 5, &reranker).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn rerank_handles_missing_scores_without_panic() {
        // The reranker only scores index 0; the others fall back to their prior.
        // The defensive None path must not panic and must keep every input.
        let results = vec![sr("A", 0.2), sr("B", 0.9)];
        let out = QueryService::rerank_results("query", results, 5, &PartialReranker).unwrap();
        assert_eq!(out.len(), 2, "no result should be dropped");
        let ids: Vec<&str> = out.iter().map(|r| r.content_id.as_str()).collect();
        assert!(ids.contains(&"A") && ids.contains(&"B"));
    }

    // rq-graph-augmented-retrieval — 1-hop ref-graph expansion.

    fn pos(out: &[SurveyResult], id: &str) -> Option<usize> {
        out.iter().position(|r| r.content_id == id)
    }

    #[test]
    fn graph_expand_pulls_in_neighbour_below_its_source() {
        // A is the top hit; N is a ref-graph neighbour the embedding missed.
        let hits = vec![sr("A", 0.9), sr("B", 0.8)];
        let out = graph_expand_results(&hits, 8, |id| {
            if id == "A" {
                vec![sr("N", 0.0)]
            } else {
                vec![]
            }
        });

        assert_eq!(out.len(), 3, "the neighbour should be added");
        let n = &out[pos(&out, "N").expect("N present")];
        assert!(
            (n.score - 0.9 * GRAPH_EXPAND_DECAY).abs() < 1e-6,
            "neighbour inherits source.score * decay"
        );
        // Neighbour ranks below both primary hits.
        assert!(pos(&out, "N") > pos(&out, "A") && pos(&out, "N") > pos(&out, "B"));
    }

    #[test]
    fn graph_expand_never_overwrites_an_existing_hit() {
        // B is already a primary hit *and* a neighbour of A — it must keep its
        // own (higher-precision) score, not be re-added with a decayed one.
        let hits = vec![sr("A", 0.9), sr("B", 0.1)];
        let out = graph_expand_results(&hits, 8, |id| {
            if id == "A" {
                vec![sr("B", 0.0)]
            } else {
                vec![]
            }
        });
        assert_eq!(out.len(), 2, "no duplicate B");
        let b = &out[pos(&out, "B").expect("B present")];
        assert!((b.score - 0.1).abs() < 1e-6, "B keeps its primary score");
    }

    #[test]
    fn graph_expand_first_source_wins_a_shared_neighbour() {
        // N is a neighbour of both A (0.9) and B (0.5); the higher source wins.
        let hits = vec![sr("A", 0.9), sr("B", 0.5)];
        let out = graph_expand_results(&hits, 8, |_| vec![sr("N", 0.0)]);
        let n = &out[pos(&out, "N").expect("N present")];
        assert!(
            (n.score - 0.9 * GRAPH_EXPAND_DECAY).abs() < 1e-6,
            "shared neighbour takes the highest source's decayed score"
        );
        // N added once.
        assert_eq!(out.iter().filter(|r| r.content_id == "N").count(), 1);
    }

    #[test]
    fn graph_expand_respects_the_cap() {
        let hits = vec![sr("A", 0.9)];
        let out = graph_expand_results(&hits, 1, |_| vec![sr("N1", 0.0), sr("N2", 0.0)]);
        assert_eq!(out.len(), 2, "only one neighbour added under the cap");
    }

    #[test]
    fn graph_expand_is_identity_when_nothing_to_do() {
        let hits = vec![sr("A", 0.9), sr("B", 0.8)];
        // No neighbours.
        let out = graph_expand_results(&hits, 8, |_| vec![]);
        assert_eq!(out.len(), 2);
        // max_expand = 0.
        let out0 = graph_expand_results(&hits, 0, |_| vec![sr("N", 0.0)]);
        assert_eq!(out0.len(), 2);
        // empty input.
        assert!(graph_expand_results(&[], 8, |_| vec![sr("N", 0.0)]).is_empty());
    }
}
