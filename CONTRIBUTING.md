# Contributing to ministr

Contributions are welcome — whether it's a bug fix, new language grammar, documentation improvement, or a new feature. This guide covers everything you need to get started.

If you're looking for a place to begin, check the [issues labeled `good first issue`](https://github.com/OlsonSoftware/ministr/labels/good%20first%20issue).

## Development setup

### Prerequisites

- **Rust 1.88+** (edition 2024) — install via [rustup](https://rustup.rs)
- **just** — task runner (`cargo install just` or `brew install just`)
- **cargo-deny** — license and advisory checks (`cargo install cargo-deny`)
- **Python 3** — runs the black-box lint (`just blackbox-lint`, part of `just validate`); `python3` on Linux/macOS, `python` on Windows

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

This builds the **local-only** `ministr` binary — every MCP tool, every
parser, every detector, full self-hosted serve. It does NOT include
the cloud-tier surface (multi-tenant Postgres, Stripe billing, GitHub
App / OIDC / SAML, Atlas curated index, license signing) — those live
in the private `ministr-private` workspace and are only built into
the cloud-capable `ministr` binary that
[`install.sh`](install.sh) downloads from this repo's GitHub Releases
page. Contributors don't need the proprietary code to develop here;
all MIT crates compile + test + ship the same way they always have.

## Architecture overview

```
ministr-core/          — domain logic, no transport dependencies
ministr-api/           — shared request/response types for daemon ↔ MCP/CLI
ministr-daemon/        — HTTP API over Unix domain socket
ministr-mcp/           — MCP server adapter (rmcp)
ministr-cli/           — binary entry point (the `ministr` CLI you build here)
ministr-app/src-tauri/ — Tauri v2 desktop app with system tray
web/                   — Next.js marketing site + fumadocs developer docs
deploy/                — Helm chart, Docker Compose, macOS installer
examples/              — sample .ministr.toml configs for different project types
```

The proprietary cloud surface (`ministr-cloud`, `ministr-atlas`,
`ministr-cloud-tools`) lives in a separate private sibling workspace
(`github.com/OlsonSoftware/ministr-private`, owner-only) per the F31
open-core split. The MIT crates above are everything a contributor
needs to build a fully-functional **local-only** `ministr` binary
from this repo. See [STEWARDSHIP.md](STEWARDSHIP.md) for the open-core
thesis and which crates live where.

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

Release authoring lives in this repo; the release **build** lives in
the private sibling. The flow (F31.5, 2026-05-27):

1. Land changes on `main` using [Conventional Commits](#commit-messages).
2. The Copilot coding agent (triggered by
   `.github/workflows/release-automation.yml`) detects releasable
   commits, bumps every crate's `version`, writes a CHANGELOG section,
   and opens a `chore: release vX.Y.Z` PR.
3. Reviewing + merging the PR triggers `release-automation.yml` to
   push the `vX.Y.Z` tag here and fire a `repository_dispatch` event
   into `OlsonSoftware/ministr-private`.
4. The private workspace's `release.yml` clones BOTH repos as
   siblings, builds the cloud-capable `ministr` binary (signed +
   notarized on macOS), and uploads the artifacts to **this repo's**
   release page at `v<X.Y.Z>` via a cross-repo PAT.
5. `install.sh` + the Homebrew tap fetch from this repo's releases as
   before — they see no change.

For the detailed checklist, secret requirements, and rollback
procedures see [RELEASE.md](RELEASE.md). The high-level open-core
framing lives in the
[Release pipeline](STEWARDSHIP.md#release-pipeline) section of
STEWARDSHIP.md. macOS signing and notarization details live in
[ministr-app/SIGNING.md](ministr-app/SIGNING.md).

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
