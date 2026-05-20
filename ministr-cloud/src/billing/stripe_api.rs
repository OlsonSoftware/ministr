//! Stripe outbound API client (F1.5 sub-bullets 1 + 2).
//!
//! Companion to [`crate::billing::stripe`] (the webhook receiver) —
//! this module owns the calls WE make to Stripe. Today:
//!
//! - `POST /v1/customers` — [`StripeClient::create_customer`] on first
//!   sign-in. The returned `cus_…` id is stored in `users.stripe_customer_id`
//!   so future Checkout sessions and Meter events can reference it.
//! - `POST /v1/billing/meter_events` — [`StripeClient::report_meter_event`]
//!   for usage-based billing. The function exists but has no internal
//!   caller in F1.5 ("wired and tolerant"); F2.3 enforcement and F2.4
//!   Checkout will start invoking it once `index.minutes` overage logic
//!   lands.
//!
//! # API conventions
//!
//! - Form-encoded bodies (`application/x-www-form-urlencoded`) — Stripe's
//!   public REST surface accepts only forms, never JSON. Matches the
//!   shape every official Stripe SDK uses internally.
//! - `Idempotency-Key` header on `POST /v1/customers` so the GitHub
//!   sign-in callback can retry without minting duplicate Customers.
//!   The key we send is `cust-create-github-{github_id}` — deterministic
//!   per user, never colliding across users.
//! - `identifier` body field on `POST /v1/billing/meter_events` — same
//!   dedup primitive but server-side; Stripe Meters do not honour
//!   Idempotency-Key, by design (per Meters docs, 2026-02-25 API
//!   version).
//! - All errors return [`StripeApiError`]; transport and protocol are
//!   collapsed at the boundary so the caller doesn't depend on
//!   `reqwest`'s error taxonomy.
//!
//! # Why thin reqwest, not `async-stripe`
//!
//! `async-stripe` pulls a substantial dependency surface (full event
//! type tree, Webhook constructors, hundreds of derives). We use a
//! tiny subset of the Stripe API; a thin reqwest wrapper costs nothing
//! and keeps the binary's audit story compact. Same posture as the
//! `GitHubIdp` client in [`crate::idp::github`].

use std::time::Duration;

use reqwest::header;
use serde::Deserialize;
use tracing::debug;

/// Default base URL of the Stripe REST API. Overridable for tests via
/// [`StripeClient::with_base_url`].
const DEFAULT_BASE_URL: &str = "https://api.stripe.com";

/// Stripe API version pinned at construction time. Mirrors Stripe's
/// "version-locked SDK" pattern — if Stripe ships a breaking change,
/// our existing requests keep getting the old schema until we bump
/// this constant deliberately.
const STRIPE_API_VERSION: &str = "2026-02-25.clover";

/// User-Agent header. Stripe's API requires every request to identify
/// itself (informational; not authentication).
const USER_AGENT: &str = "ministr-cloud-billing/1 (+https://ministr.ai)";

/// Errors surfaced by the Stripe API client. Network and protocol
/// failures are collapsed into [`StripeApiError::Transport`] /
/// [`StripeApiError::Protocol`] so call sites depend on this enum,
/// not on `reqwest::Error`.
#[derive(Debug, thiserror::Error)]
pub enum StripeApiError {
    /// The supplied API key was empty or whitespace-only. Caller forgot
    /// to set `MINISTR_STRIPE_SECRET_KEY`.
    #[error("stripe api: api key is empty")]
    EmptyApiKey,
    /// Network-layer failure (timeout, DNS, TLS).
    #[error("stripe api transport error: {0}")]
    Transport(String),
    /// Stripe returned a 4xx/5xx OR the response body could not be
    /// parsed. The inner string is the body for logging; do not surface
    /// to end users.
    #[error("stripe api protocol error: {0}")]
    Protocol(String),
}

