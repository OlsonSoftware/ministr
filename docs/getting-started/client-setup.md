# Client setup

ministr speaks MCP over stdio: any client that can spawn a process can use
it. The binary discovers `.ministr.toml` by walking up from its working
directory, so no arguments are needed.

`ministr init` writes the per-project configs for Claude Code, Cursor, and
VS Code / Copilot automatically — the sections below are for manual setup or
other clients.

## Claude Code

```sh
claude mcp add ministr -- ministr
```

Or project-scoped `.mcp.json` at the repo root:

```json
{ "mcpServers": { "ministr": { "command": "ministr", "args": [] } } }
```

Verify with `/mcp` inside Claude Code.

## Cursor

`.cursor/mcp.json` with the same shape, or add it via Settings → MCP:

```json
{ "mcpServers": { "ministr": { "command": "ministr" } } }
```

## VS Code / GitHub Copilot

`ministr init` writes `.vscode/mcp.json`. Copilot also reads the steering
files init installs (`.github/copilot-instructions.md`).

## Generic MCP client

Spawn `ministr` as a subprocess and speak MCP over stdin/stdout. A minimal
smoke test: send `initialize`, then `tools/list` — you should see the tool
surface in the [reference](../reference/tools/README.md).

## Agent steering

A client config makes the tools *available*; steering makes agents *prefer*
them. `ministr init` installs advisory rules files and hooks. The behavior
is steer-not-wall: exploration is nudged toward `ministr_survey` /
`ministr_symbols`, while the shell stays unrestricted for building, testing,
git, and filtering command output.

Advisory files (`.md`, `.mdc`) are created if missing but never overwritten —
customize them freely. Hook files are machine-generated and healed back to
the current template when you re-run `init`. Custom rules for all generated
advisory files go in `.ministr.toml`:

```toml
[agent]
rules = ["Always run just validate before committing"]
```

`ministr hooks test` validates the installation by simulating tool calls.

## Headless and CI

If you script an agent headlessly (for example `claude -p` with a restricted
`--allowedTools` list), the allowlist must include `ToolSearch` — clients
that load MCP tools on demand reach them through it, and a list without it
silently removes every MCP tool from the agent.
