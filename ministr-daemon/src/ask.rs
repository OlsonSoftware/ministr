//! Orchestration for `ministr_ask`: retrieve + infer + cache.
//!
//! The [`ask`] function is the main entry point. It checks the answer cache,
//! retrieves relevant sections via [`QueryService`], synthesizes an answer
//! via the [`Inference`] trait, and caches the result in `SQLite`.

use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

use ministr_core::service::QueryService;
use ministr_core::storage::SqliteStorage;
use ministr_core::storage::traits::{CachedAnswer, CachedAnswerSource};
use ministr_core::token::count_tokens;

use crate::inference::{Inference, InferenceError};

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

/// Maximum number of sections to retrieve for context.
const MAX_SURVEY_RESULTS: usize = 8;

/// Maximum total tokens of retrieved context to send to the sub-agent.
const MAX_CONTEXT_TOKENS: usize = 8000;

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

/// Compute a content hash for a section's text.
fn section_content_hash(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
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
    let hash = query_hash(query);

    // 1. Cache lookup
    if let Some(cached) = storage.get_cached_answer(&hash).await? {
        let sources = storage.get_cached_answer_sources(&hash).await?;
        if verify_sources(service, &sources).await {
            debug!(query_hash = %hash, "ask: cache hit (verified)");
            let source_ids = sources.into_iter().map(|s| s.section_id).collect();
            return Ok(AskResult {
                answer: cached.answer,
                source_ids,
                cached: true,
                model: cached.model,
            });
        }
        // Stale — hashes don't match. Invalidate and re-infer.
        debug!(query_hash = %hash, "ask: cache stale — re-inferring");
        let stale_ids: Vec<String> = sources.iter().map(|s| s.section_id.clone()).collect();
        let _ = storage.invalidate_answers_for_sections(&stale_ids).await;
    }

    // 2. Multi-strategy retrieval:
    //    a) Semantic survey — broad relevance
    //    b) Symbol search — targeted code symbols mentioned in the query
    //    c) Merge, dedup, expand, respect token budget
    let survey_results = service
        .survey(query, MAX_SURVEY_RESULTS)
        .await
        .map_err(|e| AskError::Retrieval(e.to_string()))?;

    // Extract likely symbol names from the query and search for them.
    let symbol_results = search_symbols_from_query(service, query).await;

    // Merge: survey results first (ranked by relevance), then symbol hits
    // that weren't already found by survey.
    let mut seen_ids = std::collections::HashSet::new();
    let mut merged: Vec<(String, String, String)> = Vec::new(); // (id, resolution, fallback_text)

    for r in &survey_results {
        if seen_ids.insert(r.content_id.clone()) {
            merged.push((r.content_id.clone(), r.resolution.clone(), r.text.clone()));
        }
    }
    for (sym_id, source) in &symbol_results {
        if seen_ids.insert(sym_id.clone()) {
            merged.push((sym_id.clone(), "symbol_full".to_string(), source.clone()));
        }
    }

    if merged.is_empty() {
        return Ok(AskResult {
            answer: "No relevant content found in the corpus.".to_string(),
            source_ids: vec![],
            cached: false,
            model: String::new(),
        });
    }

    // 3. Expand results to full text, dispatching by resolution type.
    let mut context_sections: Vec<(String, String)> = Vec::new();
    let mut sources: Vec<CachedAnswerSource> = Vec::new();
    let mut total_tokens = 0;

    for (content_id, resolution, fallback) in &merged {
        let text = expand_result(service, content_id, resolution, fallback).await;

        let tokens = count_tokens(&text);
        if total_tokens + tokens > MAX_CONTEXT_TOKENS && !context_sections.is_empty() {
            break;
        }

        let content_hash = section_content_hash(&text);
        context_sections.push((content_id.clone(), text));
        sources.push(CachedAnswerSource {
            section_id: content_id.clone(),
            section_hash: content_hash,
        });
        total_tokens += tokens;
    }

    // 4. Build prompt with retrieved context and infer.
    let prompt = build_inference_prompt(query, &context_sections);
    let response = inference.infer(&prompt).await?;

    // 5. Cache
    let answer_tokens = count_tokens(&response.answer);
    let cached_answer = CachedAnswer {
        query_hash: hash.clone(),
        query_text: query.to_string(),
        answer: response.answer.clone(),
        model: response.model.clone(),
        token_count: answer_tokens,
        created_at: String::new(), // SQLite DEFAULT handles this
    };
    if let Err(e) = storage.insert_cached_answer(&cached_answer, &sources).await {
        warn!(error = %e, "failed to cache answer");
    }

    let source_ids: Vec<String> = sources.into_iter().map(|s| s.section_id).collect();
    info!(
        query_hash = %hash,
        sources = source_ids.len(),
        tokens = answer_tokens,
        "ask: cached new answer"
    );

    Ok(AskResult {
        answer: response.answer,
        source_ids,
        cached: false,
        model: response.model,
    })
}

