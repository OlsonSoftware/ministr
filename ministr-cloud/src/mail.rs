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

use ministr_api::{InviteMessage, MailSender};
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
}
