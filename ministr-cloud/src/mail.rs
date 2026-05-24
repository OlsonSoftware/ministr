//! Concrete mail senders implementing [`ministr_api::MailSender`].
//!
//! Two impls ship today:
//!
//! - [`LogOnlyMailSender`] — safe default that logs each would-be send
//!   and drops it. Used in self-hosted / dev / no-provider deployments.
//! - [`ResendMailSender`] — Resend HTTP API (`POST
//!   https://api.resend.com/emails`). ~$0/mo at F3 launch volume.
//!
//! Provider selection at boot: [`build_mail_sender_from_env`] reads
//! `MINISTR_MAIL_PROVIDER` + provider-specific env vars and returns
//! the matching `Arc<dyn MailSender>`. Falls back to
//! [`LogOnlyMailSender`] when nothing is configured.

use std::sync::Arc;

use ministr_api::{InviteMessage, MailSender, StaleKeyDigestMessage};
use tracing::{info, warn};

/// Safe-default mailer that logs each would-be send at info level
/// (without the URL) and drops the message on the floor. Used in:
///
/// - Self-hosted serve (no provider configured).
/// - Dev deployments before a provider lands.
/// - Cloud deployments where the operator deliberately wants to defer
///   email delivery (the invite URL is still in the HTTP response
///   body — the inviter can copy-paste it out-of-band).
///
/// **Never logs the invite URL.** The URL is a bearer credential: a
/// 256-bit token that grants org membership on click. Logging it
/// would let anyone with log access escalate.
#[derive(Debug, Clone, Default)]
pub struct LogOnlyMailSender;

impl LogOnlyMailSender {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl MailSender for LogOnlyMailSender {
    fn send_invite(&self, message: InviteMessage) {
        info!(
            recipient = %message.to_email,
            org = %message.org_name,
            role = %message.role,
            inviter = %message.inviter_name.as_deref().unwrap_or("(unknown)"),
            // URL deliberately NOT logged — it's a bearer credential.
            "mail:log-only — would send invite (no provider configured)",
        );
    }

    fn send_stale_key_digest(&self, message: StaleKeyDigestMessage) {
        info!(
            recipient = %message.to_email,
            stale_key_count = message.keys.len(),
            threshold_days = message.threshold_days,
            "mail:log-only — would send stale-key digest (no provider configured)",
        );
    }
}

// ---------------------------------------------------------------------------
// ResendMailSender
// ---------------------------------------------------------------------------

const RESEND_API_URL: &str = "https://api.resend.com/emails";

/// Resend HTTP API sender. Fire-and-forget: spawns a tokio task per
/// call so the handler hot path is never blocked. Failures log at
/// `warn` level but never propagate (the invite URL is in the HTTP
/// response body regardless).
///
/// **Never logs the invite URL.** Same security posture as
/// [`LogOnlyMailSender`].
pub struct ResendMailSender {
    client: reqwest::Client,
    api_key: String,
    from_address: String,
}

#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for ResendMailSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResendMailSender")
            .field("from_address", &self.from_address)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl ResendMailSender {
    #[must_use]
    pub fn new(api_key: impl Into<String>, from_address: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
            from_address: from_address.into(),
        }
    }

