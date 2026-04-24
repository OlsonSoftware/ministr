//! Transparent stdio↔HTTP MCP proxy.
//!
//! When ministr detects that a primary instance already owns the corpus index,
//! it runs in proxy mode: reading JSON-RPC messages from stdin, forwarding
//! them to the primary's Streamable HTTP endpoint, and writing responses
//! back to stdout.
//!
//! The proxy is stateless — it holds no database, no index, no embeddings.
//! It simply bridges the MCP transport layer.

use miette::{IntoDiagnostic, WrapErr};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, warn};

/// Run the stdio↔HTTP proxy until stdin closes or the primary disconnects.
pub async fn run_stdio_proxy(mcp_url: &str) -> miette::Result<()> {
    let client = reqwest::Client::new();
    let mut stdin = BufReader::new(tokio::io::stdin());
    let mut stdout = tokio::io::stdout();
    let mut session_id: Option<String> = None;

    let mut line = String::new();

    loop {
        line.clear();
        let bytes_read = stdin
            .read_line(&mut line)
            .await
            .into_diagnostic()
            .wrap_err("failed to read from stdin")?;

        if bytes_read == 0 {
            // EOF — Claude Code closed the connection.
            debug!("stdin closed, proxy shutting down");
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Forward the JSON-RPC message to the primary.
        let mut request = client
            .post(mcp_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(trimmed.to_string());

        if let Some(ref sid) = session_id {
            request = request.header("Mcp-Session-Id", sid);
        }

        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "primary disconnected — proxy exiting");
                return Err(e)
                    .into_diagnostic()
                    .wrap_err("lost connection to primary ministr instance");
            }
        };

        // Learn the session ID from the primary's response.
        if session_id.is_none()
            && let Some(sid) = response.headers().get("mcp-session-id")
            && let Ok(s) = sid.to_str()
        {
            session_id = Some(s.to_string());
            debug!(session_id = %s, "learned session ID from primary");
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("text/event-stream") {
            // SSE response — stream each `data:` line to stdout.
            let body = response
                .text()
                .await
                .into_diagnostic()
                .wrap_err("failed to read SSE response body")?;

            for sse_line in body.lines() {
                if let Some(data) = sse_line.strip_prefix("data: ")
                    && !data.is_empty()
                {
                    stdout.write_all(data.as_bytes()).await.into_diagnostic()?;
                    stdout.write_all(b"\n").await.into_diagnostic()?;
                    stdout.flush().await.into_diagnostic()?;
                }
            }
        } else {
            // JSON response — write directly to stdout.
            let body = response
                .text()
                .await
                .into_diagnostic()
                .wrap_err("failed to read JSON response body")?;

            if !body.trim().is_empty() {
                stdout
                    .write_all(body.trim().as_bytes())
                    .await
                    .into_diagnostic()?;
                stdout.write_all(b"\n").await.into_diagnostic()?;
                stdout.flush().await.into_diagnostic()?;
            }
        }
    }

    Ok(())
}
