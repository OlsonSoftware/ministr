//! F3.1b-ii-a — concrete mail senders that implement
//! [`ministr_api::MailSender`].
//!
//! v0 ships a single impl: [`LogOnlyMailSender`]. It logs the
//! would-be send at info level WITHOUT the invite URL (the URL grants
//! org membership, so it must not land in operator logs). Two
//! downstream impls slot in behind the same trait once the operator
//! picks a provider:
//!
//! - `ResendMailSender` — Resend's HTTP API, ~$0/mo for the F3 launch
//!   volume. Likely the default for the first paid deploy.
//! - `SesMailSender` — AWS SES, for customers already on AWS.
//!
//! Either of those is a "new file + new env var + change one Arc in
//! `cmd_serve_http`" diff; everything else (handler, audit emission,
//! invite UX) stays as-is.

use ministr_api::{InviteMessage, MailSender};
use tracing::info;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn log_only_sender_implements_trait_via_arc() {
        // Compile-time check that LogOnlyMailSender satisfies the
        // dyn-shape used by OrgsState. A regression here breaks the
        // open-core seam at the cloud handler.
        let sender = Arc::new(LogOnlyMailSender::new());
        let dyn_sender: Arc<dyn MailSender> = Arc::clone(&sender) as _;
        // Smoke: the call doesn't panic / block.
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
        // Smoke: None on inviter_name shouldn't panic — the fallback
        // string is wired in the impl.
        sender.send_invite(InviteMessage {
            to_email: "test@example.com".into(),
            invite_url: "https://test/abc".into(),
            org_name: "Test Org".into(),
            inviter_name: None,
            role: "admin".into(),
        });
    }
}