    fn render_html(msg: &InviteMessage) -> String {
        let inviter = msg
            .inviter_name
            .as_deref()
            .unwrap_or("An admin");
        format!(
            "<p>{inviter} has invited you to join <strong>{org}</strong> as <em>{role}</em>.</p>\
             <p><a href=\"{url}\">Accept invite</a></p>\
             <p style=\"color:#666;font-size:12px\">If you didn't expect this email, you can safely ignore it.</p>",
            org = html_escape(&msg.org_name),
            role = html_escape(&msg.role),
            url = html_escape(&msg.invite_url),
        )
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl ResendMailSender {
    fn render_digest_html(msg: &StaleKeyDigestMessage) -> String {
        use std::fmt::Write;
        let mut rows = String::new();
        for key in &msg.keys {
            let _ = write!(
                rows,
                "<tr><td style=\"padding:4px 8px\">{name}</td>\
                 <td style=\"padding:4px 8px;font-family:monospace\">{prefix}…</td></tr>",
                name = html_escape(&key.name),
                prefix = html_escape(&key.prefix),
            );
        }
        format!(
            "<p>You have <strong>{count}</strong> API key{s} that \
             {have} not been used in the last {days} days:</p>\
             <table border=\"1\" cellspacing=\"0\" style=\"border-collapse:collapse\">\
             <tr><th style=\"padding:4px 8px\">Name</th>\
             <th style=\"padding:4px 8px\">Prefix</th></tr>\
             {rows}\
             </table>\
             <p>Consider revoking keys you no longer need.</p>",
            count = msg.keys.len(),
            s = if msg.keys.len() == 1 { "" } else { "s" },
            have = if msg.keys.len() == 1 { "has" } else { "have" },
            days = msg.threshold_days,
        )
    }
}

impl MailSender for ResendMailSender {
    fn send_invite(&self, message: InviteMessage) {
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let from = self.from_address.clone();
        let to = message.to_email.clone();
        let subject = format!(
            "You've been invited to {} on ministr",
            message.org_name
        );
        let html = Self::render_html(&message);

        tokio::spawn(async move {
            let body = serde_json::json!({
                "from": from,
                "to": [to],
                "subject": subject,
                "html": html,
            });
            match client
                .post(RESEND_API_URL)
                .header("Authorization", format!("Bearer {api_key}"))
                .json(&body)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    info!(
                        recipient = %to,
                        "mail:resend — invite email sent"
                    );
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body_text = resp.text().await.unwrap_or_default();
                    warn!(
                        recipient = %to,
                        %status,
                        body = %body_text,
                        "mail:resend — API returned non-success"
                    );
                }
                Err(e) => {
                    warn!(
                        recipient = %to,
                        error = %e,
                        "mail:resend — failed to send invite email"
                    );
                }
            }
        });
    }

