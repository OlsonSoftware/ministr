//! Stripe webhook receiver (F1.5 sub-bullet 3).
//!
//! Mounts `POST /webhooks/stripe`. Stripe hits this endpoint on every
//! subscription-related event; we verify the signature, decode the
//! event, and flip `users.plan_id` / `orgs.plan_id` accordingly.
//!
//! # Auth
//!
//! Not behind OAuth — Stripe is the caller. Security is the
//! `Stripe-Signature` HMAC check against the webhook signing secret
//! (`MINISTR_STRIPE_WEBHOOK_SECRET`). Without a valid signature the
//! handler returns 400 and never touches the database.
//!
//! # Signature scheme (Stripe v1, 2026 — unchanged since 2019)
//!
//! Header `Stripe-Signature: t=<unix_ts>,v1=<hex>,v1=<hex>,...`. The
//! `<hex>` payload is `HMAC_SHA256(secret, format!("{ts}.{body}"))`.
//! Multiple `v1=` signatures appear during a webhook-key rotation —
//! we accept the event if ANY of them matches.
//!
//! Default tolerance: 5 minutes between header `t` and wall-clock
//! now. This rejects replayed events whose signing key may have been
//! since rotated. Same window Stripe's own SDKs use.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use deadpool_postgres::Pool;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tracing::{debug, info, warn};

/// Default signature-timestamp tolerance — matches Stripe's official
/// SDKs and the documented webhook guidance.
const SIGNATURE_TOLERANCE_SECS: u64 = 300;

/// Errors surfaced by the webhook handler. All map to HTTP 400 to
/// keep the surface narrow — Stripe retries on non-2xx, and we never
/// want to leak a hint about which check failed to a forger.
#[derive(Debug, thiserror::Error)]
pub enum StripeWebhookError {
    #[error("missing Stripe-Signature header")]
    MissingSignature,
    #[error("malformed Stripe-Signature header")]
    MalformedSignature,
    #[error("signature timestamp outside tolerance window")]
    TimestampOutOfTolerance,
    #[error("no matching v1 signature")]
    SignatureMismatch,
    #[error("could not decode event JSON: {0}")]
    BadJson(String),
    #[error("database error: {0}")]
    Database(String),
}

/// Verify a Stripe webhook signature.
///
/// `header_value` is the raw `Stripe-Signature` HTTP header. `payload`
/// is the raw request body bytes — passing pre-parsed JSON breaks
/// the HMAC.
///
/// Returns `Ok(())` if at least one `v1=...` entry in the header
/// matches `HMAC_SHA256(secret, format!("{t}.{body}"))` and `t` is
/// within [`SIGNATURE_TOLERANCE_SECS`] of `now()`.
///
/// # Errors
///
/// Returns [`StripeWebhookError::MissingSignature`] /
/// [`StripeWebhookError::MalformedSignature`] /
/// [`StripeWebhookError::TimestampOutOfTolerance`] /
/// [`StripeWebhookError::SignatureMismatch`] as appropriate.
pub fn verify_signature(
    payload: &[u8],
    header_value: &str,
    secret: &str,
) -> Result<(), StripeWebhookError> {
    verify_signature_at(payload, header_value, secret, now_unix())
}

/// Test seam for [`verify_signature`] — accepts an explicit "now"
/// so timestamp-tolerance tests don't have to fudge the clock.
fn verify_signature_at(
    payload: &[u8],
    header_value: &str,
    secret: &str,
    now: u64,
) -> Result<(), StripeWebhookError> {
    let parsed = parse_signature_header(header_value)?;
    let drift = now.abs_diff(parsed.timestamp);
    if drift > SIGNATURE_TOLERANCE_SECS {
        return Err(StripeWebhookError::TimestampOutOfTolerance);
    }
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .map_err(|_| StripeWebhookError::MalformedSignature)?;
    mac.update(parsed.timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(payload);
    let expected = mac.finalize().into_bytes();
    for v1_hex in parsed.v1_signatures {
        let Some(decoded) = decode_hex(v1_hex) else {
            continue;
        };
        if decoded.ct_eq(expected.as_slice()).into() {
            return Ok(());
        }
    }
    Err(StripeWebhookError::SignatureMismatch)
}

struct ParsedHeader<'a> {
    timestamp: u64,
    v1_signatures: Vec<&'a str>,
}

