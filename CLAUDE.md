# iris-rs — contributor notes for Claude Code

This file is auto-loaded by Claude Code when working in this repo. It captures conventions and gotchas specific to developing iris itself (not using iris).

## Codebase navigation

Always use iris MCP tools for exploring this codebase. iris indexes itself — use the live MCP tools, don't spawn a second instance.

| Task | Tool |
|---|---|
| Vague question | `iris_survey` |
| Find a symbol by name | `iris_symbols` |
| Full source of a symbol | `iris_definition` |
| Callers / implementors | `iris_references` |
| Read a section | `iris_read` |
| Structural overview | `iris_toc` |

Use `Read` only immediately before `Edit`. For everything else, use iris.

## Quick start

```sh
cargo build --workspace
cargo test --workspace
just validate                 # fmt-check + lint + test
cargo install --path iris-cli # rebuild the live binary
```

Always use `--release` when running iris manually — debug builds are unusably slow due to ONNX + macOS XProtect scanning.

## Testing iris changes

**Never spin up a second iris instance against this repo.** The MCP server running in-session already indexes it. A second instance shares SQLite and HNSW with the first, corrupting results.

- Use the live MCP tools in your session — that's what they're for.
- After code changes, run `cargo install --path iris-cli`, then ask the user to restart their session to pick up the new binary. Wait for confirmation before continuing.
- Automated tests use `cargo test` with `tempdir()` fixtures — never point at the live working directory.

## Conventions

- Edition 2024 (Rust 1.85+)
- `#![deny(unsafe_code)]` in every crate
- No `.unwrap()` or `.expect()` in library code (tests are fine)
- `thiserror` for iris-core errors, `miette` for iris-cli/iris-mcp diagnostics
- `tracing` for instrumentation (not `log`)
- Clippy pedantic must pass: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic`

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full contributor guide.