    fn send_stale_key_digest(&self, message: StaleKeyDigestMessage) {
        let client = self.client.clone();
        let api_key = self.api_key.clone();
        let from = self.from_address.clone();
        let to = message.to_email.clone();
        let count = message.keys.len();
        let subject = format!(
            "{count} stale API key{s} on ministr",
            s = if count == 1 { "" } else { "s" },
        );
        let html = Self::render_digest_html(&message);

        tokio::spawn(async move {
            let body = serde_json::json!({
                "from": from,
                "to": [to],
                "subject": subject,
                "html": html,
            });
            match client
                .post(RESEND_API_URL)
                .header("Authorization", format!("Bearer {api_key}"))
                .json(&body)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    info!(
                        recipient = %to,
                        stale_key_count = count,
                        "mail:resend — stale-key digest sent"
                    );
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body_text = resp.text().await.unwrap_or_default();
                    warn!(
                        recipient = %to,
                        %status,
                        body = %body_text,
                        "mail:resend — stale-key digest API returned non-success"
                    );
                }
                Err(e) => {
                    warn!(
                        recipient = %to,
                        error = %e,
                        "mail:resend — failed to send stale-key digest"
                    );
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

const DEFAULT_RESEND_FROM: &str = "ministr <noreply@ministr.ai>";

/// Build a mail sender from env vars. Provider dispatch:
///
/// - `MINISTR_MAIL_PROVIDER=resend` + `MINISTR_RESEND_API_KEY` →
///   [`ResendMailSender`].
/// - Anything else (or unset) → [`LogOnlyMailSender`].
///
/// Optional `MINISTR_MAIL_FROM` overrides the From address (default:
/// `ministr <noreply@ministr.ai>`).
pub fn build_mail_sender_from_env() -> Arc<dyn MailSender> {
    let provider = std::env::var("MINISTR_MAIL_PROVIDER")
        .unwrap_or_default()
        .to_lowercase();

    if provider == "resend" {
        if let Ok(api_key) = std::env::var("MINISTR_RESEND_API_KEY")
            && !api_key.is_empty()
        {
            let from = std::env::var("MINISTR_MAIL_FROM")
                .unwrap_or_else(|_| DEFAULT_RESEND_FROM.to_string());
            info!(
                provider = "resend",
                from = %from,
                "mail sender wired: ResendMailSender"
            );
            return Arc::new(ResendMailSender::new(api_key, from));
        }
        warn!(
            "MINISTR_MAIL_PROVIDER=resend but MINISTR_RESEND_API_KEY is missing or empty — falling back to LogOnlyMailSender"
        );
    } else if !provider.is_empty() {
        warn!(
            provider = %provider,
            "unknown MINISTR_MAIL_PROVIDER value — falling back to LogOnlyMailSender"
        );
    }

    info!("mail sender wired: LogOnlyMailSender (no provider configured)");
    Arc::new(LogOnlyMailSender::new())
}

// ---------------------------------------------------------------------------
// Bounce tracking
// ---------------------------------------------------------------------------

type Pool = deadpool_postgres::Pool;

/// Record a bounced email address so future sends can warn.
///
/// # Errors
///
/// Database errors (pool / SQL).
pub async fn record_bounce(pool: &Pool, email: &str, reason: &str) -> Result<(), String> {
    let conn = pool.get().await.map_err(|e| format!("pool: {e}"))?;
    conn.execute(
        "INSERT INTO bounced_emails (email, reason)
         VALUES ($1, $2)
         ON CONFLICT (email) DO UPDATE SET bounced_at = now(), reason = $2",
        &[&email, &reason],
    )
    .await
    .map_err(|e| format!("insert bounce: {e}"))?;
    Ok(())
}

/// Check whether an email address has previously bounced.
///
/// # Errors
///
/// Database errors (pool / SQL).
pub async fn is_bounced(pool: &Pool, email: &str) -> Result<bool, String> {
    let conn = pool.get().await.map_err(|e| format!("pool: {e}"))?;
    let row = conn
        .query_opt(
            "SELECT 1 FROM bounced_emails WHERE email = $1",
            &[&email],
        )
        .await
        .map_err(|e| format!("check bounce: {e}"))?;
    Ok(row.is_some())
}

// ---------------------------------------------------------------------------
// Resend webhook handler
// ---------------------------------------------------------------------------

/// Svix-style signature verification for Resend webhooks.
///
/// Resend uses the Standard Webhooks / Svix signature scheme:
/// - Headers: `svix-id`, `svix-timestamp`, `svix-signature`
/// - Signed content: `{svix-id}.{svix-timestamp}.{body}`
/// - Signature: `v1,{base64(HMAC-SHA256(secret, signed_content))}`
/// - Secret is base64-encoded with a `whsec_` prefix.
fn verify_svix_signature(
    svix_id: &str,
    svix_timestamp: &str,
    body: &[u8],
    svix_signature: &str,
    secret: &str,
) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let key_b64 = secret.strip_prefix("whsec_").unwrap_or(secret);
    let Ok(key_bytes) = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        key_b64,
    ) else {
        return false;
    };

    let mut signed_content = Vec::with_capacity(svix_id.len() + 1 + svix_timestamp.len() + 1 + body.len());
    signed_content.extend_from_slice(svix_id.as_bytes());
    signed_content.push(b'.');
    signed_content.extend_from_slice(svix_timestamp.as_bytes());
    signed_content.push(b'.');
    signed_content.extend_from_slice(body);

    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(&key_bytes) else {
        return false;
    };
    mac.update(&signed_content);
    let expected = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        mac.finalize().into_bytes(),
    );
    let expected_tagged = format!("v1,{expected}");

    for sig in svix_signature.split(' ') {
        if sig == expected_tagged {
            return true;
        }
    }
    false
}

#[derive(Clone)]
pub struct ResendWebhookState {
    pool: std::sync::Arc<Pool>,
    secret: String,
}

impl ResendWebhookState {
    #[must_use]
    pub fn new(pool: std::sync::Arc<Pool>, secret: impl Into<String>) -> Self {
        Self {
            pool,
            secret: secret.into(),
        }
    }
}

pub fn resend_webhook_routes(
    state: ResendWebhookState,
) -> axum::Router {
    use axum::routing::post;
    axum::Router::new()
        .route("/webhooks/resend", post(handle_resend_webhook))
        .with_state(state)
}

async fn handle_resend_webhook(
    axum::extract::State(state): axum::extract::State<ResendWebhookState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    let svix_id = headers
        .get("svix-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let svix_timestamp = headers
        .get("svix-timestamp")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let svix_signature = headers
        .get("svix-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if svix_id.is_empty() || svix_timestamp.is_empty() || svix_signature.is_empty() {
        return (StatusCode::BAD_REQUEST, "missing svix headers").into_response();
    }

    if !verify_svix_signature(svix_id, svix_timestamp, &body, svix_signature, &state.secret) {
        return (StatusCode::UNAUTHORIZED, "invalid signature").into_response();
    }

    let payload: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return (StatusCode::BAD_REQUEST, "invalid JSON").into_response(),
    };

    let event_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
    if event_type == "email.bounced"
        && let Some(email) = payload
            .get("data")
            .and_then(|d| d.get("to"))
            .and_then(|t| t.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.as_str())
    {
        let reason = payload
            .get("data")
            .and_then(|d| d.get("bounce"))
            .and_then(|b| b.get("message"))
            .and_then(|v| v.as_str())
            .unwrap_or("bounce");
        match record_bounce(&state.pool, email, reason).await {
            Ok(()) => {
                info!(
                    email = %email,
                    reason = %reason,
                    "resend webhook: bounce recorded"
                );
            }
            Err(e) => {
                warn!(
                    email = %email,
                    error = %e,
                    "resend webhook: failed to record bounce"
                );
            }
        }
    }

    (StatusCode::OK, "ok").into_response()
}

// ---------------------------------------------------------------------------
// Tests — svix signature verification
// ---------------------------------------------------------------------------

#[cfg(test)]
mod svix_tests {
    use super::*;

