# Evaluation suites

Committed fixtures and ground truth for deterministic retrieval, navigation,
and task-level agent-efficiency gates.

## Contents

| Path | What it is |
|---|---|
| `corpus/` | 9 representative code and documentation files |
| `ground-truth.json` | 75 documentation-oriented queries with manually labelled relevance grades |
| `corpus-code/` | 6 small programs across Rust, Python, Go, TypeScript, Java, and C++ |
| `ground-truth-code.json` | 26 natural-language code retrieval queries |
| `lsp-nav/` | Definition/reference/bridge navigation ground truth and an opt-in comparison runner |
| `agent-efficiency/` | Deterministic MCP task contracts, corpora, and protocol traces |

## Running the evaluation

```sh
just eval-gate                # deterministic retrieval-quality CI gate
just eval-agent-efficiency    # deterministic calls/tokens/correctness gate
just eval-quality             # opt-in real-embedder quality run
just eval-ast-code            # opt-in real dense vs AST-sparse code run
```

The retrieval floors live in `ministr-core/tests/eval_retrieval.rs`. The
task-level contract and efficiency floor live in
`agent-efficiency/tasks.json`. Raising or lowering a floor requires a measured
result and an explanation in the change that updates it.

## Model comparison

Compare multiple embedding models side by side against the same ground truth:

```sh
just eval-bakeoff          # documentation-oriented corpus
just eval-bakeoff-code     # code-heavy corpus
```

These commands download several models and are deliberately outside the CI
gate. `ministr-core/tests/eval_model_comparison.rs` also supports a selected
model list through `MINISTR_EVAL_MODELS`; it is an ignored test, not a Just
recipe.

## Task-level agent efficiency

The deterministic task runner measures correct task completion per 1,000
literal MCP response tokens. It also records calls, bytes, modeled latency to
the first correct symbol, required-file/symbol recall, irrelevant results,
repeated deliveries, incorrect absence claims, status correctness, and inspect
versus granular-workflow savings.

The real-model companion remains explicit and opt-in:

```sh
just eval-agent-efficiency-real --dry-run --models haiku,sonnet
just eval-agent-efficiency-real --fidelity-probe --tasks realrepo-pulldown-tabstop
```

Remove `--dry-run` only when an authenticated `claude` CLI, external-repository
network access, task toolchains, and a paid-run budget are available. Use
repeated trials before treating a model comparison as evidence.

## Adding new queries

Edit `ground-truth.json` and follow the existing schema:

```json
{
  "query": "How does X work?",
  "expected": [
    { "section_id": "file.md#heading/subheading", "relevance": 3 }
  ]
}
```

Relevance grades should reflect a reasonable human's judgment. Grade 3 is used for the primary answer, 2 for supporting context, 1 for tangentially related content. Avoid grading more than 10 results per query — the tail provides little signal.
