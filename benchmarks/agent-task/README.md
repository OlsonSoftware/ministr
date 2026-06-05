# Side-by-side agent benchmark suite — ministr vs grep

Does giving a real coding agent **ministr** help it solve real tasks with fewer
tokens? This suite answers that end-to-end — not by proxy: it runs the *same*
agent on the *same* tasks twice, changing only the **discovery tool**, across a
**difficulty ladder** and **multiple models**, and checks whether each run
produced a **correct** solution.

Unlike [`../../ministr-mcp/tests/token_economics_e2e.rs`](../../ministr-mcp/tests/token_economics_e2e.rs)
(token cost of a single retrieval), this measures the full agent loop —
discover → edit → test → iterate → done — and whether the result actually
passes.

## The two arms

Runner: **headless Claude Code** (`claude -p --output-format json`) — a real
coding agent, already authenticated, that reports token usage. Both arms get
`Read`, `Edit`, `Write`, `Bash` and must make the task's test suite pass. They
differ in **one** variable, the discovery tool:

| arm | discovery tool | ministr? | grep/glob? |
|-----|----------------|----------|-----------|
| **A — ministr** | ministr MCP (`survey`/`symbols`/`definition`/…) on a pre-indexed corpus | ✅ | ❌ |
| **B — grep** | `Grep` + `Glob` | ❌ | ✅ |

Isolation: each run uses a fresh copy of the task fixture in a throwaway `/tmp`
dir, with `--strict-mcp-config` (host MCP servers never leak), `--setting-sources
user` (no project hooks), `--no-session-persistence`, and `bypassPermissions`.
Arm A pre-indexes the fixture with `ministr index --corpus .` (run from the
fixture dir — a cwd `.ministr.toml` would otherwise override `--corpus`); the
throwaway corpus is keyed by the tmp path and never touches your real corpora.

## The tasks (difficulty ladder)

Each task lives in `tasks/<id>/` and is fully self-contained:

```
tasks/<id>/
  task.json   manifest: {id, difficulty, language, summary, validate, golden}
  task.md     the natural-language prompt handed to the agent
  fixture/    the broken project (committed)
  golden/     known-good file(s) used by the no-LLM selftest
```

`validate` is the argv of a deterministic test command (exit 0 = solved);
`golden` maps fixture-relative paths to known-good replacements so the suite can
prove, **with no LLM**, that each task is a valid red→green.

| id | difficulty | what it exercises |
|----|------------|-------------------|
| `easy-roman-numeral` | easy | single-file converter missing subtractive forms (IV/IX/…). Keyword-obvious — **grep is expected to tie here.** |
| `medium-sample-variance` | medium | multi-module stats lib: sample variance uses N not N−1 (Bessel), among several variance-named distractors. |
| `hard-operator-precedence` | hard | multi-file Pratt evaluator with inverted precedence. The bug is a left-binding-power table **not named "precedence"** — a semantic gap where grepping the symptom words misses the fix site. |

Difficulty rubric: **easy** = one file, the buggy symbol is named after the
symptom; **medium** = a few modules + same-named distractors; **hard** =
multiple modules and a semantic gap between the symptom and the fix site.

Prove every task is a valid red→green (no LLM, no spend):

```sh
python3 run_sxs.py --selftest
python3 run_sxs.py --list
```

## Running the matrix

⚠️ **This calls a real LLM and spends real quota. Opt-in; never a default gate.**
Cost is hard-capped per arm with `--max-budget-usd`.

```sh
# See the whole matrix without spending:
python3 run_sxs.py --dry-run --models haiku,sonnet

# Cheapest real run — all tasks on haiku, both arms:
python3 run_sxs.py --models haiku

# Full matrix across models, 3 trials each for a spread:
python3 run_sxs.py --models haiku,sonnet --repeat 3

# A subset:
python3 run_sxs.py --tasks hard-operator-precedence --models sonnet
```

It prints a per-(task, model) side-by-side table, an aggregate scoreboard
(solved counts, total/head-to-head cost, turns), and writes `results.json`.

## Adding a task

Drop a new `tasks/<id>/` with the four pieces above, run `--selftest` to confirm
it's a valid red→green, and it's automatically in the matrix. Any language works
as long as `validate` is a deterministic command (exit 0 = solved) and `golden`
lists the file(s) a correct fix would change.

## Honest reading

- **Correctness first, then cost.** A cheaper arm that didn't solve the task is
  not a win; the scoreboard counts solves before comparing cost.
- **Headline on total cost, not `input_tokens`.** With prompt caching,
  `input_tokens` is only the uncached sliver (often a handful); `total_cost_usd`
  aggregates input + output + cache.
- **Single trials are noisy** — an agent session is non-deterministic (the
  fixtures and validators are not). Use `--repeat N`.
- **On easy tasks grep is expected to tie.** ministr's advantage shows up as the
  codebase grows and when the fix site isn't named after the symptom (the hard
  task). Results are reported as measured, win or lose.

### Measured result (provenance)

First full matrix — `2026-06-05`, all 3 tasks × both arms × {haiku, sonnet},
one trial each (12 runs):

| model | ministr solved | grep solved | ministr cost | grep cost | head-to-head |
|-------|:--:|:--:|--:|--:|--|
| haiku  | 3/3 | 3/3 | $0.133 | $0.168 | **ministr 21% cheaper** |
| sonnet | 3/3 | 3/3 | $0.264 | $0.239 | ministr 10% *more expensive* |

**Correctness was a tie everywhere (12/12 solved).** The honest, un-cherry-picked
read: on **haiku** (cheaper/weaker model) ministr lowered cost on all three
tasks (consistently fewer output tokens); on **sonnet** over these *tiny*
fixtures, the model solves in ~5 turns regardless and ministr's survey overhead
made it marginally *pricier*. A single earlier sonnet trial on `medium` had shown
"50% cheaper" — the matrix shows that was **noise** (one trial), which is exactly
why `--repeat` and bigger fixtures matter. Expect ministr's edge to grow with
codebase size; on small tasks a strong model needs little help.