/// Outbound Stripe API client.
///
/// Construct once at cloud startup via [`StripeClient::new`]; share via
/// `Clone` (the inner `reqwest::Client` is `Arc`-backed). The client
/// holds the API key in memory — if rotation is needed, build a fresh
/// instance and atomically swap.
#[derive(Debug, Clone)]
pub struct StripeClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl StripeClient {
    /// Build a client bound to the canonical Stripe API host
    /// (`https://api.stripe.com`).
    ///
    /// # Errors
    ///
    /// Returns [`StripeApiError::EmptyApiKey`] when `api_key` is empty or
    /// whitespace-only — distinguishes a misconfigured environment from
    /// a transient transport failure.
    /// Returns [`StripeApiError::Transport`] when the inner HTTP client
    /// fails to build (extremely rare — usually a system-TLS load
    /// failure).
    pub fn new(api_key: impl Into<String>) -> Result<Self, StripeApiError> {
        Self::with_base_url(api_key, DEFAULT_BASE_URL)
    }

    /// Build a client against an arbitrary base URL — only used by
    /// tests pointed at a local mock server. Production code calls
    /// [`Self::new`].
    ///
    /// # Errors
    ///
    /// Same surface as [`Self::new`]:
    /// [`StripeApiError::EmptyApiKey`] for blank credentials,
    /// [`StripeApiError::Transport`] for a failed `reqwest::Client`
    /// build.
    pub fn with_base_url(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, StripeApiError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(StripeApiError::EmptyApiKey);
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| StripeApiError::Transport(format!("build http: {e}")))?;
        Ok(Self {
            http,
            api_key,
            base_url: trim_trailing_slashes(base_url.into()),
        })
    }

    /// Create a Stripe Customer for a brand-new ministr user.
    ///
    /// `email` is the verified GitHub email; `github_id` is the
    /// GitHub user id (also persisted in `users.github_id`). The
    /// idempotency key is `cust-create-github-{github_id}` so retries
    /// after a crash return the SAME customer instead of minting
    /// duplicates.
    ///
    /// Returns the Stripe customer id (`cus_…`).
    ///
    /// # Errors
    ///
    /// - [`StripeApiError::Transport`] for network failures.
    /// - [`StripeApiError::Protocol`] for non-2xx responses, missing
    ///   `id` field, or malformed JSON.
    pub async fn create_customer(
        &self,
        email: &str,
        github_id: i64,
    ) -> Result<String, StripeApiError> {
        let form = customer_form_body(email, github_id);
        let idempotency_key = format!("cust-create-github-{github_id}");
        let url = format!("{}/v1/customers", self.base_url);

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.api_key, Some(""))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("Stripe-Version", STRIPE_API_VERSION)
            .header("Idempotency-Key", &idempotency_key)
            .body(form)
            .send()
            .await
            .map_err(|e| StripeApiError::Transport(format!("create_customer: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StripeApiError::Protocol(format!(
                "create_customer status {status}: {body}"
            )));
        }
        let parsed: CustomerResponse = resp.json().await.map_err(|e| {
            StripeApiError::Protocol(format!("create_customer parse: {e}"))
        })?;
        debug!(
            customer_id = %parsed.id,
            github_id,
            "stripe customer created"
        );
        Ok(parsed.id)
    }

    /// Report a meter event for usage-based billing.
    ///
    /// `event_name` is the meter's name configured in the Stripe
    /// dashboard (e.g. `"index_minutes"`). `customer_id` is the
    /// `cus_…` Stripe id from `users.stripe_customer_id`. `value` is
    /// the quantity attributed to this event (Stripe accumulates it
    /// per billing period). `identifier` is the dedup key — pass a
    /// stable per-event id (e.g. `usage_event_id`) so retries collapse.
    ///
    /// No internal caller in F1.5. F2.3 (quota enforcement) and the
    /// daily rollup will start invoking this once `index.minutes`
    /// overage logic lands.
    ///
    /// # Errors
    ///
    /// Same transport / protocol mapping as
    /// [`Self::create_customer`].
    pub async fn report_meter_event(
        &self,
        event_name: &str,
        customer_id: &str,
        value: f64,
        identifier: &str,
    ) -> Result<(), StripeApiError> {
        let form = meter_event_form_body(event_name, customer_id, value, identifier);
        let url = format!("{}/v1/billing/meter_events", self.base_url);

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.api_key, Some(""))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("Stripe-Version", STRIPE_API_VERSION)
            .body(form)
            .send()
            .await
            .map_err(|e| StripeApiError::Transport(format!("report_meter_event: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StripeApiError::Protocol(format!(
                "report_meter_event status {status}: {body}"
            )));
        }
        debug!(
            event_name,
            customer_id,
            identifier,
            "stripe meter event reported"
        );
        Ok(())
    }
}

