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

use getrandom::fill as getrandom_fill;
use tokio::io::AsyncWriteExt as _;

use axum::Router;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use deadpool_postgres::Pool;
use ministr_api::{AuditEntry, AuditSink};
use ministr_mcp::auth::tenant::Tenant;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::orgs::member_role;

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
            dispatch_splunk_hec(&client, url.as_str(), token.as_str(), &entry).await;
        });
    }
}

/// F5.3-d-i + F5.3-d-ii-dispatch — shared HEC POST helper. Builds
/// the Splunk-event-shaped body, signs with `Authorization: Splunk
/// <token>`, fires the POST, and logs the outcome at `debug` (ok) /
/// `warn` (non-2xx response or connect/read error). Caller's
/// responsibility to invoke from a fire-and-forget tokio task — this
/// helper does NOT spawn its own.
///
/// Pulled out to a free function so [`SplunkHecSink`] (global,
/// env-var-config) and [`PerOrgSplunkHecDispatcher`] (per-org,
/// `org_siem_configs` table) share one POST shape with no
/// duplication.
pub(crate) async fn dispatch_splunk_hec(
    client: &Client,
    url: &str,
    token: &str,
    entry: &AuditEntry,
) {
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
        .post(url)
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
}

/// F5.3-d-iii-a — Datadog Logs payload. The HTTP intake at
/// `https://http-intake.logs.datadoghq.com/api/v2/logs` accepts
/// either a single object or an array; we always send a singleton
/// array because Datadog's client SDKs assume the array shape and
/// search filters key off `ddsource` + `service`.
#[derive(Debug, Serialize)]
struct DdLogEvent<'a> {
    /// Conventional tag for "where did this come from"; Datadog UI
    /// surfaces it as a top-level filter chip.
    ddsource: &'static str,
    /// Conventional tag for "which service emitted this".
    service: &'static str,
    /// Free-text message; we use the audit action so a Datadog
    /// "search action=oidc.login" maps cleanly.
    message: &'a str,
    /// Flattened audit fields for search.
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

/// F5.3-d-iii-a — POST helper for Datadog Logs HTTP intake. Shape
/// mirrors [`dispatch_splunk_hec`]: caller's responsibility to spawn
/// the tokio task; this helper does the POST + logs the outcome at
/// `debug` (2xx) / `warn` (non-2xx or connect/read error). Datadog
/// returns `202 Accepted` on success; `reqwest::Response::is_success`
/// admits both 200 and 202 so the branch is uniform.
pub(crate) async fn dispatch_datadog_logs(
    client: &Client,
    url: &str,
    api_key: &str,
    entry: &AuditEntry,
) {
    let log = DdLogEvent {
        ddsource: "ministr",
        service: "ministr-audit",
        message: entry.action.as_str(),
        action: entry.action.as_str(),
        resource: entry.resource.as_str(),
        org_id: entry.org_id.as_deref(),
        actor: entry.actor.as_deref(),
        ip: entry.ip.as_deref(),
        user_agent: entry.user_agent.as_deref(),
    };
    let payload = [log];
    let req = client
        .post(url)
        .header("DD-API-KEY", api_key)
        .header("Content-Type", "application/json")
        .json(&payload);
    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            debug!(
                action = %entry.action,
                status = resp.status().as_u16(),
                "datadog logs dispatch ok"
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
                "datadog logs dispatch failed; row stays in Postgres audit_events"
            );
        }
        Err(e) => {
            warn!(
                action = %entry.action,
                error = %e,
                "datadog logs dispatch error; row stays in Postgres audit_events"
            );
        }
    }
}

