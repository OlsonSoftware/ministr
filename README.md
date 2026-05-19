# ministr

**Real codebase understanding for AI coding agents.**

ministr is a code intelligence MCP server. It gives AI coding agents
AST-level understanding of your codebase — semantic search across code and
docs, symbol-level navigation, real reference graphs, and cross-language
bridge detection across 40+ languages. It runs locally, embeds locally, and
works with any MCP client — Claude Code, Cursor, VS Code / Copilot.

## What it solves

Agents explore code with `grep` and `read`. Grep matches text, not meaning,
and floods the window with near-misses; reading a whole file to answer a
narrow question wastes most of what comes back. Neither tool knows that a
function has three callers, that a trait has two implementations, or that a
Rust `#[pyfunction]` is what Python calls across the boundary.

ministr replaces that with structure. It parses the codebase into an AST,
indexes it at multiple resolutions (document, section, claim, symbol, full
source), and answers in terms of symbols, references, and language bridges —
returning the exact slice that matters instead of a file dump.

## How it compares

| | ministr | semble | code-graph-mcp | Claude Context |
|---|---|---|---|---|
| Languages parsed | **40+** | 19 | 16 | 14 |
| AST symbol graph | ✅ | — | ✅ | partial |
| Reference graph (callers / implementors) | ✅ | — | ✅ | — |
| Cross-language bridges (Tauri / NAPI / PyO3 / wasm-bindgen / JNI / UniFFI / cgo / gRPC / FFI) | **✅ — 13 kinds** | — | HTTP only | — |
| Multi-resolution (doc / section / claim / symbol / source) | ✅ | chunk only | chunk only | chunk only |
| Docs + code in one corpus | ✅ | code only | code only | code + md |
| Local-only (no cloud, no API key) | ✅ | ✅ | ✅ | cloud-leaning (Milvus + OpenAI) |
| Desktop app | ✅ (Tauri v2) | — | — | — |
| Git URL on demand | ✅ (`GitInclude` + `ministr_clone`) | ✅ | — | — |

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

### As a Claude Code plugin

If you only need the MCP server (and not the desktop app), install ministr as a
Claude Code plugin — this registers the MCP server and adds slash commands
(`/ministr:survey`, `/ministr:symbols`, `/ministr:references`, `/ministr:bridges`,
`/ministr:impact`, `/ministr:dead`, `/ministr:route`):

```sh
claude /plugin install https://github.com/OlsonSoftware/ministr
```

You still need the `ministr` CLI on your `PATH` — install via the CLI one-liner
above. The plugin manifest is at [`.claude-plugin/plugin.json`](.claude-plugin/plugin.json).

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

**Cross-language bridge detection** — the sharpest reason ministr exists. ministr
links cross-language bindings automatically across **13 kinds**: Tauri commands
and events, napi-rs, PyO3, wasm-bindgen, JNI, UniFFI, cgo (Go ↔ C), Electron
IPC, Flutter platform channels, gRPC, HTTP routes (axum / actix-web / rocket /
Express / Flask / Gin), and raw FFI. When you have a Rust `#[tauri::command]`,
ministr knows the TypeScript `invoke("…")` that calls it; when you have a
`#[pyfunction]`, ministr knows the Python import that crosses the PyO3 boundary.
No other code-search tool covers this surface — they stop at the FFI line.

**Symbol navigation and reference graphs** find and trace structs, functions,
traits, and their callers / implementors / importers across 40+ languages via
tree-sitter — Rust, Python, JS/TS, Go, Java, C/C++, C#, Ruby, Swift, Kotlin,
PHP, Scala, Bash, Lua, Elixir, Haskell, OCaml, Dart, R, HCL/Terraform, SQL,
Zig, Protobuf, Svelte, plus CSS, GraphQL, Groovy, Nix, Erlang, PowerShell,
Solidity, Objective-C, Julia, CMake, Make, and JSON/YAML/TOML.

**Semantic search** runs across docs and code and returns results at the
granularity the agent needs — a summary, a section, or a single claim (one
sentence of fact pulled from a section). Code adds two more levels: a symbol
stub (signature plus doc) and full source. Same index serves all five
resolutions; the agent picks the right one instead of getting a whole file
dump.

**Local embeddings** use Candle with the Metal GPU on Apple Silicon by default,
FastEmbed + DirectML on Windows DirectX 12 GPUs (with the `directml` feature),
and FastEmbed + CPU ONNX everywhere else. Nothing leaves the machine.

**Desktop app** provides a dashboard, a live tool-call activity stream, a `⌘K`
command palette, and system-tray controls (Tauri v2; macOS, Windows, Linux).

## For agents

Drop this into your project's `CLAUDE.md` / `AGENTS.md` / `.cursorrules` (or
let `ministr init` write it for you) so the agent reaches for the right tool
instead of grepping:

```markdown
## ministr — code intelligence

Use ministr tools instead of grep / find / Read-to-explore.

**Understanding code**
- Vague question → `ministr_survey(query: "natural language question")`
- Know the symbol name → `ministr_symbols(query: "name")` → `ministr_definition(symbol_id)`
- Know the file → `ministr_toc(document_id: "path")` → `ministr_read(section_id)`
- Need project layout → `ministr_toc(limit: 100)`

**Before changing code**
- Touching shared code → `ministr_references(symbol_id)` first
- Deleting a symbol → `ministr_references(symbol_id)` — zero references means safe
- Changing any IPC / FFI / HTTP boundary → `ministr_bridge` to see every cross-language call site

**Anti-patterns**
- Don't `Read` to explore — use `ministr_read` or `ministr_definition`
- Don't skip `ministr_references` before modifying shared code
- Don't shell out to grep / rg / find — use `ministr_survey` or `ministr_symbols`
```

## Documentation

- [Docs home](https://ministr.ai/) — full overview
- [Install](https://ministr.ai/install) — desktop installers and CLI scripts
- [Tool reference](https://ministr.ai/docs/tools/) — every MCP tool, with parameters and examples
- [Architecture](https://ministr.ai/docs/architecture/) — how ministr is put together
- [Configuration](https://ministr.ai/docs/configuration/) — `.ministr.toml` options and CLI flags
- [Changelog](CHANGELOG.md)

## License and open-core posture

ministr follows an open-core model: the **local stack is MIT-licensed** and
runs entirely on your machine; the hosted **ministr Cloud** service at
`mcp.ministr.ai` and the **Enterprise** on-prem image are paid products built
on proprietary crates added in later phases.

The six workspace crates in this repository — `ministr-core`, `ministr-api`,
`ministr-daemon`, `ministr-mcp`, `ministr-cli`, and `ministr-app/src-tauri` —
are MIT-licensed. See [LICENSE-MIT](LICENSE-MIT) for the canonical text and
[STEWARDSHIP.md](STEWARDSHIP.md) for our commitment that **a feature that
ships open source will not be moved to a paid tier**.
