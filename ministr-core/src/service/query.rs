//! Search, read, and extraction query operations for [`QueryService`].
//!
//! This module contains the core query methods: survey (search), read,
//! extract claims, related claims, and the private helpers for reranking
//! and content resolution.

use std::collections::{HashMap, HashSet};

use tracing::instrument;

use crate::embedding::{DualEmbedder, Reranker};
use crate::search::{MultiResolutionSearch, ScoredResult, SearchConfig};
use crate::storage::Storage;
use crate::token::count_tokens;
use crate::types::{
    ClaimId, ContentId, ContentProvenance, DeliveryIdentity, Resolution, ResultLocator, SectionId,
    SymbolId, TextDeliveryMetadata, TextRepresentation, VectorId,
};

use super::{
    ClaimResult, QueryError, QueryService, RelatedClaimResult, SectionDetail, SurveyOptions,
    SurveyResult, cosine_similarity, is_unresolved_placeholder,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SurveyIntent {
    Code,
    Documentation,
    Mixed,
}

impl SurveyIntent {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Documentation => "documentation",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug)]
pub(super) struct ResolvedContent {
    body: String,
    summary: Option<String>,
    heading_path: Option<Vec<String>>,
    base_representation: TextRepresentation,
    continuation_resolution: Option<&'static str>,
}

const CODE_EXTENSIONS: &[&str] = &[
    ".rs", ".go", ".py", ".js", ".jsx", ".ts", ".tsx", ".java", ".kt", ".kts", ".swift", ".c",
    ".cc", ".cpp", ".h", ".hpp", ".cs", ".rb", ".php", ".scala", ".sh", ".zig", ".ex", ".exs",
    ".lua", ".dart", ".vue", ".svelte",
];

fn survey_intent(query: &str) -> SurveyIntent {
    let query = query.to_ascii_lowercase();
    let code_markers = [
        "implementation",
        "implement",
        "function",
        "method",
        "struct",
        "enum",
        "trait",
        "class",
        "symbol",
        "caller",
        "callee",
        "handler",
        "module",
        "source",
        "code",
        "bug",
        "panic",
        "compile",
        "test",
        "::",
        ".rs",
        ".ts",
        ".py",
        "src/",
    ];
    let doc_markers = [
        "documentation",
        "docs",
        "readme",
        "guide",
        "tutorial",
        "explain",
        "concept",
        "architecture",
        "adr",
        "changelog",
        "release notes",
    ];
    let code = code_markers.iter().filter(|m| query.contains(**m)).count();
    let docs = doc_markers.iter().filter(|m| query.contains(**m)).count();
    match code.cmp(&docs) {
        std::cmp::Ordering::Greater => SurveyIntent::Code,
        std::cmp::Ordering::Less => SurveyIntent::Documentation,
        std::cmp::Ordering::Equal => SurveyIntent::Mixed,
    }
}

fn is_code_result(result: &SurveyResult) -> bool {
    if result.resolution.starts_with("symbol_") || result.content_id.starts_with("sym-") {
        return true;
    }
    let path = result_document_id(&result.content_id).to_ascii_lowercase();
    CODE_EXTENSIONS
        .iter()
        .any(|extension| path.ends_with(extension))
}

fn result_document_id(content_id: &str) -> &str {
    let content_id = content_id.strip_prefix("sym-").unwrap_or(content_id);
    let symbol_boundary = content_id.find("::");
    let section_boundary = content_id.find('#');
    let boundary = match (symbol_boundary, section_boundary) {
        (Some(symbol), Some(section)) => symbol.min(section),
        (Some(symbol), None) => symbol,
        (None, Some(section)) => section,
        (None, None) => content_id.len(),
    };
    &content_id[..boundary]
}

fn query_terms(query: &str) -> Vec<String> {
    let mut terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '/' && c != '.')
        .map(str::to_ascii_lowercase)
        .filter(|term| term.len() >= 3)
        .collect();
    terms.sort();
    terms.dedup();
    terms
}

/// Classify source provenance from stable content/file paths.
#[must_use]
pub fn classify_content_provenance(content_id: &str) -> ContentProvenance {
    let path = result_document_id(content_id)
        .replace('\\', "/")
        .to_ascii_lowercase();
    if path.contains("/vendor/")
        || path.contains("/third_party/")
        || path.starts_with("vendor/")
        || path.starts_with("node_modules/")
    {
        ContentProvenance::Vendor
    } else if path.contains("generated")
        || path.contains("/gen/")
        || path.ends_with(".g.rs")
        || path.ends_with(".generated.ts")
        || path.ends_with("_pb2.py")
    {
        ContentProvenance::Generated
    } else if path.contains("fixture") || path.contains("fixtures/") {
        ContentProvenance::Fixture
    } else if path.contains("benchmark")
        || path.contains("/benches/")
        || path.starts_with("benches/")
    {
        ContentProvenance::Benchmark
    } else if path.contains("migration") || path.contains("/migrations/") {
        ContentProvenance::Migration
    } else if path.contains("/examples/")
        || path.starts_with("examples/")
        || path.contains("example")
    {
        ContentProvenance::Example
    } else if super::code::is_test_path(&path) {
        ContentProvenance::Test
    } else if path.starts_with("docs/")
        || std::path::Path::new(&path)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|extension| {
                ["md", "mdx", "rst", "adoc"]
                    .iter()
                    .any(|candidate| extension.eq_ignore_ascii_case(candidate))
            })
    {
        ContentProvenance::Documentation
    } else if CODE_EXTENSIONS.iter().any(|ext| path.ends_with(ext)) {
        ContentProvenance::Production
    } else {
        ContentProvenance::Unknown
    }
}

fn requested_provenance(query: &str) -> Option<ContentProvenance> {
    let query = query.to_ascii_lowercase();
    [
        ("generated", ContentProvenance::Generated),
        ("binding", ContentProvenance::Generated),
        ("fixture", ContentProvenance::Fixture),
        ("benchmark", ContentProvenance::Benchmark),
        ("vendor", ContentProvenance::Vendor),
        ("third party", ContentProvenance::Vendor),
        ("migration", ContentProvenance::Migration),
        ("example", ContentProvenance::Example),
        ("test", ContentProvenance::Test),
    ]
    .into_iter()
    .find_map(|(needle, provenance)| query.contains(needle).then_some(provenance))
}

fn provenance_boost(query: &str, provenance: ContentProvenance) -> f32 {
    if requested_provenance(query) == Some(provenance) {
        return 1.35;
    }
    match provenance {
        ContentProvenance::Production => 1.08,
        ContentProvenance::Documentation => 1.0,
        ContentProvenance::Test => 0.88,
        ContentProvenance::Generated => 0.78,
        ContentProvenance::Fixture => 0.72,
        ContentProvenance::Benchmark => 0.82,
        ContentProvenance::Vendor => 0.68,
        ContentProvenance::Migration | ContentProvenance::Example => 0.92,
        ContentProvenance::Unknown => 0.96,
    }
}

