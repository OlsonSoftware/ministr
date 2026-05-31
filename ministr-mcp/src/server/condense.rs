//! Token-budgeted, fidelity-preserving condensation of MCP tool results.
//!
//! Claude Code (and most MCP clients) cap a single tool result at a fixed
//! token budget — 25,000 tokens by default — and *hard-truncate* anything
//! larger, dumping it to a side file. For a code-intelligence server whose
//! whole value is getting an agent to the right context fast, a truncated
//! trailhead (`ministr_toc`, `ministr_survey`, `ministr_read`) is worse than
//! useless: the agent can't even tell what it's missing.
//!
//! This module guarantees **no tool result exceeds the budget** while
//! preserving as much signal as possible — it *condenses*, it does not blindly
//! truncate. The discipline:
//!
//! - **Cheap, high-signal fields are kept whole** (ids, scores, paths, kinds,
//!   counts) — they cost almost nothing and are what the agent navigates by.
//! - **Long free-text fields are reduced with the existing TF-IDF extractive
//!   summarizer** ([`ExtractiveSummaryGenerator`]) — information-dense
//!   sentences in original order — with a head+tail character clip as a
//!   guaranteed backstop for text the summarizer can't shrink (e.g. code with
//!   no sentence boundaries).
//! - **Arrays shrink their elements' text first**, and only trim tail elements
//!   as a last resort, recording how many were omitted.
//! - Every condensed result carries a top-level **`_condensed`** envelope
//!   (`budget_tokens`, `original_tokens`, `final_tokens`, `reduced_fields`,
//!   `omitted_elements`, `strategy`, `hint`) so the agent knows it was
//!   condensed and exactly how to get full fidelity (paginate, narrow the id,
//!   use `ministr_extract`/`ministr_definition`).
//!
//! Results already within budget pass through byte-for-byte unchanged.

use ministr_core::extraction::summary::{ExtractiveSummaryGenerator, SummaryGenerator};
use ministr_core::token::count_tokens;
use serde_json::{Map, Value, json};

/// Default per-result token budget. Deliberately well under the common
/// 25,000-token client cap: [`count_tokens`] is an approximation and the
/// client's tokenizer may count more, so we keep generous headroom plus room
/// for the `_condensed` envelope.
pub(crate) const DEFAULT_MAX_OUTPUT_TOKENS: usize = 22_000;

/// Hard floor for a usable budget — below this, condensation can't preserve
/// anything meaningful, so we refuse smaller env overrides.
const MIN_BUDGET: usize = 512;

/// Resolve the active output token budget: `MINISTR_MAX_OUTPUT_TOKENS` if set
/// to a sane value, else [`DEFAULT_MAX_OUTPUT_TOKENS`].
pub(crate) fn output_budget_tokens() -> usize {
    std::env::var("MINISTR_MAX_OUTPUT_TOKENS")
        .ok()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|&n| n >= MIN_BUDGET)
        .unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS)
}

/// Approximate token cost of a JSON value via its serialized form. Consistent
/// across tools (matches how the daemon meters response tokens).
fn value_tokens(v: &Value) -> usize {
    count_tokens(&serde_json::to_string(v).unwrap_or_default())
}

/// Running tally of what condensation did, surfaced in the `_condensed`
/// envelope so the reduction is auditable.
#[derive(Debug, Default)]
struct Stats {
    reduced_fields: usize,
    omitted_elements: usize,
}

