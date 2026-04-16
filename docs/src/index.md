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
  <a class="iris-hero__cta iris-hero__cta--primary" href="getting-started/">Get started →</a>
  <a class="iris-hero__cta iris-hero__cta--secondary" href="https://github.com/AlrikOlson/iris-rs">GitHub</a>
</div>

<div class="iris-stats" markdown>
<div class="iris-stats__item">
  <div class="iris-stats__value">~5 ms</div>
  <div class="iris-stats__label">Local embedding</div>
</div>
<div class="iris-stats__item">
  <div class="iris-stats__value">&lt; 1 ms</div>
  <div class="iris-stats__label">Warm cache hit</div>
</div>
<div class="iris-stats__item">
  <div class="iris-stats__value">94 %</div>
  <div class="iris-stats__label">Token savings vs grep + cat</div>
</div>
<div class="iris-stats__item">
  <div class="iris-stats__value">12</div>
  <div class="iris-stats__label">Languages via tree-sitter</div>
</div>
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
  <h2>How it fits together</h2>
  <p>One local binary sits between your MCP client and your corpus.</p>
</div>

<div class="iris-diagram" markdown>

```d2
direction: right

clients: MCP clients {
  claude: Claude Code
  cursor: Cursor
  agent: Custom agent
}

iris: iris {
  proxy: MCP proxy\nstdio
  daemon: Daemon\nUDS · HTTP
  session: Session shadow\n+ budget
  prefetch: Prefetch engine
  query: Query service

  proxy -> daemon
  daemon -> session
  daemon -> prefetch
  daemon -> query
}

storage: Local storage {
  sql: SQLite\ncontent + symbols {
    shape: cylinder
  }
  hnsw: HNSW\nvector index {
    shape: cylinder
  }
  embed: FastEmbed\nONNX · Metal {
    shape: cylinder
  }
}

clients.claude -> iris.proxy
clients.cursor -> iris.proxy
clients.agent -> iris.proxy

iris.query -> storage.sql
iris.query -> storage.hnsw
iris.query -> storage.embed
```

</div>

<div class="iris-section-header">
  <h2>What iris does</h2>
  <p>Six capabilities, one local binary. No API keys.</p>
</div>

<div class="iris-features" markdown>

<div class="iris-features__card" markdown>
### Semantic search
Embedding-based retrieval at document, section, and claim resolution.
Hybrid dense + sparse search with optional cross-encoder rerank.
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
compressed summaries at claim-level resolution under pressure.
</div>

<div class="iris-features__card" markdown>
### Local embeddings
FastEmbed + ONNX (~5 ms/embed). Optional Metal GPU acceleration via Candle
on Apple Silicon. No network required.
</div>

</div>

<div class="iris-section-header">
  <h2>A typical session</h2>
  <p>Every response carries budget tracking, and the prefetch engine pre-warms what's next.</p>
</div>

<div class="iris-trace" markdown>
```text
➜ iris_survey("authentication middleware")
  ranked 5 results · budget: 3% used · prefetch: warming src/auth.rs#logout

➜ iris_read("src/auth.rs#login")
  420 tokens · budget: 5% used · prefetch: warming validate_token (structural)

➜ iris_read("src/auth.rs#logout")
  CACHE HIT — delivered from prefetch · 0 ms · budget: 7% used

➜ iris_symbols(kind="function", query="validate")
  8 symbols found · budget: 8% used

... (many reads later) ...

➜ iris_survey("rate limiting")
  results at CLAIM resolution · pressure: ELEVATED · budget: 82% used
  eviction_recommendations: [src/setup.rs#prerequisites, docs/intro.md]

➜ iris_evicted(["src/setup.rs#prerequisites"])
  budget: 76% used · session shadow updated
```
</div>

<div class="iris-section-header">
  <h2>Cross-language bridges</h2>
  <p>Trace function calls across language boundaries automatically.</p>
</div>

<div class="iris-diagram" markdown>

```d2
direction: right

rust: Rust {
  napi: "#[napi]\nfn greet(s: String)" {
    shape: rectangle
  }
  pyo: "#[pyfunction]\nfn compute(x: f64)" {
    shape: rectangle
  }
  tauri: "#[tauri::command]\nfn open_file(path: &str)" {
    shape: rectangle
  }
}

js: JavaScript / Python {
  import_native: "import { greet }\nfrom './native'" {
    shape: rectangle
  }
  py_import: "from mylib import\n    compute" {
    shape: rectangle
  }
  invoke: "invoke('open_file',\n   { path })" {
    shape: rectangle
  }
}

rust.napi -> js.import_native: napi {
  style.stroke-width: 3
}
rust.pyo -> js.py_import: pyo3 {
  style.stroke-width: 3
}
rust.tauri -> js.invoke: tauri {
  style.stroke-width: 3
}
```

</div>

Query these links with [`iris_bridge`](tools/bridge.md) or trace a symbol across
language boundaries with [`iris_references`](tools/references.md).

<div class="iris-section-header">
  <h2>Twelve languages, one symbol index</h2>
  <p>tree-sitter grammars power symbol extraction, reference tracing, and bridge detection.</p>
</div>

<div class="iris-languages" markdown>
<span class="iris-lang">Rust</span>
<span class="iris-lang">Python</span>
<span class="iris-lang">JavaScript</span>
<span class="iris-lang">TypeScript</span>
<span class="iris-lang">Go</span>
<span class="iris-lang">Java</span>
<span class="iris-lang">C</span>
<span class="iris-lang">C++</span>
<span class="iris-lang">Ruby</span>
<span class="iris-lang">C#</span>
<span class="iris-lang">Swift</span>
<span class="iris-lang">Kotlin</span>
</div>

<div class="iris-section-header">
  <h2>How it compares</h2>
  <p>iris isn't a vector DB or a RAG framework. It's a cache controller.</p>
</div>

<div class="iris-compare" markdown>

|  | `grep` + `cat` | Naive RAG | **iris** |
|---|:---:|:---:|:---:|
| Semantic search | no | yes | yes |
| Code symbol index | no | no | **yes** |
| Cross-language links | no | no | **yes** |
| Tracks delivered content | no | no | **yes** |
| Deduplicates across turns | no | no | **yes** |
| Delta delivery on change | no | no | **yes** |
| Predictive prefetch | no | no | **yes** |
| Budget-aware compression | no | no | **yes** |
| Runs locally, no API keys | yes | varies | **yes** |

</div>

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
