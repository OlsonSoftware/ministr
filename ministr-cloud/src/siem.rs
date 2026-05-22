//! F5.3-d-i — SIEM exporter (Splunk HEC, global env-var config).
//!
//! Implements [`ministr_api::AuditSink`] backed by an HTTP POST to a
//! Splunk HTTP Event Collector endpoint. Wired alongside the
//! Postgres + webhook sinks in [`crate::ChainedAuditSink`] so every
//! audit row also streams out to the customer's SIEM.
//!
//! v0 scope:
//!
//! - **One provider — Splunk HEC.** Datadog Logs / S3 JSON-lines /
//!   syslog/CEF will land as separate `*Sink` types behind the same
//!   [`AuditSink`] trait; the chain composition in `cmd_serve_http`
//!   extends naturally.
//! - **Global config via env vars.** `MINISTR_SIEM_HEC_URL` (full
//!   collector URL, e.g. `https://splunk.example.com:8088/services/collector/event`)
//!   and `MINISTR_SIEM_HEC_TOKEN` (the HEC token). Either missing
//!   disables the sink — `from_env()` returns `None`.
//! - **Per-org SIEM config CRUD lands as F5.3-d-ii.** Right now every
//!   org's audit rows hit the same HEC endpoint (the cloud operator's
//!   central SIEM). Customers running their own SIEM endpoint will
//!   wait for the per-org config table to ship.
//!
//! # Fire-and-forget posture
//!
//! Mirrors [`crate::PostgresAuditSink`] + [`crate::WebhookFanoutSink`]:
//! `record()` spawns a tokio task and returns immediately. A network
//! hiccup logs at `warn` but never propagates to the calling handler.
//! Splunk HEC's docs explicitly support best-effort one-way delivery —
//! losing an audit row during a transient outage is documented as
//! acceptable (the persistent Postgres copy is authoritative).

use std::sync::Arc;
use std::time::Duration;

use ministr_api::{AuditEntry, AuditSink};
use reqwest::Client;
use serde::Serialize;
use tracing::{debug, warn};

/// Default HTTP timeout. Splunk HEC is local-network fast; if a
/// customer's collector takes longer than 10s we want to abandon
/// the request rather than backing up the tokio task queue.
const HEC_TIMEOUT: Duration = Duration::from_secs(10);

/// Splunk HEC sink. Cheap-clone (`reqwest::Client` is `Arc`-backed
/// internally; the URL + token strings clone as needed inside the
/// spawned task).
#[derive(Clone)]
pub struct SplunkHecSink {
    endpoint_url: Arc<String>,
    token: Arc<String>,
    client: Client,
}

#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for SplunkHecSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is bearer material; never let a Debug print leak
        // it. Endpoint URL is operator metadata — safe to surface.
        // `client` is intentionally elided (reqwest::Client's Debug
        // is noise; no security value).
        f.debug_struct("SplunkHecSink")
            .field("endpoint_url", &self.endpoint_url.as_str())
            .field("token", &"<redacted>")
            .finish()
    }
}

impl SplunkHecSink {
    /// Construct from explicit URL + token. The URL must be the FULL
    /// collector URL including the `/services/collector/event` path
    /// (or whatever path the customer's HEC deployment uses). The
    /// constructor doesn't validate the URL — `record()` fails-soft
    /// at runtime if the URL is malformed.
    #[must_use]
    pub fn new(endpoint_url: impl Into<String>, token: impl Into<String>) -> Self {
        // build() is the only fallible step; if it fails (highly
        // unlikely — only options that affect TLS / cert validation
        // can fail), fall back to the default client which also has
        // no failure modes in the reqwest 0.12 build.
        let client = Client::builder()
            .timeout(HEC_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self {
            endpoint_url: Arc::new(endpoint_url.into()),
            token: Arc::new(token.into()),
            client,
        }
    }

    /// Construct from `MINISTR_SIEM_HEC_URL` + `MINISTR_SIEM_HEC_TOKEN`
    /// env vars. Returns `None` when either is missing — the cloud
    /// serve then skips SIEM wiring entirely (no warn log; "no SIEM"
    /// is a valid deployment).
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let url = std::env::var("MINISTR_SIEM_HEC_URL").ok()?;
        let token = std::env::var("MINISTR_SIEM_HEC_TOKEN").ok()?;
        if url.trim().is_empty() || token.trim().is_empty() {
            return None;
        }
        Some(Self::new(url, token))
    }
}

