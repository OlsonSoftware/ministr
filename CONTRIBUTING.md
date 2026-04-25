# Contributing to ministr

Contributions are welcome — whether it's a bug fix, new language grammar, documentation improvement, or a new feature. This guide covers everything you need to get started.

If you're looking for a place to begin, check the [issues labeled `good first issue`](https://github.com/OlsonSoftware/ministr/labels/good%20first%20issue).

## Development setup

### Prerequisites

- **Rust 1.88+** (edition 2024) — install via [rustup](https://rustup.rs)
- **just** — task runner (`cargo install just` or `brew install just`)
- **cargo-deny** — license and advisory checks (`cargo install cargo-deny`)

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

## Architecture overview

```
ministr-core/          — domain logic, no transport dependencies
ministr-api/           — shared request/response types for daemon ↔ MCP/CLI
ministr-daemon/        — HTTP API over Unix domain socket
ministr-mcp/           — MCP server adapter (rmcp)
ministr-cli/           — binary entry point
ministr-app/src-tauri/ — Tauri v2 desktop app with system tray
```

### Layered architecture

Each crate follows **transport → service → storage** layering:

- **Transport** (ministr-mcp, ministr-daemon): MCP tool handlers, JSON-RPC routing, HTTP API
- **Service**: Business logic — session shadow, prefetch engine, budget manager
- **Storage**: SQLite, HNSW index, file system access

No layer may skip a level. Transport calls service; service calls storage.

### Key subsystems

| Subsystem | Location | Purpose |
|-----------|----------|---------|
| Session Shadow | `ministr-core/src/session/` | Tracks delivered content, deduplicates, detects evictions |
| Prefetch Engine | `ministr-core/src/session/prefetch/` | Six prefetch strategies. Post-read: sequential, structural, topical (always) plus cross-session (monolithic mode only — daemon-proxy path has it scaffolded, not yet wired). Post-survey: survey-expand, agent-plan (intent-based). |
| Budget Manager | `ministr-core/src/session/budget.rs` | Estimates token usage, recommends evictions |
| Coherence | `ministr-core/src/coherence.rs` | Watches filesystem, invalidates stale content |
| Bridge Linker | `ministr-core/src/code/bridge/` | Detects cross-language bindings (Tauri commands + events, napi-rs, PyO3, wasm-bindgen, HTTP routes, raw FFI) |

### Dependency rule

```
ministr-cli  →  ministr-mcp  →  ministr-core
                ↑    ↘        ↑
            uses rmcp  ministr-api (shared types)
            (MCP SDK)     ↑
                      ministr-daemon (UDS API)
```

`ministr-core` never imports MCP or transport types. `ministr-api` never depends on `ministr-core`.

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

### Quality gates

All of these must pass before merge:

```sh
just validate          # fmt-check + lint + test
just deny              # license and advisory checks
just eval-gate         # retrieval quality regression gate
```

CI runs these automatically on every PR.

## PR guidelines

- **Keep PRs focused** — one logical change per PR
- **Include tests** — every new feature or bugfix gets a test
- **Update docs** if you change public API or behavior
- **Link issues** — reference related issues in the PR description
- **Be patient** — reviews may take a few days

## Releases

- **Tag/publish checklist** — [RELEASE.md](RELEASE.md) covers the
  end-to-end flow: pre-flight gates, `just release X.Y.Z`, the two-tag
  split (`vX.Y.Z` for CLI binaries, `vX.Y.Z-app` for the Tauri
  installers), crates.io publish order, and Homebrew tap update.
- **macOS signing & notarization** — the desktop app and CLI binary both
  need a Developer ID identity before distribution. See
  [ministr-app/SIGNING.md](ministr-app/SIGNING.md) for env vars, entitlements,
  and the `just pkg` / `just pkg-dev` workflows.

## Reporting issues

- Use [GitHub Issues](https://github.com/OlsonSoftware/ministr/issues)
- Include: ministr version, OS, Rust version, and steps to reproduce
- For performance issues, include `RUST_LOG=debug` output

## License

By contributing, you agree that your contributions will be dual-licensed under MIT and Apache-2.0, as described in [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
