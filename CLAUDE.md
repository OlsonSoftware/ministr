# ministr-rs — contributor notes for Claude Code

This file is auto-loaded by Claude Code when working in this repo. It captures conventions and gotchas specific to developing ministr itself (not using ministr).

## Codebase navigation

Always use ministr MCP tools for exploring this codebase. ministr indexes itself — use the live MCP tools, don't spawn a second instance.

| Task | Tool |
|---|---|
| Vague question | `ministr_survey` |
| Find a symbol by name | `ministr_symbols` |
| Full source of a symbol | `ministr_definition` |
| Callers / implementors | `ministr_references` |
| Read a section | `ministr_read` |
| Structural overview | `ministr_toc` |

Use `Read` only immediately before `Edit`. For everything else, use ministr.

## Quick start

```sh
cargo build --workspace
cargo test --workspace
just validate                 # fmt-check + lint + test
cargo install --path ministr-cli # rebuild the live binary
```

Always use `--release` when running ministr manually — debug builds are unusably slow due to ONNX + macOS XProtect scanning.

## Testing ministr changes

**Never spin up a second ministr instance against this repo.** The MCP server running in-session already indexes it. A second instance shares SQLite and HNSW with the first, corrupting results.

- Use the live MCP tools in your session — that's what they're for.
- After code changes, run `cargo install --path ministr-cli`, then ask the user to restart their session to pick up the new binary. Wait for confirmation before continuing.
- Automated tests use `cargo test` with `tempdir()` fixtures — never point at the live working directory.

## Conventions

- Edition 2024 (Rust 1.85+)
- `#![deny(unsafe_code)]` in every crate
- No `.unwrap()` or `.expect()` in library code (tests are fine)
- `thiserror` for ministr-core errors, `miette` for ministr-cli/ministr-mcp diagnostics
- `tracing` for instrumentation (not `log`)
- Clippy pedantic must pass: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic`

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full contributor guide.