fn term_coverage(text: &str, query: &str) -> usize {
    let searchable = text.to_ascii_lowercase();
    query_terms(query)
        .iter()
        .filter(|term| searchable.contains(term.as_str()))
        .count()
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn query_match_byte(text: &str, query: &str) -> usize {
    let searchable = text.to_ascii_lowercase();
    let mut terms = query_terms(query);
    terms.sort_by_key(|term| std::cmp::Reverse(term.len()));
    terms
        .into_iter()
        .find_map(|term| searchable.find(&term))
        .unwrap_or(0)
}

fn query_window(text: &str, query: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }

    let match_at = query_match_byte(text, query);
    let markers = 8usize.min(max_bytes);
    let content_budget = max_bytes.saturating_sub(markers).max(1);
    let desired_start = match_at.saturating_sub(content_budget / 2);
    let mut start = floor_char_boundary(text, desired_start);
    let mut end = floor_char_boundary(text, start.saturating_add(content_budget));
    if end == text.len() {
        start = floor_char_boundary(text, text.len().saturating_sub(content_budget));
        end = text.len();
    } else {
        end = floor_char_boundary(text, start.saturating_add(content_budget));
    }

    let mut out = String::new();
    if start > 0 {
        out.push_str("…\n");
    }
    out.push_str(&text[start..end]);
    if end < text.len() {
        out.push_str("\n…");
    }
    while out.len() > max_bytes {
        let next = floor_char_boundary(&out, out.len().saturating_sub(1));
        out.truncate(next);
    }
    out
}

fn bounded_query_text(
    text: &str,
    query: &str,
    max_bytes: usize,
    max_tokens: usize,
) -> (String, bool) {
    if text.len() <= max_bytes && count_tokens(text) <= max_tokens {
        return (text.to_string(), false);
    }
    let mut byte_budget = max_bytes.min(text.len());
    let mut clipped = query_window(text, query, byte_budget);
    while count_tokens(&clipped) > max_tokens && byte_budget > 1 {
        byte_budget = (byte_budget * 3 / 4).max(1);
        clipped = query_window(text, query, byte_budget);
    }
    (clipped, true)
}

fn logical_family(content_id: &str) -> String {
    let mut family = content_id.to_string();
    for marker in [":c", "#claim-", "#part", "#overload-", "@overload-"] {
        if let Some((prefix, suffix)) = family.rsplit_once(marker)
            && suffix.chars().all(|character| character.is_ascii_digit())
        {
            family.truncate(prefix.len());
        }
    }
    if let Some(open) = family.rfind('(')
        && family.ends_with(')')
        && family.starts_with("sym-")
    {
        family.truncate(open);
    }
    let lower = family.to_ascii_lowercase();
    if lower.contains("/tests/")
        || lower.contains("/test/")
        || lower.contains("/generated/")
        || lower.contains("/fixtures/")
    {
        family = family
            .split("::")
            .map(|segment| {
                segment
                    .trim_end_matches(|character: char| character.is_ascii_digit())
                    .trim_end_matches(['_', '-', '#'])
            })
            .collect::<Vec<_>>()
            .join("::");
    }
    family
}

fn module_family(content_id: &str) -> String {
    let normalized = result_document_id(content_id).replace('\\', "/");
    let segments: Vec<&str> = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    if segments.is_empty() {
        return normalized;
    }
    if let Some(index) = segments.iter().position(|segment| {
        matches!(
            segment.to_ascii_lowercase().as_str(),
            "src" | "test" | "tests" | "crates" | "packages" | "vendor" | "generated"
        )
    }) {
        return segments[index..segments.len().min(index + 2)].join("/");
    }
    if normalized.starts_with('/') || normalized.get(1..2) == Some(":") {
        let start = segments.len().saturating_sub(3);
        return segments[start..segments.len().saturating_sub(1).max(start + 1)].join("/");
    }
    segments.into_iter().take(2).collect::<Vec<_>>().join("/")
}

fn is_narrow_query(query: &str) -> bool {
    query.contains("::")
        || query.contains("src/")
        || CODE_EXTENSIONS.iter().any(|ext| query.contains(ext))
        || query
            .split_whitespace()
            .any(|word| word.chars().skip(1).any(char::is_uppercase))
        || query_terms(query).len() <= 1
}

fn delivery_metadata(
    original: &str,
    returned: &str,
    representation: TextRepresentation,
    continuation: Option<ResultLocator>,
) -> TextDeliveryMetadata {
    TextDeliveryMetadata {
        truncated: returned != original || continuation.is_some(),
        original_bytes: original.len(),
        original_tokens: count_tokens(original),
        returned_bytes: returned.len(),
        returned_tokens: count_tokens(returned),
        representation,
        continuation,
    }
}

fn build_survey_result(
    query: &str,
    scored: ScoredResult,
    resolved: ResolvedContent,
    options: SurveyOptions,
) -> SurveyResult {
    let content_id = scored.vector_id.content_id().to_string();
    let continuation = resolved
        .continuation_resolution
        .map(|resolution| ResultLocator::primary(content_id.clone(), resolution.to_string()))
        .or_else(|| {
            (scored.resolution == Resolution::Claim).then(|| {
                ResultLocator::primary(
                    crate::types::parent_section_id(&content_id).unwrap_or(&content_id),
                    "section_full",
                )
            })
        });

    let excerpt = bounded_query_text(
        &resolved.body,
        query,
        options.max_result_bytes,
        options.max_result_tokens,
    );
    let excerpt_coverage = term_coverage(&excerpt.0, query);
    let preferred_summary = resolved.summary.as_ref().filter(|summary| {
        let coverage = term_coverage(summary, query);
        coverage > 0 && coverage >= excerpt_coverage
    });

    let (text, representation) = if let Some(summary) = preferred_summary {
        let (text, _) = bounded_query_text(
            summary,
            query,
            options.max_result_bytes,
            options.max_result_tokens,
        );
        (text, TextRepresentation::StoredSummary)
    } else if excerpt.1 {
        (excerpt.0, TextRepresentation::QueryExcerpt)
    } else {
        (excerpt.0, resolved.base_representation)
    };
    let continuation_needed =
        text != resolved.body || matches!(representation, TextRepresentation::SymbolStub);
    let text_metadata = delivery_metadata(
        &resolved.body,
        &text,
        representation,
        continuation.clone().filter(|_| continuation_needed),
    );
    let resolution = scored.resolution.to_string();
    let delivered_resolution = match (&text_metadata.representation, scored.resolution) {
        (TextRepresentation::StoredSummary, _) => "summary",
        (TextRepresentation::QueryExcerpt, Resolution::Section | Resolution::Summary) => {
            "section_excerpt"
        }
        (TextRepresentation::QueryExcerpt, Resolution::Claim) => "claim_excerpt",
        (_, Resolution::Section) => "section_full",
        (_, Resolution::Claim) => "claim",
        (_, Resolution::SymbolStub | Resolution::SymbolFull) => "symbol_stub",
        (_, Resolution::Summary) => "summary",
    };
    let mut explanation = scored.explanation;
    explanation.final_score = scored.score;

    SurveyResult {
        content_id: content_id.clone(),
        resolution: resolution.clone(),
        score: scored.score,
        text,
        heading_path: resolved.heading_path,
        source_corpus: None,
        locator: ResultLocator::primary(content_id.clone(), delivered_resolution),
        text_metadata,
        provenance: classify_content_provenance(&content_id),
        score_explanation: explanation,
    }
}

