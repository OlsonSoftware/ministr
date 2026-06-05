# Side-by-side agent benchmark — ministr vs grep

Does giving a real coding agent **ministr** actually help it solve a real task
with fewer tokens? This benchmark answers that end-to-end, not by proxy: it runs
the *same* agent on the *same* task twice, changing only the **discovery tool**,
and checks whether each run produced a **correct** solution.

Unlike [`../../ministr-mcp/tests/token_economics_e2e.rs`](../../ministr-mcp/tests/token_economics_e2e.rs)
(which measures the token cost of a single retrieval), this measures the full
agent loop: discover → edit → test → iterate → done, and whether the result
actually passes.

## The two arms

The runner is **headless Claude Code** (`claude -p --output-format json`) — a
real coding agent, already authenticated, that reports token usage. Both arms
get `Read`, `Edit`, `Write`, `Bash` and must make the test suite pass. They
differ in **one** variable, the discovery tool:

| arm | discovery tool | has ministr? | has grep/glob? |
|-----|----------------|--------------|----------------|
| **A — ministr** | ministr MCP (`survey`/`symbols`/`definition`/…) on a pre-indexed corpus | ✅ | ❌ |
| **B — grep** | `Grep` + `Glob` | ❌ | ✅ |

Isolation: each arm runs on a fresh copy of the fixture in a throwaway `/tmp`
dir, with `--strict-mcp-config` (so the host's own MCP servers never leak) and
`--setting-sources user` (no project hooks). Arm A pre-indexes the fixture with
`ministr index --corpus <tmp>`; the throwaway corpus is keyed by the tmp path and
never touches your real corpora.

## The task (deterministic validator)

`fixture/` is a small multi-module Python library (`minicsv`) with a genuine
bug: `stats.sample_variance` divides the sum of squared deviations by *N*
(population) instead of *N − 1* (Bessel's correction), so the sample standard
deviation is biased low. There are several variance-related functions across
modules (`population_variance`, `streaming.running_variance`,
`aggregate.covariance`) as realistic distractors, so *locating* the right one is
a real navigation step. [`task.md`](task.md) is the natural-language prompt the
agent receives. The validator is the hidden test suite:

```sh
python3 -m unittest discover -s tests   # exit 0 = solved
```

The fixture is a verified red→green task (broken fails, the golden fix passes) —
prove it with no LLM and no spend:

```sh
python3 run_sxs.py --selftest
```

## Running it

⚠️ **This calls a real LLM and spends real quota. It is opt-in and never part of
any default gate.** Cost is hard-capped per arm with `--max-budget-usd`.

```sh
# See the exact commands without spending anything:
python3 run_sxs.py --dry-run

# Run both arms (default model: sonnet; per-arm cap $0.75):
python3 run_sxs.py

# Options:
python3 run_sxs.py --model opus --max-budget-usd 1.50 --repeat 3 --arms both --keep
```

It prints a side-by-side table (solved? · turns · input/output/cache tokens ·
cost · wall-clock) and writes `results.json`.

## Latest measured result

First real run — `2026-06-05`, model `sonnet`, one trial per arm
(`results.json`). **Both arms produced a correct fix** (the one-line Bessel
correction; all 16 tests green), so this is a cost/efficiency comparison at
equal correctness:

| metric | ministr | grep | ministr |
|--------|--------:|-----:|--------:|
| solved? | ✅ | ✅ | tie |
| turns | 5 | 7 | −29% |
| output tokens | 708 | 1,000 | −29% |
| cache-read tokens | 120,194 | 159,691 | −25% |
| **total cost** | **$0.0845** | **$0.1701** | **−50%** |
| wall-clock | 17 s | 29 s | −41% |

On this *small* fixture both agents find the bug, but the ministr arm gets there
in fewer turns and at roughly **half the cost** — it navigates to
`sample_variance` directly instead of grepping across the variance-named
distractors and reading whole files. (`input_tokens` alone — 6 vs 8 — is *not* a
meaningful headline: prompt caching puts the real input volume in
`cache_read_tokens`, which is why the harness headlines **total cost**.) One
trial is not significant; use `--repeat N` for a spread.

## Honest reading of the result

- The headline is **correctness first, then tokens**: a cheaper arm that didn't
  solve the task is not a win. The harness prints the verdict as measured and
  does **not** assume ministr wins.
- A single agent session is **non-deterministic** (the fixture and validator are
  not). Use `--repeat N` and read the spread, not one point.
- On a small fixture, `grep` is strong (few files to scan). ministr's advantage
  grows with codebase size and when the relevant code isn't named after the
  query terms — the same shape the retrieval-token benchmark shows. Bigger,
  realer fixtures plug into the same harness (point `FIXTURE`/`task.md` at them).
