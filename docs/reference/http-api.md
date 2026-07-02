# Daemon HTTP API

<!-- hand-maintained; source of truth: the sub-router definitions in
     ministr-daemon/src/daemon.rs. Update when routes change. -->

The ministr daemon serves this API to local clients (the desktop app and the
CLI's `status`/`search` commands). Most tool-shaped routes mirror the
[MCP tools](tools/README.md) — agents should use the tools; this surface is
for programmatic and UI access.

## Read routes

| Route | Method |
|---|---|
| `/api/v1/status` | GET |
| `/api/v1/corpora` | GET |
| `/api/v1/corpora/{id}` | GET |
| `/api/v1/corpora/{id}/survey` | POST |
| `/api/v1/corpora/{id}/symbols` | POST |
| `/api/v1/corpora/{id}/definition/{sym}` | GET |
| `/api/v1/corpora/{id}/references/{sym}` | GET |
| `/api/v1/corpora/{id}/impact/{sym}` | GET |
| `/api/v1/corpora/{id}/diff-impact` | GET |
| `/api/v1/corpora/{id}/dead` | POST |
| `/api/v1/corpora/{id}/solid` | POST |
| `/api/v1/corpora/{id}/diagnostics` | POST |
| `/api/v1/corpora/{id}/files` | GET |
| `/api/v1/corpora/{id}/freshness` | GET |
| `/api/v1/corpora/{id}/freshness-summary` | GET |
| `/api/v1/corpora/{id}/outcomes` | GET |
| `/api/v1/corpora/{id}/indexed-file` | POST |
| `/api/v1/corpora/{id}/file` | POST |
| `/api/v1/corpora/{id}/occurrences` | POST |
| `/api/v1/corpora/{id}/read/{section}` | GET |
| `/api/v1/corpora/{id}/extract` | POST |
| `/api/v1/corpora/{id}/toc` | POST |
| `/api/v1/corpora/{id}/related` | POST |
| `/api/v1/corpora/{id}/bridge` | POST |
| `/api/v1/corpora/{id}/bridge/graph` | GET |
| `/api/v1/corpora/{id}/compress` | POST |
| `/api/v1/corpora/{id}/progress` | GET (SSE) |
| `/api/v1/progress` | GET (SSE) |
| `/api/v1/corpora/{id}/coherence` | GET (SSE) |
| `/api/v1/corpora/{id}/prefetch` | GET |
| `/api/v1/corpora/{id}/sessions/{sid}/usage` | GET |
| `/api/v1/corpora/{id}/sessions/{sid}/read/{section}` | GET |
| `/api/v1/sessions` | GET |

## Write routes

| Route | Method |
|---|---|
| `/api/v1/corpora` | POST (register) |
| `/api/v1/corpora/{id}` | DELETE (unregister) |
| `/api/v1/corpora/{id}/clone` | POST |
| `/api/v1/corpora/{id}/reindex` | POST |
| `/api/v1/corpora/{id}/paths` | PUT |
| `/api/v1/corpora/{id}/sessions` | POST (create) / DELETE (clear all) |
| `/api/v1/corpora/{id}/sessions/{sid}` | DELETE |
| `/api/v1/corpora/{id}/sessions/{sid}/dropped` | POST |

## Bundles

| Route | Method |
|---|---|
| `/api/v1/corpora/import` | POST |
| `/api/v1/corpora/{id}/export` | POST |

## Ask

| Route | Method | Note |
|---|---|---|
| `/api/v1/corpora/{id}/ask` | POST | requires a `claude` binary on the daemon's PATH; returns 404 where it isn't mounted |

## Observability

| Route | Method |
|---|---|
| `/activity` | GET (`?limit=&since=` snapshot of recent tool-call activity) |
| `/coherence-events` | GET |

## Recorded execution

The exec engine is hosted in the daemon — one engine per daemon, so kills and
live log tails work across client processes. Commands run cwd-restricted to
indexed corpus roots.

| Route | Method |
|---|---|
| `/exec/runs` | POST (start) / GET (list) |
| `/exec/runs/{id}` | GET |
| `/exec/runs/{id}/logs` | GET |
| `/exec/runs/{id}/kill` | POST |
