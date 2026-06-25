# Example `.ministr.toml` configurations

Starter templates for common project layouts. Copy the one that matches your project structure to `.ministr.toml` at your project root, or run `ministr init` to auto-generate one tailored to your project.

## Templates

| Template | When to use |
|---|---|
| [`rust-workspace.ministr.toml`](rust-workspace.ministr.toml) | Cargo workspaces with multiple member crates |
| [`tauri-app.ministr.toml`](tauri-app.ministr.toml) | Tauri v2 apps with Rust backend + TypeScript frontend |
| [`pyo3-project.ministr.toml`](pyo3-project.ministr.toml) | PyO3 extensions with Rust source + Python package |
| [`react-node-monorepo.ministr.toml`](react-node-monorepo.ministr.toml) | React + Node monorepos (pnpm/npm/yarn workspaces) |

## Usage

```sh
# Copy a template to your project
cp examples/rust-workspace.ministr.toml /path/to/your-project/.ministr.toml

# Edit paths to match your layout, then connect your MCP client
claude mcp add ministr -- ministr
```

## Common patterns

All templates share the same structure:

```toml
[corpus]
paths = [
    # Source code directories to index
]

ignore = [
    # Glob patterns to exclude
]
```

ministr automatically ignores `target/`, `node_modules/`, `__pycache__/`, `.git/`, and other common build/dependency directories — you don't need to add them to `ignore`.

## Cross-language bridge detection

ministr auto-detects cross-language bindings when both sides of the boundary are indexed:

- **Tauri** — index both `src-tauri/src` and the frontend source
- **PyO3** — index both the Rust `src` and the Python package
- **napi-rs** — index both the Rust source and the JS/TS consumer
- **HTTP routes** — index both the server and client code

Use `ministr_bridge` to query the detected bindings (see the [README](../README.md#what-it-does)).
