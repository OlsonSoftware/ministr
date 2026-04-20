---
title: iris
description: Context cache for LLM agents — MCP server with semantic search, code navigation, session tracking, and budget awareness.
hide:
  - navigation
  - toc
---

<div class="iris-hero" markdown>

<span class="iris-hero__eyebrow">
  <svg class="icon icon-sm"><use href="assets/icons.svg#cube-focus"/></svg>
  MCP server · runs locally
</span>

# iris { .iris-hero__title }

<p class="iris-hero__tagline">
  Serve context to your LLM agent like an L1 cache — with session tracking,
  predictive prefetch, and budget awareness.
</p>

<div class="iris-hero__install">claude mcp add iris -- iris</div>

<div class="iris-hero__ctas">
  <a class="iris-hero__cta iris-hero__cta--primary" href="getting-started/">Get started <svg class="icon icon-sm"><use href="assets/icons.svg#arrow-right"/></svg></a>
  <a class="iris-hero__cta iris-hero__cta--secondary" href="https://github.com/AlrikOlson/iris-rs">GitHub</a>
</div>

<div data-iris-trace class="iris-trace-live iris-trace-live--hero"></div>

<noscript>
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
</noscript>

<div class="iris-stats" markdown>
<div class="iris-stats__item">
  <div class="iris-stats__value">Local</div>
  <div class="iris-stats__label">No API keys</div>
</div>
<div class="iris-stats__item">
  <div class="iris-stats__value">Session-aware</div>
  <div class="iris-stats__label">Remembers what it sent</div>
</div>
<div class="iris-stats__item">
  <div class="iris-stats__value">Predictive</div>
  <div class="iris-stats__label">Warms what's next</div>
</div>
<div class="iris-stats__item">
  <div class="iris-stats__value">12</div>
  <div class="iris-stats__label">Languages indexed</div>
</div>
</div>

</div>

<div class="iris-section-header">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#circle"/></svg>
    Problem
  </span>
  <h2>Why iris</h2>
  <p>LLM agents waste most of their context window. iris fixes the three root causes.</p>
</div>

<div class="iris-why" markdown>

<div class="iris-why__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#stack"/></svg> Re-reading
Agents fetch the same file over and over. iris remembers what it sent this
session and deduplicates. When a section changes, it delivers only the delta.
</div>

<div class="iris-why__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#magnifying-glass"/></svg> Blind retrieval
`grep` + `cat` burn tokens on code that isn't relevant. iris indexes
your corpus semantically and returns just the piece that answers the
question.
</div>

<div class="iris-why__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#lightning"/></svg> Cold reads
Every new fetch is a round trip your agent waits on. iris predicts what
it's going to ask for next and warms it in the background.
</div>

</div>

<div class="iris-section-header">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#squares-four"/></svg>
    Architecture
  </span>
  <h2>How it fits together</h2>
  <p>One local binary sits between your MCP client and your files.</p>
</div>

<div class="iris-diagram" markdown>

```d2
direction: right

clients: Your MCP client {
  claude: Claude Code
  cursor: Cursor
  agent: …any MCP client
}

iris: iris {
  shape: rectangle
}

storage: Your corpus {
  files: Source files {
    shape: cylinder
  }
  index: Local index {
    shape: cylinder
  }
}

clients.claude -> iris
clients.cursor -> iris
clients.agent -> iris

iris -> storage.files
iris -> storage.index
```

</div>
<p class="iris-diagram__caption"><a href="architecture/">See the full architecture →</a></p>

<div class="iris-section-header">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#sparkle-fill"/></svg>
    Capabilities
  </span>
  <h2>What iris does</h2>
  <p>One local binary, no API keys.</p>
</div>

<div class="iris-features" markdown>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#magnifying-glass"/></svg> Semantic search
Search across your codebase and docs by meaning, not just text. Returns
the specific section that answers the question.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#code"/></svg> Code symbol navigation
Find and trace functions, types, and callers across your project — not
just file-level matches.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#graph"/></svg> Cross-language bridges
Follows function calls where one language hands off to another — Rust to
JavaScript, Python to Rust, front-end to back-end.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#stack"/></svg> Session tracking
Remembers what it sent your agent this session. Skips repeats; ships only
the changed part when a section is edited.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#gauge"/></svg> Budget awareness
Tracks context-window usage. When it fills up, older material gets
compressed instead of silently dropping.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#cpu"/></svg> Local embeddings
Embeddings run on your machine, not a third-party API. No network calls,
no API keys, no tokens leaving the box.
</div>

</div>