/// F5.3-d-iii-c — escape a CEF extension value. CEF v0 reserves
/// `|`, `\`, and `=`; the canonical escape is backslash. Newline
/// inside a value is also illegal because lines end syslog messages.
/// Returns a freshly-allocated `String` only when the input contains
/// one of those characters — otherwise returns the input unchanged
/// (zero-alloc fast path).
fn cef_escape(value: &str) -> std::borrow::Cow<'_, str> {
    if !value.contains(['|', '\\', '=', '\n', '\r']) {
        return std::borrow::Cow::Borrowed(value);
    }
    let mut out = String::with_capacity(value.len() + 4);
    for ch in value.chars() {
        match ch {
            '|' => out.push_str("\\|"),
            '\\' => out.push_str("\\\\"),
            '=' => out.push_str("\\="),
            // Newlines collapse to spaces so a single CEF line stays a
            // single line. ArcSight + QRadar both treat \n inside the
            // extension as an end-of-message terminator otherwise.
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    std::borrow::Cow::Owned(out)
}

/// F5.3-d-iii-c — format an [`AuditEntry`] as one CEF v0 line.
/// Public-ish via `pub(crate)` so the dispatch helper + unit tests
/// share one implementation. Header fields per the CEF spec:
/// `CEF:Version|Vendor|Product|Version|Signature|Name|Severity|Ext`.
fn format_cef_line(entry: &AuditEntry) -> String {
    let action = cef_escape(entry.action.as_str());
    let resource = cef_escape(entry.resource.as_str());
    let header = format!(
        "CEF:0|ministr|ministr-cloud-audit|1|{action}|{action}|5|"
    );
    let mut ext = String::new();
    // Standard CEF labels first (src, suser); custom labels for the
    // org context.
    if let Some(ip) = entry.ip.as_deref() {
        ext.push_str("src=");
        ext.push_str(&cef_escape(ip));
        ext.push(' ');
    }
    if let Some(actor) = entry.actor.as_deref() {
        ext.push_str("suser=");
        ext.push_str(&cef_escape(actor));
        ext.push(' ');
    }
    if let Some(org_id) = entry.org_id.as_deref() {
        ext.push_str("orgId=");
        ext.push_str(&cef_escape(org_id));
        ext.push(' ');
    }
    if let Some(ua) = entry.user_agent.as_deref() {
        ext.push_str("requestClientApplication=");
        ext.push_str(&cef_escape(ua));
        ext.push(' ');
    }
    ext.push_str("resource=");
    ext.push_str(&resource);
    format!("{header}{ext}")
}

/// F5.3-d-iii-c — TCP-syslog dispatch helper. Opens a TCP connection
/// to `tcp://host:port`, sends one CEF v0 line terminated by `\n`,
/// closes. 10s connect+write timeout matching the HTTP sinks.
///
/// Network-layer auth (mTLS / VPN / source-IP allowlist) is the
/// expected security model for syslog — there's no per-event bearer.
/// Customers running an unprotected collector accept the obvious
/// risk; ministr's CRUD validator doesn't object.
/// F5.3-d-iii-c + F5.3-d-iii-c-udp — scheme-routing entrypoint.
/// Parses the endpoint prefix and dispatches to the right transport
/// helper. Unknown scheme → warn log + skip (validator should have
/// rejected at CRUD POST time; defensive against a row written
/// outside the CRUD path).
pub(crate) async fn dispatch_syslog_cef(endpoint: &str, entry: &AuditEntry) {
    if let Some(host_port) = endpoint.strip_prefix("tcp://") {
        dispatch_syslog_cef_tcp(host_port, endpoint, entry).await;
    } else if let Some(host_port) = endpoint.strip_prefix("udp://") {
        dispatch_syslog_cef_udp(host_port, endpoint, entry).await;
    } else {
        warn!(
            endpoint = %endpoint,
            "syslog cef dispatch: endpoint must start with tcp:// or udp://"
        );
    }
}

/// TCP variant — opens a connection, writes one newline-terminated
/// CEF v0 line, flushes, drops. 10s connect + write timeout. Used
/// when the customer's collector speaks TCP syslog (RFC 6587 octet-
/// stuffing isn't implemented yet — one connection per message is
/// the simplest interop pattern with most production collectors).
async fn dispatch_syslog_cef_tcp(
    host_port: &str,
    endpoint: &str,
    entry: &AuditEntry,
) {
    // `rsplit_once(':')` would route the port off the trailing colon
    // for an IPv6 literal like `tcp://[::1]:5514`, but the validator
    // doesn't normalize bracketed-IPv6 yet — documented for the IPv6
    // follow-up.
    if host_port.rsplit_once(':').is_none() {
        warn!(
            endpoint = %endpoint,
            "syslog cef dispatch (tcp): endpoint missing :port suffix"
        );
        return;
    }
    let line = format_cef_line(entry);
    let payload = format!("{line}\n");
    let connect = tokio::net::TcpStream::connect(host_port);
    let stream = match tokio::time::timeout(HEC_TIMEOUT, connect).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            warn!(
                action = %entry.action,
                endpoint = %endpoint,
                error = %e,
                "syslog cef dispatch (tcp): connect failed"
            );
            return;
        }
        Err(_) => {
            warn!(
                action = %entry.action,
                endpoint = %endpoint,
                "syslog cef dispatch (tcp): connect timeout (>10s)"
            );
            return;
        }
    };
    let mut stream = stream;
    let write = stream.write_all(payload.as_bytes());
    match tokio::time::timeout(HEC_TIMEOUT, write).await {
        Ok(Ok(())) => {
            // Best-effort flush so the collector sees the line before
            // we drop the socket. Ignore flush errors — the bytes
            // already went out the door.
            let _ = stream.flush().await;
            debug!(
                action = %entry.action,
                bytes = payload.len(),
                "syslog cef dispatch (tcp) ok"
            );
        }
        Ok(Err(e)) => {
            warn!(
                action = %entry.action,
                endpoint = %endpoint,
                error = %e,
                "syslog cef dispatch (tcp): write failed"
            );
        }
        Err(_) => {
            warn!(
                action = %entry.action,
                endpoint = %endpoint,
                "syslog cef dispatch (tcp): write timeout (>10s)"
            );
        }
    }
}

/// F5.3-d-iii-c-udp — UDP variant. Binds an ephemeral local
/// socket, sends one datagram containing the CEF v0 line, drops.
/// UDP is fire-and-forget: there's no connect or ack handshake;
/// `send_to` returns `Ok(n_bytes)` if the local kernel accepted
/// the packet for transmission. Packet loss on the wire is
/// documented as the trade-off for UDP-syslog deployments (BSD
/// historical default; RFC 5424 §A.2 reaffirms acceptable).
///
/// No trailing newline on the payload — each UDP datagram IS one
/// syslog message per RFC 3164 / 5424.
async fn dispatch_syslog_cef_udp(
    host_port: &str,
    endpoint: &str,
    entry: &AuditEntry,
) {
    if host_port.rsplit_once(':').is_none() {
        warn!(
            endpoint = %endpoint,
            "syslog cef dispatch (udp): endpoint missing :port suffix"
        );
        return;
    }
    let line = format_cef_line(entry);
    let bind = tokio::net::UdpSocket::bind("0.0.0.0:0");
    let sock = match tokio::time::timeout(HEC_TIMEOUT, bind).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            warn!(
                action = %entry.action,
                endpoint = %endpoint,
                error = %e,
                "syslog cef dispatch (udp): local bind failed"
            );
            return;
        }
        Err(_) => {
            warn!(
                action = %entry.action,
                endpoint = %endpoint,
                "syslog cef dispatch (udp): local bind timeout (>10s)"
            );
            return;
        }
    };
    // `send_to` requires a resolved addr. tokio resolves a `&str`
    // via `ToSocketAddrs::to_socket_addrs`, so passing host_port
    // directly is fine for IPv4 numeric + DNS hostnames.
    let send = sock.send_to(line.as_bytes(), host_port);
    match tokio::time::timeout(HEC_TIMEOUT, send).await {
        Ok(Ok(n)) => {
            debug!(
                action = %entry.action,
                bytes = n,
                "syslog cef dispatch (udp) ok"
            );
        }
        Ok(Err(e)) => {
            warn!(
                action = %entry.action,
                endpoint = %endpoint,
                error = %e,
                "syslog cef dispatch (udp): send failed"
            );
        }
        Err(_) => {
            warn!(
                action = %entry.action,
                endpoint = %endpoint,
                "syslog cef dispatch (udp): send timeout (>10s)"
            );
        }
    }
}

// ─── F5.3-d-iii-b-dispatch — S3 JSON-lines dispatch ────────────────
//
// Builds an `aws-sdk-s3` Client per audit event from the per-org
// config row's parsed credentials, PUTs one JSON object at a
// date-partitioned key. The PUT shape mirrors the JSONL convention
// (one JSON document per line / file) — each event lands as its own
// S3 object so customer-side queries via Athena / Glue can shard
// over the `year=…/month=…/day=…/` Hive-style prefix.