    fn test_secret() -> String {
        use base64::Engine;
        let key = b"test-signing-key-32-bytes-long!!";
        format!(
            "whsec_{}",
            base64::engine::general_purpose::STANDARD.encode(key)
        )
    }

    fn sign(id: &str, ts: &str, body: &[u8], secret: &str) -> String {
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let key_b64 = secret.strip_prefix("whsec_").unwrap();
        let key_bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, key_b64).unwrap();
        let mut content = Vec::new();
        content.extend_from_slice(id.as_bytes());
        content.push(b'.');
        content.extend_from_slice(ts.as_bytes());
        content.push(b'.');
        content.extend_from_slice(body);
        let mut mac = Hmac::<Sha256>::new_from_slice(&key_bytes).unwrap();
        mac.update(&content);
        let sig = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            mac.finalize().into_bytes(),
        );
        format!("v1,{sig}")
    }

    #[test]
    fn verify_valid_signature() {
        let secret = test_secret();
        let body = b"{\"type\":\"email.bounced\"}";
        let sig = sign("msg_123", "1700000000", body, &secret);
        assert!(verify_svix_signature("msg_123", "1700000000", body, &sig, &secret));
    }

    #[test]
    fn reject_wrong_secret() {
        let secret = test_secret();
        let body = b"{\"type\":\"email.bounced\"}";
        let sig = sign("msg_123", "1700000000", body, &secret);
        assert!(!verify_svix_signature(
            "msg_123",
            "1700000000",
            body,
            &sig,
            "whsec_d3JvbmctLWtleS0tLW5vdC10aGUtcmlnaHQtb25l"
        ));
    }

    #[test]
    fn reject_tampered_body() {
        let secret = test_secret();
        let body = b"{\"type\":\"email.bounced\"}";
        let sig = sign("msg_123", "1700000000", body, &secret);
        assert!(!verify_svix_signature(
            "msg_123",
            "1700000000",
            b"{\"type\":\"email.delivered\"}",
            &sig,
            &secret
        ));
    }

    #[test]
    fn accept_multi_signature_header() {
        let secret = test_secret();
        let body = b"{}";
        let real = sign("msg_x", "123", body, &secret);
        let multi = format!("v1,fakesig {real}");
        assert!(verify_svix_signature("msg_x", "123", body, &multi, &secret));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_only_sender_implements_trait_via_arc() {
        let sender = Arc::new(LogOnlyMailSender::new());
        let dyn_sender: Arc<dyn MailSender> = Arc::clone(&sender) as _;
        dyn_sender.send_invite(InviteMessage {
            to_email: "test@example.com".into(),
            invite_url: "https://test/abc".into(),
            org_name: "Test Org".into(),
            inviter_name: Some("Alice".into()),
            role: "member".into(),
        });
    }

    #[test]
    fn log_only_sender_handles_missing_inviter_name() {
        let sender = LogOnlyMailSender::new();
        sender.send_invite(InviteMessage {
            to_email: "test@example.com".into(),
            invite_url: "https://test/abc".into(),
            org_name: "Test Org".into(),
            inviter_name: None,
            role: "admin".into(),
        });
    }

    #[test]
    fn resend_sender_implements_trait_via_arc() {
        let sender = Arc::new(ResendMailSender::new("re_test_key", "noreply@test.com"));
        let dyn_sender: Arc<dyn MailSender> = Arc::clone(&sender) as _;
        let _ = dyn_sender;
    }

    #[test]
    fn resend_debug_does_not_leak_api_key() {
        let sender = ResendMailSender::new("re_secret_key_abc123", "noreply@test.com");
        let debug = format!("{sender:?}");
        assert!(
            !debug.contains("re_secret_key_abc123"),
            "Debug output must not contain the API key"
        );
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn render_html_escapes_org_name() {
        let msg = InviteMessage {
            to_email: "a@b.com".into(),
            invite_url: "https://test/abc".into(),
            org_name: "<script>alert(1)</script>".into(),
            inviter_name: Some("Alice".into()),
            role: "member".into(),
        };
        let html = ResendMailSender::render_html(&msg);
        assert!(!html.contains("<script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn render_html_falls_back_to_an_admin_when_no_inviter() {
        let msg = InviteMessage {
            to_email: "a@b.com".into(),
            invite_url: "https://test/abc".into(),
            org_name: "Acme".into(),
            inviter_name: None,
            role: "admin".into(),
        };
        let html = ResendMailSender::render_html(&msg);
        assert!(html.contains("An admin"));
    }

    #[test]
    fn html_escape_handles_all_metacharacters() {
        assert_eq!(html_escape("a&b<c>d\"e"), "a&amp;b&lt;c&gt;d&quot;e");
    }

    #[test]
    fn html_escape_passthrough_on_clean_string() {
        assert_eq!(html_escape("hello world"), "hello world");
    }

    #[test]
    fn render_digest_html_contains_key_count_and_names() {
        let msg = StaleKeyDigestMessage {
            to_email: "alice@example.com".into(),
            keys: vec![
                ministr_api::StaleKeyEntry {
                    name: "CI runner".into(),
                    prefix: "mst_pk_A".into(),
                },
                ministr_api::StaleKeyEntry {
                    name: "staging".into(),
                    prefix: "mst_pk_B".into(),
                },
            ],
            threshold_days: 90,
        };
        let html = ResendMailSender::render_digest_html(&msg);
        assert!(html.contains("<strong>2</strong>"));
        assert!(html.contains("CI runner"));
        assert!(html.contains("mst_pk_B"));
        assert!(html.contains("90 days"));
    }

    #[test]
    fn render_digest_html_singular_for_one_key() {
        let msg = StaleKeyDigestMessage {
            to_email: "a@b.com".into(),
            keys: vec![ministr_api::StaleKeyEntry {
                name: "lone".into(),
                prefix: "mst_pk_X".into(),
            }],
            threshold_days: 30,
        };
        let html = ResendMailSender::render_digest_html(&msg);
        assert!(html.contains("1</strong> API key that has"));
    }

    #[test]
    fn render_digest_html_escapes_key_name() {
        let msg = StaleKeyDigestMessage {
            to_email: "a@b.com".into(),
            keys: vec![ministr_api::StaleKeyEntry {
                name: "<b>evil</b>".into(),
                prefix: "mst_pk_Z".into(),
            }],
            threshold_days: 90,
        };
        let html = ResendMailSender::render_digest_html(&msg);
        assert!(!html.contains("<b>evil</b>"));
        assert!(html.contains("&lt;b&gt;evil&lt;/b&gt;"));
    }

    #[test]
    fn log_only_sender_send_digest_does_not_panic() {
        let sender = LogOnlyMailSender::new();
        sender.send_stale_key_digest(StaleKeyDigestMessage {
            to_email: "a@b.com".into(),
            keys: vec![],
            threshold_days: 90,
        });
    }
}