<div class="iris-section-header">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#circuitry"/></svg>
    Desktop app
  </span>
  <h2>The observatory, for when you want to watch</h2>
  <p>A desktop companion that attaches to the same local daemon your agents use — inspect corpora, replay sessions, and tune configuration without leaving the GUI.</p>
</div>

<div class="iris-app-preview" role="img" aria-label="Preview of the iris desktop observatory — macOS window showing a sidebar of three corpora (iris-rs active with 4128 docs, docs, research-notes), two live sessions with budget percentages, a query playground displaying two ranked results for authentication middleware, and an indexing progress bar at 68 percent">
  <div class="iris-app-preview__chrome">
    <div class="iris-app-preview__dots">
      <span class="iris-app-preview__dot iris-app-preview__dot--r"></span>
      <span class="iris-app-preview__dot iris-app-preview__dot--y"></span>
      <span class="iris-app-preview__dot iris-app-preview__dot--g"></span>
    </div>
    <span class="iris-app-preview__title">iris — observatory</span>
    <span class="iris-app-preview__status">
      <span class="iris-app-preview__led"></span>
      daemon connected
    </span>
  </div>
  <div class="iris-app-preview__body">
    <aside class="iris-app-preview__sidebar">
      <div class="iris-app-preview__sidebar-label">Corpora · 3</div>
      <ul class="iris-app-preview__list">
        <li class="iris-app-preview__row iris-app-preview__row--active">
          <span class="iris-app-preview__row-name">iris-rs</span>
          <span class="iris-app-preview__row-meta">4128 docs</span>
        </li>
        <li class="iris-app-preview__row">
          <span class="iris-app-preview__row-name">docs/</span>
          <span class="iris-app-preview__row-meta">312 docs</span>
        </li>
        <li class="iris-app-preview__row">
          <span class="iris-app-preview__row-name">research-notes</span>
          <span class="iris-app-preview__row-meta">57 docs</span>
        </li>
      </ul>
      <div class="iris-app-preview__sidebar-label">Sessions · 2 live</div>
      <ul class="iris-app-preview__list">
        <li class="iris-app-preview__row">
          <span class="iris-app-preview__row-name">claude-code · main</span>
          <span class="iris-app-preview__row-meta">42% budget</span>
        </li>
        <li class="iris-app-preview__row">
          <span class="iris-app-preview__row-name">cursor · refactor</span>
          <span class="iris-app-preview__row-meta">18% budget</span>
        </li>
      </ul>
    </aside>
    <div class="iris-app-preview__main">
      <div class="iris-app-preview__panel">
        <div class="iris-app-preview__panel-header">
          <span class="iris-app-preview__panel-title">Query playground</span>
          <span class="iris-app-preview__panel-meta">iris_survey · 5 hits · 42 ms</span>
        </div>
        <div class="iris-app-preview__query">authentication middleware</div>
        <div class="iris-app-preview__results">
          <div class="iris-app-preview__result">
            <div class="iris-app-preview__result-head">
              <span class="iris-app-preview__result-path">src/auth.rs › login</span>
              <span class="iris-app-preview__score">0.91</span>
            </div>
            <p class="iris-app-preview__snippet">Validates JWT tokens using RS256 and calls <code>validate_token</code>…</p>
          </div>
          <div class="iris-app-preview__result">
            <div class="iris-app-preview__result-head">
              <span class="iris-app-preview__result-path">src/auth.rs › logout</span>
              <span class="iris-app-preview__score">0.87</span>
            </div>
            <p class="iris-app-preview__snippet">Revokes the session cookie and blacklists the refresh token until…</p>
          </div>
        </div>
      </div>
      <div class="iris-app-preview__panel">
        <div class="iris-app-preview__panel-header">
          <span class="iris-app-preview__panel-title">Indexing · iris-rs</span>
          <span class="iris-app-preview__panel-meta">2812 / 4128 sections</span>
        </div>
        <div class="iris-app-preview__progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="68" aria-label="Indexing progress">
          <span class="iris-app-preview__progress-fill" style="width: 68%"></span>
        </div>
      </div>
    </div>
  </div>
</div>

<div class="iris-features" markdown>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#squares-four"/></svg> Overview
Live counts of files, code symbols, and active sessions across every registered corpus — plus a live feed of what's being indexed.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#magnifying-glass"/></svg> Query playground
Run `iris_survey`, `iris_symbols`, `iris_definition`, and `iris_references` against any registered corpus. See the same ranked results your agent sees, with heading paths and previews.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#gauge"/></svg> Session dashboard
Replay a session turn by turn: which sections were delivered, which got evicted, how the budget tracked across the conversation, and what got warmed in the background.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#graph"/></svg> Symbol graph
An interactive map of your codebase as a collapsible graph. Navigate callers, implementors, and cross-language bridges visually.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#stack"/></svg> Corpus treemap
Treemap of disk and token footprint per path. Spot a runaway directory before it bloats your index.
</div>

