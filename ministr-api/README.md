# ministr-api

Shared API types for the ministr daemon, MCP proxy, and CLI: the request and
response types exchanged over the daemon's HTTP API, the `DaemonClient` that
speaks it, and the `CloudRouterMounter` trait — the single seam where
optional non-MIT features attach (the public binary passes `None`).

Transport is platform-native IPC: Unix domain sockets on macOS/Linux, named
pipes on Windows. Dependencies stay light (serde, schemars, tokio) so every
other crate can depend on this one.

Place in the workspace: see the
[architecture overview](../docs/concepts/architecture.md).

```rust,no_run
use ministr_api::client::DaemonClient;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let client = DaemonClient::new();
let corpora = client.list_corpora().await?;
# Ok(())
# }
```