/// Reapply one aggregate discovery budget to an already-ranked result set.
///
/// This is used after cross-corpus merging so per-corpus bounds cannot add up
/// to an unbounded federated response. Existing routing context is preserved
/// on any continuation locator created by the aggregate clip.
#[must_use]
pub fn bound_survey_results(
    query: &str,
    results: Vec<SurveyResult>,
    options: SurveyOptions,
) -> Vec<SurveyResult> {
    let mut remaining_bytes = options.max_total_bytes;
    let mut remaining_tokens = options.max_total_tokens;
    let mut bounded = Vec::with_capacity(results.len());
    for mut result in results {
        if remaining_bytes == 0 || remaining_tokens == 0 {
            break;
        }
        if result.text.len() > remaining_bytes || count_tokens(&result.text) > remaining_tokens {
            let (text, _) =
                bounded_query_text(&result.text, query, remaining_bytes, remaining_tokens);
            if text.is_empty() {
                break;
            }
            result.text = text;
            result.text_metadata.truncated = true;
            result.text_metadata.representation = TextRepresentation::QueryExcerpt;
            result.text_metadata.returned_bytes = result.text.len();
            result.text_metadata.returned_tokens = count_tokens(&result.text);
            if result.text_metadata.continuation.is_none() {
                let mut continuation = result.locator.clone();
                if result.resolution == "claim" {
                    continuation.identity.content_id =
                        crate::types::parent_section_id(&result.content_id)
                            .unwrap_or(&result.content_id)
                            .to_string();
                    continuation.identity.resolution = "section_full".to_string();
                } else if result.resolution.starts_with("symbol_") {
                    continuation
                        .identity
                        .content_id
                        .clone_from(&result.content_id);
                    continuation.identity.resolution = "symbol_full".to_string();
                } else if result.resolution == "summary" {
                    continuation
                        .identity
                        .content_id
                        .clone_from(&result.content_id);
                    continuation.identity.resolution = "document_toc".to_string();
                } else {
                    continuation
                        .identity
                        .content_id
                        .clone_from(&result.content_id);
                    continuation.identity.resolution = "section_full".to_string();
                }
                result.text_metadata.continuation = Some(continuation);
            }
        }
        remaining_bytes = remaining_bytes.saturating_sub(result.text.len());
        remaining_tokens = remaining_tokens.saturating_sub(count_tokens(&result.text));
        bounded.push(result);
    }
    bounded
}

fn apply_total_survey_budget(
    query: &str,
    results: Vec<SurveyResult>,
    options: SurveyOptions,
) -> Vec<SurveyResult> {
    bound_survey_results(query, results, options)
}

fn route_result_and_check_exclusion(
    result: &mut SurveyResult,
    corpus_id: &str,
    exclude: &HashSet<DeliveryIdentity>,
) -> bool {
    result.locator.identity.corpus_id = corpus_id.to_string();
    if let Some(continuation) = &mut result.text_metadata.continuation {
        continuation.identity.corpus_id = corpus_id.to_string();
    }
    exclude.contains(&result.locator.identity)
}

