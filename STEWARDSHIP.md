# Stewardship

ministr is open-core. The local stack is MIT-licensed and runs entirely on
your machine. A hosted cloud service and an enterprise edition are separate
paid products.

## What's MIT

The seven workspace crates that make up the local stack are MIT-licensed:

| Crate | Role |
|---|---|
| [`ministr-core`](ministr-core/) | Indexing, embedding, the SOLID detector, the cross-language bridge graph, 42 language parsers, claim extraction |
| [`ministr-api`](ministr-api/) | Shared request/response types |
| [`ministr-daemon`](ministr-daemon/) | HTTP API over a Unix domain socket |
| [`ministr-backend`](ministr-backend/) | The shared `QueryBackend` seam — one query surface over the in-process engine or a running daemon |
| [`ministr-mcp`](ministr-mcp/) | MCP server adapter |
| [`ministr-cli`](ministr-cli/) | The `ministr` binary |
| [`ministr-tui`](ministr-tui/) | The terminal console (`ministr ui`) |

Together they build a complete, self-hosted `ministr` with the full MCP tool
surface (`cargo build --workspace`). Running it on your own machine gives you
the same indexing, parsers, detectors, and tools the cloud uses — the cloud
sells hosting, scale, and team/compliance features, not the toolset.

## What's commercial

The hosted cloud service and the enterprise edition are paid products. The code
that exists only to run a multi-tenant service — the tenant model, billing,
quota, and the curated index — lives in a separate private repository. None of
it is needed to run ministr locally.

## Commitments

- A feature released under MIT in this repository stays MIT. We won't move
  existing open-source functionality behind a paywall.
- Contributions are inbound=outbound under MIT: you keep copyright in your
  work, and no copyright assignment is required.
- Forks are welcome under the MIT terms — keep the copyright notice.

The shape of this document is borrowed from
[GitLab's stewardship handbook](https://handbook.gitlab.com/handbook/company/stewardship/).
