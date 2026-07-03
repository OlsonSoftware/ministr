# ministr-core

Domain logic for the ministr code intelligence server: the indexing pipeline
(discovery, parsing, sectioning, claim extraction), tree-sitter symbol
extraction across 40+ languages, local embedding, dense + sparse search,
cross-language bridge linking, and SQLite/HNSW storage.

This crate has no transport dependencies and no knowledge of MCP — it is the
engine the daemon, MCP server, and CLI compose. How the pieces work is
documented in [concepts](../docs/concepts/README.md) (indexing, search,
bridges, freshness, sessions).

Place in the workspace: see the
[architecture overview](../docs/concepts/architecture.md).
