# Example `.iris.toml` configurations

Starter templates for common project layouts. Copy the one that matches your project structure to `.iris.toml` at your project root, or run `iris init` to auto-generate one tailored to your project.

## Templates

| Template | When to use |
|---|---|
| [`rust-workspace.iris.toml`](rust-workspace.iris.toml) | Cargo workspaces with multiple member crates |
| [`tauri-app.iris.toml`](tauri-app.iris.toml) | Tauri v2 apps with Rust backend + TypeScript frontend |
| [`pyo3-project.iris.toml`](pyo3-project.iris.toml) | PyO3 extensions with Rust source + Python package |
| [`react-node-monorepo.iris.toml`](react-node-monorepo.iris.toml) | React + Node monorepos (pnpm/npm/yarn workspaces) |

## Usage

```sh
# Copy a template to your project
cp examples/rust-workspace.iris.toml /path/to/your-project/.iris.toml

# Edit paths to match your layout, then connect your MCP client
claude mcp add iris -- iris
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

iris automatically ignores `target/`, `node_modules/`, `__pycache__/`, `.git/`, and other common build/dependency directories — you don't need to add them to `ignore`.

## Cross-language bridge detection

iris auto-detects cross-language bindings when both sides of the boundary are indexed:

- **Tauri** — index both `src-tauri/src` and the frontend source
- **PyO3** — index both the Rust `src` and the Python package
- **napi-rs** — index both the Rust source and the JS/TS consumer
- **HTTP routes** — index both the server and client code

See [`iris_bridge`](https://AlrikOlson.github.io/iris-rs/tools/bridge.html) for querying detected bindings.
