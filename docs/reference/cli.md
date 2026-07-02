# CLI

<!-- hand-maintained; source of truth: `cargo run -p ministr-cli -- help` and
     per-subcommand `--help`. Update when the clap surface changes. -->

`ministr` runs an MCP server over stdio by default (`ministr` with no
subcommand is `ministr serve`).

Global options, accepted by every subcommand:

| Option | Effect |
|---|---|
| `-c, --corpus <CORPUS>` | corpus sources: local paths, `https://` URLs, or `github://` URLs; repeatable |
| `-C, --config <CONFIG>` | path to the global config file (default `~/.ministr/config.toml`) |

Note on `--corpus` precedence as currently implemented: when the working
directory has a `.ministr.toml`, its `[corpus]` paths take precedence over an
explicit `--corpus` flag.

## `ministr serve`

Start the MCP server. Default transport is stdio; `--transport http` starts a
Streamable HTTP server for remote or multi-client use.

| Option | Default | Effect |
|---|---|---|
| `-t, --transport <stdio\|http>` | `stdio` | `stdio` = JSON-RPC over stdin/stdout; `http` = Streamable HTTP (MCP spec 2025-03-26) |
| `--host <HOST>` | `127.0.0.1` | bind host (HTTP only) |
| `-p, --port <PORT>` | `8080` | bind port (HTTP only) |
| `--oauth` | off | require OAuth 2.1 Bearer tokens on the MCP endpoint and expose OAuth discovery endpoints (HTTP only) |
| `--oauth-issuer <URL>` | `http://<host>:<port>` | issuer URL in OAuth discovery metadata; set to the public URL behind a reverse proxy |

Without `--oauth`, HTTP serving is unauthenticated — local development only.

## `ministr index`

Run corpus ingestion synchronously and exit, without starting a server.
Useful for pre-warming the index, debugging ingestion, or CI pipelines.

## `ministr init`

Generate `.ministr.toml` with auto-detected settings: scans for project
manifests (`Cargo.toml`, `package.json`, `pyproject.toml`), detects workspace
layouts and bridge frameworks, and writes MCP client configs and agent
steering (see [client setup](../getting-started/client-setup.md)).

| Option | Effect |
|---|---|
| `--force` | overwrite an existing `.ministr.toml` |
| `-i, --interactive` | show the detected project type and exactly what will be written, confirm before scaffolding |
| `--exec-only` | opt in to exec-only steering: the scaffolded hook denies the raw Bash tool and redirects the agent to the recorded `ministr_run` tool family; reversible by deleting `.claude/hooks/ministr-exec-only` |

## `ministr status`

Show daemon status. Requires the ministr daemon to be running.

## `ministr search <QUERY>`

Search the corpus via the daemon. `-k, --top-k <N>` caps results (default 10).
Requires the daemon to be running.

## `ministr export` / `ministr import <BUNDLE>`

`export` writes the corpus index to a portable `.ministr-index` bundle — a
zstd-compressed archive of the content database (session-local data
stripped), the vector index, and a metadata manifest; `-o, --output <PATH>`
overrides the destination (default `<corpus-name>.ministr-index`). `import`
loads a bundle into the local corpus store, ready for querying without
re-parsing or re-embedding.

## `ministr hooks test`

Validate installed agent hooks by simulating tool calls.

## `ministr setup`

Add the `ministr` binary's directory to the user's PATH. Detects installed
shells (bash, zsh, fish, nushell, PowerShell, tcsh, xonsh) and edits the
appropriate rc files; on Windows, writes the per-user registry PATH entry.
Idempotent.

| Option | Effect |
|---|---|
| `--bin-dir <DIR>` | directory to add or remove (default: parent of the running binary) |
| `--dry-run` | print what would be edited without writing |
| `--uninstall` | remove the directory from PATH instead |
