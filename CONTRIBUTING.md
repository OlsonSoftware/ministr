# Contributing to ministr

Contributions are welcome — whether it's a bug fix, new language grammar, documentation improvement, or a new feature. This guide covers everything you need to get started.

If you're looking for a place to begin, check the [issues labeled `good first issue`](https://github.com/OlsonSoftware/ministr/labels/good%20first%20issue).

## Development setup

### Prerequisites

- **Rust 1.95** (edition 2024) — pinned via `rust-toolchain.toml`; rustup installs it automatically. Get rustup at [rustup.rs](https://rustup.rs)
- **just** — task runner (`cargo install just` or `brew install just`)
- **cargo-deny** — license and advisory checks (`cargo install cargo-deny`)
- **Python 3** — runs the black-box lint (part of `just validate`); `python3` on Linux/macOS, `python` on Windows

### Clone and build

```sh
git clone https://github.com/OlsonSoftware/ministr.git
cd ministr
cargo build --workspace
```

### Run tests

```sh
just test              # Run all tests
just lint              # Run clippy with pedantic lints
just fmt-check         # Check formatting
just validate          # All three: fmt-check + lint + test
```

### Run ministr locally

Always use `--release` — debug mode is unusably slow due to ONNX runtime + macOS XProtect scanning:

```sh
cargo install --path ministr-cli
ministr index             # Pre-warm the index
ministr serve             # Start MCP server (stdio)
```

This builds the `ministr` binary with the full MCP tool surface, all parsers
and detectors, and self-hosted serve. The cloud service is a separate product
whose code lives in a private repository; you don't need it to develop here.

## Architecture overview

```
ministr-core/          — domain logic, no transport dependencies
ministr-api/           — shared request/response types for daemon ↔ MCP/CLI
ministr-daemon/        — HTTP API over Unix domain socket
ministr-mcp/           — MCP server adapter (rmcp)
ministr-cli/           — binary entry point (the `ministr` CLI you build here)
ministr-app/src-tauri/ — Tauri v2 desktop app with system tray
web/                   — Next.js landing site (static export to GitHub Pages)
deploy/                — self-host reverse-proxy configs + macOS installer
examples/              — sample .ministr.toml configs for different project types
```

The MIT crates above are everything you need to build a fully-functional
`ministr` from this repository. The cloud service's code lives in a separate
private repository. See [STEWARDSHIP.md](STEWARDSHIP.md) for the open-core split.

### Layered architecture

Each crate follows **transport → service → storage** layering:

- **Transport** (ministr-mcp, ministr-daemon): MCP tool handlers, JSON-RPC routing, HTTP API
- **Service**: Business logic — session shadow, prefetch engine, budget manager
- **Storage**: SQLite, HNSW index, file system access

No layer may skip a level. Transport calls service; service calls storage.

### Key subsystems

| Subsystem | Location | Purpose |
|-----------|----------|---------|
| Session Shadow | `ministr-core/src/session/` | Tracks delivered content, skips re-sends, detects drops |
| Prefetch Engine | `ministr-core/src/session/prefetch/` | Six prefetch strategies for proactive content delivery |
| Budget Manager | `ministr-core/src/session/budget.rs` | Advisory token-usage estimate (internal accounting; name pending rename) |
| Coherence | `ministr-core/src/coherence.rs` | Watches filesystem, invalidates stale content |
| Bridge Linker | `ministr-core/src/code/bridge/` | Detects cross-language bindings — 13 kinds: Tauri commands + events, napi-rs, PyO3, wasm-bindgen, HTTP routes, FFI, cgo, JNI, UniFFI, gRPC, Flutter channels, Electron IPC |

### Dependency rule

```
ministr-cli ─── ministr-mcp ─── ministr-core
                    │                 │
                    │            ministr-api
                    │           (shared types)
                  rmcp              │
                (MCP SDK)     ministr-daemon
                              (HTTP/UDS API)
```

Arrows point from consumer to dependency (left to right, top to
bottom). `ministr-core` never imports MCP or transport types.
`ministr-api` never depends on `ministr-core`.

## Making changes

### Workflow

1. **Fork and branch** — create a feature branch from `main`
2. **Write tests first** — we follow TDD (red-green-refactor)
3. **Implement** — follow the conventions below
4. **Validate** — run `just validate` (must pass)
5. **Commit** — use [conventional commits](#commit-messages)
6. **Open a PR** — target `main`, fill in the template

### Coding conventions

- **No `.unwrap()` or `.expect()`** in library code (tests are fine)
- **`#![deny(unsafe_code)]`** in every crate
- **`thiserror`** for error types in ministr-core; **`miette`** for diagnostics in ministr-cli/ministr-mcp
- **`tracing`** for all instrumentation (not `log`)
- **Clippy pedantic** — `cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic`
- **Edition 2024** — use modern Rust idioms

### Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add wasm-bindgen bridge detection
fix: handle empty sections in budget estimation
refactor: extract prefetch strategies into separate modules
test: add integration tests for session persistence
docs: update architecture diagram with bridge linker
chore: bump fastembed to 5.1
```

### Testing

- **Unit tests**: In-module `#[cfg(test)] mod tests` blocks
- **Integration tests**: `tests/` directory with real SQLite + HNSW indexes
- **No mocking storage** — integration tests use real databases
- Use `tempfile::tempdir()` for test fixtures — never test against a live working directory

### Adding a language to the code-intelligence test matrix

The multi-language test suite under `ministr-core/tests/` is gated so a new
language can't land with silent coverage holes. The shared harness is
`tests/langtest/mod.rs` (`IngestedProject::from_files` + `assert_symbol` /
`assert_cross_file_ref` / `assert_range_invariant`). When you add a grammar to
`GrammarRegistry` (or a `BridgeKind`), work this checklist — the coverage
guards **fail CI** until it's done:

1. **Extraction** (`tests/fe2_extraction.rs`): add a `<lang>_extraction_edge_cases`
   test exercising the language's edge cases, and list the language in
   `FE2_COVERED` (or, for a config/markup grammar with no symbol model, add it
   to `EXTRACTION_DEFERRED` with a reason).
2. **References** (`tests/fe3_refs.rs`): add a `<lang>_cross_file_ref_both_orders`
   test (both ingest orders) and list it in `FE3_COVERED` (or `REF_DEFERRED`).
3. **Import edge cases** (`tests/fe3b_import_edges.rs`): cover the language's
   aliased / star / re-export / namespace import shapes, or characterize the
   ones the name-based resolver doesn't map (with a follow-up).
4. **Occurrence index** (`tests/fe5_invariants.rs` + `code::occurrence`): only if
   the language gains an `extract_occurrences` arm.
5. **Bridges** (`tests/bridge_fixtures.rs`): if the language participates in a
   `BridgeKind`, add an e2e link fixture and list the kind in `BRIDGE_COVERED`.
6. **The GA gate** (`tests/fe6_coverage_guard.rs`): update `CODE_LANGUAGES` /
   `NON_CODE_DEFERRED` (code-language dimension) and `BRIDGE_COVERED` (bridge
   dimension). This is the single cross-suite guard — it cross-checks the
   `GrammarRegistry` and `BridgeKind` enums against the fixtures and is what
   turns a missing fixture into a red CI run.

All of these run under `just validate` (they are ordinary `cargo test`
integration tests).

### Quality gates

All of these must pass before merge:

```sh
just validate          # fmt-check + lint + test + black-box guard
just release-preflight # validate + cargo audit + cargo deny + retrieval eval gate + web build
```

CI runs these automatically on every PR.

## PR guidelines

- **Keep PRs focused** — one logical change per PR
- **Include tests** — every new feature or bugfix gets a test
- **Update docs** if you change public API or behavior
- **Link issues** — reference related issues in the PR description
- **Be patient** — reviews may take a few days

## Releases

Releases are cut from this repository's tags using Conventional Commit
messages. See [RELEASE.md](RELEASE.md) for details, and
[ministr-app/SIGNING.md](ministr-app/SIGNING.md) for macOS signing.

## Reporting issues

- Use [GitHub Issues](https://github.com/OlsonSoftware/ministr/issues)
- Include: ministr version, OS, Rust version, and steps to reproduce
- For performance issues, include `RUST_LOG=debug` output

## License

The six MIT-licensed workspace crates are described in
[LICENSE-MIT](LICENSE-MIT). Contributions are accepted under the same MIT
license as outbound (the standard inbound=outbound model) — you retain
copyright in your contribution and grant ministr's users the MIT permissions.
No copyright assignment is required.

See [STEWARDSHIP.md](STEWARDSHIP.md) for the open-core thesis, the list of
what stays MIT, and our public commitment that a feature that ships open
source will not be moved to a paid tier.