/// Condense `value` so its serialized form fits within `budget` tokens,
/// preserving as much fidelity as possible. Returns the value unchanged when
/// it already fits; otherwise returns a value carrying a top-level
/// `_condensed` envelope describing the reduction.
#[must_use]
pub(crate) fn fit_to_budget(value: Value, budget: usize) -> Value {
    let original = value_tokens(&value);
    if original <= budget {
        return value;
    }

    // Reserve room for the `_condensed` envelope so the final result (payload
    // + envelope) still fits the caller's budget.
    let target = budget.saturating_sub(ENVELOPE_RESERVE).max(MIN_BUDGET / 2);

    let mut stats = Stats::default();
    let mut reduced = fit(value, target, &mut stats);
    enforce_hard_cap(&mut reduced, target, &mut stats);

    let final_tokens = value_tokens(&reduced);
    let meta = json!({
        "budget_tokens": budget,
        "original_tokens": original,
        "final_tokens": final_tokens,
        "reduced_fields": stats.reduced_fields,
        "omitted_elements": stats.omitted_elements,
        "strategy": "extractive-summary + structural-trim",
        "hint": "Output exceeded the MCP token budget and was condensed (not truncated). \
                 For full fidelity, narrow the request: pass offset/limit to paginate, a more \
                 specific section_id/document_id, ministr_extract for a section's claims, or \
                 ministr_definition for a single symbol.",
    });

    match reduced {
        Value::Object(mut m) => {
            m.insert("_condensed".to_string(), meta);
            Value::Object(m)
        }
        other => json!({ "_condensed": meta, "result": other }),
    }
}

/// Token headroom reserved for the `_condensed` envelope.
const ENVELOPE_RESERVE: usize = 220;

/// Recursively reduce `value` toward `budget` tokens. Structure is preserved;
/// only oversized leaves and arrays are reduced.
fn fit(value: Value, budget: usize, stats: &mut Stats) -> Value {
    if value_tokens(&value) <= budget {
        return value;
    }
    match value {
        Value::String(s) => {
            stats.reduced_fields += 1;
            Value::String(reduce_text(&s, budget))
        }
        Value::Array(items) => fit_array(items, budget, stats),
        Value::Object(map) => fit_object(map, budget, stats),
        // Numbers / bools / null are tiny and irreducible.
        other => other,
    }
}

/// Fit an object: spend the budget cheapest-field-first so small, high-signal
/// fields (ids, scores, counts) survive whole and only the heavy fields are
/// reduced, each getting a fair share of what's left.
fn fit_object(map: Map<String, Value>, budget: usize, stats: &mut Stats) -> Value {
    let mut entries: Vec<(String, Value, usize)> = map
        .into_iter()
        .map(|(k, v)| {
            let cost = value_tokens(&v);
            (k, v, cost)
        })
        .collect();
    entries.sort_by_key(|(_, _, cost)| *cost);

    let total = entries.len();
    let mut remaining = budget;
    let mut out = Map::new();
    for (i, (k, v, cost)) in entries.into_iter().enumerate() {
        let key_overhead = count_tokens(&k) + 4;
        remaining = remaining.saturating_sub(key_overhead);
        if cost <= remaining {
            remaining -= cost;
            out.insert(k, v);
        } else {
            let fields_left = total - i;
            let share = (remaining / fields_left).max(8);
            let reduced = fit(v, share, stats);
            remaining = remaining.saturating_sub(value_tokens(&reduced));
            out.insert(k, reduced);
        }
    }
    Value::Object(out)
}

/// Fit an array: prefer to keep every element and shrink their text; only when
/// the element *count* itself is the problem do we trim the tail (recording
/// how many were dropped).
fn fit_array(items: Vec<Value>, budget: usize, stats: &mut Stats) -> Value {
    let len = items.len();
    if len == 0 {
        return Value::Array(items);
    }
    let total: usize = items.iter().map(value_tokens).sum();
    let avg = total / len;

    if avg <= SMALL_ROW_TOKENS {
        // Many small rows — the count is the cost. Keep a prefix that fits.
        let per = avg.max(1) + 2;
        let keep = (budget / per).max(1).min(len);
        if keep < len {
            stats.omitted_elements += len - keep;
        }
        Value::Array(items.into_iter().take(keep).collect())
    } else {
        // Heavy rows — cap the count so each survivor gets a workable share,
        // then reduce each survivor's contents.
        let max_keep = (budget / HEAVY_ROW_FLOOR).max(1).min(len);
        if max_keep < len {
            stats.omitted_elements += len - max_keep;
        }
        let share = (budget / max_keep).max(16);
        Value::Array(
            items
                .into_iter()
                .take(max_keep)
                .map(|v| fit(v, share, stats))
                .collect(),
        )
    }
}

/// Average element token cost at or below which an array is treated as "many
/// small rows" (trim count) rather than "few heavy rows" (reduce contents).
const SMALL_ROW_TOKENS: usize = 48;