/// One audit event as it lands in the customer's S3 bucket.
#[derive(Debug, Serialize)]
struct S3JsonlEvent<'a> {
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
    /// Unix epoch seconds. Same field shape as the Splunk + Datadog
    /// payloads so customers can use one query template against any
    /// of the three sinks.
    ts_unix_secs: u64,
}

/// Howard Hinnant's `civil_from_days` — converts days-since-1970-01-01
/// to (year, month, day) in the proleptic Gregorian calendar. Pure
/// integer arithmetic; no `chrono` / `time` dep needed. Algorithm
/// reference: <https://howardhinnant.github.io/date_algorithms.html>.
/// Tested via the unit test directly below the dispatcher.
///
/// The casts are all within the algorithm's mathematically-bounded
/// ranges (doe ≤ 146 096, doy ≤ 365, mp ≤ 11, year fits in i32 for
/// any input that fits in u64-seconds-since-epoch). Clippy's pedantic
/// wrap/truncation/sign-loss lints don't know that, so we silence
/// them locally rather than restructure the canonical algorithm.
#[allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::bool_to_int_with_if
)]
fn civil_from_unix_secs(secs: u64) -> (i32, u32, u32) {
    // secs / 86_400 is the day index since 1970-01-01.
    let z_signed: i64 = (secs / 86_400) as i64 + 719_468;
    let era = if z_signed >= 0 { z_signed } else { z_signed - 146_096 } / 146_097;
    let doe = (z_signed - era * 146_097) as u64; // [0, 146_096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    let y = (y + if m <= 2 { 1 } else { 0 }) as i32;
    (y, m, d)
}

/// F5.3-d-iii-b-dispatch — POST helper for S3 JSON-lines. Parses the
/// `s3://bucket/prefix/` endpoint + JSON-shape token (validated at
/// the CRUD edge by [`validate_s3_credentials_shape`]), builds an
/// `aws-sdk-s3` Client with static credentials + optional endpoint
/// override (for `MinIO` / R2 / B2 / fake S3 in the e2e harness),
/// and PUTs one JSON object at
/// `<prefix>/year=YYYY/month=MM/day=DD/<unix_ms>-<rand_u64_hex>.json`.
///
/// 10s timeout on the PUT call so a slow customer endpoint doesn't
/// back up the audit pipeline. Failures log `warn`; the persistent
/// Postgres copy stays authoritative.
#[allow(clippy::too_many_lines)] // SDK boilerplate + key construction + body serialization → one cohesive flow
pub(crate) async fn dispatch_s3_jsonl(
    endpoint: &str,
    token_json: &str,
    entry: &AuditEntry,
) {
    let Some(after_scheme) = endpoint.strip_prefix("s3://") else {
        warn!(
            endpoint = %endpoint,
            "s3 jsonl dispatch: endpoint must start with s3://"
        );
        return;
    };
    let (bucket, key_prefix) = match after_scheme.split_once('/') {
        Some((b, p)) => (b.to_string(), p.trim_end_matches('/').to_string()),
        None => (after_scheme.to_string(), String::new()),
    };
    if bucket.is_empty() {
        warn!(
            endpoint = %endpoint,
            "s3 jsonl dispatch: bucket name is empty"
        );
        return;
    }

    let creds: S3JsonlCredentials = match serde_json::from_str(token_json) {
        Ok(c) => c,
        Err(e) => {
            // Validator should have caught this at CRUD POST time;
            // surface defensively in case a row was written outside
            // the CRUD path.
            warn!(
                error = %e,
                "s3 jsonl dispatch: token JSON malformed (should have been rejected at CRUD edge)"
            );
            return;
        }
    };

    let aws_creds = aws_credential_types::Credentials::new(
        creds.access_key_id.clone(),
        creds.secret_access_key.clone(),
        None, // session token — not used; per-org rows don't carry STS sessions
        None, // expiry — static credentials, no expiry
        "ministr-per-org-siem",
    );
    let region = aws_sdk_s3::config::Region::new(creds.region.clone());
    let mut config_builder = aws_sdk_s3::Config::builder()
        .behavior_version(aws_config::BehaviorVersion::latest())
        .region(region)
        .credentials_provider(aws_creds);
    if let Some(override_url) = creds.endpoint_url_override.as_deref() {
        // S3-compatible providers (MinIO / R2 / B2) AND the e2e
        // harness's fake S3 don't support virtual-hosted-style
        // bucket addressing (`bucket.endpoint.com`). Force path-style
        // (`endpoint.com/bucket/key`) when an override is set.
        config_builder = config_builder
            .endpoint_url(override_url.to_string())
            .force_path_style(true);
    }
    let s3_config = config_builder.build();
    let client = aws_sdk_s3::Client::from_conf(s3_config);

    let now_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (year, month, day) = civil_from_unix_secs(now_secs);
    let now_ms = now_secs.saturating_mul(1000);
    let mut nonce_bytes = [0u8; 8];
    // Best-effort RNG; if it fails the key still works (the unix-ms
    // timestamp + bucket isolation makes collision astronomically
    // unlikely even with a zero nonce).
    let _ = getrandom_fill(&mut nonce_bytes);
    let nonce = u64::from_be_bytes(nonce_bytes);
    let key = if key_prefix.is_empty() {
        format!("year={year:04}/month={month:02}/day={day:02}/{now_ms}-{nonce:016x}.json")
    } else {
        format!("{key_prefix}/year={year:04}/month={month:02}/day={day:02}/{now_ms}-{nonce:016x}.json")
    };

    let body_obj = S3JsonlEvent {
        action: entry.action.as_str(),
        resource: entry.resource.as_str(),
        org_id: entry.org_id.as_deref(),
        actor: entry.actor.as_deref(),
        ip: entry.ip.as_deref(),
        user_agent: entry.user_agent.as_deref(),
        ts_unix_secs: now_secs,
    };
    let body = match serde_json::to_vec(&body_obj) {
        Ok(b) => b,
        Err(e) => {
            warn!(
                error = %e,
                "s3 jsonl dispatch: body serialization failed"
            );
            return;
        }
    };

    let put = client
        .put_object()
        .bucket(&bucket)
        .key(&key)
        .content_type("application/json")
        .body(aws_sdk_s3::primitives::ByteStream::from(body))
        .send();
    match tokio::time::timeout(HEC_TIMEOUT, put).await {
        Ok(Ok(_)) => {
            debug!(
                action = %entry.action,
                bucket = %bucket,
                key = %key,
                "s3 jsonl dispatch ok"
            );
        }
        Ok(Err(e)) => {
            warn!(
                action = %entry.action,
                bucket = %bucket,
                error = %e,
                "s3 jsonl dispatch: PUT failed; row stays in Postgres audit_events"
            );
        }
        Err(_) => {
            warn!(
                action = %entry.action,
                bucket = %bucket,
                "s3 jsonl dispatch: PUT timeout (>10s)"
            );
        }
    }
}

