# Quickstart

```sh
cd your-project
ministr init
```

`init` detects your project structure and writes:

- **`.ministr.toml`** — what to index: detected source and doc paths, with
  a `sparse_weight` set for code projects (see
  [configuration](../guides/configuration.md))
- **MCP client configs** — `.mcp.json` (Claude Code), `.cursor/mcp.json`
  (Cursor), `.vscode/mcp.json` (VS Code / Copilot), merged non-destructively
- **Agent steering** — advisory rules files plus hooks that nudge agents
  toward ministr's tools for code exploration, and a universal `AGENTS.md`

Restart your coding tool (in Claude Code, `/mcp` verifies the connection).
Indexing happens automatically the first time an agent connects; `ministr
index` is optional pre-warming.

`ministr hooks test` validates the installed steering by simulating tool
calls.

## What just happened

Your agent now has 25 extra tools — semantic search over code and docs,
symbol navigation with real reference graphs, cross-language bridge
detection, structured diagnostics, and recorded shell execution. The MCP
server carries its own usage instructions to the agent, so there is nothing
to prompt. See the [tool reference](../reference/tools/README.md).

## Next

- [Client setup](client-setup.md) — every supported client, and how steering works
- [Configuration](../guides/configuration.md) — `.ministr.toml` in full
