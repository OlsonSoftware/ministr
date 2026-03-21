# iris — Coding Conventions

## Toolchain

- **Edition**: 2024 (Rust 1.85+)
- **Build**: `cargo build --workspace`
- **Test**: `cargo test --workspace` or `just test`
- **Lint**: `cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic`
- **Format**: `cargo fmt --all`
- **Coverage**: `cargo llvm-cov --workspace --html`
- **Audit**: `cargo audit` + `cargo deny check`
- **Task runner**: `just` — all commands must be runnable via justfile recipes

## Workspace Structure

```
iris-core/     — domain logic, no transport dependencies
iris-mcp/      — MCP server, depends on iris-core + rmcp
iris-cli/      — binary entry point, depends on iris-mcp
```

iris-core MUST NOT depend on rmcp or any MCP protocol types. The service layer
in iris-core exposes plain Rust traits and types; iris-mcp adapts them to MCP.

## Layered Architecture

Each crate follows transport → service → storage layering:

- **Transport** (iris-mcp only): MCP tool handlers, JSON-RPC routing, request/response mapping
- **Service**: Business logic — session shadow, prefetch engine, budget manager, query orchestration
- **Storage**: SQLite via rusqlite, HNSW index, file system access, memory-mapped I/O

No layer may skip a level. Transport calls service; service calls storage.
Storage never calls service; service never calls transport.

## Error Handling

- **iris-core**: Use `thiserror` for all error types. Define one error enum per module
  (e.g., `IndexError`, `SessionError`, `StorageError`). Errors must be matchable.
- **iris-cli / iris-mcp**: Use `miette` for diagnostic error reports. Wrap core errors
  with `.into_diagnostic()` and add `.context()` for user-facing messages.
- Use `#[from]` for automatic error conversions within a crate.
- Never use `.unwrap()` or `.expect()` in library code. In tests, prefer `.unwrap()` over `?`.
- Return `Result<T, E>` from all fallible functions. Never silently swallow errors.

## Naming

- `snake_case` for functions, methods, variables, modules, and file names
- `CamelCase` for types (structs, enums, traits) and type parameters
- `SCREAMING_SNAKE_CASE` for constants and statics
- Follow Rust API Guidelines (RFC 430) for naming conventions
- Prefix private helper functions with `_` only if needed to avoid name collisions

## Async

- All async code runs on the tokio runtime
- Use `#[tokio::test]` for async tests
- Never block the async runtime: no `std::thread::sleep`, no synchronous file I/O
  on the async path. Use `tokio::fs` or `spawn_blocking` for blocking operations.
- Use `tokio::select!` for concurrent operations, not manual polling

## Testing

- **Unit tests**: In-module `#[cfg(test)] mod tests { ... }` blocks
- **Integration tests**: `tests/` directory, test against real SQLite + HNSW indexes
- **Doc tests**: Required for non-trivial public API examples
- **Coverage**: Maintain coverage with `cargo-llvm-cov`. Track via `just coverage`
- **No mocking storage**: Integration tests use real databases, not mocks
- Prefer `assert_eq!` and `assert!` with descriptive messages
- Test the session shadow, prefetch engine, and budget manager exhaustively —
  these are the novel subsystems where correctness is critical

## Tracing & Logging

- Use `tracing` crate for all instrumentation (not `log`)
- Add `#[instrument]` to public functions with meaningful skip/fields attributes
- Use spans for MCP tool calls, prefetch operations, and index queries
- Log levels: ERROR (unrecoverable), WARN (degraded), INFO (lifecycle events),
  DEBUG (internal state), TRACE (hot path details)
- Configure via `RUST_LOG` env var with `tracing-subscriber::EnvFilter`

## Documentation

- Every public function, type, trait, and module gets a `///` or `//!` doc comment
- Doc comments describe **what** and **why**, not **how** (the code shows how)
- Include `# Examples` sections with doc tests for non-trivial APIs
- Architecture docs live in the mdBook (`docs/`) — keep code comments focused on API usage
- Reference DESIGN.md for architectural rationale, don't duplicate it in code comments

## Dependencies

- Use well-known crates for solved problems: tokio, serde, rusqlite, fastembed, rmcp, comrak
- Build custom only for the three novel subsystems: session shadow, prefetch engine, budget manager
- Pin major versions in Cargo.toml. Use `cargo update` deliberately, not automatically
- All deps must pass `cargo audit` and `cargo deny check` — these are CI gates

## Security

- `#![deny(unsafe_code)]` in every crate — no exceptions
- Rely on safe abstractions from dependencies (fastembed, rusqlite, memmap2)
- `cargo-audit` and `cargo-deny` run in CI as blocking gates
- Validate all external input (file paths, MCP tool parameters)
- Never execute user-provided strings as code or shell commands

## Anti-Patterns

- No `clone()` to satisfy the borrow checker without understanding why — fix the ownership
- No `Arc<Mutex<T>>` when a channel or message-passing design would be cleaner
- No `Box<dyn Error>` in library code — use typed errors
- No premature optimization — profile first with `cargo flamegraph`, then optimize hot paths
- No global mutable state — pass dependencies via function parameters or constructor injection
- No `.unwrap()` in non-test code