/// Apply intent-aware authority, identifier matching, family deduplication,
/// and source diversity after semantic retrieval. Search scores alone are not
/// comparable across prose and code: prose has more natural-language surface
/// area and otherwise crowds symbols out of code-oriented queries.
#[allow(clippy::too_many_lines)]
fn shape_survey_results(
    query: &str,
    mut results: Vec<SurveyResult>,
    top_k: usize,
) -> Vec<SurveyResult> {
    if top_k == 0 || results.is_empty() {
        return Vec::new();
    }

    let intent = survey_intent(query);
    let terms = query_terms(query);
    for result in &mut results {
        let code = is_code_result(result);
        result.provenance = classify_content_provenance(&result.content_id);
        let intent_boost = match (intent, code) {
            (SurveyIntent::Code, true) => 1.25,
            (SurveyIntent::Code, false) => 0.88,
            (SurveyIntent::Documentation, true) => 0.90,
            (SurveyIntent::Documentation, false) => 1.12,
            (SurveyIntent::Mixed, true) => 1.06,
            (SurveyIntent::Mixed, false) => 1.0,
        };
        result.score *= intent_boost;
        result.score *= provenance_boost(query, result.provenance);

        let searchable = format!(
            "{} {}",
            result.content_id.to_ascii_lowercase(),
            result
                .heading_path
                .as_ref()
                .map(|p| p.join(" ").to_ascii_lowercase())
                .unwrap_or_default()
        );
        let identifier_segments: Vec<&str> = searchable
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|part| !part.is_empty())
            .collect();
        let matched = terms
            .iter()
            .filter(|term| searchable.contains(term.as_str()))
            .count();
        result.score *= 1.0 + (matched.min(3) as f32 * 0.06);
        result.score_explanation.exact_match = terms
            .iter()
            .any(|term| identifier_segments.iter().any(|part| *part == term));
        result.score_explanation.prefix_match = !result.score_explanation.exact_match
            && terms.iter().any(|term| {
                identifier_segments
                    .iter()
                    .any(|part| part.starts_with(term) || term.starts_with(part))
            });
        result.score_explanation.identifier_match = matched > 0;
        if result.score_explanation.exact_match {
            result.score *= 1.18;
        } else if result.score_explanation.prefix_match {
            result.score *= 1.09;
        }
        result.score_explanation.intent = Some(intent.as_str().to_string());
        result.score_explanation.intent_boost = Some(intent_boost);
        result.score_explanation.final_score = result.score;
    }
    results.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.content_id.cmp(&b.content_id))
    });
    let code_candidates: Vec<SurveyResult> = results
        .iter()
        .filter(|result| is_code_result(result))
        .cloned()
        .collect();

    // Deterministic novelty-aware selection across logical item, parent/file,
    // module, resolution, and provenance families. Exact identifiers remain
    // protected as the seed; narrow path/symbol queries relax file diversity.
    let per_document_cap = if is_narrow_query(query) {
        top_k.clamp(1, 4)
    } else if top_k <= 3 {
        1
    } else {
        2
    };
    let mut selected: Vec<SurveyResult> = Vec::with_capacity(top_k);
    let mut document_counts: HashMap<String, usize> = HashMap::new();
    let mut seen_ids = HashSet::new();
    results.retain(|result| seen_ids.insert(result.content_id.clone()));

    while selected.len() < top_k && !results.is_empty() {
        let mut best: Option<(usize, f32)> = None;
        for (index, candidate) in results.iter().enumerate() {
            let document = result_document_id(&candidate.content_id);
            if document_counts.get(document).copied().unwrap_or_default() >= per_document_cap {
                continue;
            }
            let logical = logical_family(&candidate.content_id);
            let module = module_family(&candidate.content_id);
            let mut penalty = 0.0f32;
            for chosen in &selected {
                if logical_family(&chosen.content_id) == logical {
                    penalty = penalty.max(0.45);
                }
                if result_document_id(&chosen.content_id) == document {
                    penalty = penalty.max(0.22);
                }
                if module_family(&chosen.content_id) == module {
                    penalty = penalty.max(0.10);
                }
                if chosen.resolution == candidate.resolution {
                    penalty += 0.015;
                }
                if chosen.provenance == candidate.provenance {
                    penalty += 0.01;
                }
            }
            let protected_exact = selected.is_empty() && candidate.score_explanation.exact_match;
            let novelty_score = if protected_exact {
                f32::INFINITY
            } else {
                candidate.score * (1.0 - penalty.min(0.65))
            };
            if best.is_none_or(|(_, current)| novelty_score > current) {
                best = Some((index, novelty_score));
            }
        }
        let Some((index, _)) = best else { break };
        let mut result = results.remove(index);
        *document_counts
            .entry(result_document_id(&result.content_id).to_string())
            .or_default() += 1;
        result.score_explanation.diversity_selected = true;
        selected.push(result);
    }

    // If strict family caps exhausted the candidate pool, fill deterministically
    // rather than returning an unexpectedly short page.
    while selected.len() < top_k && !results.is_empty() {
        let mut result = results.remove(0);
        result.score_explanation.diversity_selected = true;
        selected.push(result);
    }

    // A code-intent query should expose code whenever retrieval found it.
    // Reserve half the page for symbols, replacing the weakest prose hits.
    if intent == SurveyIntent::Code {
        let target = top_k.div_ceil(2);
        let mut code_count = selected.iter().filter(|r| is_code_result(r)).count();
        for candidate in &code_candidates {
            if code_count >= target {
                break;
            }
            if selected
                .iter()
                .any(|r| r.content_id == candidate.content_id)
            {
                continue;
            }
            if let Some(index) = selected.iter().rposition(|r| !is_code_result(r)) {
                let mut candidate = candidate.clone();
                candidate.score_explanation.quota_selected = true;
                selected[index] = candidate;
                code_count += 1;
            }
        }
    }

    selected.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.content_id.cmp(&b.content_id))
    });
    for result in &mut selected {
        result.score_explanation.final_score = result.score;
    }
    selected
}

/// How strongly Matryoshka full-dim rescoring overrides the prior (RRF-fused
/// dense + sparse + resolution-weighted) score. `0.7` = the new signal
/// dominates but the prior stays in the mix so sparse/lexical contributions
/// aren't erased.
const MATRYOSHKA_BLEND: f32 = 0.7;

/// How strongly cross-encoder reranking overrides the prior composed score.
/// `0.8` — the reranker is our best signal, but we keep some memory of the
/// upstream retrieval stack.
const RERANK_BLEND: f32 = 0.8;