/// F5.3-d-ii-dispatch + F5.3-d-iii — per-org SIEM dispatcher. Looks
/// up `org_siem_configs` on every audit event with `org_id IS NOT
/// NULL` and dispatches via the right helper based on `row.kind`:
/// `"splunk_hec"` → [`dispatch_splunk_hec`], `"datadog_logs"` →
/// [`dispatch_datadog_logs`]. Unknown kinds log warn and skip
/// (defensive against a row written outside the CRUD path).
///
/// Personal-account events (`org_id IS NULL`) are skipped — the
/// per-org promise covers org-scoped actions only, same policy as
/// F5.3-a's tier-aware retention.
///
/// No cache in v0 — the lookup is one indexed query per audit event,
/// well under the audit volume threshold where caching would pay off.
/// A `Arc<RwLock<HashMap<org_id, (kind, url, token, enabled)>>>`
/// cache layer can land in a follow-up chunk if volume grows.
///
/// Fires IN ADDITION to the global env-var sink (operator's central
/// SIEM still receives every event; customers' per-org endpoints
/// receive their org's slice).
#[derive(Clone)]
pub struct PerOrgSiemDispatcher {
    pool: Arc<Pool>,
    client: Client,
}

impl std::fmt::Debug for PerOrgSiemDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PerOrgSiemDispatcher")
            .field("pool", &"<Pool>")
            .finish()
    }
}

impl PerOrgSiemDispatcher {
    /// Construct from a shared pool. The internal `reqwest::Client`
    /// is built once with the same 10s timeout as
    /// [`SplunkHecSink`] so a slow customer endpoint can't back up
    /// the audit pipeline. The same client is reused for every
    /// provider (Splunk HEC, Datadog Logs, etc) — `reqwest` pools
    /// connections per origin host.
    #[must_use]
    pub fn new(pool: Arc<Pool>) -> Self {
        let client = Client::builder()
            .timeout(HEC_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self { pool, client }
    }
}

impl AuditSink for PerOrgSiemDispatcher {
    fn record(&self, entry: AuditEntry) {
        // Skip personal-account events — per-org dispatch only
        // covers org-scoped actions.
        let Some(org_id) = entry.org_id.clone() else {
            return;
        };
        let pool = Arc::clone(&self.pool);
        let client = self.client.clone();
        tokio::spawn(async move {
            let conn = match pool.get().await {
                Ok(c) => c,
                Err(e) => {
                    warn!(
                        org_id = %org_id,
                        error = %e,
                        "per-org SIEM lookup: pool get failed"
                    );
                    return;
                }
            };
            // Single query: WHERE org_id = $1 AND enabled = TRUE.
            // The partial index from migration 0014 on (org_id)
            // WHERE enabled = TRUE makes this an index lookup. The
            // `kind` value drives the dispatch branch below; the
            // CRUD validator (ALLOWED_SIEM_KINDS) keeps invalid
            // values out of the table, but we still defensively
            // handle the "unknown kind" arm.
            let row = match conn
                .query_opt(
                    "SELECT kind, endpoint_url, token \
                     FROM org_siem_configs \
                     WHERE org_id = $1::text::uuid \
                       AND enabled = TRUE",
                    &[&org_id],
                )
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!(
                        org_id = %org_id,
                        error = %e,
                        "per-org SIEM lookup: query failed"
                    );
                    return;
                }
            };
            let Some(row) = row else {
                // No per-org config for this org — that's normal,
                // not an error. The global env-var sink (if wired)
                // still receives the event.
                return;
            };
            let kind: String = row.get(0);
            let url: String = row.get(1);
            let token: String = row.get(2);
            match kind.as_str() {
                "splunk_hec" => {
                    dispatch_splunk_hec(&client, &url, &token, &entry).await;
                }
                "datadog_logs" => {
                    // F5.3-d-iii-a — `token` column carries the
                    // `DD-API-KEY` value for this provider. Same
                    // column, provider-specific semantics.
                    dispatch_datadog_logs(&client, &url, &token, &entry).await;
                }
                "syslog_cef" => {
                    // F5.3-d-iii-c — `token` is unused (auth is at
                    // the network layer); endpoint_url is a
                    // `tcp://host:port` collector address.
                    let _ = token;
                    dispatch_syslog_cef(&url, &entry).await;
                }
                "s3_jsonl" => {
                    // F5.3-d-iii-b-dispatch — `token` carries JSON
                    // credentials (parsed inside dispatch_s3_jsonl
                    // via S3JsonlCredentials); `endpoint_url` is the
                    // `s3://bucket/prefix/` target. Validator at the
                    // CRUD edge enforces both shapes upfront.
                    dispatch_s3_jsonl(&url, &token, &entry).await;
                }
                other => {
                    warn!(
                        org_id = %org_id,
                        kind = %other,
                        "per-org SIEM lookup: unknown kind; row written outside the CRUD path"
                    );
                }
            }
        });
    }
}

// ─── F5.3-d-ii — per-org SIEM config CRUD ────────────────────────
//
// Three routes mounted at `/api/v1/orgs/{id}/siem/config`. Same
// shape as F5.2-d's OIDC config CRUD: owner-only via
// `assert_owner_or_admin`, upsert via `ON CONFLICT (org_id)`, GET
// returns the row with `token` REDACTED, DELETE returns 204.
//
// Lookup state for the dispatch path (F5.3-d-ii-dispatch) will
// land in a future chunk; this chunk just persists customer
// config. With the schema seeded customers can pre-configure
// before the dispatcher wiring goes live.

