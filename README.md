# ministr

ministr is an MCP server that gives AI coding agents real codebase
intelligence: semantic search across your code and docs, symbol-level
navigation, and cross-language bridge detection. It also remembers what it has
already shown the agent, so the same context is never re-fetched on the next
turn. It runs locally, embeds locally, and works with any MCP client — Claude
Code, Cursor, VS Code / Copilot.

## What it solves

AI agents burn their context window three ways, and ministr closes each one.
They re-read the same files every turn — ministr tracks what the agent has seen,
deduplicates it, and ships only the delta when something changes. They retrieve
blindly, pulling whole files to answer a narrow question — ministr indexes the
corpus at multiple resolutions (document, section, claim, symbol, full source)
and returns only the slice that matters. And they never look ahead — ministr
predicts the next request and pre-warms it, so the following tool call is
already in hand instead of a cold fetch.

## Install

The desktop app is the primary way to run ministr. Go to
**[ministr.ai/install](https://ministr.ai/install)**; the page detects your OS
and leads with the matching double-click installer — `.pkg` on macOS, `.exe` on
Windows, `.deb` / `.rpm` / `.AppImage` on Linux. The same bundles are attached
to every [GitHub release](https://github.com/OlsonSoftware/ministr/releases) if
you prefer to download them directly.

Every installer does the same two things: it places the desktop app and it adds
the `ministr` CLI to your PATH. On first launch the app opens a short Setup
screen — identical on macOS, Windows, and Linux — that confirms the CLI is wired
up (with one-click repair if it isn't), then walks you through indexing a
project and connecting your agent. No terminal step is required.

For headless or scripted installs, a CLI one-liner is available below the
installer on the [install page](https://ministr.ai/install).

## Use it in a project

From the app's guided Setup, or from a terminal:

```sh
cd your-project
ministr init
```

`ministr init` configures every supported agent at once. It writes
`.ministr.toml` — the corpus paths and config, auto-detected from your project
manifests (`Cargo.toml`, `package.json`, `pyproject.toml`) — and the MCP client
configs: `.mcp.json` (Claude Code), `.cursor/mcp.json` (Cursor), and
`.vscode/mcp.json` (VS Code / Copilot).

To configure the corpus by hand, `.ministr.toml` only needs the paths to index:

```toml
[corpus]
paths = ["src", "docs", "README.md"]
```

There is no separate build step. ministr indexes automatically the first time an
agent connects.

## What it does

**Semantic search** runs across docs and code and returns results at the
granularity the agent needs — a summary, a section, or a single claim (one
sentence of fact pulled from a section). Code adds two more levels: a symbol
stub (signature plus doc) and full source.

**Symbol navigation** finds and traces structs, functions, and traits across
~29 languages via tree-sitter — Rust, Python, JS/TS, Go, Java, C/C++, C#, Ruby,
Swift, Kotlin, PHP, Scala, Bash, Lua, Elixir, Haskell, OCaml, Dart, R,
HCL/Terraform, SQL, Zig, Protobuf, Svelte, and JSON/YAML/TOML.

**Cross-language bridge detection** links bindings automatically: Tauri commands
and events, napi-rs, PyO3, wasm-bindgen, HTTP routes (actix-web / axum /
rocket), cgo (Go ↔ C), JNI, UniFFI, gRPC, and raw FFI.

**Session tracking** carries deduplication, delta delivery, predictive
prefetch, and budget management — token monitoring, eviction recommendations,
and compressed summaries when the window is under pressure.

**Local embeddings** use Candle with the Metal GPU on Apple Silicon by default,
FastEmbed + DirectML on Windows DirectX 12 GPUs (with the `directml` feature),
and FastEmbed + CPU ONNX everywhere else. Nothing leaves the machine.

**Desktop app** provides a dashboard, a live tool-call activity stream, a `⌘K`
command palette, and system-tray controls (Tauri v2; macOS, Windows, Linux).

## Documentation

- [Docs home](https://ministr.ai/) — full overview
- [Install](https://ministr.ai/install) — desktop installers and CLI scripts
- [Tool reference](https://ministr.ai/docs/tools/) — every MCP tool, with parameters and examples
- [Architecture](https://ministr.ai/docs/architecture-deep-dive/) — system and subsystem deep dive
- [Configuration](https://ministr.ai/docs/configuration/) — `.ministr.toml` options and CLI flags
- [Changelog](CHANGELOG.md)
