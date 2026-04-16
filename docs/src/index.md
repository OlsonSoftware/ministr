---
title: iris
description: Context cache controller for LLM agents — MCP server with semantic search, code navigation, session tracking, and budget management.
hide:
  - navigation
  - toc
---

<div class="iris-hero" markdown>

<span class="iris-hero__eyebrow">MCP server · written in Rust</span>

# iris { .iris-hero__title }

<p class="iris-hero__tagline">
  Manage your LLM agent's context window like L1 cache — with session tracking,
  predictive prefetch, budget management, and code navigation across 12 languages.
</p>

<div class="iris-hero__install">claude mcp add iris -- iris</div>

<div class="iris-hero__ctas">
  <a class="iris-hero__cta iris-hero__cta--primary" href="getting-started.md">Get started →</a>
  <a class="iris-hero__cta iris-hero__cta--secondary" href="https://github.com/AlrikOlson/iris-rs">GitHub</a>
</div>

</div>

<div class="iris-section-header">
  <h2>Why iris</h2>
  <p>LLM agents waste most of their context window. iris fixes the three root causes.</p>
</div>

<div class="iris-why" markdown>

<div class="iris-why__card" markdown>
### Re-reading
Agents fetch the same file over and over. iris tracks what the agent has seen and
deduplicates. When a section changes, it delivers only the delta.
</div>

<div class="iris-why__card" markdown>
### Blind retrieval
`grep` + `cat` burns tokens on irrelevant code. iris indexes your codebase at
multiple resolutions — documents, sections, claims, symbols — and returns
precisely what matters.
</div>

<div class="iris-why__card" markdown>
### No lookahead
Cold retrievals cost latency and tokens. iris predicts the next read and
pre-warms it with sequential, structural, and topical prefetch strategies.
</div>

</div>

<div class="iris-section-header">
  <h2>What iris does</h2>
  <p>Six capabilities, one local binary. No API keys.</p>
</div>

<div class="iris-features" markdown>

<div class="iris-features__card" markdown>
### Semantic search
Embedding-based retrieval across docs and code at document, section, and
claim resolution. Hybrid dense + sparse + reranking.
</div>

<div class="iris-features__card" markdown>
### Code symbol navigation
Find and trace structs, functions, traits, and enums across 12 languages.
Cross-crate references and method-level precision.
</div>

<div class="iris-features__card" markdown>
### Cross-language bridges
Automatic linking of Tauri commands, napi bindings, PyO3 functions,
wasm-bindgen exports, and HTTP routes.
</div>

<div class="iris-features__card" markdown>
### Session tracking
Shadow the agent's context window. Deduplicate deliveries, detect evictions,
deliver deltas instead of full re-reads.
</div>

<div class="iris-features__card" markdown>
### Budget management
Monitor token usage, flag pressure, rank eviction candidates, provide
compressed summaries under pressure.
</div>

<div class="iris-features__card" markdown>
### Local embeddings
FastEmbed + ONNX (~5 ms/embed). Optional Metal GPU acceleration via Candle
on Apple Silicon. No network required.
</div>

</div>

<div class="iris-section-header">
  <h2>Cross-language bridges</h2>
  <p>Trace function calls across language boundaries automatically.</p>
</div>

<pre class="iris-bridge-diagram">
 Rust                              JavaScript / Python
┌──────────────────────────┐      ┌──────────────────────────┐
│ #[napi]                  │══════│ import { greet }          │
│ fn greet(s: String)      │ napi │ from './native'           │
│                          │      │                           │
│ #[pyfunction]            │══════│ from mylib import         │
│ fn compute(x: f64)       │ pyo3 │     compute               │
│                          │      │                           │
│ #[tauri::command]        │══════│ invoke('open_file',       │
│ fn open_file(path: &str) │tauri │    { path })              │
└──────────────────────────┘      └──────────────────────────┘
</pre>

Query these links with [`iris_bridge`](tools/bridge.md) or trace a symbol across
language boundaries with [`iris_references`](tools/references.md).

<div class="iris-section-header">
  <h2>Get started in 30 seconds</h2>
</div>

=== ":material-apple: macOS"

    ```sh
    brew install AlrikOlson/tap/iris
    ```

=== ":material-linux: Linux"

    ```sh
    curl -fsSL https://raw.githubusercontent.com/AlrikOlson/iris-rs/main/install.sh | bash
    ```

=== ":material-language-rust: Cargo"

    ```sh
    cargo install iris-cli
    ```

Then initialize and connect:

```sh
cd your-project
iris init                          # creates .iris.toml + .mcp.json
claude mcp add iris -- iris        # Claude Code
```

iris auto-discovers `.iris.toml` from the working directory. No flags needed.

---

<div class="iris-section-header">
  <h2>Dig deeper</h2>
</div>

<div class="iris-features" markdown>

<div class="iris-features__card" markdown>
### [Getting started →](getting-started.md)
Installation, configuration, and first query.
</div>

<div class="iris-features__card" markdown>
### [Tool reference →](tools/README.md)
Every MCP tool with parameters, response schemas, and behavior notes.
</div>

<div class="iris-features__card" markdown>
### [Architecture →](architecture.md)
Crate structure, daemon topology, layered design, on-disk format.
</div>

<div class="iris-features__card" markdown>
### [Benchmarks →](benchmarks.md)
Token savings, latency, recall quality, and indexing throughput.
</div>

</div>
