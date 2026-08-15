# Daemon HTTP API

<!-- hand-maintained; source of truth: the sub-router definitions in
     ministr-daemon/src/daemon.rs. Update when routes change. -->

The ministr daemon serves this API to local clients (e.g. the CLI's
`status`/`search` commands). Most tool-shaped routes mirror the
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
| `/api/v1/corpora/{id}/inspect` | POST |
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

Tool-shaped responses distinguish operation `status` (`ok`, `partial`, or
`error`) from index `completeness` (`complete`, `partial`, `stale`, or
`unavailable`). Collection responses include deterministic pagination totals
and continuation state. Cross-corpus responses retain successful data and
identify any failed corpus with a stable error code and retryability.

`inspect` accepts either a symbol ID or file position and returns bounded
groups for definition, callers, callees, implementations, imports/type uses,
tests, bridges, impact, and suggested next actions. Group totals and omitted
counts make truncation explicit.

Definition requests accept line/context/body/outline bounds plus `start_byte`
for Unicode-safe continuation of very large one-line generated sources. A
bounded definition reports original and returned ranges, omitted lines, a
continuation locator, and `source_error` when indexed metadata is available
but the source file cannot be read; that case is partial data rather than an
empty success.

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