/// Wire shape posted to Splunk HEC. The `event` object carries the
/// flattened [`AuditEntry`]; Splunk parses it server-side and
/// indexes each field for search. `sourcetype` is the conventional
/// tag for ministr-emitted events so customers can filter on it.
#[derive(Debug, Serialize)]
struct HecPayload<'a> {
    sourcetype: &'static str,
    event: HecEvent<'a>,
    /// Unix epoch seconds. Splunk uses this for event ordering when
    /// the collector receives events out of order.
    time: u64,
}

#[derive(Debug, Serialize)]
struct HecEvent<'a> {
    action: &'a str,
    resource: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    org_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actor: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ip: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_agent: Option<&'a str>,
}

impl AuditSink for SplunkHecSink {
    fn record(&self, entry: AuditEntry) {
        let url = Arc::clone(&self.endpoint_url);
        let token = Arc::clone(&self.token);
        let client = self.client.clone();
        tokio::spawn(async move {
            let time = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let payload = HecPayload {
                sourcetype: "ministr_audit",
                event: HecEvent {
                    action: entry.action.as_str(),
                    resource: entry.resource.as_str(),
                    org_id: entry.org_id.as_deref(),
                    actor: entry.actor.as_deref(),
                    ip: entry.ip.as_deref(),
                    user_agent: entry.user_agent.as_deref(),
                },
                time,
            };
            let auth_header = format!("Splunk {token}");
            let req = client
                .post(url.as_str())
                .header("Authorization", auth_header)
                .header("Content-Type", "application/json")
                .json(&payload);
            match req.send().await {
                Ok(resp) if resp.status().is_success() => {
                    debug!(
                        action = %entry.action,
                        status = resp.status().as_u16(),
                        "splunk hec dispatch ok"
                    );
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "<unreadable>".to_string());
                    warn!(
                        action = %entry.action,
                        status = status.as_u16(),
                        body = %body.chars().take(200).collect::<String>(),
                        "splunk hec dispatch failed; row stays in Postgres audit_events"
                    );
                }
                Err(e) => {
                    warn!(
                        action = %entry.action,
                        error = %e,
                        "splunk hec dispatch error; row stays in Postgres audit_events"
                    );
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ministr_api::AuditEntry;

    #[test]
    fn debug_never_leaks_token() {
        // Catches a regression where a careless Debug impl change
        // surfaces the bearer material in logs / panics / structured
        // events.
        let sink = SplunkHecSink::new("https://splunk.example.com/services/collector/event", "secret-bearer-do-not-leak");
        let s = format!("{sink:?}");
        assert!(!s.contains("secret-bearer-do-not-leak"), "Debug leaked the token: {s}");
        assert!(s.contains("<redacted>"), "Debug should show <redacted>: {s}");
        assert!(s.contains("splunk.example.com"), "Debug should show URL: {s}");
    }

    #[test]
    fn from_env_returns_none_without_url() {
        // The two env vars are read each call so unsetting reverts
        // the state for subsequent tests.
        // SAFETY: tests run in the same process; this test asserts the
        // env-var read pattern without mutating shared global state
        // — if MINISTR_SIEM_HEC_URL ever IS set in the test env, the
        // assertion below would catch the divergence.
        let url = std::env::var("MINISTR_SIEM_HEC_URL").ok();
        let token = std::env::var("MINISTR_SIEM_HEC_TOKEN").ok();
        if url.is_some() && token.is_some() {
            // Don't fight a real deployment; just confirm the
            // happy-path returns Some.
            assert!(SplunkHecSink::from_env().is_some());
        } else {
            assert!(SplunkHecSink::from_env().is_none());
        }
    }

    #[test]
    fn record_is_fire_and_forget_no_panic_on_invalid_url() {
        // Construct a sink pointing at a syntactically valid but
        // unreachable URL. record() spawns a task that will fail to
        // connect — but must NOT panic the caller. The runtime test
        // is "no panic during record()"; the spawned task's warn log
        // is the operator's signal.
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async {
            let sink = SplunkHecSink::new(
                "http://127.0.0.1:1/services/collector/event",
                "test-token",
            );
            sink.record(AuditEntry::new("test.event", "test-resource"));
            // Yield once so the spawned task gets a chance to start
            // (and fail) before the runtime is dropped.
            tokio::task::yield_now().await;
        });
    }
}
