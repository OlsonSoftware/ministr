# Recorded execution

Agents need to run commands; you need to know what ran. The `ministr_run`
tool family replaces fire-and-forget shell-outs with recorded execution:
every run is captured, bounded, attributed, and auditable.

## What a run gives you

- **Bounded capture** — output is captured head and tail with a fixed cap
  per side, so a runaway command can't exhaust memory; total byte and line
  counts are kept even when the middle is dropped.
- **A token-lean digest** — the agent gets the exit code plus a digest of
  the error lines, not a wall of build output. The full log stays available
  through `ministr_run_logs`, which pages only what the agent hasn't seen
  yet.
- **An audit trail** — every run lands in `~/.ministr/exec_runs.db` with
  command, working directory, status, timing, and session attribution. The
  desktop app reads the same store.
- **Cross-process control** — runs execute in the daemon-hosted engine (one
  per machine), so a run started by an agent can be watched and killed from
  the desktop app, and survives the agent's own process.
- **Searchable history** — a finished run is rendered as a bounded report
  and indexed into your corpus, so a later `ministr_survey` can find what
  failed previously.

## Guardrails

- A run's working directory must be inside an indexed corpus root — the
  engine canonicalizes and enforces this. Agents execute where your code
  is, not anywhere on disk.
- Timeouts: 600 seconds by default, 3600 maximum. Background runs return a
  `run_id` immediately; poll with `ministr_run_status`.
- Cancellation kills the whole process group on Unix.
- Honest limit: steering an agent toward recorded runs is a workflow
  improvement, not a security boundary. The enforced boundary is the
  working-directory policy above.

## Exec-only mode

```bash
ministr init --exec-only
```

writes a marker that makes the installed Claude Code hooks deny the raw
Bash tool and redirect to the recorded run family. Delete
`.claude/hooks/ministr-exec-only` to reverse it.

## The four tools

| Tool | Purpose |
|---|---|
| `ministr_run` | run a command, foreground or `background: true` |
| `ministr_run_logs` | page the captured log (delta) or filter with `query` |
| `ministr_run_status` | poll status, exit code, duration |
| `ministr_run_kill` | cancel a running run |

Schemas and parameters: the [tool reference](../reference/tools/README.md).