/// Allowed `kind` values for `org_siem_configs.kind`. F5.3-d-i
/// admitted `"splunk_hec"` only; F5.3-d-iii-a added
/// `"datadog_logs"`; F5.3-d-iii-c added `"syslog_cef"`;
/// F5.3-d-iii-b-shim added `"s3_jsonl"` (CRUD-validatable;
/// dispatch lands in F5.3-d-iii-b-dispatch with `aws-sdk-s3`).
/// The `PerOrgSiemDispatcher`'s `record()` branches on this set.
const ALLOWED_SIEM_KINDS: &[&str] =
    &["splunk_hec", "datadog_logs", "syslog_cef", "s3_jsonl"];

/// `kind` values where the `token` column is unused — auth happens
/// at the network layer (mTLS / VPN / source-IP allowlist) rather
/// than via a per-event bearer. The CRUD validator skips its
/// empty-token check for these kinds.
const TOKENLESS_SIEM_KINDS: &[&str] = &["syslog_cef"];

/// Sentinel string returned in place of the real `token` on every
/// HTTP read. Mirrors F5.2-d's `REDACTED_CLIENT_SECRET` exactly so
/// frontend code that branches on the sentinel value handles both
/// configs uniformly.
pub const REDACTED_TOKEN: &str = "[REDACTED]";

/// F5.3-d-ii-config — per-org SIEM config CRUD router. Mount under
/// the `OAuth`-protected branch in `cmd_serve_http`; owner-only ACL
/// is enforced by each handler via [`assert_siem_owner_or_admin`].
pub fn siem_config_routes(state: SiemConfigState) -> Router {
    Router::new()
        .route(
            "/api/v1/orgs/{id}/siem/config",
            post(handle_siem_config_upsert)
                .get(handle_siem_config_get)
                .delete(handle_siem_config_delete),
        )
        .with_state(state)
}

/// Per-route shared state. Holds the Postgres pool the handlers use
/// for org-membership ACL + config table reads/writes.
#[derive(Clone)]
pub struct SiemConfigState {
    pub pool: Arc<Pool>,
}

impl std::fmt::Debug for SiemConfigState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SiemConfigState")
            .field("pool", &"<Pool>")
            .finish()
    }
}

impl SiemConfigState {
    /// Construct from a shared `Arc<Pool>`.
    #[must_use]
    pub fn from_arc(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

#[derive(Debug)]
enum SiemConfigError {
    Unauthenticated,
    Forbidden,
    NotFound,
    Invalid(&'static str),
    Db(String),
}

impl IntoResponse for SiemConfigError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => {
                (StatusCode::UNAUTHORIZED, "unauthenticated").into_response()
            }
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden").into_response(),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found").into_response(),
            Self::Invalid(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            Self::Db(msg) => {
                tracing::warn!(error = %msg, "siem config db error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal").into_response()
            }
        }
    }
}

/// POST body for `/api/v1/orgs/{id}/siem/config`. `enabled` is
/// optional + defaults to true. F5.3-d-iii will admit more `kind`
/// values; v0 rejects anything that isn't `"splunk_hec"`.
#[derive(Deserialize)]
struct SiemConfigUpsertBody {
    kind: String,
    endpoint_url: String,
    token: String,
    #[serde(default)]
    enabled: Option<bool>,
}

/// GET / upsert response shape. `token` is always [`REDACTED_TOKEN`]
/// — the only writers are the upsert handler and the harness's
/// direct INSERT path; reads never expose it.
#[derive(Serialize)]
struct SiemConfigView {
    org_id: String,
    kind: String,
    endpoint_url: String,
    token: String,
    enabled: bool,
    created_at: String,
    updated_at: String,
}

/// Owner / admin ACL, identical shape to [`crate::oidc`]'s helper.
/// Duplicated rather than shared because both modules want their own
/// `*ConfigError` variants; the helper's body is two lines.
async fn assert_siem_owner_or_admin(
    pool: &Pool,
    org_id: &str,
    user_id: &str,
) -> Result<(), SiemConfigError> {
    let role = member_role(pool, org_id, user_id)
        .await
        .map_err(|e| SiemConfigError::Db(format!("member_role: {e}")))?;
    if !matches!(role.as_deref(), Some("owner" | "admin")) {
        return Err(SiemConfigError::Forbidden);
    }
    Ok(())
}

/// F5.3-d-iii-b-shim — credentials shape carried in the `token`
/// column for `kind = "s3_jsonl"`. The dispatcher (F5.3-d-iii-b-dispatch)
/// will deserialize this same struct at audit-emission time, build an
/// `aws_sdk_s3` Config from it, and PUT one JSONL object per event.
///
/// `endpoint_url_override` is optional. AWS S3 deployments omit it
/// (the SDK derives the endpoint from `region` + bucket). S3-compatible
/// providers (`MinIO`, Cloudflare R2, Backblaze B2) require it to point
/// at their custom endpoint.
#[derive(Debug, Deserialize)]
struct S3JsonlCredentials {
    access_key_id: String,
    secret_access_key: String,
    region: String,
    #[serde(default)]
    #[allow(dead_code)] // Honest finding: read by F5.3-d-iii-b-dispatch, not yet.
    endpoint_url_override: Option<String>,
}

/// Parse + validate the token field for `kind = "s3_jsonl"`. Pure;
/// pulled out for unit-testability without HTTP.
fn validate_s3_credentials_shape(token: &str) -> Result<(), SiemConfigError> {
    let creds: S3JsonlCredentials = serde_json::from_str(token).map_err(|_| {
        SiemConfigError::Invalid(
            "s3_jsonl token must be JSON with {access_key_id, secret_access_key, region}",
        )
    })?;
    // serde_json's deserialization already enforces presence of the
    // three required fields. Catch empty strings (which deserialize
    // fine but render the credentials useless).
    if creds.access_key_id.trim().is_empty() {
        return Err(SiemConfigError::Invalid(
            "s3_jsonl token: access_key_id must not be empty",
        ));
    }
    if creds.secret_access_key.trim().is_empty() {
        return Err(SiemConfigError::Invalid(
            "s3_jsonl token: secret_access_key must not be empty",
        ));
    }
    if creds.region.trim().is_empty() {
        return Err(SiemConfigError::Invalid(
            "s3_jsonl token: region must not be empty",
        ));
    }
    Ok(())
}

