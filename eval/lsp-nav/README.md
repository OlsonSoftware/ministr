# Code-Navigation Benchmark — ministr vs. LSP

Head-to-head: ministr's code intelligence vs. a Rust language server
(`rust-analyzer`) on the **same** navigation tasks over this repository,
scored against hand-verified ground truth.

> Status: **Phase 1 (dataset + methodology).** `ground-truth.json` holds
> verified seed tasks; the diff-runner and the heavy comparison run are
> Phase 2 (see *Harness*, below). No metrics are published yet — none
> have been measured.

## Why this benchmark

A language server answers "go to definition / find references" within
**one** language and only for a project it can fully build. ministr
indexes any tree once and additionally resolves **cross-language
bridges** (Tauri command ↔ TS `invoke`, NAPI, PyO3, FFI, HTTP routes) —
which a Rust-only LSP is structurally blind to. This benchmark
quantifies both the overlap (def/refs accuracy) and the gap (coverage a
single-language LSP cannot reach).

## What is measured

| Metric | Definition |
|---|---|
| definition accuracy | resolved definition location matches ground truth |
| references precision / recall | returned reference set vs. the verified set |
| **coverage** | fraction of tasks each engine can answer at all — the headline: `bridge` tasks are `lsp_can_answer: false` |
| index build time | `rust-analyzer` full project load vs. ministr index of the same tree |
| per-query latency | time to answer one navigation task |
| setup cost (qualitative) | LSP needs a compiling project + toolchain; ministr indexes a bare tree |

## Ground truth

`ground-truth.json` — see its embedded `_schema`. Tasks are byte-anchored
to a specific commit; **re-verify line ranges before trusting metrics**
(symbols move). Definitions are exact (`ministr_definition` yields a
precise file + line range); `references` tasks are added as their full
expected set is hand-verified; `bridge` tasks are LSP-blind by
construction and exercise the differentiator.

## Harness (Phase 2 — not yet implemented)

1. `just bench-lsp-index` installs `rust-analyzer` (`rustup component add
   rust-analyzer`) and emits an LSIF index of the repo
   (`rust-analyzer lsif .` → `eval/lsp-nav/ra.lsif`, gitignored). LSIF is
   line-delimited JSON — no protobuf dependency (vs. SCIP).
2. The runner indexes this repo with ministr in-process (same pattern as
   `ministr-core/tests/eval_retrieval.rs`, via
   `ministr_core::service::code::QueryService`), then for each task
   compares ministr's answer and rust-analyzer's (resolved from the LSIF
   graph) against `ground-truth.json`, printing the metrics table above.
3. Optional CI gate mirrors `just eval-gate` once a baseline is committed.

Both indexers are minutes-long and memory-heavy, so this runs on demand
(and in a dedicated CI job), never in the default test run.

## Public-results note

ministr is not open source; any published comparison stays black-box —
report metrics and methodology, not internal symbol/schema names.