fn parse_signature_header(header: &str) -> Result<ParsedHeader<'_>, StripeWebhookError> {
    let mut timestamp: Option<u64> = None;
    let mut v1_signatures: Vec<&str> = Vec::new();
    for part in header.split(',') {
        let (k, v) = part
            .split_once('=')
            .ok_or(StripeWebhookError::MalformedSignature)?;
        match k.trim() {
            "t" => {
                timestamp = Some(
                    v.trim()
                        .parse::<u64>()
                        .map_err(|_| StripeWebhookError::MalformedSignature)?,
                );
            }
            "v1" => v1_signatures.push(v.trim()),
            _ => {} // ignore unknown schemes (e.g. v0 deprecated)
        }
    }
    let timestamp = timestamp.ok_or(StripeWebhookError::MissingSignature)?;
    if v1_signatures.is_empty() {
        return Err(StripeWebhookError::MissingSignature);
    }
    Ok(ParsedHeader {
        timestamp,
        v1_signatures,
    })
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for chunk in s.as_bytes().chunks(2) {
        let hi = hex_digit(chunk[0])?;
        let lo = hex_digit(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

// ── Event dispatch ─────────────────────────────────────────────────────────

/// State for the webhook router.
#[derive(Clone)]
pub struct StripeWebhookState {
    pool: Arc<Pool>,
    secret: Arc<String>,
}

impl StripeWebhookState {
    /// Construct from an existing pool + signing secret. The secret
    /// is the value of `MINISTR_STRIPE_WEBHOOK_SECRET` (Stripe
    /// dashboard → endpoint signing secret, prefixed `whsec_`).
    #[must_use]
    pub fn new(pool: Arc<Pool>, secret: String) -> Self {
        Self {
            pool,
            secret: Arc::new(secret),
        }
    }
}

/// Build the webhook router. Mount at the application root —
/// `POST /webhooks/stripe` lives outside the OAuth-protected branch.
pub fn stripe_webhook_routes(state: StripeWebhookState) -> Router {
    Router::new()
        .route("/webhooks/stripe", post(handle_webhook))
        .with_state(state)
}

async fn handle_webhook(
    State(state): State<StripeWebhookState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<&'static str, StripeWebhookError> {
    let sig_header = headers
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or(StripeWebhookError::MissingSignature)?;
    verify_signature(&body, sig_header, &state.secret)?;
    let event: StripeEvent = serde_json::from_slice(&body)
        .map_err(|e| StripeWebhookError::BadJson(e.to_string()))?;
    dispatch(&state.pool, &event).await?;
    debug!(event_type = %event.event_type, "stripe webhook accepted");
    Ok("ok")
}

#[derive(Debug, serde::Deserialize)]
struct StripeEvent {
    #[serde(rename = "type")]
    event_type: String,
    data: StripeEventData,
}

#[derive(Debug, serde::Deserialize)]
struct StripeEventData {
    object: StripeEventObject,
}

#[derive(Debug, serde::Deserialize)]
struct StripeEventObject {
    /// Subscription's customer id (`cus_…`). Present on the
    /// subscription-shaped events this handler cares about.
    #[serde(default)]
    customer: Option<String>,
    /// Subscription status (`active`, `past_due`, `canceled`, …).
    #[serde(default)]
    status: Option<String>,
    /// Items array — first item's price metadata carries the plan
    /// name. Decoded only when we need it.
    #[serde(default)]
    items: Option<StripeItemsContainer>,
}

#[derive(Debug, serde::Deserialize)]
struct StripeItemsContainer {
    #[serde(default)]
    data: Vec<StripeItem>,
}

#[derive(Debug, serde::Deserialize)]
struct StripeItem {
    #[serde(default)]
    price: Option<StripePrice>,
}

#[derive(Debug, serde::Deserialize)]
struct StripePrice {
    /// Stripe price `lookup_key` (e.g. `"pro"`, `"team"`). We rely on
    /// the operator setting this in the Stripe dashboard for each
    /// price; the `lookup_key` is the wire string we persist into
    /// `plan_id`.
    #[serde(default)]
    lookup_key: Option<String>,
}

async fn dispatch(pool: &Pool, event: &StripeEvent) -> Result<(), StripeWebhookError> {
    let Some(customer) = event.data.object.customer.as_deref() else {
        // Events without a customer slot are ignored — F1.5 only
        // cares about subscription updates / deletions.
        return Ok(());
    };
    match event.event_type.as_str() {
        "customer.subscription.updated" | "customer.subscription.created" => {
            let plan_id = plan_id_from_event(&event.data.object);
            apply_plan(pool, customer, plan_id.as_deref()).await
        }
        "customer.subscription.deleted" => apply_plan(pool, customer, None).await,
        other => {
            debug!(event = %other, "stripe webhook: event type not handled");
            Ok(())
        }
    }
}

/// Pull the plan id (e.g. `"pro"`) from a subscription event. Looks
/// at the first item's price `lookup_key`. The "canceled" status
/// shortcuts to None so a still-attached but cancelled subscription
/// downgrades the user.
fn plan_id_from_event(obj: &StripeEventObject) -> Option<String> {
    if obj.status.as_deref() == Some("canceled") {
        return None;
    }
    obj.items
        .as_ref()?
        .data
        .first()?
        .price
        .as_ref()?
        .lookup_key
        .clone()
}

async fn apply_plan(
    pool: &Pool,
    stripe_customer_id: &str,
    plan_id: Option<&str>,
) -> Result<(), StripeWebhookError> {
    let client = pool
        .get()
        .await
        .map_err(|e| StripeWebhookError::Database(format!("get conn: {e}")))?;

    let plan_for_sql = plan_id.unwrap_or("free");
    let user_updated = client
        .execute(
            "UPDATE users SET plan_id = $2 WHERE stripe_customer_id = $1",
            &[&stripe_customer_id, &plan_for_sql],
        )
        .await
        .map_err(|e| StripeWebhookError::Database(format!("users update: {e}")))?;
    let org_updated = client
        .execute(
            "UPDATE orgs SET plan_id = $2 WHERE stripe_customer_id = $1",
            &[&stripe_customer_id, &plan_for_sql],
        )
        .await
        .map_err(|e| StripeWebhookError::Database(format!("orgs update: {e}")))?;
    info!(
        stripe_customer = %stripe_customer_id,
        plan = %plan_for_sql,
        users_updated = user_updated,
        orgs_updated = org_updated,
        "stripe webhook: plan applied"
    );
    Ok(())
}

impl IntoResponse for StripeWebhookError {
    fn into_response(self) -> axum::response::Response {
        match &self {
            Self::Database(msg) => warn!(error = %msg, "stripe webhook db error"),
            Self::BadJson(msg) => warn!(error = %msg, "stripe webhook bad json"),
            // Auth-related rejections: log at debug — these are
            // expected when a forger pokes the endpoint.
            other => debug!(error = %other, "stripe webhook auth rejected"),
        }
        // Always 400 — never reveal which check failed.
        (StatusCode::BAD_REQUEST, "bad webhook").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fmt::Write as _;

    /// Build a valid Stripe-Signature header for known inputs.
    fn sign(payload: &[u8], secret: &str, ts: u64) -> String {
        let mut mac =
            <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(ts.to_string().as_bytes());
        mac.update(b".");
        mac.update(payload);
        let bytes = mac.finalize().into_bytes();
        let mut hex = String::with_capacity(bytes.len() * 2);
        for b in &bytes {
            write!(&mut hex, "{b:02x}").expect("writing to String never fails");
        }
        format!("t={ts},v1={hex}")
    }

    #[test]
    fn signature_passes_for_known_good_inputs() {
        let payload = br#"{"id":"evt_x","type":"customer.subscription.updated"}"#;
        let secret = "whsec_test_secret";
        let now = 1_700_000_000;
        let header = sign(payload, secret, now);
        verify_signature_at(payload, &header, secret, now).expect("must verify");
    }

    #[test]
    fn signature_rejects_tampered_payload() {
        let payload = br#"{"id":"evt_x"}"#;
        let secret = "whsec_test_secret";
        let now = 1_700_000_000;
        let header = sign(payload, secret, now);
        let tampered = br#"{"id":"evt_y"}"#;
        assert!(matches!(
            verify_signature_at(tampered, &header, secret, now),
            Err(StripeWebhookError::SignatureMismatch),
        ));
    }

    #[test]
    fn signature_rejects_wrong_secret() {
        let payload = br#"{"id":"evt_x"}"#;
        let now = 1_700_000_000;
        let header = sign(payload, "whsec_real", now);
        assert!(matches!(
            verify_signature_at(payload, &header, "whsec_imposter", now),
            Err(StripeWebhookError::SignatureMismatch),
        ));
    }

    #[test]
    fn signature_rejects_old_timestamp() {
        let payload = br#"{"id":"evt_x"}"#;
        let secret = "whsec_test_secret";
        let signed_at = 1_700_000_000;
        let header = sign(payload, secret, signed_at);
        let now = signed_at + SIGNATURE_TOLERANCE_SECS + 1;
        assert!(matches!(
            verify_signature_at(payload, &header, secret, now),
            Err(StripeWebhookError::TimestampOutOfTolerance),
        ));
    }

    #[test]
    fn signature_rejects_far_future_timestamp() {
        let payload = br#"{"id":"evt_x"}"#;
        let secret = "whsec_test_secret";
        let now = 1_700_000_000;
        // Stripe's signing wall clock somehow drifted way ahead — we
        // refuse it as a possible replay-with-stolen-key attempt.
        let header = sign(payload, secret, now + SIGNATURE_TOLERANCE_SECS + 60);
        assert!(matches!(
            verify_signature_at(payload, &header, secret, now),
            Err(StripeWebhookError::TimestampOutOfTolerance),
        ));
    }

    #[test]
    fn signature_accepts_multiple_v1_during_key_rotation() {
        let payload = br#"{"id":"evt_x"}"#;
        let secret_new = "whsec_new";
        let now = 1_700_000_000;
        let real_header = sign(payload, secret_new, now);
        // Stripe sends the old signature first during rotation;
        // accept if ANY v1 entry matches our current secret.
        let combined = format!(
            "t={now},v1=deadbeef0000000000000000000000000000000000000000000000000000beef,{}",
            real_header.split_once(',').unwrap().1
        );
        verify_signature_at(payload, &combined, secret_new, now).expect("must verify");
    }

    #[test]
    fn parse_signature_rejects_missing_t() {
        assert!(matches!(
            parse_signature_header("v1=abcdef"),
            Err(StripeWebhookError::MissingSignature),
        ));
    }

    #[test]
    fn parse_signature_rejects_missing_v1() {
        assert!(matches!(
            parse_signature_header("t=1700000000,v0=ignored"),
            Err(StripeWebhookError::MissingSignature),
        ));
    }

    #[test]
    fn parse_signature_rejects_garbage() {
        // No `=` anywhere → first split_once fails → MalformedSignature.
        assert!(matches!(
            parse_signature_header("no-equals-anywhere"),
            Err(StripeWebhookError::MalformedSignature),
        ));
        // A `t=…` chunk followed by an unparseable chunk should also
        // trip the malformed branch.
        assert!(matches!(
            parse_signature_header("t=1700000000,broken-chunk-no-equals"),
            Err(StripeWebhookError::MalformedSignature),
        ));
    }

    #[test]
    fn plan_id_extracts_from_first_item() {
        let obj = StripeEventObject {
            customer: Some("cus_x".into()),
            status: Some("active".into()),
            items: Some(StripeItemsContainer {
                data: vec![StripeItem {
                    price: Some(StripePrice {
                        lookup_key: Some("pro".into()),
                    }),
                }],
            }),
        };
        assert_eq!(plan_id_from_event(&obj).as_deref(), Some("pro"));
    }

    #[test]
    fn plan_id_none_for_canceled_subscription() {
        let obj = StripeEventObject {
            customer: Some("cus_x".into()),
            status: Some("canceled".into()),
            items: Some(StripeItemsContainer {
                data: vec![StripeItem {
                    price: Some(StripePrice {
                        lookup_key: Some("pro".into()),
                    }),
                }],
            }),
        };
        assert!(plan_id_from_event(&obj).is_none());
    }
}