fn validate_siem_upsert(body: &SiemConfigUpsertBody) -> Result<(), SiemConfigError> {
    if !ALLOWED_SIEM_KINDS.contains(&body.kind.as_str()) {
        return Err(SiemConfigError::Invalid(
            "kind must be one of: splunk_hec, datadog_logs, syslog_cef, s3_jsonl",
        ));
    }
    if body.endpoint_url.trim().is_empty() {
        return Err(SiemConfigError::Invalid("endpoint_url is required"));
    }
    // Per-kind scheme branching:
    //   splunk_hec / datadog_logs → http(s)://
    //   syslog_cef                 → tcp:// or udp:// (F5.3-d-iii-c + -c-udp)
    //   s3_jsonl                   → s3:// (F5.3-d-iii-b-shim)
    let scheme_ok = match body.kind.as_str() {
        "syslog_cef" => {
            body.endpoint_url.starts_with("tcp://")
                || body.endpoint_url.starts_with("udp://")
        }
        "s3_jsonl" => body.endpoint_url.starts_with("s3://"),
        _ => {
            body.endpoint_url.starts_with("http://")
                || body.endpoint_url.starts_with("https://")
        }
    };
    if !scheme_ok {
        return Err(SiemConfigError::Invalid(
            "endpoint_url scheme mismatch for kind \
             (http/https for splunk_hec + datadog_logs; \
              tcp:// or udp:// for syslog_cef; \
              s3:// for s3_jsonl)",
        ));
    }
    // TOKENLESS_SIEM_KINDS skips the empty-token check (syslog_cef's
    // auth is at the network layer). All other kinds require a token;
    // s3_jsonl further requires it to be JSON-shaped.
    let tokenless = TOKENLESS_SIEM_KINDS.contains(&body.kind.as_str());
    if !tokenless && body.token.trim().is_empty() {
        return Err(SiemConfigError::Invalid("token is required"));
    }
    if body.kind == "s3_jsonl" {
        validate_s3_credentials_shape(&body.token)?;
    }
    Ok(())
}

// `parse_uuid` would normally go here; reuse the existing one from
// the oidc module since it has identical semantics.
fn parse_uuid_local(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    let dashes = [8usize, 13, 18, 23];
    for (i, &b) in bytes.iter().enumerate() {
        if dashes.contains(&i) {
            if b != b'-' {
                return None;
            }
        } else if !b.is_ascii_hexdigit() {
            return None;
        }
    }
    Some(s)
}

async fn handle_siem_config_upsert(
    State(state): State<SiemConfigState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
    axum::Json(body): axum::Json<SiemConfigUpsertBody>,
) -> Result<(StatusCode, axum::Json<SiemConfigView>), SiemConfigError> {
    let tenant = tenant.ok_or(SiemConfigError::Unauthenticated)?;
    if parse_uuid_local(&org_id).is_none() {
        return Err(SiemConfigError::Invalid("invalid org id"));
    }
    assert_siem_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;
    validate_siem_upsert(&body)?;

    let enabled = body.enabled.unwrap_or(true);

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| SiemConfigError::Db(format!("pool get: {e}")))?;
    let row = client
        .query_one(
            "INSERT INTO org_siem_configs (\
                org_id, kind, endpoint_url, token, enabled) \
             VALUES ($1::text::uuid, $2, $3, $4, $5) \
             ON CONFLICT (org_id) DO UPDATE SET \
                kind = EXCLUDED.kind, \
                endpoint_url = EXCLUDED.endpoint_url, \
                token = EXCLUDED.token, \
                enabled = EXCLUDED.enabled, \
                updated_at = NOW() \
             RETURNING kind, endpoint_url, enabled, \
                       to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                       to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
            &[
                &org_id,
                &body.kind,
                &body.endpoint_url,
                &body.token,
                &enabled,
            ],
        )
        .await
        .map_err(|e| SiemConfigError::Db(format!("upsert: {e:?}")))?;

    // `token` is intentionally NOT in the RETURNING clause —
    // a stray log of `row` can't leak it. Hardcoded REDACTED below.
    let view = SiemConfigView {
        org_id: org_id.clone(),
        kind: row.get(0),
        endpoint_url: row.get(1),
        token: REDACTED_TOKEN.to_string(),
        enabled: row.get(2),
        created_at: row.get(3),
        updated_at: row.get(4),
    };
    Ok((StatusCode::OK, axum::Json(view)))
}

async fn handle_siem_config_get(
    State(state): State<SiemConfigState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
) -> Result<axum::Json<SiemConfigView>, SiemConfigError> {
    let tenant = tenant.ok_or(SiemConfigError::Unauthenticated)?;
    if parse_uuid_local(&org_id).is_none() {
        return Err(SiemConfigError::Invalid("invalid org id"));
    }
    assert_siem_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| SiemConfigError::Db(format!("pool get: {e}")))?;
    let row = client
        .query_opt(
            "SELECT kind, endpoint_url, enabled, \
                    to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                    to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') \
             FROM org_siem_configs WHERE org_id = $1::text::uuid",
            &[&org_id],
        )
        .await
        .map_err(|e| SiemConfigError::Db(format!("select: {e:?}")))?
        .ok_or(SiemConfigError::NotFound)?;

    Ok(axum::Json(SiemConfigView {
        org_id: org_id.clone(),
        kind: row.get(0),
        endpoint_url: row.get(1),
        token: REDACTED_TOKEN.to_string(),
        enabled: row.get(2),
        created_at: row.get(3),
        updated_at: row.get(4),
    }))
}