/// F37 — maximum candidates the cross-encoder scores per query. Results
/// arrive at [`QueryService::rerank_results`] sorted by the prior composed
/// score, so pairs past the head add linear inference cost for vanishing
/// precision gain; anything deeper keeps its normalized prior. Bounds the
/// stage independently of the survey over-fetch (`top_k.max(10) * 3`),
/// which would otherwise scale pair count with `top_k`.
const CROSS_ENCODER_RERANK_DEPTH: usize = 20;

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
            neighbour.score_explanation.graph_expanded = true;
            neighbour.score_explanation.final_score = neighbour.score;
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
        self.survey_with_options(query, top_k, SurveyOptions::default())
            .await
    }

    /// Search with explicit per-result and total response-cost bounds.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError`] if embedding, search, or storage operations fail.
    #[instrument(skip(self), fields(query_len = query.len(), top_k))]
    pub async fn survey_with_options(
        &self,
        query: &str,
        top_k: usize,
        options: SurveyOptions,
    ) -> Result<Vec<SurveyResult>, QueryError> {
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
        // Always over-fetch: intent-aware balancing and source diversity need
        // alternatives even when no cross-encoder is configured.
        let search_top_k = top_k.max(10) * 3;
        let rerank_top_k = self.reranker.as_ref().map(|_| search_top_k);
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

            let resolved = self
                .resolve_content(&sr.vector_id, resolution)
                .await
                .unwrap_or_else(|_| ResolvedContent {
                    body: format!("[content unavailable: {content_id}]"),
                    summary: None,
                    heading_path: None,
                    base_representation: TextRepresentation::Full,
                    continuation_resolution: None,
                });

            // Skip unresolved placeholders (e.g. during indexing)
            if is_unresolved_placeholder(&resolved.body) {
                continue;
            }

            results.push(build_survey_result(query, sr, resolved, options));
        }

        // Apply cross-encoder reranking if configured
        if let Some(reranker) = &self.reranker {
            results = Self::rerank_results(query, results, search_top_k, reranker.as_ref())?;
        }

        let results = shape_survey_results(query, results, top_k);
        Ok(apply_total_survey_budget(query, results, options))
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
        self.survey_excluding_with_options(query, top_k, exclude_ids, SurveyOptions::default())
            .await
    }

    /// Exclusion-aware survey with explicit response-cost bounds.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError`] if embedding, search, or storage operations fail.
    #[instrument(skip(self, exclude_ids), fields(query_len = query.len(), top_k, exclude_count = exclude_ids.len()))]
    pub async fn survey_excluding_with_options(
        &self,
        query: &str,
        top_k: usize,
        exclude_ids: &HashSet<String>,
        options: SurveyOptions,
    ) -> Result<(Vec<SurveyResult>, usize), QueryError> {
        let (results, deduplicated_count, _) = self
            .survey_excluding_internal(query, top_k, Some(exclude_ids), None, options)
            .await?;
        Ok((results, deduplicated_count))
    }

    /// Corpus- and resolution-aware exclusion for daemon/linked transports.
    ///
    /// Only an exact `(corpus_id, content_id, delivered resolution)` match is
    /// suppressed. The legacy bare-ID methods remain intentionally broader.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError`] if embedding, search, or storage operations fail.
    #[instrument(skip(self, exclude), fields(query_len = query.len(), top_k, exclude_count = exclude.len(), corpus_id))]
    pub async fn survey_excluding_identities_with_options(
        &self,
        query: &str,
        top_k: usize,
        corpus_id: &str,
        exclude: &HashSet<DeliveryIdentity>,
        options: SurveyOptions,
    ) -> Result<(Vec<SurveyResult>, usize), QueryError> {
        let (results, deduplicated_count, _) = self
            .survey_excluding_internal(query, top_k, None, Some((corpus_id, exclude)), options)
            .await?;
        Ok((results, deduplicated_count))
    }

    /// Corpus-aware exclusion that also reports every exact identity suppressed.
    ///
    /// The detailed identities let transport/session layers account for the
    /// actual previously delivered token cost instead of estimating a uniform
    /// saving from the aggregate count.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError`] if embedding, search, or storage operations fail.
    #[instrument(skip(self, exclude), fields(query_len = query.len(), top_k, exclude_count = exclude.len(), corpus_id))]
    pub async fn survey_excluding_identities_detailed_with_options(
        &self,
        query: &str,
        top_k: usize,
        corpus_id: &str,
        exclude: &HashSet<DeliveryIdentity>,
        options: SurveyOptions,
    ) -> Result<(Vec<SurveyResult>, Vec<DeliveryIdentity>), QueryError> {
        let (results, _, suppressed_identities) = self
            .survey_excluding_internal(query, top_k, None, Some((corpus_id, exclude)), options)
            .await?;
        Ok((results, suppressed_identities))
    }

    async fn survey_excluding_internal(
        &self,
        query: &str,
        top_k: usize,
        bare_exclude: Option<&HashSet<String>>,
        identity_exclude: Option<(&str, &HashSet<DeliveryIdentity>)>,
        options: SurveyOptions,
    ) -> Result<(Vec<SurveyResult>, usize, Vec<DeliveryIdentity>), QueryError> {
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
        let mut suppressed_identities = Vec::new();

        // Collect the full candidate pool so balancing can recover code hidden
        // below prose and can diversify documents after exclusions.
        let collect_k = fetch_k;

        for sr in scored {
            let content_id = sr.vector_id.content_id().to_string();

            if bare_exclude.is_some_and(|exclude| exclude.contains(&content_id)) {
                deduplicated_count += 1;
                continue;
            }

            let resolution = sr.resolution;
            let resolved = self
                .resolve_content(&sr.vector_id, resolution)
                .await
                .unwrap_or_else(|_| ResolvedContent {
                    body: format!("[content unavailable: {content_id}]"),
                    summary: None,
                    heading_path: None,
                    base_representation: TextRepresentation::Full,
                    continuation_resolution: None,
                });

            // Skip unresolved placeholders (e.g. during indexing)
            if is_unresolved_placeholder(&resolved.body) {
                continue;
            }

            let mut result = build_survey_result(query, sr, resolved, options);
            if let Some((corpus_id, exclude)) = identity_exclude
                && route_result_and_check_exclusion(&mut result, corpus_id, exclude)
            {
                deduplicated_count += 1;
                suppressed_identities.push(result.locator.identity.clone());
                continue;
            }
            results.push(result);

            if results.len() >= collect_k {
                break;
            }
        }

        // Apply cross-encoder reranking if configured
        if let Some(reranker) = &self.reranker {
            results = Self::rerank_results(query, results, fetch_k, reranker.as_ref())?;
        }
        results = shape_survey_results(query, results, top_k);
        results = apply_total_survey_budget(query, results, options);

        Ok((results, deduplicated_count, suppressed_identities))
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

        // F37 — bound the cross-encoder to the HEAD of the candidate list.
        // `results` arrives sorted by the prior composed score (RRF +
        // Matryoshka), so the head is where cross-encoder precision pays;
        // inference cost is linear in pair count, and the survey over-fetch
        // (`top_k.max(10) * 3`) would otherwise scale the pair count with
        // top_k. Results past the depth keep their normalized prior.
        let depth = results.len().min(CROSS_ENCODER_RERANK_DEPTH);

        // Compute reranker scores (index-aligned to `results` input order).
        let texts: Vec<&str> = results[..depth].iter().map(|r| r.text.as_str()).collect();
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
                r.score_explanation.reranked = rerank_norm[i].is_some();
                r.score_explanation.final_score = r.score;
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
    #[allow(clippy::too_many_lines)]
    pub(super) async fn resolve_content(
        &self,
        vector_id: &VectorId,
        resolution: Resolution,
    ) -> Result<ResolvedContent, QueryError> {
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
                        Ok(ResolvedContent {
                            body: text,
                            summary: None,
                            heading_path: None,
                            base_representation: TextRepresentation::StoredSummary,
                            continuation_resolution: Some("document_toc"),
                        })
                    } else {
                        Err(QueryError::SectionNotFound {
                            id: content_id.to_string(),
                        })
                    }
                } else {
                    // Section summary — look up section record
                    let sid = SectionId(content_id.to_string());
                    if let Some(section) = self.storage.get_section(&sid).await? {
                        Ok(ResolvedContent {
                            body: section.text,
                            summary: section.summary,
                            heading_path: Some(section.heading_path),
                            base_representation: TextRepresentation::StoredSummary,
                            continuation_resolution: Some("section_full"),
                        })
                    } else {
                        Err(QueryError::SectionNotFound {
                            id: content_id.to_string(),
                        })
                    }
                }
            }
            Resolution::Section => {
                let sid = SectionId(content_id.to_string());
                if let Some(section) = self.storage.get_section(&sid).await? {
                    Ok(ResolvedContent {
                        body: section.text,
                        summary: section.summary,
                        heading_path: Some(section.heading_path),
                        base_representation: TextRepresentation::Full,
                        continuation_resolution: Some("section_full"),
                    })
                } else {
                    Err(QueryError::SectionNotFound {
                        id: content_id.to_string(),
                    })
                }
            }
            Resolution::Claim => {
                let cid = ClaimId(content_id.to_string());
                if let Some(claim) = self.storage.get_claim(&cid).await? {
                    Ok(ResolvedContent {
                        body: claim.text,
                        summary: None,
                        heading_path: None,
                        base_representation: TextRepresentation::Full,
                        continuation_resolution: None,
                    })
                } else {
                    Err(QueryError::ClaimNotFound {
                        id: content_id.to_string(),
                    })
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
                    Ok(ResolvedContent {
                        body: text,
                        summary: None,
                        heading_path: Some(heading),
                        base_representation: TextRepresentation::SymbolStub,
                        continuation_resolution: Some("symbol_full"),
                    })
                } else {
                    Err(QueryError::SymbolNotFound {
                        id: content_id.to_string(),
                    })
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
                    Ok(ResolvedContent {
                        body: text,
                        summary: None,
                        heading_path: Some(heading),
                        base_representation: TextRepresentation::SymbolStub,
                        continuation_resolution: Some("symbol_full"),
                    })
                } else {
                    Err(QueryError::SymbolNotFound {
                        id: content_id.to_string(),
                    })
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
            candidate.explanation.matryoshka = matryoshka_norm[i].is_some();
            candidate.explanation.final_score = candidate.score;
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
        CROSS_ENCODER_RERANK_DEPTH, GRAPH_EXPAND_DECAY, QueryService, RERANK_BLEND,
        ResolvedContent, SurveyResult, apply_total_survey_budget, bounded_query_text,
        build_survey_result, classify_content_provenance, graph_expand_results, module_family,
        result_document_id, route_result_and_check_exclusion, shape_survey_results,
    };
    use crate::embedding::{DualEmbedder, DualEmbeddings, Embedder, RerankScore, Reranker};
    use crate::error::IndexError;
    use crate::index::HnswIndex;
    use crate::search::ScoredResult;
    use crate::service::SurveyOptions;
    use crate::storage::SqliteStorage;
    use crate::types::{
        ContentProvenance, DeliveryIdentity, ResultLocator, ScoreExplanation, TextRepresentation,
        VectorId,
    };
    use std::collections::HashSet;

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

    struct FixedDualEmbedder;

    impl Embedder for FixedDualEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
            Ok(texts.iter().map(|_| vec![1.0]).collect())
        }

        fn dimension(&self) -> usize {
            1
        }
    }

    impl DualEmbedder for FixedDualEmbedder {
        fn embed_dual(&self, texts: &[&str]) -> Result<DualEmbeddings, IndexError> {
            Ok(DualEmbeddings {
                truncated: texts.iter().map(|_| vec![1.0]).collect(),
                full: texts.iter().map(|_| vec![1.0, 0.0]).collect(),
            })
        }

        fn full_dimension(&self) -> usize {
            2
        }
    }

    fn sr(content_id: &str, score: f32) -> SurveyResult {
        let text = format!("text for {content_id}");
        SurveyResult {
            content_id: content_id.to_string(),
            resolution: "section".to_string(),
            score,
            text: text.clone(),
            heading_path: None,
            source_corpus: None,
            locator: crate::types::ResultLocator::primary(content_id, "section"),
            text_metadata: crate::types::TextDeliveryMetadata {
                truncated: false,
                original_bytes: text.len(),
                original_tokens: crate::token::count_tokens(&text),
                returned_bytes: text.len(),
                returned_tokens: crate::token::count_tokens(&text),
                representation: crate::types::TextRepresentation::Full,
                continuation: None,
            },
            provenance: crate::types::ContentProvenance::Unknown,
            score_explanation: ScoreExplanation {
                final_score: score,
                ..ScoreExplanation::default()
            },
        }
    }

    fn symbol(content_id: &str, score: f32) -> SurveyResult {
        SurveyResult {
            resolution: "symbol_full".to_string(),
            ..sr(content_id, score)
        }
    }

    fn scored(vector_id: &str, score: f32) -> ScoredResult {
        let vector_id = VectorId::parse(vector_id).unwrap();
        ScoredResult {
            resolution: vector_id.resolution(),
            vector_id,
            raw_distance: 1.0 - score,
            score,
            explanation: ScoreExplanation {
                dense_rank: Some(1),
                dense_score: Some(score),
                final_score: score,
                ..ScoreExplanation::default()
            },
        }
    }

    #[test]
    fn excerpts_are_query_centered_at_beginning_middle_and_end_and_unicode_safe() {
        let unicode = "前置🙂 ".repeat(100);
        for text in [
            format!("needle {unicode}"),
            format!("{unicode} needle {unicode}"),
            format!("{unicode} needle"),
        ] {
            let (excerpt, truncated) = bounded_query_text(&text, "needle", 96, 64);
            assert!(truncated);
            assert!(excerpt.contains("needle"));
            assert!(excerpt.len() <= 96);
            assert!(std::str::from_utf8(excerpt.as_bytes()).is_ok());
        }
    }

    #[test]
    fn diversity_families_preserve_windows_drives_and_absolute_modules() {
        assert_eq!(
            result_document_id(r"sym-C:\repo\src\router.rs::Router::run"),
            r"C:\repo\src\router.rs"
        );
        assert_eq!(
            module_family(r"sym-C:\repo\src\router.rs::Router::run"),
            "src/router.rs"
        );
        assert_eq!(
            module_family("sym-/Users/alrik/Code/project/crates/core/src/lib.rs::run"),
            "crates/core"
        );
        assert_ne!(
            module_family("sym-/Users/a/Code/one/src/router.rs::run"),
            module_family("sym-/Users/a/Code/two/tests/router.rs::run")
        );
    }

    #[test]
    fn representative_stored_summary_beats_a_poor_body_excerpt() {
        let body = "implementation details with no requested phrase ".repeat(100);
        let result = build_survey_result(
            "quorum coordinator",
            scored("section::docs/coordination.md#quorum", 0.9),
            ResolvedContent {
                body: body.clone(),
                summary: Some("The quorum coordinator selects a leader.".to_string()),
                heading_path: Some(vec!["Coordination".to_string()]),
                base_representation: TextRepresentation::Full,
                continuation_resolution: Some("section_full"),
            },
            SurveyOptions {
                max_result_bytes: 128,
                max_result_tokens: 64,
                ..SurveyOptions::default()
            },
        );
        assert_eq!(
            result.text_metadata.representation,
            TextRepresentation::StoredSummary
        );
        assert!(result.text.contains("quorum coordinator"));
        assert!(result.text_metadata.truncated);
        assert_eq!(result.text_metadata.original_bytes, body.len());
        assert!(result.text_metadata.continuation.is_some());
    }

    #[test]
    fn typed_exclusion_is_exact_by_corpus_and_delivered_resolution() {
        let mut result = build_survey_result(
            "needle",
            scored("section::docs/large.md#needle", 0.9),
            ResolvedContent {
                body: format!("{}needle{}", "前🙂".repeat(100), "後🙂".repeat(100)),
                summary: None,
                heading_path: None,
                base_representation: TextRepresentation::Full,
                continuation_resolution: Some("section_full"),
            },
            SurveyOptions {
                max_result_bytes: 96,
                max_result_tokens: 64,
                ..SurveyOptions::default()
            },
        );
        assert_eq!(result.locator.identity.resolution, "section_excerpt");

        let different_corpus = HashSet::from([DeliveryIdentity::new(
            "other",
            result.content_id.clone(),
            "section_excerpt",
        )]);
        assert!(!route_result_and_check_exclusion(
            &mut result,
            "linked",
            &different_corpus
        ));

        let different_resolution = HashSet::from([DeliveryIdentity::new(
            "linked",
            result.content_id.clone(),
            "section_full",
        )]);
        assert!(!route_result_and_check_exclusion(
            &mut result,
            "linked",
            &different_resolution
        ));

        let exact = HashSet::from([DeliveryIdentity::new(
            "linked",
            result.content_id.clone(),
            "section_excerpt",
        )]);
        assert!(route_result_and_check_exclusion(
            &mut result,
            "linked",
            &exact
        ));
    }

    #[test]
    fn total_survey_budget_is_a_hard_unicode_safe_gate() {
        let mut results = vec![sr("docs/a.md#one", 0.9), sr("src/a.rs#two", 0.8)];
        for result in &mut results {
            result.text = "資料🙂 bounded response ".repeat(50);
            result.text_metadata.original_bytes = result.text.len();
            result.text_metadata.returned_bytes = result.text.len();
        }
        results[0].locator = ResultLocator::primary("docs/a.md#one", "section_excerpt").routed(
            "linked-corpus",
            Some("linked-label".into()),
            Some("atlas/project".into()),
            Some("tenant-a".into()),
        );
        let out = apply_total_survey_budget(
            "bounded response",
            results,
            SurveyOptions {
                max_result_bytes: 512,
                max_result_tokens: 128,
                max_total_bytes: 180,
                max_total_tokens: 80,
            },
        );
        assert!(out.iter().map(|result| result.text.len()).sum::<usize>() <= 180);
        assert!(out.iter().all(|result| result.text_metadata.truncated));
        let continuation = out[0]
            .text_metadata
            .continuation
            .as_ref()
            .expect("aggregate clipping must expose a routed continuation");
        assert_eq!(continuation.identity.corpus_id, "linked-corpus");
        assert_eq!(continuation.project.as_deref(), Some("linked-label"));
        assert_eq!(continuation.source_corpus.as_deref(), Some("atlas/project"));
        assert_eq!(continuation.tenant.as_deref(), Some("tenant-a"));
    }

    #[test]
    fn provenance_classifier_covers_trust_classes() {
        assert_eq!(
            classify_content_provenance("src/service.rs#run"),
            ContentProvenance::Production
        );
        assert_eq!(
            classify_content_provenance("tests/service_test.rs#run"),
            ContentProvenance::Test
        );
        assert_eq!(
            classify_content_provenance("src/generated/bindings.rs#ffi"),
            ContentProvenance::Generated
        );
        assert_eq!(
            classify_content_provenance("vendor/pkg/lib.rs#run"),
            ContentProvenance::Vendor
        );
    }

    #[test]
    fn vendor_is_suppressed_unless_the_query_explicitly_requests_it() {
        let ordinary = shape_survey_results(
            "run implementation",
            vec![
                symbol("sym-vendor/pkg/runtime.rs::run", 0.99),
                symbol("sym-src/runtime.rs::run", 0.78),
            ],
            2,
        );
        assert_eq!(ordinary[0].provenance, ContentProvenance::Production);

        let explicit = shape_survey_results(
            "vendor pkg runtime run",
            vec![
                symbol("sym-vendor/pkg/runtime.rs::run", 0.82),
                symbol("sym-src/runtime.rs::run", 0.82),
            ],
            2,
        );
        assert_eq!(explicit[0].provenance, ContentProvenance::Vendor);
    }

    #[test]
    fn provenance_suppression_yields_to_explicit_intent() {
        let ordinary = shape_survey_results(
            "parser implementation",
            vec![
                symbol("sym-tests/parser_test.rs::parser", 0.91),
                symbol("sym-src/parser.rs::parser", 0.82),
            ],
            2,
        );
        assert_eq!(ordinary[0].provenance, ContentProvenance::Production);

        let explicit = shape_survey_results(
            "parser test implementation",
            vec![
                symbol("sym-tests/parser_test.rs::parser", 0.80),
                symbol("sym-src/parser.rs::parser", 0.82),
            ],
            2,
        );
        assert_eq!(explicit[0].provenance, ContentProvenance::Test);

        let generated = shape_survey_results(
            "generated bindings symbol",
            vec![
                symbol("sym-src/generated/bindings.rs::call", 0.75),
                symbol("sym-src/runtime.rs::call", 0.90),
            ],
            2,
        );
        assert_eq!(generated[0].provenance, ContentProvenance::Generated);
    }

    #[test]
    fn score_explanation_tracks_identifier_intent_and_final_score() {
        let result = shape_survey_results(
            "QueryService::survey",
            vec![symbol("sym-src/query.rs::QueryService::survey", 0.8)],
            1,
        )
        .remove(0);
        assert!(result.score_explanation.exact_match);
        assert!(result.score_explanation.identifier_match);
        assert_eq!(result.score_explanation.intent.as_deref(), Some("code"));
        assert!((result.score_explanation.final_score - result.score).abs() < f32::EPSILON);
        assert!(result.score_explanation.diversity_selected);
    }

    #[test]
    fn logical_family_diversity_resists_repeated_claims_and_generated_wrappers() {
        let mut candidates = (0..10)
            .map(|index| {
                sr(
                    &format!("docs/repeated.md#same:c{index}"),
                    1.0 - index as f32 * 0.01,
                )
            })
            .collect::<Vec<_>>();
        candidates.extend([
            sr("docs/other.md#answer", 0.78),
            symbol("sym-src/generated/wrappers.rs::wrapper_a", 0.85),
            symbol("sym-src/real.rs::real_answer", 0.76),
        ]);
        let out = shape_survey_results("find answer implementation", candidates, 5);
        assert!(
            out.iter()
                .any(|result| result.content_id == "docs/other.md#answer")
        );
        assert!(
            out.iter()
                .any(|result| result.content_id == "sym-src/real.rs::real_answer")
        );
        assert!(
            out.iter()
                .filter(|result| result.content_id.starts_with("docs/repeated.md"))
                .count()
                <= 2
        );
    }

    #[test]
    fn diversity_resists_ten_near_identical_tests_and_generated_wrappers() {
        let mut candidates = (0..10)
            .map(|index| {
                symbol(
                    &format!("sym-tests/generated/dispatch_test_{index}::run"),
                    0.99 - index as f32 * 0.005,
                )
            })
            .collect::<Vec<_>>();
        candidates.extend((0..10).map(|index| {
            symbol(
                &format!("sym-src/generated/wrapper_{index}::dispatch"),
                0.92 - index as f32 * 0.005,
            )
        }));
        candidates.extend([
            symbol("sym-src/runtime/dispatcher.rs::dispatch", 0.86),
            symbol("sym-src/http/router.rs::dispatch_request", 0.84),
            sr("docs/dispatch.md#contract", 0.82),
        ]);

        let out = shape_survey_results("find dispatch implementation and contract", candidates, 6);
        assert!(
            out.iter()
                .any(|item| item.content_id.contains("dispatcher.rs"))
        );
        assert!(out.iter().any(|item| item.content_id.contains("router.rs")));
        assert!(
            out.iter()
                .any(|item| item.content_id == "docs/dispatch.md#contract")
        );
        assert!(
            out.iter()
                .filter(|item| item.content_id.contains("dispatch_test_"))
                .count()
                <= 2
        );
        assert!(
            out.iter()
                .filter(|item| item.content_id.contains("generated/wrapper_"))
                .count()
                <= 2
        );
    }

    #[test]
    fn narrow_overload_query_keeps_multiple_overloads_and_a_sibling() {
        let candidates = vec![
            symbol("sym-src/parser.rs::parse(str)#overload-1", 0.99),
            symbol("sym-src/parser.rs::parse(bytes)#overload-2", 0.98),
            symbol("sym-src/parser.rs::parse(reader)#overload-3", 0.97),
            symbol("sym-src/parser.rs::parse_config", 0.91),
            symbol("sym-src/lexer.rs::tokenize", 0.89),
        ];
        let out = shape_survey_results("parser.rs::parse", candidates, 4);
        assert!(
            out.iter()
                .filter(|item| item.content_id.contains("::parse("))
                .count()
                >= 2,
            "a narrow exact query may legitimately need several overloads: {out:?}"
        );
        assert!(
            out.iter()
                .any(|item| item.content_id.contains("parse_config"))
        );
    }

    #[test]
    fn code_queries_cannot_be_drowned_out_by_prose() {
        let mut candidates = (0..8)
            .map(|i| {
                sr(
                    &format!("docs/guide.md#section-{i}"),
                    0.99 - i as f32 * 0.01,
                )
            })
            .collect::<Vec<_>>();
        candidates.extend([
            symbol("sym-src/search.rs::survey", 0.70),
            symbol("sym-src/ranking.rs::rank", 0.69),
            symbol("sym-src/query.rs::Query", 0.68),
        ]);

        let out = shape_survey_results("survey implementation function ranking", candidates, 5);
        assert_eq!(out.len(), 5);
        assert!(
            out.iter()
                .filter(|result| result.resolution.starts_with("symbol_"))
                .count()
                >= 3,
            "at least half of a code-intent result page should be code"
        );
    }

    #[test]
    fn survey_limits_repeated_resolutions_from_one_document() {
        let candidates = vec![
            sr("docs/search.md#overview", 0.99),
            sr("docs/search.md#overview:c0", 0.98),
            sr("docs/search.md#overview:c1", 0.97),
            sr("docs/config.md#weights", 0.80),
            sr("README.md#search", 0.70),
        ];
        let out = shape_survey_results("search documentation", candidates, 4);
        assert_eq!(out.len(), 4);
        assert_eq!(
            out.iter()
                .filter(|result| result.content_id.starts_with("docs/search.md"))
                .count(),
            2
        );
    }

    #[test]
    fn identifier_matches_break_close_semantic_ties() {
        let out = shape_survey_results(
            "QueryService survey implementation",
            vec![
                symbol("sym-src/unrelated.rs::search", 0.80),
                symbol("sym-src/query.rs::QueryService::survey", 0.79),
            ],
            2,
        );
        assert_eq!(out[0].content_id, "sym-src/query.rs::QueryService::survey");
    }

    #[test]
    fn source_sections_count_as_code_not_documentation() {
        let out = shape_survey_results(
            "search implementation code",
            vec![
                sr("docs/search.md#overview", 0.95),
                sr("src/search.rs#MultiResolutionSearch#part0", 0.70),
            ],
            2,
        );
        assert_eq!(
            out[0].content_id,
            "src/search.rs#MultiResolutionSearch#part0"
        );
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
        assert!(out.iter().all(|result| result.score_explanation.reranked));
        assert!(out.iter().all(|result| {
            (result.score_explanation.final_score - result.score).abs() < f32::EPSILON
        }));
    }

    /// F37 — the cross-encoder must only see the head of the candidate list
    /// ([`CROSS_ENCODER_RERANK_DEPTH`] pairs), never the full over-fetched
    /// set; deeper results keep their normalized prior and stay unflagged.
    #[test]
    fn rerank_depth_caps_cross_encoder_pairs() {
        struct CountingReranker {
            seen: std::sync::Mutex<usize>,
        }
        impl Reranker for CountingReranker {
            fn rerank(
                &self,
                _query: &str,
                documents: &[&str],
            ) -> Result<Vec<RerankScore>, IndexError> {
                *self.seen.lock().unwrap() = documents.len();
                Ok(documents
                    .iter()
                    .enumerate()
                    .map(|(index, _)| RerankScore { index, score: 0.5 })
                    .collect())
            }
        }

        let n = CROSS_ENCODER_RERANK_DEPTH + 5;
        let results: Vec<SurveyResult> = (0..n)
            .map(|i| sr(&format!("r{i}"), 1.0 - (i as f32) / (n as f32)))
            .collect();
        let reranker = CountingReranker {
            seen: std::sync::Mutex::new(0),
        };

        let out = QueryService::rerank_results("query", results, n, &reranker).unwrap();

        assert_eq!(
            *reranker.seen.lock().unwrap(),
            CROSS_ENCODER_RERANK_DEPTH,
            "cross-encoder must see exactly the depth-capped head"
        );
        assert_eq!(out.len(), n);
        assert_eq!(
            out.iter().filter(|r| r.score_explanation.reranked).count(),
            CROSS_ENCODER_RERANK_DEPTH,
            "only head results are flagged reranked"
        );
    }

    #[tokio::test]
    async fn matryoshka_explanation_matches_the_rescored_order() {
        let storage = SqliteStorage::open_in_memory().unwrap();
        storage
            .store_full_dim_vectors(&[
                ("section::A".to_string(), vec![0.0, 1.0]),
                ("section::B".to_string(), vec![1.0, 0.0]),
            ])
            .await
            .unwrap();
        let embedder = std::sync::Arc::new(FixedDualEmbedder);
        let service = QueryService::new(
            storage,
            embedder.clone(),
            std::sync::Arc::new(HnswIndex::new(1, 8).unwrap()),
        );

        let rescored = service
            .rescore_with_full_dim(
                "query",
                vec![scored("section::A", 0.9), scored("section::B", 0.1)],
                embedder.as_ref(),
            )
            .await
            .unwrap();

        assert_eq!(rescored[0].vector_id.as_str(), "section::B");
        assert!(rescored.iter().all(|result| result.explanation.matryoshka));
        assert!(rescored.iter().all(|result| {
            (result.explanation.final_score - result.score).abs() < f32::EPSILON
        }));
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
