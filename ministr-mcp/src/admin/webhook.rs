//! GitHub push-event webhook with HMAC-SHA256 signature verification.
//!
//! Disabled when `AdminState::webhook_secret` is `None` — the route still
//! exists but unconditionally returns 404 to avoid leaking that the
//! endpoint is configurable.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use hmac::{Hmac, Mac};
use serde::Serialize;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tracing::{debug, warn};

use super::AdminState;
use super::jobs::JobTrigger;

const SIGNATURE_HEADER: &str = "x-hub-signature-256";

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Serialize)]
pub(super) struct WebhookResponse {
    job_id: String,
}

/// `POST /webhook/github` — enqueue an indexer job if the HMAC signature
/// matches the configured secret.
pub(super) async fn github_webhook(
    State(state): State<AdminState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<WebhookResponse>, StatusCode> {
    let Some(secret) = state.webhook_secret() else {
        debug!("github webhook hit but no secret configured");
        return Err(StatusCode::NOT_FOUND);
    };

    let Some(signature) = headers
        .get(SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("sha256="))
    else {
        debug!("missing or malformed signature header");
        return Err(StatusCode::UNAUTHORIZED);
    };

    if !verify(secret, &body, signature) {
        warn!("github webhook signature mismatch");
        return Err(StatusCode::UNAUTHORIZED);
    }

    let payload = parse_push_payload(&body).ok_or(StatusCode::BAD_REQUEST)?;

    let job = state
        .queue
        .enqueue(
            payload.repository.full_name,
            JobTrigger::Github {
                reference: payload.r#ref,
                commit: payload.after,
            },
        )
        .await
        .map_err(|e| {
            warn!(error = %e, "failed to enqueue webhook-triggered job");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(WebhookResponse { job_id: job.id }))
}

fn verify(secret: &str, body: &[u8], hex_signature: &str) -> bool {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let computed = mac.finalize().into_bytes();
    let Ok(provided) = decode_hex(hex_signature) else {
        return false;
    };
    computed.as_slice().ct_eq(&provided).into()
}

fn decode_hex(s: &str) -> Result<Vec<u8>, ()> {
    if !s.len().is_multiple_of(2) {
        return Err(());
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(2) {
        let hi = hex_value(chunk[0])?;
        let lo = hex_value(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_value(b: u8) -> Result<u8, ()> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(()),
    }
}

#[derive(Debug, serde::Deserialize)]
struct PushPayload {
    r#ref: String,
    after: String,
    repository: Repository,
}

#[derive(Debug, serde::Deserialize)]
struct Repository {
    full_name: String,
}

fn parse_push_payload(body: &[u8]) -> Option<PushPayload> {
    serde_json::from_slice(body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, body: &[u8]) -> String {
        use std::fmt::Write as _;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let bytes = mac.finalize().into_bytes();
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            let _ = write!(s, "{b:02x}");
        }
        s
    }

    #[test]
    fn verify_accepts_correct_signature() {
        let secret = "it's-a-secret";
        let body = br#"{"hello":"world"}"#;
        let sig = sign(secret, body);
        assert!(verify(secret, body, &sig));
    }

    #[test]
    fn verify_rejects_wrong_signature() {
        let secret = "it's-a-secret";
        let body = br#"{"hello":"world"}"#;
        let bad = "0".repeat(64);
        assert!(!verify(secret, body, &bad));
    }

    #[test]
    fn verify_rejects_tampered_body() {
        let secret = "it's-a-secret";
        let sig = sign(secret, br#"{"hello":"world"}"#);
        assert!(!verify(secret, br#"{"hello":"worlD"}"#, &sig));
    }

    #[test]
    fn parses_valid_push_payload() {
        let body = br#"{"ref":"refs/heads/main","after":"abc","repository":{"full_name":"alrik/ministr"}}"#;
        let p = parse_push_payload(body).unwrap();
        assert_eq!(p.r#ref, "refs/heads/main");
        assert_eq!(p.after, "abc");
        assert_eq!(p.repository.full_name, "alrik/ministr");
    }

    #[test]
    fn decode_hex_rejects_odd_length() {
        assert!(decode_hex("abc").is_err());
    }

    #[test]
    fn decode_hex_round_trips() {
        let s = "deadbeef";
        let decoded = decode_hex(s).unwrap();
        assert_eq!(decoded, vec![0xde, 0xad, 0xbe, 0xef]);
    }
}
