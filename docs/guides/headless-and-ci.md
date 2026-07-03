# Headless and CI

Interactive sessions need none of this — ministr just works. But scripted,
headless agent runs (`claude -p`, CI jobs, benchmark harnesses) configure
tools explicitly, and two flags interact in a way that can make ministr
silently absent while everything looks correctly wired.

## The gotcha

In headless `claude -p` runs, MCP tool schemas can be deferred: the harness
ships only tool names, and the agent must call the loader tool `ToolSearch`
to load a schema before it can call the tool. Two consequences:

1. A restrictive `--allowedTools` list must include `ToolSearch` in
   addition to `mcp__ministr__*`. Without it, every ministr tool is
   unreachable — not declined, not refused: the agent cannot load the
   schemas, and you observe zero ministr calls with no error anywhere.
2. `--setting-sources` must include `project`, or the project-level
   `.claude` deployment (the steering rules and hooks `ministr init`
   installed) is never loaded.

A known-good invocation:

```bash
claude -p "your task" \
  --output-format json \
  --mcp-config .mcp.json --strict-mcp-config \
  --setting-sources project,local \
  --allowedTools "Read Edit Write Bash ToolSearch mcp__ministr__*"
```

The flag behavior above is Claude Code's, not ministr's — re-check it
against the Claude Code documentation when a run misbehaves.

## Verify, don't assume

Treatment received is not the same as treatment configured. Cheap checks:

- **Probe run** — give the agent a trivial prompt asking it to attempt a
  `grep` via Bash and report whether it was steered, then call
  `ministr_toc` and report the result. A correctly wired environment shows
  both.
- **Transcript audit** — count `mcp__ministr__` tool-use entries in the
  session transcript. Zero calls in a run that was supposed to use ministr
  means the wiring failed, not that the agent chose grep.
- `ministr hooks test` validates the hook installation itself.

## CI indexing

Index ahead of time so agent runs don't pay the first-index cost:

```bash
ministr index
```

For ephemeral runners, build the index once and ship it: `ministr export`
produces a portable `.ministr-index` bundle; `ministr import` restores it
on the runner. Containers can override corpus locations with
`MINISTR_CORPUS_PATHS` (colon-separated; overrides every other corpus
source).

## Recorded execution in pipelines

If your agents execute shell commands, the
[recorded execution](recorded-execution.md) family gives you bounded
capture, an audit trail, and cross-process kill.
