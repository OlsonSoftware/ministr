# MCP Client Setup

iris communicates over **stdio** using the Model Context Protocol (MCP). Any MCP-compatible client can connect by spawning the `iris` binary as a subprocess.

This guide covers configuration for popular clients and a generic JSON-RPC approach.

## Claude Code

Claude Code supports three MCP configuration scopes. Pick the one that fits your workflow.

### Project-scoped (recommended for teams)

Create `.mcp.json` at your project root. This file can be checked into version control so every team member gets iris automatically.

```json
{
  "mcpServers": {
    "iris": {
      "command": "iris",
      "args": ["--corpus", "./docs"]
    }
  }
}
```

### User-scoped (all projects)

Add iris to `~/.claude.json` to make it available across all projects:

```json
{
  "mcpServers": {
    "iris": {
      "command": "iris",
      "args": ["--corpus", "/absolute/path/to/docs"]
    }
  }
}
```

### CLI shorthand

```sh
claude mcp add iris -- iris --corpus ./docs
```

This writes the entry to your local-scoped configuration in `~/.claude.json`.

### Verifying the connection

After adding the server, start Claude Code and run:

```
/mcp
```

You should see `iris` listed with its tools: `iris_survey`, `iris_read`, `iris_extract`, `iris_related`, `iris_budget`, `iris_compress`, and `iris_evicted`.

## Cursor

Cursor reads MCP configuration from `.cursor/mcp.json` (project-scoped) or `~/.cursor/mcp.json` (global).

### Project-scoped

Create `.cursor/mcp.json` in your project root:

```json
{
  "mcpServers": {
    "iris": {
      "command": "iris",
      "args": ["--corpus", "./docs"]
    }
  }
}
```

### Global

Create or edit `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "iris": {
      "command": "iris",
      "args": ["--corpus", "/absolute/path/to/docs"]
    }
  }
}
```

### Settings UI

Alternatively, open **Cursor Settings > Tools & MCP > Add New MCP Server** and enter:

- **Name**: `iris`
- **Command**: `iris`
- **Arguments**: `--corpus /path/to/docs`

Restart Cursor after adding the configuration.

## Generic JSON-RPC Client

iris speaks JSON-RPC 2.0 over stdin/stdout. Any client that can spawn a subprocess and exchange newline-delimited JSON can connect.

### Spawning iris

```sh
iris --corpus /path/to/docs
```

iris reads JSON-RPC requests from stdin and writes responses to stdout. Logs go to stderr, so they don't interfere with the protocol stream.

### Initialize handshake

Send the MCP `initialize` request:

```json
{"jsonrpc": "2.0", "method": "initialize", "params": {"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "my-client", "version": "1.0"}}, "id": 1}
```

iris responds with its server capabilities and the list of available tools.

### Calling a tool

```json
{"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "iris_survey", "arguments": {"query": "authentication"}}, "id": 2}
```

### Quick smoke test

Pipe a single request to verify iris starts correctly:

```sh
echo '{"jsonrpc":"2.0","method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0.1"}},"id":1}' \
  | iris --corpus ./docs 2>/dev/null
```

## Environment Variables

| Variable | Description |
|---|---|
| `RUST_LOG` | Controls log verbosity. Example: `RUST_LOG=iris_core=debug,iris_mcp=info` |

You can pass environment variables in MCP configurations:

```json
{
  "mcpServers": {
    "iris": {
      "command": "iris",
      "args": ["--corpus", "./docs"],
      "env": {
        "RUST_LOG": "iris_core=info"
      }
    }
  }
}
```

## Troubleshooting

**iris not found**: Ensure the `iris` binary is on your `PATH`. If installed via `cargo install --path iris-cli`, it will be at `~/.cargo/bin/iris`. Add `~/.cargo/bin` to your shell profile if needed.

**No tools listed**: Check that the `--corpus` path exists and contains `.md`, `.html`, or `.pdf` files. iris logs indexing progress to stderr — run it manually to see diagnostics.

**Connection drops**: iris communicates over stdio. Make sure no other process is reading stdin or writing to stdout in the same pipe. Logs always go to stderr.

**Slow first start**: On first run, iris downloads the embedding model (~80 MB for `all-MiniLM-L6-v2`). Subsequent starts reuse the cached model.
