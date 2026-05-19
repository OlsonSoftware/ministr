# ministr benchmarks

This directory will hold reproducible benchmark results for ministr — index time, query latency, retrieval quality, and token efficiency vs grep+read — across a fixed set of public repositories.

The harness already exists; only the published numbers are missing. This file is the scaffold for that write-up. When you run any of the suites below, paste the output into the matching `results/` file and commit.

## Suites

ministr ships three benchmark surfaces. All are reproducible from a clean clone.

### 1. Retrieval quality (NDCG@10, MRR, P@k, R@k)

Lives in [`ministr-core/tests/eval_retrieval.rs`](../ministr-core/tests/eval_retrieval.rs). Loads a labelled query set against an indexed corpus and computes ranking metrics.

```sh
cargo test --release -p ministr-core --test eval_retrieval -- --nocapture
```

Headline number for the README: NDCG@10 on a fixed CoIR subset (see [CoIR](https://github.com/CoIR-team/coir)) — comparable to semble's published `0.854` and CodeRankEmbed's published numbers on the same set.

### 2. Latency (criterion)

Lives in [`ministr-core/benches/`](../ministr-core/benches/):

| Bench | Measures |
|---|---|
| `search.rs` | HNSW insert + search latency across 1k–100k vector sizes |
| `prefetch.rs` | Prefetch cache hit-rate; target <1 ms warm hit vs 50–200 ms cold |
| `ingestion.rs` | End-to-end ingestion throughput (files/sec) |
| `embedding.rs` | Embedder throughput on Candle vs FastEmbed paths |
| `token_economics.rs` | Token savings vs full-file delivery |

Run individually (each is heavy — don't run the whole suite back-to-back):

```sh
cargo bench -p ministr-core --bench search
cargo bench -p ministr-core --bench prefetch
cargo bench -p ministr-core --bench ingestion
cargo bench -p ministr-core --bench embedding
cargo bench -p ministr-core --bench token_economics
```

Headline numbers for the README:

- **Cold-index time** on the ministr workspace itself (≈ 6 crates, mixed Rust + TS + MDX) — the demo number that anchors all comparisons.
- **Warm query latency** for `ministr_survey` (target: single-digit ms warm).
- **Cold query latency** for `ministr_survey` (target: <100 ms cold).

### 3. Token efficiency

Token savings vs grep + Read at equivalent recall levels — semble published a recall-vs-tokens curve; ministr should publish the same curve plus a multi-resolution breakdown showing how `ministr_survey` returns claim / section / symbol / full-source slices instead of whole files.

Source data is `ministr-core/tests/token_baseline.rs` (existing) plus per-tool token costs already documented in [`docs-next/content/docs/tools/index.mdx`](../docs-next/content/docs/tools/index.mdx).

## Per-language matrix

semble publishes per-language NDCG@10 across 19 languages. ministr parses 40+; the matrix should cover at least these tiers:

**Tier 1 (full extraction):** Rust, Python, JS, TS, Go, Java, C, C++, C#, Ruby, Swift, Kotlin, PHP, Scala
**Tier 2 (smoke-tested):** Bash, Lua, Elixir, Haskell, OCaml, Dart, R, HCL/Terraform, SQL, Zig, Protobuf
**Tier 3 (limited):** Svelte, CSS, GraphQL, Groovy, Nix, Erlang, PowerShell, Solidity, Objective-C, Julia, CMake, Make, JSON, YAML, TOML

For each language pick 2–3 representative repos from the [CoIR](https://github.com/CoIR-team/coir) / CodeSearchNet selection. Score NDCG@10 + cold-index per repo. Aggregate.

## Methodology notes

- **Always use `--release`.** Debug builds are unusably slow (ONNX + macOS XProtect).
- **Pin a model.** Default is `all-MiniLM-L6-v2` (384d). The Matryoshka embedder allows truncation — document which dimension the published numbers use.
- **Fix the rerank depth.** Different rerank depths shift latency dramatically; pin in `MinistrConfig`.
- **Reproducibility.** Capture `cargo --version`, `rustc --version`, host machine, and the model + dimension in every results file.

## Results

Empty until populated. When you publish a number, add a file under `results/`:

- `results/cold-index-2026-05-<repo>.md` — cold index of `<repo>` on host `<host>`.
- `results/ndcg-2026-05-coir.md` — CoIR retrieval-quality run.
- `results/latency-2026-05.md` — criterion summary.

The top-level [`README.md`](../README.md) then pulls headline numbers from these files.
