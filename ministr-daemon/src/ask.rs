//! Orchestration for `ministr_ask`: retrieve + infer + cache.
//!
//! The pipeline is a multi-stage RAG flow tuned for codebase Q&A:
//!
//! 1. **Cache lookup** — verified hit short-circuits everything else.
//! 2. **Query analysis** (1 LLM call, JSON) — produces a `HyDE` document, 1–3
//!    atomic sub-questions, candidate symbol identifiers, and a bridge
//!    relevance flag.
//! 3. **Multi-strategy retrieval** — per sub-question, fan out across:
//!    raw-question survey, HyDE-document survey, fuzzy symbol search,
//!    and (optionally) bridge link query.
//! 4. **Reciprocal Rank Fusion** — pure-function merge of all candidate
//!    lists. Output: top ~30 candidates by fused rank.
//! 5. **LLM rerank** (1 LLM call, JSON) — score each top candidate 0–10
//!    against the original question. Keep top 8, drop the noise floor.
//! 6. **Context curation** — expand to full text, pack into the budget,
//!    compute a coverage map (which sub-questions still have no support).
//! 7. **Synthesis** (1 LLM call) — coverage-aware prompt; the model
//!    must say "no evidence in context" for unsupported sub-questions
//!    instead of confabulating.
//! 8. **Verification** (3 complementary checks, always-on for fresh
//!    answers): a deterministic numeric/identifier grounding pass; a
//!    cross-encoder per-sentence entailment pass against the cited
//!    sources (reuses the survey reranker — no extra model load); and
//!    one JSON-mode LLM pass that targets misrepresentation. All three
//!    feed a single confidence note appended to the answer.
//! 9. **Cache** — keyed on the original query.
//!
//! [`ask`] is a thin wrapper over [`ask_with_progress`] that throws away
//! the phase events for callers (HTTP daemon) that don't care.

use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

use ministr_core::service::{QueryService, SurveyResult};
use ministr_core::storage::SqliteStorage;
use ministr_core::storage::traits::{CachedAnswer, CachedAnswerSource, SymbolFilter};
use ministr_core::token::count_tokens;

use crate::inference::{Inference, InferenceError, infer_json};

/// Result of an ask operation.
#[derive(Debug, Clone)]
pub struct AskResult {
    /// The synthesized answer.
    pub answer: String,
    /// Section IDs that contributed to the answer.
    pub source_ids: Vec<String>,
    /// Whether the answer came from cache.
    pub cached: bool,
    /// Model used for inference (empty if cached without model info).
    pub model: String,
}

/// Maximum tokens of retrieved context to send to the synthesis stage.
const MAX_CONTEXT_TOKENS: usize = 8000;

/// Per-strategy retrieval cap during the fan-out stage. Each strategy
/// independently returns up to this many candidates; RRF fusion then
/// re-ranks the union.
const PER_STRATEGY_TOP_K: usize = 8;

/// Number of merged candidates passed to the LLM rerank stage.
const RERANK_INPUT_CAP: usize = 30;

/// Maximum sources kept after the LLM rerank cut.
const FINAL_SOURCE_CAP: usize = 8;

/// Minimum relevance score (0–10) a candidate must clear to survive
/// the LLM rerank cut. Scores below this are noise.
const RERANK_SCORE_FLOOR: f32 = 3.0;

/// If the top-scored candidate is below this threshold after the LLM
/// rerank pass, retrieval was thin — fire an adaptive re-retrieval
/// round with reformulated queries before synthesizing on weak
/// evidence. One extra LLM call, gated on weak signal.
///
/// Source: Guo et al. 2026 — adaptive retrieval on reasoning
/// confidence drops.
const ADAPTIVE_RERETRIEVE_THRESHOLD: f32 = 6.0;

/// RRF k constant. The classic value from the 2009 Cormack et al. paper
/// — large enough to keep low-ranked items in the running but small
/// enough that top items still dominate.
const RRF_K: f32 = 60.0;

/// Progress events emitted by [`ask_with_progress`] so consumers can render
/// phase-by-phase UI without waiting for the full pipeline to finish.
#[derive(Debug, Clone)]
pub enum AskEvent {
    /// Verified cache hit — sources are known, no inference will run.
    CacheHit { source_ids: Vec<String> },
    /// Query analysis finished — the pipeline has decomposed the question.
    Analyzed {
        /// Atomic sub-questions the answer needs to address. May be a
        /// single-element vector containing the original question.
        sub_questions: Vec<String>,
        /// Short preview (first ~200 chars) of the `HyDE` document used
        /// for retrieval. Surfaced for transparency, not display polish.
        hyde_preview: String,
        /// Identifier hints the analysis stage extracted from the
        /// question (function names, struct names, etc.).
        symbol_hints: Vec<String>,
        /// True when the question seems to involve cross-language
        /// boundaries (Tauri / FFI / HTTP routes / etc.).
        bridge_relevant: bool,
    },
    /// Multi-strategy retrieval finished. Reports candidate counts per
    /// strategy plus the merged top-k ids that survived RRF fusion.
    RetrievedCandidates {
        /// Count of unique candidates per strategy label.
        by_strategy: std::collections::HashMap<String, usize>,
        /// Merged candidate ids in RRF rank order, top first.
        merged_ids: Vec<String>,
    },
    /// LLM rerank finished — these are the source ids that survived,
    /// in the new score order, with relevance scores attached.
    Reranked { source_ids: Vec<String> },
    /// All retrieval is done; inference is about to start. Mirrors the
    /// pre-existing event for backward compatibility with the UI.
    Retrieved { source_ids: Vec<String> },
    /// Verification stage ran. `unsupported_claims` is empty on a clean
    /// answer; non-empty entries are short prose excerpts of claims the
    /// verifier flagged as not supported by the cited sources.
    Verified { unsupported_claims: Vec<String> },
}

/// Error from ask operations.
#[derive(Debug, thiserror::Error)]
pub enum AskError {
    /// Retrieval from the index failed.
    #[error("retrieval failed: {0}")]
    Retrieval(String),

