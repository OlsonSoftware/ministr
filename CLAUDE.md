# iris-rs

A Rust-native MCP server that manages LLM agent context windows like a CPU cache controller — with session tracking, predictive prefetching, budget management, and coherence.

## MCP Tool Priorities

1. **Brave Search** (`brave_web_search`) — use liberally for research, best practices, API docs, ecosystem patterns. Always search before building.
2. **iris** — use for all codebase navigation. Do NOT use Glob, Grep, or Read for discovery.
3. **magistr** — use for quality gates, roadmap tracking, and workflow phase management.

## Codebase Navigation

**Always use iris MCP tools** for exploring this codebase. Do NOT use Glob, Grep, or Read for discovery.

- `iris_survey` — semantic search across docs and source code. Start here.
- `iris_symbols` — find structs, functions, traits, enums by name/kind/module.
- `iris_definition` — get full source of a symbol by ID.
- `iris_references` — find callers, implementors, importers of a symbol.
- `iris_read` — read a section by ID (with deduplication and delta delivery).
- `iris_extract` — get atomic claims from a section, optionally filtered by query.
- `iris_toc` — structural overview of the indexed corpus.

Use `Read` only immediately before `Edit`. For everything else, use iris.

See `.claude/rules/tools.md` for the full tool guide including magistr workflow tools.

## Quick Start

```sh
cargo build --workspace          # build
cargo test --workspace           # test
just validate                    # fmt-check + lint + test
iris index --corpus ./iris-core/src  # pre-warm the index
```

Always use `--release` when running iris (debug mode is unusably slow due to ONNX + XProtect).

## Testing iris Changes

**NEVER spin up a second iris instance against this repo.** The iris MCP server is already running on this codebase. A second instance causes conflicts — shared SQLite, shared HNSW indexes, shared session state — leading to corrupted results and false conclusions.

- **Using the live MCP tools** (iris_survey, iris_symbols, etc.) in a session is fine — that's what they're for
- **After implementing changes to iris**, run `cargo install --path iris-cli` to rebuild, then ask the user to restart the session so the new binary is picked up by the MCP server. **Wait for explicit go-ahead** before continuing — do not proceed until the user confirms the new session is ready
- For automated tests: use `cargo test` with `tempdir()` fixtures — never point test harnesses at the live working directory
- Never run `iris index --corpus ./iris-core/src` or spin up a CLI instance against this repo while the MCP server is running

## Key Conventions

- See `.claude/rules/conventions.md` for full coding conventions
- See `.claude/rules/workflow.md` for the Research → Plan → Execute → Verify workflow
- No `.unwrap()` or `.expect()` in library code (tests are fine)
- `#![deny(unsafe_code)]` in every crate
- All quality gates must pass before committing