/// Subset of Stripe's `Customer` object the client reads. Stripe
/// returns many more fields; we only need the id.
#[derive(Debug, Deserialize)]
struct CustomerResponse {
    id: String,
}

/// Build the form-encoded body for `POST /v1/customers`. Pulled out so
/// the encoding can be unit-tested without an HTTP round-trip.
fn customer_form_body(email: &str, github_id: i64) -> String {
    let mut body = String::new();
    body.push_str("email=");
    body.push_str(&form_encode(email));
    body.push_str("&metadata[github_id]=");
    body.push_str(&form_encode(&github_id.to_string()));
    body.push_str("&metadata[source]=ministr-cloud-signin");
    body
}

/// Build the form-encoded body for `POST /v1/billing/meter_events`.
/// Stripe nests payload fields under `payload[...]` per the form-
/// encoding convention their SDKs use for sub-objects.
fn meter_event_form_body(
    event_name: &str,
    customer_id: &str,
    value: f64,
    identifier: &str,
) -> String {
    let mut body = String::new();
    body.push_str("event_name=");
    body.push_str(&form_encode(event_name));
    body.push_str("&payload[stripe_customer_id]=");
    body.push_str(&form_encode(customer_id));
    body.push_str("&payload[value]=");
    body.push_str(&form_encode(&value.to_string()));
    body.push_str("&identifier=");
    body.push_str(&form_encode(identifier));
    body
}

/// Form-encode a single value. Stripe accepts the standard
/// application/x-www-form-urlencoded encoding — same RFC 3986
/// unreserved alphabet as the rest of the workspace's encoders.
fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b == b' ' {
            out.push('+');
        } else if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_nibble(b >> 4));
            out.push(hex_nibble(b & 0x0f));
        }
    }
    out
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'A' + (n - 10)) as char,
        _ => '0',
    }
}

