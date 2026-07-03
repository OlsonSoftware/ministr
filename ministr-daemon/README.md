# ministr-daemon

The shared background engine — one per machine. An axum HTTP API served over
platform-native IPC (Unix domain sockets on macOS/Linux, named pipes on
Windows) providing the corpus registry, background indexing, freshness
sweeps, session state, and the recorded-execution engine.

Clients: the stdio MCP proxy in `ministr-cli` and the Tauri desktop app —
both speak the same surface, documented in the
[daemon HTTP API reference](../docs/reference/http-api.md).

Place in the workspace: see the
[architecture overview](../docs/concepts/architecture.md).
