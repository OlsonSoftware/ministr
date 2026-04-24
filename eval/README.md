# Retrieval Evaluation Suite

Ground-truth annotations and a representative corpus for measuring ministr retrieval quality. Used by the CI regression gate to catch quality drops before they ship.

## Contents

| Path | What it is |
|---|---|
| `corpus/` | 6 representative documents (HTML, Markdown) covering auth, deployment, testing, database design |
| `ground-truth.json` | 200+ queries with manually-labeled relevance grades (3 = highly relevant, 2 = relevant, 1 = marginal) |

## Running the evaluation

```sh
just bench-eval       # run and print metrics (MRR, Recall@k, nDCG@k)
just eval-gate        # CI regression gate: fails if metrics drop below threshold
```

The gate compares metrics against the committed baseline at `ministr-core/tests/eval_retrieval.rs`. Raising the bar requires updating the baseline and justifying the change in the PR.

## Model comparison

Compare multiple embedding models side-by-side against the same ground truth:

```sh
just bench-models                                    # all registered models (~1 GB download)
just bench-model bge-small-en-v1.5                   # single model
just bench-model nomic-embed-text-v1.5@512           # Matryoshka truncation variant
```

Results are written to `eval/model-comparison.json` (gitignored).

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