/// Extract likely symbol names from the query and search for matching code symbols.
///
/// Looks for `CamelCase` identifiers (struct/trait names) and `snake_case`
/// identifiers that look like function/method names. Returns (`symbol_id`, `source_context`)
/// pairs for any matches found.
async fn search_symbols_from_query(service: &QueryService, query: &str) -> Vec<(String, String)> {
    use ministr_core::storage::SymbolFilter;

    // Extract candidate symbol names from the query:
    // - CamelCase words (struct/trait names)
    // - snake_case words (function names)
    // - Any word 3+ chars that isn't a common English word (potential method names)
    const STOP: &[&str] = &[
        "the", "and", "for", "from", "with", "what", "how", "does", "are", "that", "this", "which",
        "where", "when", "before", "after", "into", "not", "its", "has", "have", "had", "been",
        "will", "was", "were", "being", "about", "between", "each", "all", "any", "both", "but",
        "can", "did", "get", "got", "may", "use", "used", "using", "method", "function", "struct",
        "fields", "response", "handler", "called", "returns", "returned", "why", "dropped",
    ];
    let candidates: Vec<&str> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 3 && !STOP.contains(&w.to_lowercase().as_str()))
        .collect();

    // Extract file path hint (e.g. "proxy.rs" → filter symbols by file path).
    let file_hint: Option<String> = query
        .split_whitespace()
        .find(|w| {
            std::path::Path::new(w)
                .extension()
                .is_some_and(|ext| matches!(ext.to_str(), Some("rs" | "ts" | "py")))
        })
        .map(std::string::ToString::to_string);

    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // Strategy 1: search each candidate name, preferring file-path matches.
    for name in &candidates {
        let filter = SymbolFilter {
            name: Some(name.to_string()),
            ..SymbolFilter::default()
        };
        if let Ok(symbols) = service.search_symbols(&filter).await {
            // If we have a file hint, prioritize symbols in that file.
            let mut sorted = symbols;
            if let Some(ref hint) = file_hint {
                sorted.sort_by_key(|s| i32::from(!s.file_path.ends_with(hint)));
            }
            for sym in sorted.into_iter().take(2) {
                let sym_id = sym.id.0.clone();
                if !seen.insert(sym_id.clone()) {
                    continue;
                }
                if let Ok(def) = service.get_symbol_definition(&sym_id).await
                    && !def.source_context.is_empty()
                {
                    results.push((sym_id, def.source_context));
                }
            }
        }
    }

    // Strategy 2: if a file is mentioned, get ALL symbols in that file
    // and include the most relevant ones (by name overlap with query).
    if let Some(ref hint) = file_hint {
        let filter = SymbolFilter {
            file_path: Some(hint.clone()),
            ..SymbolFilter::default()
        };
        if let Ok(symbols) = service.search_symbols(&filter).await {
            let query_lower = query.to_lowercase();
            let mut file_syms: Vec<_> = symbols
                .into_iter()
                .map(|s| {
                    let relevance = if query_lower.contains(&s.name.to_lowercase()) {
                        2
                    } else {
                        i32::from(query_lower.contains(&s.kind.to_lowercase()))
                    };
                    (s, relevance)
                })
                .collect();
            file_syms.sort_by_key(|s| std::cmp::Reverse(s.1));

            for (sym, _) in file_syms.into_iter().take(3) {
                let sym_id = sym.id.0.clone();
                if !seen.insert(sym_id.clone()) {
                    continue;
                }
                if let Ok(def) = service.get_symbol_definition(&sym_id).await
                    && !def.source_context.is_empty()
                {
                    results.push((sym_id, def.source_context));
                }
            }
        }
    }

    results
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
fn build_inference_prompt(query: &str, sections: &[(String, String)]) -> String {
    use std::fmt::Write as _;
    let mut prompt = String::with_capacity(4096);

    prompt.push_str(SYSTEM_PROMPT);
    prompt.push_str("\n\n---\n\n## Retrieved Context\n\n");

    for (id, text) in sections {
        let _ = write!(prompt, "### [{id}]\n\n{text}\n\n");
    }

    prompt.push_str("---\n\n");
    let _ = write!(prompt, "## Question\n\n{query}\n");

    prompt
}

const SYSTEM_PROMPT: &str = "\
You are a codebase expert answering questions about a software project. \
You have been given retrieved sections from the project's documentation and source code.

Rules:
1. Answer ONLY from the provided context. If the context is insufficient, say so.
2. Be concise — aim for 2-5 sentences unless the question requires more.
3. Reference section IDs in brackets like [section_id] when citing sources.
4. Focus on accuracy over completeness.
5. When listing struct fields, function parameters, or type definitions, \
   quote the exact source code if available. If only documentation is provided \
   (not source code), explicitly note that the exact definition is not in the \
   retrieved context.
6. Go straight to the answer — no preamble.";

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
        let prompt = build_inference_prompt("What is Foo?", &sections);
        assert!(prompt.contains("[sec1]"));
        assert!(prompt.contains("[sec2]"));
        assert!(prompt.contains("fn main()"));
        assert!(prompt.contains("What is Foo?"));
        assert!(prompt.contains(SYSTEM_PROMPT));
    }
}