/// Minimum token share a retained heavy row should get.
const HEAVY_ROW_FLOOR: usize = 64;

/// Reduce a long string toward `budget` tokens: extractive TF-IDF summary
/// first (keeps the most informative sentences in order), with a head+tail
/// character clip as a guaranteed backstop.
fn reduce_text(s: &str, budget: usize) -> String {
    if count_tokens(s) <= budget {
        return s.to_string();
    }
    // ~18 tokens/sentence is a reasonable target granularity.
    let max_sentences = (budget / 18).max(1);
    let summary = ExtractiveSummaryGenerator::new().summarize(s, max_sentences);
    let candidate = if summary.trim().is_empty() {
        s.to_string()
    } else {
        summary
    };
    if count_tokens(&candidate) <= budget {
        return candidate;
    }
    clip_to_tokens(&candidate, budget)
}

/// Clip `s` to at most `budget` tokens by keeping a head and a tail with an
/// elision marker between. Guaranteed to return something within budget.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "char-budget estimation is heuristic and re-verified by the loop below"
)]
fn clip_to_tokens(s: &str, budget: usize) -> String {
    let toks = count_tokens(s);
    if toks <= budget {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let nchars = chars.len();
    let chars_per_token = (s.len() as f64 / toks.max(1) as f64).max(1.0);
    let mut target_chars = ((budget as f64) * chars_per_token * 0.85) as usize;
    loop {
        if target_chars >= nchars {
            return s.to_string();
        }
        let candidate = head_tail(&chars, target_chars);
        if count_tokens(&candidate) <= budget || target_chars <= 32 {
            return candidate;
        }
        target_chars = target_chars * 4 / 5;
    }
}

/// Build a `head … [elided] … tail` string keeping ~`target_chars` characters.
fn head_tail(chars: &[char], target_chars: usize) -> String {
    let n = chars.len();
    if target_chars >= n {
        return chars.iter().collect();
    }
    let head_len = (target_chars * 4 / 5).max(1);
    let tail_len = target_chars.saturating_sub(head_len);
    let elided = n.saturating_sub(head_len + tail_len);
    let head: String = chars[..head_len].iter().collect();
    let tail: String = chars[n - tail_len..].iter().collect();
    format!("{head}\n…[condensed: {elided} chars elided]…\n{tail}")
}

/// Final guarantee that `value` fits `budget`: repeatedly halve the single
/// largest reducible contributor until it fits. Each pass at least halves the
/// dominant node, so this converges quickly; the iteration cap is a backstop.
fn enforce_hard_cap(value: &mut Value, budget: usize, stats: &mut Stats) {
    let mut guard = 0;
    while value_tokens(value) > budget && guard < 256 {
        if !reduce_largest(value, stats) {
            break;
        }
        guard += 1;
    }
}

/// Reduce the single largest reducible node in `value` by roughly half.
/// Returns `false` when nothing further can be reduced.
fn reduce_largest(value: &mut Value, stats: &mut Stats) -> bool {
    match value {
        Value::String(s) => {
            let t = count_tokens(s);
            if t <= 8 {
                return false;
            }
            *s = clip_to_tokens(s, t / 2);
            stats.reduced_fields += 1;
            true
        }
        Value::Array(items) => {
            if items.is_empty() {
                return false;
            }
            let total: usize = items.iter().map(value_tokens).sum();
            let (idx, biggest) = items
                .iter()
                .enumerate()
                .map(|(i, x)| (i, value_tokens(x)))
                .max_by_key(|(_, t)| *t)
                .unwrap_or((0, 0));
            // If one element dominates, drill into it; otherwise the length is
            // the cost, so trim the tail.
            if biggest * 3 >= total && reduce_largest(&mut items[idx], stats) {
                true
            } else if items.len() > 1 {
                let new_len = (items.len() / 2).max(1);
                stats.omitted_elements += items.len() - new_len;
                items.truncate(new_len);
                true
            } else {
                reduce_largest(&mut items[idx], stats)
            }
        }
        Value::Object(map) => {
            let key = map
                .iter()
                .map(|(k, x)| (k.clone(), value_tokens(x)))
                .max_by_key(|(_, t)| *t)
                .map(|(k, _)| k);
            match key.and_then(|k| map.get_mut(&k)) {
                Some(child) => reduce_largest(child, stats),
                None => false,
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(v: &Value) -> usize {
        value_tokens(v)
    }

    #[test]
    fn within_budget_is_unchanged() {
        let v = json!({ "results": [{"id": "a", "score": 0.9}], "total": 1 });
        let out = fit_to_budget(v.clone(), 22_000);
        assert_eq!(out, v, "small payloads must pass through byte-for-byte");
        assert!(out.get("_condensed").is_none());
    }

    #[test]
    fn oversized_text_field_is_condensed_under_budget() {
        // A huge text field that blows a tiny budget.
        let big = "Rust is a systems programming language. ".repeat(4000);
        let v = json!({
            "section_id": "docs/huge.md#root",
            "heading_path": ["docs", "huge"],
            "text": big,
            "claims_available": 7,
        });
        let original = tokens(&v);
        let budget = 800;
        assert!(original > budget, "fixture must exceed the budget");

        let out = fit_to_budget(v, budget);
        assert!(
            tokens(&out) <= budget,
            "condensed result must fit the budget, got {} > {budget}",
            tokens(&out)
        );
        // High-signal fields survive intact.
        assert_eq!(out["section_id"], json!("docs/huge.md#root"));
        assert_eq!(out["claims_available"], json!(7));
        // Envelope is present and honest.
        let c = &out["_condensed"];
        assert_eq!(c["budget_tokens"], json!(budget));
        assert!(c["original_tokens"].as_u64().unwrap() > budget as u64);
        // `final_tokens` is the condensed payload size (before the envelope is
        // added), so it is within budget and no larger than the whole result.
        let final_tokens = c["final_tokens"].as_u64().unwrap();
        assert!(final_tokens <= budget as u64);
        assert!(final_tokens <= tokens(&out) as u64);
    }

    #[test]
    fn many_rows_are_trimmed_with_omission_count_and_ids_kept() {
        let rows: Vec<Value> = (0..5000)
            .map(|i| json!({ "id": format!("sym-{i}"), "kind": "function", "line": i }))
            .collect();
        let v = json!({ "symbols": rows, "total": 5000 });
        let budget = 1000;
        assert!(tokens(&v) > budget);

        let out = fit_to_budget(v, budget);
        assert!(tokens(&out) <= budget, "got {}", tokens(&out));
        // Some rows survive, with their identifiers intact.
        let kept = out["symbols"].as_array().expect("symbols array");
        assert!(!kept.is_empty(), "at least one row must survive");
        assert_eq!(kept[0]["id"], json!("sym-0"));
        // The total count is preserved as a cheap high-signal field.
        assert_eq!(out["total"], json!(5000));
        // Omission is recorded.
        assert!(out["_condensed"]["omitted_elements"].as_u64().unwrap() > 0);
    }

    #[test]
    fn clip_guarantees_budget_on_sentence_free_text() {
        // Code-like text with no sentence boundaries — the summarizer can't
        // help, so the clip backstop must still enforce the budget.
        let code = "fn x(){let a=1;}\n".repeat(5000);
        let clipped = clip_to_tokens(&code, 200);
        assert!(count_tokens(&clipped) <= 200, "clip must honor the budget");
        assert!(clipped.contains("chars elided"), "marker present");
    }

    #[test]
    fn reduce_text_prefers_summary_then_clips() {
        let prose = "The cache stores tokens. It evicts the oldest entries. \
                     Eviction is LRU. The budget is fixed. Pressure rises with use. "
            .repeat(200);
        let out = reduce_text(&prose, 120);
        assert!(count_tokens(&out) <= 120);
        assert!(!out.is_empty());
    }

    #[test]
    fn non_object_payload_is_wrapped() {
        let big: Vec<Value> = (0..5000).map(|i| json!(format!("item-{i}"))).collect();
        let out = fit_to_budget(Value::Array(big), 600);
        assert!(tokens(&out) <= 600);
        assert!(out["_condensed"].is_object());
        assert!(out.get("result").is_some(), "bare array wrapped under result");
    }
}
