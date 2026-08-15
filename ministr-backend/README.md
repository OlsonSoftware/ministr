# ministr-backend

The shared query-backend seam for ministr.

One trait — `QueryBackend` — with concrete implementations for the two
deployment shapes, plus the `Backend` dispatch enum that surfaces hold:

- `LocalBackend` — runs every query in-process against a `QueryService`.
- `DaemonBackend` — forwards every query over HTTP to a running
  `ministr-daemon`.
- `DaemonMultiBackend` — daemon forwarding with per-call linked-project
  routing.
- `Backend::Registry` — shares the daemon's in-process `CorpusRegistry`
  (the `serve --transport http` shape).

Consumed by `ministr-mcp` (which re-exports it as `ministr_mcp::backend`
for compatibility) and by `ministr-cli`'s query/index commands. The TUI
deliberately stays on the raw `DaemonClient` — it is management-plane,
not query-plane.
