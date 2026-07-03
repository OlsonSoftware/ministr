# Architecture

## The pieces

| Crate | Role |
|---|---|
| `ministr-core` | indexing pipeline, parsers, embedders, search, storage |
| `ministr-api` | shared types + the daemon HTTP client + the cloud seam trait |
| `ministr-daemon` | the shared background engine (one per machine) |
| `ministr-mcp` | the MCP server: the tool surface, sessions, agent instructions |
| `ministr-cli` | the `ministr` binary: serve, init, index, setup, … |
| `ministr-app` | the Tauri desktop app |

All six are MIT-licensed and build a complete local product with no cloud
dependencies. Layering and contribution conventions live in
[CONTRIBUTING](../../CONTRIBUTING.md).

## One daemon, many clients

`ministr serve` (stdio, the default) is a thin proxy. The engine lives in a
single shared daemon per machine, reached over a Unix domain socket (a named
pipe on Windows) — not a TCP port, so access control is filesystem
permissions.

- The first client (MCP proxy or desktop app) auto-spawns the daemon as a
  detached process. It deliberately outlives its spawner: closing your
  editor leaves indexes warm for the next session.
- The proxy serves the MCP handshake immediately; corpus registration and
  warm-up happen in the background, and the daemon lazy-loads indexes on
  first query.
- The daemon owns the state: per-corpus indexes under `~/.ministr`, run
  history, activity tracking, the freshness cache. Corpus identity is a
  deterministic hash of the canonicalized path set, so the CLI, daemon, and
  app can never disagree about which index a project maps to.
- The desktop app is another daemon client, polling the same
  [HTTP surface](../reference/http-api.md).

`ministr serve --transport http` is the opposite mode: a full in-process
engine serving MCP over Streamable HTTP (see the
[CLI reference](../reference/cli.md)).

## The open-core seam

Optional non-MIT features attach through exactly one seam: the
`CloudRouterMounter` trait in `ministr-api`, consulted once at HTTP-serve
boot. The MIT binary always passes `None`, and no proprietary crates are
compiled into it. See [STEWARDSHIP](../../STEWARDSHIP.md) for the split and
its commitments.

## Where data flows

```
your repo ──(index: hash, parse, embed)──▶ ~/.ministr/corpora/<id>/
                                              │  SQLite (source of truth)
                                              │  + HNSW vector index (derived cache)
agent ──MCP/stdio──▶ proxy ──UDS──▶ daemon ───┘
desktop app ─────────polls──UDS──▶ daemon
```

SQLite is the source of truth; the vector index is a derived cache, rebuilt
from the store whenever the two could disagree. Nothing leaves your machine.