async fn handle_siem_config_delete(
    State(state): State<SiemConfigState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
) -> Result<StatusCode, SiemConfigError> {
    let tenant = tenant.ok_or(SiemConfigError::Unauthenticated)?;
    if parse_uuid_local(&org_id).is_none() {
        return Err(SiemConfigError::Invalid("invalid org id"));
    }
    assert_siem_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| SiemConfigError::Db(format!("pool get: {e}")))?;
    let deleted = client
        .execute(
            "DELETE FROM org_siem_configs WHERE org_id = $1::text::uuid",
            &[&org_id],
        )
        .await
        .map_err(|e| SiemConfigError::Db(format!("delete: {e:?}")))?;
    if deleted == 0 {
        return Err(SiemConfigError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
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

    fn body(kind: &str, url: &str, token: &str) -> SiemConfigUpsertBody {
        SiemConfigUpsertBody {
            kind: kind.to_string(),
            endpoint_url: url.to_string(),
            token: token.to_string(),
            enabled: None,
        }
    }

    #[test]
    fn validate_siem_upsert_admits_splunk_hec_with_https() {
        let b = body("splunk_hec", "https://splunk.example.com:8088/services/collector/event", "tok");
        assert!(validate_siem_upsert(&b).is_ok());
    }

    #[test]
    fn validate_siem_upsert_rejects_unknown_kind() {
        let b = body("future_provider", "https://x.example.com", "tok");
        let e = validate_siem_upsert(&b).expect_err("must reject unknown kind");
        assert!(matches!(e, SiemConfigError::Invalid(msg) if msg.contains("kind")));
    }

    #[test]
    fn validate_siem_upsert_rejects_url_without_scheme() {
        let b = body("splunk_hec", "splunk.example.com:8088", "tok");
        let e = validate_siem_upsert(&b).expect_err("must reject missing scheme");
        assert!(matches!(e, SiemConfigError::Invalid(msg) if msg.contains("http")));
    }

    #[test]
    fn validate_siem_upsert_rejects_empty_token() {
        let b = body("splunk_hec", "https://splunk.example.com", "");
        let e = validate_siem_upsert(&b).expect_err("must reject empty token");
        assert!(matches!(e, SiemConfigError::Invalid(msg) if msg.contains("token")));
    }

    #[test]
    fn allowed_siem_kinds_includes_all_four_providers() {
        // F5.3-d-i shipped "splunk_hec"; F5.3-d-iii-a added
        // "datadog_logs"; F5.3-d-iii-c added "syslog_cef";
        // F5.3-d-iii-b-shim added "s3_jsonl" (CRUD-validatable;
        // dispatch lands in F5.3-d-iii-b-dispatch).
        assert_eq!(
            ALLOWED_SIEM_KINDS,
            &["splunk_hec", "datadog_logs", "syslog_cef", "s3_jsonl"]
        );
    }

    #[test]
    fn tokenless_kinds_only_lists_syslog_cef() {
        // Locking the tokenless-kinds set means a future provider
        // can't accidentally claim "no token needed" — the choice
        // is explicit per-kind.
        assert_eq!(TOKENLESS_SIEM_KINDS, &["syslog_cef"]);
    }

    #[test]
    fn validate_siem_upsert_admits_datadog_logs() {
        let b = body(
            "datadog_logs",
            "https://http-intake.logs.datadoghq.com/api/v2/logs",
            "dd-api-key-xxx",
        );
        assert!(validate_siem_upsert(&b).is_ok());
    }

    #[test]
    fn validate_siem_upsert_admits_syslog_cef_with_empty_token() {
        // F5.3-d-iii-c — syslog/CEF uses network-layer auth, so an
        // empty token is admitted. tcp:// scheme is required.
        let b = body("syslog_cef", "tcp://syslog.example.com:5514", "");
        assert!(validate_siem_upsert(&b).is_ok());
    }

    #[test]
    fn validate_siem_upsert_admits_syslog_cef_with_udp_scheme() {
        // F5.3-d-iii-c-udp — UDP fallback. Default syslog port is
        // 514; some legacy collectors only listen UDP.
        let b = body("syslog_cef", "udp://syslog.example.com:514", "");
        assert!(validate_siem_upsert(&b).is_ok());
    }

    #[test]
    fn validate_siem_upsert_rejects_udp_for_splunk() {
        // Inverse cross-kind mismatch — Splunk HEC is HTTP, not UDP.
        let b = body("splunk_hec", "udp://splunk.example.com:8088", "tok");
        let e = validate_siem_upsert(&b).expect_err("must reject udp scheme for HEC");
        assert!(matches!(e, SiemConfigError::Invalid(msg) if msg.contains("scheme")));
    }

    fn s3_token_json(access: &str, secret: &str, region: &str) -> String {
        format!(
            r#"{{"access_key_id":"{access}","secret_access_key":"{secret}","region":"{region}"}}"#
        )
    }

    #[test]
    fn validate_siem_upsert_admits_s3_jsonl_with_credentials_json() {
        // F5.3-d-iii-b-shim — happy path: JSON-shape token + s3:// scheme.
        let tok = s3_token_json("AKIA…", "secret…", "us-east-1");
        let b = body("s3_jsonl", "s3://my-bucket/audit-prefix/", &tok);
        assert!(validate_siem_upsert(&b).is_ok());
    }

    #[test]
    fn validate_siem_upsert_rejects_s3_jsonl_with_http_scheme() {
        let tok = s3_token_json("AKIA…", "secret…", "us-east-1");
        let b = body("s3_jsonl", "https://s3.amazonaws.com/bucket/", &tok);
        let e = validate_siem_upsert(&b).expect_err("must reject http scheme for s3_jsonl");
        assert!(matches!(e, SiemConfigError::Invalid(msg) if msg.contains("scheme")));
    }

    #[test]
    fn validate_siem_upsert_rejects_s3_jsonl_with_malformed_json_token() {
        // Token is not valid JSON — the validator surfaces the JSON
        // shape error rather than letting it crash at dispatch time.
        let b = body("s3_jsonl", "s3://my-bucket/", "this-is-not-json");
        let e = validate_siem_upsert(&b).expect_err("must reject non-JSON token");
        assert!(
            matches!(e, SiemConfigError::Invalid(msg) if msg.contains("JSON")),
            "expected JSON-shape error, got {e:?}"
        );
    }

    #[test]
    fn validate_siem_upsert_rejects_s3_jsonl_with_missing_access_key_id() {
        // JSON parses but a required field is missing — serde's
        // deserialization fails, surfaced as the JSON-shape error.
        let tok = r#"{"secret_access_key":"secret…","region":"us-east-1"}"#;
        let b = body("s3_jsonl", "s3://my-bucket/", tok);
        let e = validate_siem_upsert(&b)
            .expect_err("must reject token missing access_key_id");
        assert!(matches!(e, SiemConfigError::Invalid(msg) if msg.contains("JSON")));
    }

    #[test]
    fn validate_siem_upsert_rejects_s3_jsonl_with_empty_field_value() {
        // JSON parses + has all 3 fields, but `region` is "" — the
        // dispatch path would fail with an unhelpful AWS error
        // ("invalid region"); rejecting at the CRUD edge surfaces it
        // immediately.
        let tok = s3_token_json("AKIA…", "secret…", "");
        let b = body("s3_jsonl", "s3://my-bucket/", &tok);
        let e = validate_siem_upsert(&b).expect_err("must reject empty region");
        assert!(matches!(e, SiemConfigError::Invalid(msg) if msg.contains("region")));
    }

    #[test]
    fn validate_s3_credentials_shape_admits_optional_endpoint_override() {
        // S3-compatible providers (MinIO, R2, B2) use the optional
        // endpoint_url_override field. Serde's #[serde(default)] keeps
        // the validator silent when it's omitted; including it must
        // also pass cleanly.
        let tok = r#"{"access_key_id":"AKIA…","secret_access_key":"secret…","region":"us-east-1","endpoint_url_override":"https://r2.example.com"}"#;
        assert!(validate_s3_credentials_shape(tok).is_ok());
    }

    #[test]
    fn civil_from_unix_secs_known_dates() {
        // F5.3-d-iii-b-dispatch — Howard Hinnant's algorithm.
        // Check against well-known unix epochs.
        // 0 = 1970-01-01.
        assert_eq!(civil_from_unix_secs(0), (1970, 1, 1));
        // 86_399 = same day (just before midnight UTC).
        assert_eq!(civil_from_unix_secs(86_399), (1970, 1, 1));
        // 86_400 = 1970-01-02.
        assert_eq!(civil_from_unix_secs(86_400), (1970, 1, 2));
        // 2024-01-01 = 1_704_067_200 unix.
        assert_eq!(civil_from_unix_secs(1_704_067_200), (2024, 1, 1));
        // 2026-05-22 = 1_779_408_000 unix (this session's date).
        // 56 years * 365 + 14 leap days + 141 days-into-2026 = 20_595
        // days since epoch; × 86_400 = 1_779_408_000 seconds.
        assert_eq!(civil_from_unix_secs(1_779_408_000), (2026, 5, 22));
        // 2024-02-29 (leap year) = 1_709_164_800 unix.
        assert_eq!(civil_from_unix_secs(1_709_164_800), (2024, 2, 29));
        // 2100-01-01 = 4_102_444_800 unix (centennial non-leap).
        assert_eq!(civil_from_unix_secs(4_102_444_800), (2100, 1, 1));
    }

    #[test]
    fn validate_siem_upsert_rejects_syslog_cef_with_http_scheme() {
        // Cross-kind scheme mismatch — syslog_cef requires tcp://,
        // not http(s)://. Catches the common mistake of "reuse the
        // Datadog URL pattern".
        let b = body("syslog_cef", "https://syslog.example.com", "");
        let e = validate_siem_upsert(&b).expect_err("must reject http scheme");
        assert!(matches!(e, SiemConfigError::Invalid(msg) if msg.contains("scheme")));
    }

    #[test]
    fn validate_siem_upsert_rejects_splunk_with_tcp_scheme() {
        // Inverse cross-kind mismatch — Splunk HEC requires http(s)://.
        let b = body("splunk_hec", "tcp://splunk.example.com:8088", "tok");
        let e = validate_siem_upsert(&b).expect_err("must reject tcp scheme for HEC");
        assert!(matches!(e, SiemConfigError::Invalid(msg) if msg.contains("scheme")));
    }

    #[test]
    fn cef_escape_passes_through_safe_strings() {
        // Zero-alloc fast path for strings without metachars.
        let pass = cef_escape("oidc.login");
        assert!(matches!(pass, std::borrow::Cow::Borrowed("oidc.login")));
    }

    #[test]
    fn cef_escape_handles_pipe_backslash_equals() {
        // The three CEF metachars per the spec.
        assert_eq!(cef_escape("a|b").as_ref(), r"a\|b");
        assert_eq!(cef_escape("a\\b").as_ref(), r"a\\b");
        assert_eq!(cef_escape("k=v").as_ref(), r"k\=v");
    }

    #[test]
    fn cef_escape_collapses_newlines_to_spaces() {
        // Newlines would end the syslog message prematurely.
        assert_eq!(cef_escape("a\nb").as_ref(), "a b");
        assert_eq!(cef_escape("a\r\nb").as_ref(), "a  b");
    }

    #[test]
    fn format_cef_line_has_correct_header_shape() {
        let entry = ministr_api::AuditEntry::new("oidc.login", "user-uuid-x")
            .with_org("org-uuid-y")
            .with_actor("user-uuid-x");
        let line = format_cef_line(&entry);
        // Header — 7 fields delimited by `|` (vendor-side parsers
        // count from CEF:0).
        assert!(
            line.starts_with("CEF:0|ministr|ministr-cloud-audit|1|oidc.login|oidc.login|5|"),
            "CEF header malformed: {line}"
        );
        assert!(line.contains("orgId=org-uuid-y"), "orgId extension missing: {line}");
        assert!(line.contains("suser=user-uuid-x"), "suser extension missing: {line}");
        assert!(line.contains("resource=user-uuid-x"), "resource extension missing: {line}");
        // No newlines inside the rendered line — it's the syslog
        // message terminator.
        assert!(!line.contains('\n'), "CEF line contains a newline: {line}");
    }

    #[test]
    fn parse_uuid_local_admits_canonical_form() {
        assert!(parse_uuid_local("00000000-0000-0000-0000-000000000000").is_some());
        assert!(parse_uuid_local("deadbeef-1234-5678-90ab-cdef00000000").is_some());
    }

    #[test]
    fn parse_uuid_local_rejects_garbage() {
        assert!(parse_uuid_local("").is_none());
        assert!(parse_uuid_local("not-a-uuid").is_none());
        assert!(parse_uuid_local("00000000-0000-0000-0000-0000000000ZZ").is_none());
        // Wrong dash positions.
        assert!(parse_uuid_local("000000000-000-0000-0000-000000000000").is_none());
    }

    #[test]
    fn redacted_token_sentinel_matches_oidc() {
        // F5.2-d ships REDACTED_CLIENT_SECRET = "[REDACTED]" and
        // frontend code branches on the literal string. Locking
        // REDACTED_TOKEN to the same value keeps the UI handling
        // uniform across both providers.
        assert_eq!(REDACTED_TOKEN, "[REDACTED]");
    }
}
