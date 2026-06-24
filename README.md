# ministr

**Real codebase understanding for AI coding agents.**

ministr is a local, MIT-licensed code intelligence MCP server. It gives AI
coding agents AST-level understanding of your codebase — semantic search across
code and docs, symbol-level navigation, real reference graphs, and
cross-language bridge detection across 40+ languages. It runs locally, embeds
locally, and works with any MCP client — Claude Code, Cursor, VS Code / Copilot.

## What it solves

Agents explore code with `grep` and `read`. Grep matches text, not meaning, and
floods the window with near-misses; reading a whole file to answer a narrow
question wastes most of what comes back. Neither tool knows that a function has
three callers, that a trait has two implementations, or that a Rust
`#[pyfunction]` is what Python calls across the boundary.

ministr replaces that with structure. It parses the codebase into an AST,
indexes it at multiple resolutions (document, section, claim, symbol, full
source), and answers in terms of symbols, references, and language bridges —
returning the exact slice that matters instead of a file dump.

## Install from source

ministr builds from source with one command on macOS, Linux, and Windows.

**Prerequisites**

- [Rust](https://rustup.rs) (the toolchain is pinned to 1.95.0 via
  `rust-toolchain.toml`; rustup installs it automatically on first build).
- A C toolchain for native dependencies: Xcode Command Line Tools on macOS
  (`xcode-select --install`), `build-essential` on Debian/Ubuntu (or the
  equivalent `gcc`/`clang` + `make` on other distros), and the
  [Visual Studio C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/)
  on Windows.

**One line — macOS / Linux**

```sh
cargo install --git https://github.com/OlsonSoftware/ministr --locked ministr-cli
```

**One line — Windows (PowerShell)**

```powershell
cargo install --git https://github.com/OlsonSoftware/ministr --locked ministr-cli
```

That builds the workspace and installs the `ministr` binary into Cargo's bin
directory (`~/.cargo/bin` on macOS/Linux, `%USERPROFILE%\.cargo\bin` on
Windows), which rustup already puts on your `PATH`. Confirm it:

```sh
ministr --version
```

**From a local clone** (any OS — handy if you're hacking on ministr):

```sh
git clone https://github.com/OlsonSoftware/ministr
cd ministr
cargo install --path ministr-cli --locked
```

On Windows you can opt into DirectML GPU acceleration by adding
`--features directml` to either command.

> Verified by build: `cargo install` from source compiles the MIT workspace and
> installs a working `ministr 0.7.0` (validated on macOS / Apple Silicon with the
> pinned Rust 1.95.0 toolchain). The local-clone form and the `--git` form share
> the same compile path.

Prebuilt binaries and the desktop app are also attached to each
[GitHub release](https://github.com/OlsonSoftware/ministr/releases) when one is
published.

### As a Claude Code plugin

If you only need the MCP server, install ministr as a Claude Code plugin — this
registers the MCP server and adds slash commands (`/ministr:survey`,
`/ministr:symbols`, `/ministr:references`, `/ministr:bridges`, `/ministr:impact`,
`/ministr:dead`, `/ministr:route`):

```sh
claude /plugin install https://github.com/OlsonSoftware/ministr
```

You still need the `ministr` CLI on your `PATH` (install from source above). The
plugin manifest is at [`.claude-plugin/plugin.json`](.claude-plugin/plugin.json).

## Use it in a project

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

**Cross-language bridge detection** — the sharpest reason ministr exists. It
links cross-language bindings automatically across **13 kinds**: Tauri commands
and events, napi-rs, PyO3, wasm-bindgen, JNI, UniFFI, cgo (Go ↔ C), Electron
IPC, Flutter platform channels, gRPC, HTTP routes (axum / actix-web / rocket /
Express / Flask / Gin), and raw FFI. When you have a Rust `#[tauri::command]`,
ministr knows the TypeScript `invoke("…")` that calls it; when you have a
`#[pyfunction]`, ministr knows the Python import that crosses the PyO3 boundary.

**Symbol navigation and reference graphs** find and trace structs, functions,
traits, and their callers / implementors / importers across 40+ languages via
tree-sitter — Rust, Python, JS/TS, Go, Java, C/C++, C#, Ruby, Swift, Kotlin,
PHP, Scala, Bash, Lua, Elixir, Haskell, OCaml, Dart, R, HCL/Terraform, SQL,
Zig, Protobuf, Svelte, and more.

**Semantic search** runs across docs and code and returns results at the
granularity the agent needs — a summary, a section, or a single claim. Code adds
two more levels: a symbol stub (signature plus doc) and full source. The same
index serves all five resolutions; the agent picks the right one instead of
getting a whole file dump.

**Local embeddings** use Candle with the Metal GPU on Apple Silicon by default,
FastEmbed + DirectML on Windows DirectX 12 GPUs (with the `directml` feature),
and FastEmbed + CPU ONNX everywhere else. Nothing leaves the machine.

## For agents

Drop this into your project's `CLAUDE.md` / `AGENTS.md` / `.cursorrules` (or let
`ministr init` write it for you) so the agent reaches for the right tool instead
of grepping:

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

## Repository

- [`CHANGELOG.md`](CHANGELOG.md) — release history
- [`CONTRIBUTING.md`](CONTRIBUTING.md) — how to build, test, and contribute
- [`SECURITY.md`](SECURITY.md) — reporting vulnerabilities
- [`STEWARDSHIP.md`](STEWARDSHIP.md) — the open-core thesis and our commitments
- [`AGENTS.md`](AGENTS.md) — agent-facing project notes

## License and open-core posture

ministr follows an open-core model: the **local stack is MIT-licensed** and runs
entirely on your machine. The six MIT-licensed workspace crates — `ministr-core`,
`ministr-api`, `ministr-daemon`, `ministr-mcp`, `ministr-cli`, and
`ministr-app/src-tauri` — build a complete, fully-functional `ministr` binary
from this repo via `cargo build --workspace`; they have no cloud or proprietary
dependencies.

A hosted cloud service and an on-prem Enterprise image are separate paid products
built on proprietary crates in a private sibling repository. See
[LICENSE](LICENSE) for the canonical MIT text and [STEWARDSHIP.md](STEWARDSHIP.md)
for the open-core thesis and our commitment that **a feature that ships open
source will not be moved to a paid tier**.