<div class="iris-features__card" markdown>
### <svg class="icon icon-md"><use href="assets/icons.svg#terminal-window"/></svg> Log viewer + settings
Tail daemon logs with filtering, and tune budget, prefetch, and embedding settings from the UI — changes apply without a restart.
</div>

</div>

<p class="iris-hero__tagline iris-hero__tagline--wide">
One download, one daemon. Run the CLI for agents, the app for humans — both see the same corpora.
<br><br>
<a class="iris-hero__cta iris-hero__cta--secondary" href="https://github.com/AlrikOlson/iris-rs/tree/main/iris-app"><svg class="icon icon-sm"><use href="assets/icons.svg#code"/></svg> iris-app source</a>
</p>

<div class="iris-section-header">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#git-branch"/></svg>
    Bridges
  </span>
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
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#code"/></svg>
    Languages
  </span>
  <h2>Twelve languages, one symbol index</h2>
  <p>Real parsers, not regex — symbol extraction, reference tracing, and bridge detection across the stack.</p>
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
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#compass-tool"/></svg>
    Comparison
  </span>
  <h2>How it compares</h2>
  <p>iris isn't a vector DB, a RAG framework, or a search tool. It's a stateful, cache-aware context source exposed as MCP tools.</p>
</div>

<div class="iris-compare" markdown>

|  | `grep` + `cat` | Vector DB / RAG | **iris** |
|---|:---:|:---:|:---:|
| **Retrieval** | Full-text match | Embeddings | Embeddings + symbols |
| **Code symbol index** | – | – | **12 languages** |
| **Cross-language bridges** | – | – | **yes** |
| **Session memory** | – | – | **per-corpus shadow** |
| **Dedup across turns** | – | – | **yes** |
| **Delta on change** | – | – | **yes** |
| **Predictive prefetch** | – | – | **yes** |
| **Budget awareness** | – | – | **tracks + suggests evictions** |
| **Agent protocol** | Shell | Custom API | **MCP (tool-native)** |
| **Runs locally** | yes | varies | **yes** |

</div>

<div class="iris-section-header">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#package"/></svg>
    Install
  </span>
  <h2>Get started in 30 seconds</h2>
  <p>The signed PKG installer drops the desktop app in <code>/Applications</code> and the CLI on your <code>PATH</code> — one click, no terminal.</p>
</div>

<div class="iris-hero__ctas iris-hero__ctas--spaced">
  <a class="iris-hero__cta iris-hero__cta--primary" href="download/">
    <svg class="icon icon-sm"><use href="assets/icons.svg#package"/></svg>
    Download iris
  </a>
  <a class="iris-hero__cta iris-hero__cta--secondary" href="installation/">All install methods</a>
</div>

=== ":material-apple: macOS (CLI only)"

    ```sh
    curl -fsSL https://raw.githubusercontent.com/AlrikOlson/iris-rs/main/install.sh | bash
    ```

=== ":material-linux: Linux"

    ```sh
    curl -fsSL https://raw.githubusercontent.com/AlrikOlson/iris-rs/main/install.sh | bash
    ```

=== ":material-language-rust: Cargo"

    ```sh
    cargo install --git https://github.com/AlrikOlson/iris-rs iris-cli
    ```

Then initialize and connect:

```sh
cd your-project
iris init                          # creates .iris.toml + .mcp.json
claude mcp add iris -- iris        # Claude Code
```

iris auto-discovers `.iris.toml` from the working directory. No flags needed.

<div class="iris-section-header">
  <span class="iris-section-header__eyebrow">
    <svg class="icon icon-sm"><use href="assets/icons.svg#book-open"/></svg>
    Explore
  </span>
  <h2>Dig deeper</h2>
</div>

<div class="iris-features" markdown>

<div class="iris-features__card" markdown>
### [<svg class="icon icon-md"><use href="assets/icons.svg#arrow-up-right"/></svg> Getting started](getting-started.md)
Installation, configuration, and first query.
</div>

<div class="iris-features__card" markdown>
### [<svg class="icon icon-md"><use href="assets/icons.svg#arrow-up-right"/></svg> Tool reference](tools/README.md)
Every MCP tool with parameters, response schemas, and behavior notes.
</div>

<div class="iris-features__card" markdown>
### [<svg class="icon icon-md"><use href="assets/icons.svg#arrow-up-right"/></svg> Architecture](architecture.md)
Project layout, daemon topology, layered design, on-disk format.
</div>

</div>
