# CLI Reference

The `iris` binary provides subcommands for running the MCP server, indexing corpora, managing projects, and distributing pre-built indexes.

```
iris [OPTIONS] [COMMAND]
```

If no subcommand is given, `iris` runs `serve` with stdio transport — the default behavior for MCP clients.

## Commands

| Command | Purpose |
|---|---|
| `serve` | Start the MCP server (stdio or HTTP transport) |
| `index` | Run ingestion synchronously and exit (no MCP server) |
| `init` | Generate `.iris.toml` and scaffold agent configs |
| `status` | Show daemon status |
| `search` | Search the corpus via the daemon |
| `export` | Export the corpus index to a portable bundle |
| `import` | Import a bundle into the local corpus store |
| `hooks test` | Validate installed agent hooks |

## Global options

| Flag | Description |
|---|---|
| `-c, --corpus <PATH>` | Corpus source (repeatable): local paths, `https://` URLs, or `github://` URLs |
| `-C, --config <PATH>` | Path to config file (default: `~/.iris/config.toml`) |
| `-h, --help` | Print help |
| `-V, --version` | Print version |

## `iris serve`

Starts the MCP server. Default transport is stdio — any MCP client can spawn `iris` as a subprocess and exchange JSON-RPC over stdin/stdout.

```sh
iris serve                                              # stdio (default)
iris serve --transport http --host 0.0.0.0 --port 8080  # Streamable HTTP
iris serve --transport http --oauth                     # with OAuth 2.1
iris serve --proxy                                      # delegate to iris daemon
```

| Flag | Description |
|---|---|
| `-t, --transport <TRANSPORT>` | `stdio` (default) or `http` (Streamable HTTP) |
| `--host <HOST>` | Bind host for HTTP transport (default: `127.0.0.1`) |
| `-p, --port <PORT>` | HTTP port (default: `8080`) |
| `--proxy` | Run as thin proxy to the iris daemon over `~/.iris/irisd.sock` |
| `--oauth` | Enable OAuth 2.1 authentication (HTTP transport only) |
| `--oauth-issuer <URL>` | OAuth issuer URL (default: `http://<host>:<port>`) |

The `--proxy` mode uses ~20 MB of RAM vs ~2 GB for the monolithic server, making it ideal when running alongside the iris desktop app.

## `iris index`

Runs corpus ingestion synchronously and exits. Useful for pre-warming the index, debugging ingestion issues, or running in CI.

```sh
iris index                                  # index from .iris.toml
iris index --corpus ./src --corpus ./docs   # explicit paths
```

iris indexes on first MCP connection automatically — `iris index` is optional.

## `iris init`

Generates `.iris.toml` with auto-detected corpus paths and scaffolds agent configuration files. See [iris init](tools/init.md) for the full reference.

```sh
iris init                  # non-interactive, auto-detected
iris init --interactive    # wizard with prompts
iris init --force          # overwrite existing .iris.toml
```

## `iris status`

Shows the iris daemon status — whether it's running, which corpora are registered, ingestion state, and socket path. Requires the iris desktop app or `iris-daemon` to be running.

## `iris search`

One-shot search against the daemon without going through the MCP protocol. Useful for debugging and shell scripts.

```sh
iris search "authentication" --top-k 5
```

## `iris export` / `iris import`

Export the corpus index to a portable `.iris-index` bundle (zstd-compressed archive of the content database and HNSW index). Import on another machine to skip re-indexing.

```sh
iris export -o my-corpus.iris-index
iris import my-corpus.iris-index
```

Session-local data is stripped during export — only the corpus content and embeddings are included.

## `iris hooks test`

Validates that installed agent hooks are correctly configured and simulates tool calls to report which would be blocked.

```sh
iris hooks test
```

## Environment variables

| Variable | Description |
|---|---|
| `RUST_LOG` | Log verbosity (`iris=debug`, `iris_core=trace`, etc.) |
| `IRIS_DATA_DIR` | Override the data directory (default: `~/.iris`) |