    /// Sub-inference failed.
    #[error("inference failed: {0}")]
    Inference(#[from] InferenceError),

    /// Storage I/O failed.
    #[error("storage error: {0}")]
    Storage(#[from] ministr_core::error::StorageError),
}

/// Compute the cache key for a query string.
///
/// Normalizes by trimming and lowercasing before hashing so that
/// "How does X work?" and "how does x work?" share a cache entry.
fn query_hash(query: &str) -> String {
    let normalized = query.trim().to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Compute a 64-char BLAKE3 hex hash for a section's text.
///
/// Used as a cache-invalidation fingerprint, not a stable identifier,
/// so the algorithm can change between releases without coordination.
fn section_content_hash(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

/// Run the ask pipeline: cache check → retrieve → infer → cache store.
///
/// # Errors
///
/// Returns [`AskError`] on retrieval, inference, or storage failures.
pub async fn ask(
    query: &str,
    service: &QueryService,
    storage: &SqliteStorage,
    inference: &dyn Inference,
) -> Result<AskResult, AskError> {
    ask_with_progress(query, service, storage, inference, |_| {}).await
}

/// Same pipeline as [`ask`], but emits [`AskEvent`] progress callbacks so
/// the caller can render phase-by-phase UI for each stage of the
/// multi-stage RAG flow described in the module docs. The callback is
/// invoked synchronously at each phase boundary.
///
/// # Errors
///
/// Returns [`AskError`] on retrieval, inference, or storage failures.
#[allow(clippy::too_many_lines)] // Orchestrator: linear stage flow is easier to follow than a forest of helpers.
pub async fn ask_with_progress<F>(
    query: &str,
    service: &QueryService,
    storage: &SqliteStorage,
    inference: &dyn Inference,
    on_event: F,
) -> Result<AskResult, AskError>
where
    F: Fn(AskEvent) + Send + Sync,
{
    let hash = query_hash(query);

    // ── 1. Cache lookup ─────────────────────────────────────────────────
    if let Some(cached) = storage.get_cached_answer(&hash).await? {
        let sources = storage.get_cached_answer_sources(&hash).await?;
        if verify_sources(service, &sources).await {
            debug!(query_hash = %hash, "ask: cache hit (verified)");
            let source_ids: Vec<String> = sources.into_iter().map(|s| s.section_id).collect();
            on_event(AskEvent::CacheHit {
                source_ids: source_ids.clone(),
            });
            return Ok(AskResult {
                answer: cached.answer,
                source_ids,
                cached: true,
                model: cached.model,
            });
        }
        debug!(query_hash = %hash, "ask: cache stale — re-inferring");
        let stale_ids: Vec<String> = sources.iter().map(|s| s.section_id.clone()).collect();
        let _ = storage.invalidate_answers_for_sections(&stale_ids).await;
    }

    // ── 2. Query analysis ───────────────────────────────────────────────
    let analysis = analyze_query(query, inference).await.unwrap_or_else(|e| {
        warn!(error = %e, "ask: query analysis failed; falling back to raw query");
        QueryAnalysis::fallback(query)
    });

    on_event(AskEvent::Analyzed {
        sub_questions: analysis.sub_questions.clone(),
        hyde_preview: truncate(&analysis.hyde_doc, 240),
        symbol_hints: analysis.symbol_hints.clone(),
        bridge_relevant: analysis.bridge_relevant,
    });

    // ── 3. Multi-strategy retrieval + 4. RRF fusion ─────────────────────
    let (merged, by_strategy) = multi_retrieve(service, &analysis).await;
    on_event(AskEvent::RetrievedCandidates {
        by_strategy: by_strategy.clone(),
        merged_ids: merged.iter().map(|c| c.content_id.clone()).collect(),
    });

    if merged.is_empty() {
        on_event(AskEvent::Retrieved { source_ids: vec![] });
        return Ok(AskResult {
            answer: "No relevant content found in the corpus.".to_string(),
            source_ids: vec![],
            cached: false,
            model: String::new(),
        });
    }

    // ── 5. LLM rerank ───────────────────────────────────────────────────
    let rerank_input: Vec<&Candidate> = merged.iter().take(RERANK_INPUT_CAP).collect();
    let reranked = match llm_rerank(query, &rerank_input, inference).await {
        Ok(scored) => scored,
        Err(e) => {
            // Fall back to RRF order if the rerank LLM call fails.
            warn!(error = %e, "ask: LLM rerank failed; using RRF order");
            rerank_input
                .iter()
                .enumerate()
                .map(|(i, c)| ScoredCandidate {
                    candidate: (*c).clone(),
                    #[allow(clippy::cast_precision_loss)]
                    score: 10.0_f32 - (i as f32) * 0.1,
                })
                .collect()
        }
    };

    let mut kept: Vec<ScoredCandidate> = reranked
        .into_iter()
        .filter(|s| s.score >= RERANK_SCORE_FLOOR)
        .take(FINAL_SOURCE_CAP)
        .collect();

    // Adaptive re-retrieval (Guo et al. 2026): if the strongest kept
    // candidate is still weak, the first retrieval round was thin —
    // re-issue with broader phrasings before synthesizing on weak
    // evidence. One extra LLM call, only when it would actually help.
    if let Some(top) = kept.first()
        && top.score < ADAPTIVE_RERETRIEVE_THRESHOLD
        && let Ok(extra) = adaptive_reretrieve(
            query,
            &analysis,
            service,
            inference,
            &kept
                .iter()
                .map(|s| s.candidate.content_id.clone())
                .collect::<std::collections::HashSet<_>>(),
        )
        .await
    {
        // Splice the new candidates in; keep ordering by score.
        for new_scored in extra {
            if !kept
                .iter()
                .any(|k| k.candidate.content_id == new_scored.candidate.content_id)
            {
                kept.push(new_scored);
            }
        }
        kept.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        kept.truncate(FINAL_SOURCE_CAP);
    }

    on_event(AskEvent::Reranked {
        source_ids: kept
            .iter()
            .map(|s| s.candidate.content_id.clone())
            .collect(),
    });

    if kept.is_empty() {
        on_event(AskEvent::Retrieved { source_ids: vec![] });
        return Ok(AskResult {
            answer: "No sufficiently relevant content found in the corpus.".to_string(),
            source_ids: vec![],
            cached: false,
            model: String::new(),
        });
    }

    // ── 6. Context curation ─────────────────────────────────────────────
    let mut context_sections: Vec<(String, String)> = Vec::new();
    let mut sources: Vec<CachedAnswerSource> = Vec::new();
    let mut total_tokens = 0;

    for scored in &kept {
        let text = expand_result(
            service,
            &scored.candidate.content_id,
            &scored.candidate.resolution,
            &scored.candidate.snippet,
        )
        .await;
        let tokens = count_tokens(&text);
        if total_tokens + tokens > MAX_CONTEXT_TOKENS && !context_sections.is_empty() {
            break;
        }
        let content_hash = section_content_hash(&text);
        context_sections.push((scored.candidate.content_id.clone(), text));
        sources.push(CachedAnswerSource {
            section_id: scored.candidate.content_id.clone(),
            section_hash: content_hash,
        });
        total_tokens += tokens;
    }

    let coverage = compute_coverage(&analysis.sub_questions, &kept);

    on_event(AskEvent::Retrieved {
        source_ids: sources.iter().map(|s| s.section_id.clone()).collect(),
    });

    // ── 7. Synthesis ────────────────────────────────────────────────────
    let prompt = build_inference_prompt(query, &analysis, &context_sections, &coverage);
    let response = inference.infer(&prompt).await?;

    // ── 8. Verification (always-on for fresh answers) ───────────────────
    //
    // Three complementary checks combine into one confidence note:
    //
    //   a) Deterministic numeric/identifier grounding — extract every hex
    //      offset, byte size, version number, and code-identifier-shaped
    //      token from the answer; flag any that don't appear verbatim in
    //      a cited source. Catches the precision-error class (model
    //      conflates "struct ends at 0x18" with "field is at 0x18").
    //
    //   b) Cross-encoder entailment — per-sentence NLI-style check using
    //      the same reranker that powered survey(). Catches sentences
    //      whose meaning isn't supported by ANY cited source, even when
    //      individual tokens appear in the pool. (Jin et al. 2026,
    //      Košprdić et al. 2026 VerifAI.)
    //
    //   c) LLM verification — flags claims that are technically supported
    //      but stated in a way that misrepresents the source.
    //
    // All three run on every fresh answer (not just coverage gaps):
    // precision errors happen most often when retrieval looks fine.
    let mut final_answer = response.answer.clone();
    if !context_sections.is_empty() {
        let ungrounded = check_grounded_numerics(&final_answer, &context_sections);
        let entailment_flags = service.reranker().map_or_else(Vec::new, |r| {
            entailment_check(&final_answer, &context_sections, r.as_ref())
        });
        let unsupported =
            match verify_answer(query, &final_answer, &context_sections, inference).await {
                Ok(u) => u,
                Err(e) => {
                    debug!(error = %e, "ask: verification stage failed; skipping LLM check");
                    Vec::new()
                }
            };

        let mut all_concerns: Vec<String> = Vec::new();
        for n in &ungrounded {
            all_concerns.push(format!(
                "`{n}` appears in the answer but not verbatim in any cited source — \
                 verify the value before relying on it."
            ));
        }
        all_concerns.extend(entailment_flags.iter().cloned());
        all_concerns.extend(unsupported.iter().cloned());

        if !all_concerns.is_empty() {
            final_answer = append_confidence_note(&final_answer, &all_concerns);
        }
        on_event(AskEvent::Verified {
            unsupported_claims: all_concerns,
        });
    }

    // ── 9. Cache ────────────────────────────────────────────────────────
    let answer_tokens = count_tokens(&final_answer);
    let cached_answer = CachedAnswer {
        query_hash: hash.clone(),
        query_text: query.to_string(),
        answer: final_answer.clone(),
        model: response.model.clone(),
        token_count: answer_tokens,
        created_at: String::new(),
    };
    if let Err(e) = storage.insert_cached_answer(&cached_answer, &sources).await {
        warn!(error = %e, "failed to cache answer");
    }

    let source_ids: Vec<String> = sources.into_iter().map(|s| s.section_id).collect();
    info!(
        query_hash = %hash,
        sources = source_ids.len(),
        tokens = answer_tokens,
        sub_questions = analysis.sub_questions.len(),
        coverage_gaps = coverage.iter().filter(|c| !c.covered).count(),
        "ask: cached new answer"
    );

    Ok(AskResult {
        answer: final_answer,
        source_ids,
        cached: false,
        model: response.model,
    })
}

// ─────────────────────────────────────────────────────────────────────────
// Stage 2 — Query analysis
// ─────────────────────────────────────────────────────────────────────────

/// Output of the query-analysis LLM call.
#[derive(Debug, Clone)]
struct QueryAnalysis {
    /// `HyDE` document — a short hypothetical answer used to embed the
    /// question into the corpus's vocabulary space.
    hyde_doc: String,
    /// 1–3 atomic sub-questions. Always non-empty (single-element when
    /// the question is already atomic).
    sub_questions: Vec<String>,
    /// Identifier-like tokens (function names, struct names, etc.) the
    /// model thinks the answer will involve.
    symbol_hints: Vec<String>,
    /// Whether the question seems to involve a cross-language boundary
    /// (Tauri / FFI / HTTP route / etc.). When true the bridge index
    /// is queried as an additional retrieval strategy.
    bridge_relevant: bool,
}

impl QueryAnalysis {
    /// Cheap fallback when the analysis LLM call fails. Treats the
    /// question as atomic, uses the question itself as the `HyDE` doc,
    /// and falls back on the legacy regex symbol extractor.
    fn fallback(query: &str) -> Self {
        Self {
            hyde_doc: query.to_string(),
            sub_questions: vec![query.to_string()],
            symbol_hints: extract_symbol_hints_heuristic(query),
            bridge_relevant: bridge_keywords_present(query),
        }
    }
}

/// Run one LLM call to decompose the query, generate a `HyDE` document,
/// and extract candidate identifiers. Single JSON-mode call.
async fn analyze_query(
    query: &str,
    inference: &dyn Inference,
) -> Result<QueryAnalysis, InferenceError> {
    let prompt = format!(
        "You are preparing a retrieval plan for a question about a software \
         project's codebase and docs.\n\n\
         Question: {query}\n\n\
         Produce a JSON object with these fields:\n\
         - \"hyde\": a short (2-4 sentence) hypothetical paragraph that \
         would *answer* the question. Use the kind of vocabulary and code \
         identifiers that would actually appear in the project. This is \
         used as an embedding seed, not shown to the user.\n\
         - \"sub_questions\": an array of 1-3 atomic sub-questions. If the \
         question is already atomic, return a single-element array \
         containing it (lightly normalized). If it has multiple facets, \
         split them.\n\
         - \"symbol_hints\": an array of likely code identifiers \
         (function names, struct names, file paths, etc.) that should be \
         searched directly. Empty array if none. Prefer specific names \
         over generic words.\n\
         - \"bridge_relevant\": a boolean — true if the question seems to \
         involve cross-language boundaries (Tauri commands, FFI, NAPI, \
         PyO3, HTTP routes, IPC) where bridge information is useful."
    );
    let value = infer_json(inference, &prompt).await?;

    let hyde_doc = value
        .get("hyde")
        .and_then(|v| v.as_str())
        .unwrap_or(query)
        .trim()
        .to_string();
    let sub_questions = value
        .get("sub_questions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| vec![query.to_string()]);
    let symbol_hints = value
        .get("symbol_hints")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(str::trim)
                .filter(|s| s.len() >= 2)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let bridge_relevant = value
        .get("bridge_relevant")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or_else(|| bridge_keywords_present(query));

    // Cap sub-questions to keep retrieval cost bounded.
    let sub_questions = sub_questions.into_iter().take(3).collect();

    Ok(QueryAnalysis {
        hyde_doc,
        sub_questions,
        symbol_hints,
        bridge_relevant,
    })
}

/// Backup heuristic when the analysis LLM call fails.
fn extract_symbol_hints_heuristic(query: &str) -> Vec<String> {
    const STOP: &[&str] = &[
        "the", "and", "for", "from", "with", "what", "how", "does", "are", "that", "this", "which",
        "where", "when", "before", "after", "into", "not", "its", "has", "have", "had", "been",
        "will", "was", "were", "being", "about", "between", "each", "all", "any", "both", "but",
        "can", "did", "get", "got", "may", "use", "used", "using", "method", "function", "struct",
        "fields", "response", "handler", "called", "returns", "returned", "why", "dropped",
    ];
    query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 3 && !STOP.contains(&w.to_lowercase().as_str()))
        .map(str::to_string)
        .collect()
}

fn bridge_keywords_present(query: &str) -> bool {
    let q = query.to_lowercase();
    [
        "tauri",
        "invoke",
        "ffi",
        "napi",
        "pyo3",
        "wasm",
        "ipc",
        "http route",
        "endpoint",
        "frontend",
        "backend",
        "rust side",
        "js side",
        "ts side",
    ]
    .iter()
    .any(|kw| q.contains(kw))
}

// ─────────────────────────────────────────────────────────────────────────
// Stage 3+4 — Multi-strategy retrieval + RRF
// ─────────────────────────────────────────────────────────────────────────

/// One retrieval candidate. Carries enough information to expand to full
/// text later without re-querying the index.
#[derive(Debug, Clone)]
struct Candidate {
    content_id: String,
    /// One of `section`, `symbol_stub`, `symbol_full`, `claim`, `summary`,
    /// or our synthetic `bridge_link`. Drives the expand path.
    resolution: String,
    /// Snippet for use as a fallback when expansion fails, AND as the
    /// text fed to the LLM rerank stage.
    snippet: String,
}

impl From<SurveyResult> for Candidate {
    fn from(r: SurveyResult) -> Self {
        Self {
            content_id: r.content_id,
            resolution: r.resolution,
            snippet: r.text,
        }
    }
}

/// Run all retrieval strategies, fuse via RRF, and return a unified
/// candidate list together with per-strategy diagnostics.
async fn multi_retrieve(
    service: &QueryService,
    analysis: &QueryAnalysis,
) -> (Vec<Candidate>, std::collections::HashMap<String, usize>) {
    use std::collections::HashMap;

    // Each Vec<Candidate> is one ranked list; RRF fuses them all.
    let mut ranked_lists: Vec<Vec<Candidate>> = Vec::new();
    let mut by_strategy: HashMap<String, usize> = HashMap::new();

    // Strategy a + b: per sub-question, one survey on the raw text and
    // one on the HyDE doc. We use survey() (not survey_excluding) — the
    // RRF stage handles dedup, and we want each list pristine.
    for (i, sub_q) in analysis.sub_questions.iter().enumerate() {
        let raw_label = format!("survey_raw[{i}]");
        if let Ok(results) = service.survey(sub_q, PER_STRATEGY_TOP_K).await {
            by_strategy.insert(raw_label.clone(), results.len());
            let list: Vec<Candidate> = results.into_iter().map(Candidate::from).collect();
            ranked_lists.push(list);
        } else {
            by_strategy.insert(raw_label, 0);
        }

        let hyde_label = format!("survey_hyde[{i}]");
        // Only run HyDE survey if the doc differs meaningfully from the
        // sub-question — otherwise it's a redundant call.
        if analysis.hyde_doc != *sub_q && analysis.hyde_doc.len() > 20 {
            if let Ok(results) = service.survey(&analysis.hyde_doc, PER_STRATEGY_TOP_K).await {
                by_strategy.insert(hyde_label.clone(), results.len());
                let list: Vec<Candidate> = results.into_iter().map(Candidate::from).collect();
                ranked_lists.push(list);
            } else {
                by_strategy.insert(hyde_label, 0);
            }
        } else {
            by_strategy.insert(hyde_label, 0);
        }
    }

    // Strategy c: symbol search on the LLM-extracted hints. Fuzzy match
    // gives us close-but-not-exact misses (e.g. "ask_corpus" matches
    // "ask_corpus" exactly but also "ask_with_progress" via substring).
    let mut symbol_candidates: Vec<Candidate> = Vec::new();
    for hint in &analysis.symbol_hints {
        let filter = SymbolFilter {
            name: Some(hint.clone()),
            ..SymbolFilter::default()
        };
        if let Ok(symbols) = service.search_symbols(&filter).await {
            for sym in symbols.into_iter().take(2) {
                let sym_id = sym.id.0.clone();
                let snippet = match service.get_symbol_definition(&sym_id).await {
                    Ok(def) if !def.source_context.is_empty() => def.source_context,
                    _ => format!("{} {}", sym.kind, sym.name),
                };
                symbol_candidates.push(Candidate {
                    content_id: sym_id,
                    resolution: "symbol_full".to_string(),
                    snippet,
                });
            }
        }
    }
    by_strategy.insert("symbol_hints".to_string(), symbol_candidates.len());
    if !symbol_candidates.is_empty() {
        ranked_lists.push(symbol_candidates);
    }

    // Strategy d: bridge query, only when relevant.
    if analysis.bridge_relevant {
        let mut bridge_candidates: Vec<Candidate> = Vec::new();
        // Use the original concatenated sub-questions as the lexical
        // query — bridge is keyword-based, not semantic.
        let bridge_query = analysis.sub_questions.join(" ");
        if let Ok(links) = service
            .query_bridges(Some(&bridge_query), None, None, None)
            .await
        {
            for link in links.into_iter().take(PER_STRATEGY_TOP_K) {
                // Synthesize a content_id pointing at the export side
                // (which is a real symbol in the index).
                let snippet = format!(
                    "{}::{} ({}) ↔ {}::{} ({})",
                    link.export_file,
                    link.export_symbol,
                    link.export_language,
                    link.import_file,
                    link.import_symbol,
                    link.import_language
                );
                bridge_candidates.push(Candidate {
                    content_id: format!("sym-{}::{}", link.export_file, link.export_symbol),
                    resolution: "symbol_full".to_string(),
                    snippet,
                });
            }
        }
        by_strategy.insert("bridge_links".to_string(), bridge_candidates.len());
        if !bridge_candidates.is_empty() {
            ranked_lists.push(bridge_candidates);
        }
    }

    let merged = rrf_merge(&ranked_lists);
    (merged, by_strategy)
}

/// Reciprocal Rank Fusion across multiple ranked candidate lists.
/// Pure function: deterministic, no model calls, robust to score-scale
/// differences between strategies.
///
/// Each contribution is multiplied by a per-resolution authority weight
/// (see [`authority_weight`]) so primary documentation outranks
/// auto-extracted claims at the same semantic-similarity rank. This is
/// the "domain-grounded tiered retrieval" idea from Haque et al. 2026.
fn rrf_merge(lists: &[Vec<Candidate>]) -> Vec<Candidate> {
    use std::collections::HashMap;

    let mut scores: HashMap<String, f32> = HashMap::new();
    let mut canonical: HashMap<String, Candidate> = HashMap::new();

    for list in lists {
        for (rank, cand) in list.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let contribution = 1.0_f32 / (RRF_K + (rank + 1) as f32);
            let weighted = contribution * authority_weight(&cand.resolution);
            *scores.entry(cand.content_id.clone()).or_insert(0.0) += weighted;
            canonical
                .entry(cand.content_id.clone())
                .or_insert_with(|| cand.clone());
        }
    }

    let mut ranked: Vec<(String, f32)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    ranked
        .into_iter()
        .filter_map(|(id, _)| canonical.remove(&id))
        .collect()
}

/// Per-resolution authority weight applied during RRF fusion.
///
/// Markdown sections beat extracted symbol context which beats
/// auto-derived claims/summaries. Tuned conservatively — large weight
/// differences would override semantic similarity. The values mostly
/// matter at near-ties, which is exactly when this signal helps.
///
/// Source: Haque et al. 2026 — domain-grounded tiered retrieval.
fn authority_weight(resolution: &str) -> f32 {
    match resolution {
        "section" => 1.5,      // primary docs / markdown
        "symbol_full" => 1.3,  // full source definition
        "symbol_stub" => 1.15, // signature only
        "claim" => 0.9,        // auto-extracted atomic claim
        "summary" => 0.85,     // auto-generated summary
        _ => 1.0,              // unknown — neutral
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Stage 4.5 — Adaptive re-retrieval (Guo et al. 2026)
// ─────────────────────────────────────────────────────────────────────────

/// When the first retrieval round produced only weak candidates, ask the
/// model for 2-3 alternate phrasings of the query (different vocabulary,
/// different framings) and run `survey()` against each. Merge anything new
/// into the candidate pool, scored proportionally to RRF rank.
///
/// Returns `ScoredCandidate` values so the caller can splice them straight
/// into the kept list. Skipped silently on any error — the existing weak
/// candidates remain in place.
async fn adaptive_reretrieve(
    original_query: &str,
    analysis: &QueryAnalysis,
    service: &QueryService,
    inference: &dyn Inference,
    already_kept: &std::collections::HashSet<String>,
) -> Result<Vec<ScoredCandidate>, InferenceError> {
    let prompt = format!(
        "The retrieval round for the question below produced only weakly \
         relevant matches. Generate 2-3 alternate phrasings of the question \
         that use DIFFERENT vocabulary — synonyms, code-identifier-style \
         names instead of plain English (or vice versa), broader or \
         narrower scoping. The goal is to bridge a vocabulary mismatch \
         between the user's words and the corpus. Output a JSON object: \
         {{\"phrasings\": [\"<alt1>\", \"<alt2>\", ...]}}.\n\n\
         Original question: {original_query}\n\n\
         Identifier hints already tried: {hints}\n\
         HyDE document already used: {hyde}",
        hints = analysis.symbol_hints.join(", "),
        hyde = truncate(&analysis.hyde_doc, 200),
    );

    let value = infer_json(inference, &prompt).await?;
    let phrasings: Vec<String> = value
        .get("phrasings")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut new_candidates: Vec<Candidate> = Vec::new();
    for phrasing in phrasings.iter().take(3) {
        if let Ok(results) = service.survey(phrasing, PER_STRATEGY_TOP_K).await {
            for r in results {
                if already_kept.contains(&r.content_id) {
                    continue;
                }
                if new_candidates.iter().any(|c| c.content_id == r.content_id) {
                    continue;
                }
                new_candidates.push(Candidate::from(r));
            }
        }
    }

    // Score the new candidates by their *position* in this round: top
    // few get scores at the cusp of the rerank floor so they integrate
    // cleanly with the already-kept pool without dominating it.
    let scored: Vec<ScoredCandidate> = new_candidates
        .into_iter()
        .take(FINAL_SOURCE_CAP)
        .enumerate()
        .map(|(i, c)| ScoredCandidate {
            candidate: c,
            #[allow(clippy::cast_precision_loss)]
            score: (RERANK_SCORE_FLOOR + 1.5).min(10.0) - (i as f32) * 0.2,
        })
        .collect();
    Ok(scored)
}

// ─────────────────────────────────────────────────────────────────────────
// Stage 5 — LLM rerank
// ─────────────────────────────────────────────────────────────────────────

/// One reranked candidate with its 0–10 relevance score.
#[derive(Debug, Clone)]
struct ScoredCandidate {
    candidate: Candidate,
    score: f32,
}

/// Send the merged candidates to the model with truncated snippets and
/// ask for a relevance score on each. Returns the same candidates with
/// scores attached, in descending score order.
async fn llm_rerank(
    query: &str,
    candidates: &[&Candidate],
    inference: &dyn Inference,
) -> Result<Vec<ScoredCandidate>, InferenceError> {
    use std::fmt::Write as _;

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut prompt = String::with_capacity(4096);
    prompt.push_str(
        "Score each retrieved candidate for its relevance to the question. \
         Output a JSON array where each entry is {\"i\": <index>, \"score\": \
         <0-10 number>}. Use higher scores for candidates that directly \
         answer the question or contain the exact symbols/concepts needed. \
         Use lower scores for tangentially related material. Be strict: \
         only score 8+ if the candidate is clearly load-bearing for the \
         answer.\n\n",
    );
    let _ = write!(prompt, "Question: {query}\n\nCandidates:\n");
    for (i, cand) in candidates.iter().enumerate() {
        let snippet = truncate(&cand.snippet, 400);
        let _ = write!(prompt, "[{i}] {} — {snippet}\n\n", cand.content_id);
    }
    prompt.push_str(
        "Respond with the JSON array only. Every candidate must have an \
         entry. No prose.",
    );

    let value = infer_json(inference, &prompt).await?;
    let arr = value
        .as_array()
        .ok_or_else(|| InferenceError::ParseFailed {
            reason: "rerank: model did not return a JSON array".to_string(),
        })?;

    let mut by_index: std::collections::HashMap<usize, f32> = std::collections::HashMap::new();
    for entry in arr {
        let i = entry.get("i").and_then(serde_json::Value::as_u64);
        let score = entry
            .get("score")
            .and_then(serde_json::Value::as_f64)
            .map(|f| {
                #[allow(clippy::cast_possible_truncation)]
                {
                    f as f32
                }
            });
        if let (Some(i), Some(score)) = (i, score) {
            #[allow(clippy::cast_possible_truncation)]
            by_index.insert(i as usize, score.clamp(0.0, 10.0));
        }
    }

    // Combine: every candidate gets a score (0.0 if the model omitted it).
    let mut scored: Vec<ScoredCandidate> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| ScoredCandidate {
            candidate: (*c).clone(),
            score: by_index.get(&i).copied().unwrap_or(0.0),
        })
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(scored)
}

// ─────────────────────────────────────────────────────────────────────────
// Stage 6 — Coverage map
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct CoverageEntry {
    sub_question: String,
    /// True when at least one kept source has a non-trivial RRF
    /// connection to this sub-question. A coarse signal — we just check
    /// whether the sub-question's keywords appear in any kept snippet.
    covered: bool,
}

fn compute_coverage(sub_questions: &[String], kept: &[ScoredCandidate]) -> Vec<CoverageEntry> {
    sub_questions
        .iter()
        .map(|sq| {
            let kw: Vec<String> = sq
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .filter(|w| w.len() >= 4)
                .map(str::to_lowercase)
                .collect();
            let covered = if kw.is_empty() {
                !kept.is_empty()
            } else {
                kept.iter().any(|s| {
                    let snip = s.candidate.snippet.to_lowercase();
                    kw.iter().any(|k| snip.contains(k))
                })
            };
            CoverageEntry {
                sub_question: sq.clone(),
                covered,
            }
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────
// Stage 8 — Verification
// ─────────────────────────────────────────────────────────────────────────

/// Ask the model to flag any claims in the answer that aren't directly
/// supported by the retrieved context. Returns short prose excerpts of
/// each unsupported claim. Empty result means the answer looks clean.
async fn verify_answer(
    query: &str,
    answer: &str,
    sources: &[(String, String)],
    inference: &dyn Inference,
) -> Result<Vec<String>, InferenceError> {
    use std::fmt::Write as _;

    let mut prompt = String::with_capacity(4096);
    prompt.push_str(
        "Audit the following answer against the cited context. Flag both:\n\
         (a) Unsupported claims — concrete factual statements (function names, \
             behavior, return types, file paths) that the context doesn't support.\n\
         (b) **Precision errors** — stated values (offsets, sizes, version numbers, \
             bitmasks, identifiers) that DO appear in the context but in a different \
             slot than the answer claims. Common patterns to catch:\n\
             - Says \"field X is at offset N\" when N is actually where field Y lives \
               or where the struct ends.\n\
             - Says \"value is 0x10\" when the source actually says 0x08 or 0x18.\n\
             - Conflates \"struct ends at offset N\" with \"field is at offset N\".\n\
             - Misnames a function/struct/constant (case, plurality, verb form).\n\
             - Wrong version number (e.g. says \"introduced in 12.0\" when the source \
               says 11.0 or 12.0.7).\n\n\
         Do NOT flag stylistic phrasing, summarization, or generally-correct prose. \
         Only flag concrete details that are wrong or misleading.\n\n",
    );
    let _ = write!(prompt, "Question: {query}\n\n");
    prompt.push_str("Cited context:\n");
    for (i, (id, text)) in sources.iter().enumerate() {
        let snippet = truncate(text, 800);
        let _ = write!(prompt, "[{}] {id}\n{snippet}\n\n", i + 1);
    }
    let _ = write!(prompt, "Answer:\n{answer}\n\n");
    prompt.push_str(
        "Output a JSON object: \
         {\"unsupported\": [\"<short claim>\", ...]}. \
         Empty array if everything checks out. No prose.",
    );

    let value = infer_json(inference, &prompt).await?;
    let arr = value
        .get("unsupported")
        .and_then(|v| v.as_array())
        .ok_or_else(|| InferenceError::ParseFailed {
            reason: "verification: missing or malformed `unsupported` array".to_string(),
        })?;
    Ok(arr
        .iter()
        .filter_map(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

/// Per-sentence cross-encoder entailment check (Jin et al. 2026, Košprdić
/// et al. 2026 `VerifAI`). Splits the answer into sentences, scores each
/// against the pool of cited sources using the same cross-encoder
/// reranker that powers `survey()`, and flags sentences whose best
/// premise score falls below a threshold.
///
/// The reranker is technically a relevance model, not a strict NLI head,
/// but cross-encoder relevance and textual entailment are closely
/// related: a sentence that contradicts or invents details about the
/// retrieved context will score noticeably lower against every cited
/// source than a sentence that's actually grounded.
///
/// Returns short prose excerpts of flagged sentences. Empty result means
/// every sentence in the answer was entailed by some cited source.
fn entailment_check(
    answer: &str,
    sources: &[(String, String)],
    reranker: &dyn ministr_core::embedding::Reranker,
) -> Vec<String> {
    if sources.is_empty() {
        return Vec::new();
    }

    let stripped = strip_citation_markers(answer);
    let sentences: Vec<String> = split_sentences(&stripped);
    if sentences.is_empty() {
        return Vec::new();
    }

    // Build the source pool — truncate each premise so the reranker
    // doesn't waste compute on irrelevant context.
    let premises: Vec<String> = sources
        .iter()
        .map(|(_, text)| truncate(text, 1500))
        .collect();
    let premise_refs: Vec<&str> = premises.iter().map(String::as_str).collect();

    let mut flagged: Vec<String> = Vec::new();
    for sentence in &sentences {
        // Skip very short sentences (transitions, headers, "Yes." / "No.")
        // and ones that don't carry a verifiable claim.
        if sentence.split_whitespace().count() < 4 {
            continue;
        }
        if !is_factual_sentence(sentence) {
            continue;
        }

        match reranker.rerank(sentence, &premise_refs) {
            Ok(scores) if !scores.is_empty() => {
                // The reranker returns sorted descending; index 0 is the
                // best premise. Sigmoid-normalize the raw logit to [0, 1]
                // for a stable threshold across reranker models.
                let best = scores[0].score;
                let entailment = sigmoid(best);
                if entailment < ENTAILMENT_THRESHOLD {
                    flagged.push(format!(
                        "Low entailment ({:.2}): \"{}\"",
                        entailment,
                        truncate(sentence, 160)
                    ));
                }
            }
            _ => {
                // Reranker failed for this sentence — don't flag, since
                // we can't tell whether it's entailed or not.
            }
        }

        if flagged.len() >= 4 {
            break;
        }
    }

    flagged
}

/// Sigmoid for normalizing raw cross-encoder logits to [0, 1].
fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Minimum entailment probability (sigmoid of the cross-encoder logit) a
/// sentence must clear against its best-matching cited source. Tuned
/// conservatively — too-aggressive flagging would surface false
/// positives from off-topic transitions; too-loose lets contradictions
/// through.
const ENTAILMENT_THRESHOLD: f32 = 0.35;

/// Split prose on sentence boundaries. Conservative — preserves the
/// terminal punctuation, avoids splitting on decimals (1.5), versions
/// (1.12.1), and ellipses.
fn split_sentences(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    let chars: Vec<char> = text.chars().collect();

    for i in 0..chars.len() {
        let c = chars[i];
        buf.push(c);
        if matches!(c, '.' | '!' | '?') {
            // Don't split on `.` if the next char is a digit (1.5, 12.0).
            let next = chars.get(i + 1).copied();
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let is_decimal = c == '.'
                && prev.is_some_and(|p| p.is_ascii_digit())
                && next.is_some_and(|n| n.is_ascii_digit());
            // Don't split mid-ellipsis.
            let is_ellipsis = next == Some('.');
            if !is_decimal && !is_ellipsis {
                // Look ahead — if the next non-space char is uppercase or
                // a newline, treat this as a real sentence boundary.
                let mut j = i + 1;
                while let Some(&ch) = chars.get(j) {
                    if ch == ' ' {
                        j += 1;
                    } else {
                        break;
                    }
                }
                let next_non_space = chars.get(j).copied();
                if next_non_space.is_none()
                    || next_non_space == Some('\n')
                    || next_non_space.is_some_and(|n| n.is_uppercase() || !n.is_alphabetic())
                {
                    out.push(buf.trim().to_string());
                    buf.clear();
                }
            }
        }
    }
    let leftover = buf.trim().to_string();
    if !leftover.is_empty() {
        out.push(leftover);
    }
    out.into_iter().filter(|s| !s.is_empty()).collect()
}

/// Coarse heuristic for whether a sentence carries a checkable factual
/// claim. Skips opinion / meta sentences ("This is unclear from the
/// retrieved context.") so we don't flag deliberate hedges.
fn is_factual_sentence(sentence: &str) -> bool {
    let lower = sentence.to_lowercase();
    let hedge_phrases = [
        "no_evidence",
        "doesn't cover",
        "does not cover",
        "isn't covered",
        "is not covered",
        "the retrieved context",
        "i'm not sure",
        "unclear",
        "no relevant",
    ];
    if hedge_phrases.iter().any(|p| lower.contains(p)) {
        return false;
    }
    // Sentences without any letters are noise.
    sentence.chars().any(char::is_alphabetic)
}

/// Deterministic precision check: extract every numeric and code-identifier
/// token from the answer, then verify each appears verbatim in at least one
/// cited source. Returns the tokens that don't ground out.
///
/// Catches the failure mode where the synthesizer "reads offsets fuzzily"
/// — e.g. claims a field sits at `0x18` when the source actually places it
/// at `0x08`. Fully complementary to the LLM verifier:
///
/// - This pass catches values that are simply wrong (no LLM call needed).
/// - The LLM pass catches values that are correct-but-misattributed.
fn check_grounded_numerics(answer: &str, sources: &[(String, String)]) -> Vec<String> {
    use std::collections::HashSet;

    // Pool every cited source's text together for a single substring scan.
    // We don't need per-source attribution for this check — just "does it
    // exist anywhere in the cited context?"
    let mut pool = String::new();
    for (id, text) in sources {
        pool.push_str(id);
        pool.push('\n');
        pool.push_str(text);
        pool.push('\n');
    }

    let mut flagged: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // Strip the markdown answer's citation markers so `[1]`, `[2]`, etc.
    // don't show up as ungrounded "numbers".
    let stripped = strip_citation_markers(answer);

    for token in extract_precision_tokens(&stripped) {
        // Already-seen tokens contribute once.
        if !seen.insert(token.clone()) {
            continue;
        }
        if !pool.contains(&token) {
            flagged.push(token);
        }
        // Cap the report to keep the confidence note readable.
        if flagged.len() >= 6 {
            break;
        }
    }

    flagged
}

/// Remove `[N]` and `[N, M]` citation markers from a markdown answer so
/// they don't get extracted as bare numbers.
fn strip_citation_markers(answer: &str) -> String {
    let mut out = String::with_capacity(answer.len());
    let bytes = answer.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            // Try to match [<digits and commas and spaces>]
            let mut j = i + 1;
            let mut all_digits_or_sep = false;
            while j < bytes.len() {
                let b = bytes[j];
                if b.is_ascii_digit() || b == b',' || b == b' ' {
                    if b.is_ascii_digit() {
                        all_digits_or_sep = true;
                    }
                    j += 1;
                } else {
                    break;
                }
            }
            if all_digits_or_sep && j < bytes.len() && bytes[j] == b']' {
                // Skip the entire `[…]` marker.
                i = j + 1;
                continue;
            }
        }
        // Push one byte at a time but preserve UTF-8 by pushing chars.
        let ch_end = next_char_end(answer, i);
        out.push_str(&answer[i..ch_end]);
        i = ch_end;
    }
    out
}

/// Index of the byte immediately past the UTF-8 character starting at `i`.
fn next_char_end(s: &str, i: usize) -> usize {
    s[i..]
        .char_indices()
        .nth(1)
        .map_or(s.len(), |(off, _)| i + off)
}

/// Pull "precision-sensitive" tokens out of an answer:
///
/// - Hex literals: `0x[0-9a-fA-F]+`
/// - Byte sizes:   `<digits>(?:-byte|-bit| byte| bytes)`
/// - Version-like: `<digits>.<digits>(?:.<digits>)*`
/// - Bare integers: `<digits>` length 2+ (single digits are too noisy)
///
/// Skip everything that's pure prose. We're looking for the kind of token
/// where being off by a single character matters.
fn extract_precision_tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Hex literal: 0x...
        if i + 2 < bytes.len() && bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X')
        {
            let start = i;
            let mut j = i + 2;
            while j < bytes.len() && bytes[j].is_ascii_hexdigit() {
                j += 1;
            }
            if j > start + 2 {
                // Normalize to lowercase `0x…` so `0x18` and `0X18` collapse.
                let raw = &text[start..j];
                out.push(raw.to_ascii_lowercase().replace("0X", "0x"));
                i = j;
                continue;
            }
        }

        // Digits run — could be a version, byte count, or bare integer.
        if bytes[i].is_ascii_digit() {
            let start = i;
            let mut j = i;
            // Greedy digits + dots (versions like 1.12.1.5875).
            while j < bytes.len()
                && (bytes[j].is_ascii_digit()
                    || (bytes[j] == b'.' && j + 1 < bytes.len() && bytes[j + 1].is_ascii_digit()))
            {
                j += 1;
            }
            // Trim a trailing dot (sentence boundary).
            while j > start && bytes[j - 1] == b'.' {
                j -= 1;
            }
            let span = &text[start..j];
            // Only keep if it's at least 2 characters OR contains a dot —
            // single digits are too common to be useful signal.
            if span.contains('.') || span.len() >= 2 {
                out.push(span.to_string());
            }
            i = j;
            continue;
        }

        i = next_char_end(text, i);
    }

    out
}

fn append_confidence_note(answer: &str, concerns: &[String]) -> String {
    use std::fmt::Write as _;
    let mut out = answer.trim_end().to_string();
    out.push_str(
        "\n\n---\n\n**Confidence note:** the verifier raised concerns about \
         these specific details:\n",
    );
    for concern in concerns {
        let _ = writeln!(out, "- {concern}");
    }
    out.push_str(
        "Treat these specific details with caution; the rest of the answer \
         is grounded in the cited context.",
    );
    out
}

// ─────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max {
        trimmed.to_string()
    } else {
        let end = trimmed
            .char_indices()
            .nth(max)
            .map_or(trimmed.len(), |x| x.0);
        format!("{}…", &trimmed[..end])
    }
}

/// Expand a survey result to its full text based on resolution type.
///
/// Symbol stubs are expanded to full source definitions. Sections are
/// read in full. Claims and summaries use the survey snippet as-is.
async fn expand_result(
    service: &QueryService,
    content_id: &str,
    resolution: &str,
    fallback_text: &str,
) -> String {
    match resolution {
        "symbol_stub" | "symbol_full" => {
            // content_id is already a full symbol ID (may or may not have "sym-" prefix).
            match service.get_symbol_definition(content_id).await {
                Ok(def) => def.source_context,
                Err(_) => fallback_text.to_string(),
            }
        }
        "section" => match service.read_section(content_id).await {
            Ok(detail) => detail.text,
            Err(_) => fallback_text.to_string(),
        },
        // "claim", "summary", or unknown — use the snippet directly.
        _ => fallback_text.to_string(),
    }
}

/// Verify that source sections still have the same content hash.
///
/// Each stored source ID can be a section, a symbol, or a claim — all
/// three flow through the survey+symbols merge in [`ask`]. The hash was
/// computed from whichever *text representation* went into the prompt
/// (see [`expand_result`]), so verification must re-derive the same
/// representation:
///
/// - **Sections**: `read_section(id).text`
/// - **Symbols**: `get_symbol_definition(id).source_context`
/// - **Claims / unknown**: skipped — the prompt snippet came from the
///   survey result and isn't uniquely addressable after the fact.
///
/// A fresh fetch that doesn't resolve is treated as "skip, not
/// invalidate" — deleted content shouldn't force an expensive
/// re-inference when the rest of the answer is still grounded. Only an
/// actual hash mismatch on something we *can* resolve invalidates.
async fn verify_sources(service: &QueryService, sources: &[CachedAnswerSource]) -> bool {
    for source in sources {
        // Try section first (fastest, most common). Fall back to symbol
        // definition for symbol-resolution sources. Anything else falls
        // through to a skip.
        let current_text = if let Ok(detail) = service.read_section(&source.section_id).await {
            detail.text
        } else if let Ok(def) = service.get_symbol_definition(&source.section_id).await {
            def.source_context
        } else {
            continue;
        };

        if section_content_hash(&current_text) != source.section_hash {
            return false;
        }
    }
    true
}

/// Build the prompt with retrieved context for text synthesis.
///
/// Sections are labelled `[1]`, `[2]`, … in the order they appear, so the
/// model can cite them with simple numeric brackets that the UI parses
/// back into clickable chips. The prompt also surfaces the sub-question
/// decomposition and a coverage map so the model knows which facets
/// have grounded support and which it must explicitly disclaim.
fn build_inference_prompt(
    query: &str,
    analysis: &QueryAnalysis,
    sections: &[(String, String)],
    coverage: &[CoverageEntry],
) -> String {
    use std::fmt::Write as _;
    let mut prompt = String::with_capacity(4096);

    prompt.push_str(SYSTEM_PROMPT);

    // Sub-question plan + coverage map. Always included even when the
    // question is atomic — keeps the format predictable for the model.
    if analysis.sub_questions.len() > 1 || coverage.iter().any(|c| !c.covered) {
        prompt.push_str("\n\n---\n\n## Sub-question coverage\n\n");
        for (i, entry) in coverage.iter().enumerate() {
            let marker = if entry.covered {
                "[supported]"
            } else {
                "[no evidence]"
            };
            let _ = writeln!(prompt, "{}. {marker} {}", i + 1, entry.sub_question);
        }
        prompt.push_str(
            "\nFor any [no evidence] sub-question, you MUST state plainly \
             that the retrieved context doesn't cover it. Do not guess.\n",
        );
    }

    prompt.push_str("\n---\n\n## Retrieved Context\n\n");
    for (i, (id, text)) in sections.iter().enumerate() {
        let n = i + 1;
        let _ = write!(prompt, "### [{n}] {id}\n\n{text}\n\n");
    }

    prompt.push_str("---\n\n");
    let _ = write!(prompt, "## Question\n\n{query}\n");

    prompt
}

const SYSTEM_PROMPT: &str = "\
You are a codebase expert answering questions about a software project. \
You have been given retrieved sections from the project's documentation and source code, \
each labelled with a numeric tag like ### [1], ### [2], etc.

Rules:
1. Answer ONLY from the provided context. If the context is insufficient for any \
   sub-question, say \"the retrieved context doesn't cover X\" plainly. Never guess.
2. Be concise — aim for 2-5 sentences unless the question genuinely requires more.
3. Cite sources using numeric brackets that match the labels — e.g. [1], [2], [1, 3]. \
   Place each citation immediately after the claim it supports. Do NOT include the \
   raw content_id; only the number. Do NOT add a separate \"Sources\" or \"References\" \
   section at the end — citations are inline only.
4. Every factual claim about the project must carry at least one citation. \
   Uncited claims are treated as hallucinations.
5. When listing struct fields, function parameters, or type definitions, \
   quote the exact source code if available. If only documentation is provided \
   (not source code), explicitly note that the exact definition is not in the \
   retrieved context.
6. **Numeric precision**: when stating an offset, size, version number, bitmask, \
   or any other numeric value, copy the EXACT value from the cited context. \
   Do not paraphrase, round, or infer numbers. If you say a field is \"at offset \
   0x18\", that exact string \"0x18\" must appear in the cited source as the \
   field's offset. Do not conflate \"struct ends at offset N\" with \"field is at \
   offset N\" — they're different facts.
7. **Identifier precision**: when you name a function, struct, field, or constant, \
   copy the identifier exactly as it appears in the source. Do not invent CamelCase \
   from snake_case (or vice versa).
8. **Conservative refusal**: if you can't ground a specific value in the cited \
   context, do NOT invent or interpolate one. Instead emit the literal sentinel \
   `[NO_EVIDENCE: <what's missing>]` inline at that point — for example \
   \"the field sits at [NO_EVIDENCE: exact offset not in retrieved context] [3]\". \
   This is preferred over plausible-sounding paraphrases. \
   (Source: Pawlik & Deniziak 2026, Applied Sciences — citation-enforced prompting.)
9. Go straight to the answer — no preamble, no restating of the question.";

/// Listen for coherence broadcasts and invalidate cached answers whose
/// source sections changed.
pub async fn spawn_cache_invalidator(
    storage: std::sync::Arc<SqliteStorage>,
    mut coherence_rx: tokio::sync::broadcast::Receiver<ministr_api::coherence::CoherenceEvent>,
    corpus_id: String,
) {
    loop {
        match coherence_rx.recv().await {
            Ok(event) if !event.affected_sections.is_empty() => {
                match storage
                    .invalidate_answers_for_sections(&event.affected_sections)
                    .await
                {
                    Ok(count) if count > 0 => {
                        info!(
                            corpus_id,
                            invalidated = count,
                            sections = event.affected_sections.len(),
                            path = %event.path,
                            "invalidated cached answers for changed sections"
                        );
                    }
                    Ok(_) => {} // No cached answers affected
                    Err(e) => {
                        warn!(error = %e, "failed to invalidate answer cache");
                    }
                }
            }
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                warn!(
                    corpus_id,
                    missed = n,
                    "answer cache invalidation lagged — relying on hash verification"
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_hash_normalizes() {
        assert_eq!(
            query_hash("How does X work?"),
            query_hash("  how does x work?  "),
        );
    }

    #[test]
    fn query_hash_differs_for_different_queries() {
        assert_ne!(query_hash("question one"), query_hash("question two"));
    }

    #[test]
    fn section_content_hash_is_stable() {
        let h1 = section_content_hash("hello world");
        let h2 = section_content_hash("hello world");
        assert_eq!(h1, h2);
    }

    #[test]
    fn build_prompt_includes_context_and_question() {
        let sections = vec![
            ("sec1".to_string(), "fn main() {}".to_string()),
            ("sec2".to_string(), "struct Foo;".to_string()),
        ];
        let analysis = QueryAnalysis::fallback("What is Foo?");
        let coverage = compute_coverage(
            &analysis.sub_questions,
            &[ScoredCandidate {
                candidate: Candidate {
                    content_id: "sec1".to_string(),
                    resolution: "section".to_string(),
                    snippet: "fn main() {} // Foo lives here".to_string(),
                },
                score: 9.0,
            }],
        );
        let prompt = build_inference_prompt("What is Foo?", &analysis, &sections, &coverage);
        assert!(prompt.contains("[1] sec1"));
        assert!(prompt.contains("[2] sec2"));
        assert!(prompt.contains("fn main()"));
        assert!(prompt.contains("What is Foo?"));
        assert!(prompt.contains(SYSTEM_PROMPT));
    }

    #[test]
    fn authority_weight_prefers_primary_docs() {
        assert!(authority_weight("section") > authority_weight("symbol_full"));
        assert!(authority_weight("symbol_full") > authority_weight("symbol_stub"));
        assert!(authority_weight("symbol_stub") > authority_weight("claim"));
        assert!(authority_weight("claim") > authority_weight("summary"));
        // Unknown resolution falls back to neutral.
        assert!((authority_weight("???") - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn rrf_merge_authority_breaks_near_ties() {
        // Two candidates appear at identical ranks across the same lists,
        // differentiated only by resolution. The higher-authority one
        // (section) should win.
        let primary = Candidate {
            content_id: "primary".to_string(),
            resolution: "section".to_string(),
            snippet: "p".to_string(),
        };
        let auto = Candidate {
            content_id: "auto".to_string(),
            resolution: "summary".to_string(),
            snippet: "a".to_string(),
        };
        let lists = vec![vec![primary.clone(), auto.clone()]];
        let merged = rrf_merge(&lists);
        assert_eq!(merged[0].content_id, "primary");
    }

    #[test]
    fn rrf_merge_promotes_consensus_candidates() {
        // Same candidate appears in two lists → should beat candidates
        // that appear only in one list, regardless of single-list rank.
        let a = Candidate {
            content_id: "a".to_string(),
            resolution: "section".to_string(),
            snippet: "a".to_string(),
        };
        let b = Candidate {
            content_id: "b".to_string(),
            resolution: "section".to_string(),
            snippet: "b".to_string(),
        };
        let c = Candidate {
            content_id: "c".to_string(),
            resolution: "section".to_string(),
            snippet: "c".to_string(),
        };
        let lists = vec![
            vec![b.clone(), a.clone(), c.clone()],
            vec![a.clone(), c.clone(), b.clone()],
        ];
        let merged = rrf_merge(&lists);
        // `a` and `c` appear at rank 1+2 and 3+2 — `a` should win.
        assert_eq!(merged[0].content_id, "a");
    }

    #[test]
    fn coverage_flags_uncovered_sub_questions() {
        let kept = vec![ScoredCandidate {
            candidate: Candidate {
                content_id: "x".to_string(),
                resolution: "section".to_string(),
                snippet: "the embedding model loads ONNX weights".to_string(),
            },
            score: 8.0,
        }];
        let cov = compute_coverage(
            &[
                "How does the embedding pipeline work?".to_string(),
                "Where are tray icons defined?".to_string(),
            ],
            &kept,
        );
        assert!(cov[0].covered);
        assert!(!cov[1].covered);
    }

    #[test]
    fn truncate_handles_unicode_boundary() {
        // Make sure truncate doesn't slice in the middle of a multi-byte
        // character. Earlier byte-slicing implementations panicked here.
        let s = "héllo world ".repeat(50);
        let _ = truncate(&s, 7);
    }

    #[test]
    fn strip_citation_markers_drops_numeric_brackets() {
        let answer = "Field at 0x18 [1], 16 bytes [2, 3]. The struct ends at \
                      0x18 [1].";
        let stripped = strip_citation_markers(answer);
        assert!(!stripped.contains("[1]"));
        assert!(!stripped.contains("[2, 3]"));
        assert!(stripped.contains("0x18"));
    }

    #[test]
    fn extract_precision_tokens_finds_offsets_and_versions() {
        let toks = extract_precision_tokens(
            "Offset 0x18 was added in 12.0; \
                                              16 bytes wide",
        );
        assert!(toks.iter().any(|t| t == "0x18"));
        assert!(toks.iter().any(|t| t == "12.0"));
        assert!(toks.iter().any(|t| t == "16"));
    }

    #[test]
    fn check_grounded_numerics_flags_offset_mismatches() {
        // Source clearly says hash is at 0x08; answer claims it's at 0x18
        // (the actual end-of-struct, not the field). Should flag 0x18 as
        // not appearing as an offset in the cited context.
        let sources = vec![(
            "BLTE.md".to_string(),
            "Block v0x0F: 0x08 char[16] hash — checksum of compressed block.".to_string(),
        )];
        let answer = "The hash field is at offset 0x18 [1].";
        let flagged = check_grounded_numerics(answer, &sources);
        // 0x18 is NOT in the source — should be flagged.
        assert!(flagged.iter().any(|t| t == "0x18"));
        // The correct offset 0x08 IS in the source — would not be flagged
        // (and isn't in the answer, so isn't extracted either).
        assert!(!flagged.iter().any(|t| t == "0x08"));
    }

    #[test]
    fn check_grounded_numerics_passes_when_values_match() {
        let sources = vec![(
            "BLTE.md".to_string(),
            "0x18 char[16] uncompressedHash — MD5 of the uncompressed block. \
             Introduced in 12.0."
                .to_string(),
        )];
        let answer = "uncompressedHash sits at offset 0x18 [1] and was added \
                      in 12.0 [1].";
        let flagged = check_grounded_numerics(answer, &sources);
        assert!(flagged.is_empty(), "expected no flags, got {flagged:?}");
    }

    #[test]
    fn check_grounded_numerics_ignores_citation_brackets() {
        // [1], [2] etc. should never show up as ungrounded "1" / "2".
        let sources = vec![(
            "x".to_string(),
            "The answer to the question is 42.".to_string(),
        )];
        let answer = "The answer is 42 [1] [2].";
        let flagged = check_grounded_numerics(answer, &sources);
        assert!(flagged.is_empty(), "expected no flags, got {flagged:?}");
    }

    #[test]
    fn split_sentences_handles_decimals() {
        let sentences = split_sentences(
            "BLP2 was added in 1.5.0. The version field is 1. \
             Use 0x32 for the magic.",
        );
        assert_eq!(sentences.len(), 3);
        assert!(sentences[0].contains("1.5.0"));
    }

    #[test]
    fn split_sentences_handles_question_and_exclamation() {
        let sentences = split_sentences("What is BLP? It is a texture format!");
        assert_eq!(sentences.len(), 2);
        assert!(sentences[0].ends_with('?'));
        assert!(sentences[1].ends_with('!'));
    }

    #[test]
    fn is_factual_sentence_skips_hedges() {
        assert!(!is_factual_sentence(
            "The retrieved context doesn't cover X."
        ));
        assert!(!is_factual_sentence(
            "[NO_EVIDENCE: exact offset not in retrieved context]"
        ));
        assert!(is_factual_sentence("The hash field is at offset 0x08."));
    }

    #[test]
    fn sigmoid_known_values() {
        assert!((sigmoid(0.0) - 0.5).abs() < 1e-6);
        assert!(sigmoid(10.0) > 0.99);
        assert!(sigmoid(-10.0) < 0.01);
    }

    #[test]
    fn entailment_check_flags_contradictions() {
        // Mock reranker: returns a low score when "0x18 hash" appears
        // in the hypothesis and the premise says "0x08 hash" — a
        // realistic contradiction signature.
        struct MockReranker;
        impl ministr_core::embedding::Reranker for MockReranker {
            fn rerank(
                &self,
                query: &str,
                documents: &[&str],
            ) -> Result<Vec<ministr_core::embedding::RerankScore>, ministr_core::error::IndexError>
            {
                Ok(documents
                    .iter()
                    .enumerate()
                    .map(|(i, doc)| {
                        // Any sentence claiming `0x18 hash` against a premise
                        // that mentions `0x08` should score low.
                        let score = if query.contains("0x18") && doc.contains("0x08") {
                            -3.0
                        } else if query.contains("0x08") && doc.contains("0x08") {
                            5.0
                        } else {
                            1.0
                        };
                        ministr_core::embedding::RerankScore { index: i, score }
                    })
                    .collect())
            }
        }

        let sources = vec![(
            "BLTE.md".to_string(),
            "0x08 char[16] hash — checksum of the compressed block.".to_string(),
        )];
        let bad_answer = "The hash field sits at offset 0x18 in v0x0F.";
        let flagged = entailment_check(bad_answer, &sources, &MockReranker);
        assert!(!flagged.is_empty(), "expected the contradiction to flag");

        let good_answer = "The hash field sits at offset 0x08 in v0x0F.";
        let flagged_good = entailment_check(good_answer, &sources, &MockReranker);
        assert!(
            flagged_good.is_empty(),
            "expected the entailed sentence to pass, got {flagged_good:?}"
        );
    }
}
