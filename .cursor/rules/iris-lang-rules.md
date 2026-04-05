# Language Best Practices

Auto-generated rules based on detected project languages.

## Rust

- Use `Result<T, E>` for fallible operations; avoid `.unwrap()` and `.expect()` in library code
- Prefer `&str` over `String` in function parameters; return `String` when ownership is needed
- Use `clippy` lints: `cargo clippy -- -D warnings`
- Prefer iterators and combinators over manual loops
- Use `#[must_use]` on functions returning values that should not be silently ignored
- Derive `Debug` on all public types; derive `Clone`, `PartialEq` where appropriate
- Prefer `thiserror` for library error types, `anyhow`/`miette` for application errors
- Use `cargo fmt` (rustfmt) for consistent formatting
- Place unit tests in the same file with `#[cfg(test)]`; integration tests in `tests/`