fn trim_trailing_slashes(mut s: String) -> String {
    while s.ends_with('/') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_empty_api_key() {
        assert!(matches!(
            StripeClient::new(""),
            Err(StripeApiError::EmptyApiKey)
        ));
        assert!(matches!(
            StripeClient::new("   "),
            Err(StripeApiError::EmptyApiKey)
        ));
    }

    #[test]
    fn new_accepts_realistic_key_shape() {
        // Stripe test keys are prefixed `sk_test_`; live keys `sk_live_`.
        // The client doesn't validate the prefix (Stripe accepts either
        // against the right base URL) — it only checks non-emptiness.
        let c = StripeClient::new("sk_test_xyz").expect("non-empty key accepted");
        assert!(c.base_url.starts_with("https://api.stripe.com"));
    }

    #[test]
    fn customer_form_body_encodes_email_and_github_id() {
        let body = customer_form_body("user+plus@example.com", 42);
        assert!(body.contains("email=user%2Bplus%40example.com"), "got {body}");
        assert!(body.contains("metadata[github_id]=42"), "got {body}");
        assert!(body.contains("metadata[source]=ministr-cloud-signin"));
    }

    #[test]
    fn meter_event_form_body_nests_payload_fields() {
        let body = meter_event_form_body("index_minutes", "cus_abc", 12.5, "evt-1");
        assert!(body.contains("event_name=index_minutes"));
        assert!(body.contains("payload[stripe_customer_id]=cus_abc"));
        assert!(body.contains("payload[value]=12.5"));
        assert!(body.contains("identifier=evt-1"));
    }

    #[test]
    fn form_encode_preserves_unreserved_chars() {
        assert_eq!(form_encode("abcDEF-_.~012"), "abcDEF-_.~012");
        assert_eq!(form_encode("a b"), "a+b");
        assert_eq!(form_encode("a:b"), "a%3Ab");
        assert_eq!(form_encode("a@b"), "a%40b");
    }

    #[test]
    fn trim_trailing_slashes_normalises_base_url() {
        assert_eq!(
            trim_trailing_slashes("https://api.stripe.com///".into()),
            "https://api.stripe.com"
        );
    }

    /// Read enough bytes from `stream` to capture the full HTTP request
    /// (headers + body). reqwest can split the request across multiple
    /// TCP segments, so a single `read` would miss part of it. We loop
    /// until we see the headers terminator AND enough body bytes to
    /// satisfy `Content-Length`, then return the accumulated string.
    async fn drain_http_request(
        stream: &mut tokio::net::TcpStream,
    ) -> String {
        use tokio::io::AsyncReadExt as _;
        let mut buf = Vec::with_capacity(4096);
        let mut tmp = [0u8; 1024];
        loop {
            let n = stream.read(&mut tmp).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            let text = String::from_utf8_lossy(&buf);
            if let Some(idx) = text.find("\r\n\r\n") {
                let headers = &text[..idx];
                let content_length = headers
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                    .and_then(|l| l.split(':').nth(1))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if buf.len() >= idx + 4 + content_length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    #[tokio::test]
    async fn create_customer_round_trips_against_local_mock() {
        // Spin up a single-shot HTTP server that responds with a
        // canned Customer JSON. Verifies the wire shape end-to-end
        // (URL, method, body, headers) without contacting Stripe.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            let (mut stream, _) = listener.accept().await.unwrap();
            let req = drain_http_request(&mut stream).await;
            let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 28\r\nConnection: close\r\n\r\n{\"id\":\"cus_test_round_trip\"}";
            stream.write_all(resp.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            req
        });

        let base = format!("http://{addr}");
        let client = StripeClient::with_base_url("sk_test_dummy", base).unwrap();
        let id = client
            .create_customer("u@example.com", 99)
            .await
            .expect("create_customer succeeds against mock");
        assert_eq!(id, "cus_test_round_trip");

        let req = server.await.unwrap();
        let req_lower = req.to_ascii_lowercase();
        assert!(req.starts_with("POST /v1/customers"), "request line: {req}");
        assert!(
            req_lower.contains("stripe-version: 2026-02-25.clover"),
            "request: {req}"
        );
        assert!(
            req_lower.contains("idempotency-key: cust-create-github-99"),
            "request: {req}"
        );
        assert!(req.contains("email=u%40example.com"), "request: {req}");
        assert!(req.contains("metadata[github_id]=99"), "request: {req}");
    }

    #[tokio::test]
    async fn create_customer_surfaces_protocol_error_on_4xx() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = drain_http_request(&mut stream).await;
            let body = "{\"error\":{\"message\":\"bad key\"}}";
            let resp = format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
        });

        let base = format!("http://{addr}");
        let client = StripeClient::with_base_url("sk_test_dummy", base).unwrap();
        let err = client
            .create_customer("u@example.com", 100)
            .await
            .expect_err("4xx should surface as Protocol");
        assert!(matches!(err, StripeApiError::Protocol(ref m) if m.contains("401")));
    }

    #[tokio::test]
    async fn report_meter_event_posts_expected_body() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            let (mut stream, _) = listener.accept().await.unwrap();
            let req = drain_http_request(&mut stream).await;
            let body = "{\"object\":\"billing.meter_event\"}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            req
        });

        let base = format!("http://{addr}");
        let client = StripeClient::with_base_url("sk_test_dummy", base).unwrap();
        client
            .report_meter_event("index_minutes", "cus_abc", 3.0, "evt-42")
            .await
            .expect("meter event posts");

        let req = server.await.unwrap();
        let req_lower = req.to_ascii_lowercase();
        assert!(
            req.starts_with("POST /v1/billing/meter_events"),
            "request line: {req}"
        );
        assert!(req.contains("event_name=index_minutes"), "request: {req}");
        assert!(
            req.contains("payload[stripe_customer_id]=cus_abc"),
            "request: {req}"
        );
        assert!(req.contains("payload[value]=3"), "request: {req}");
        assert!(req.contains("identifier=evt-42"), "request: {req}");
        // Meter events do NOT use Idempotency-Key; dedup is via the
        // `identifier` body field.
        assert!(
            !req_lower.contains("idempotency-key:"),
            "meter events must not carry Idempotency-Key: {req}"
        );
    }
}
