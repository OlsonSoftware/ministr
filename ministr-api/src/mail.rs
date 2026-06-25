//! Outbound mail-delivery hook.
//!
//! `MailSender` is the open-core seam for sending
//! transactional email (invite magic-links today; weekly
//! stale-key digests in a follow-up). The trait lives here so
//! `ministr-cloud`'s handlers can hold an `Arc<dyn MailSender>`
//! without depending on a specific provider crate.
//!
//! Concrete implementations slot in behind this trait:
//!
//! - [`LogOnlyMailSender`] (in `ministr-cloud`) — logs the would-be
//!   send at info level. Default for dev + self-hosted serve, and the
//!   shipping safety net when no provider env vars are configured.
//! - A `ResendMailSender` / `SesMailSender` lands once
//!   the operator picks a provider; threading a different concrete
//!   into [`OrgsState::with_mailer`] is the entire wiring change.
//!
//! # Shape parallels [`AuditSink`]
//!
//! Fire-and-forget posture: the trait method spawns its own work and
//! returns immediately. A delivery failure logs inside the impl but
//! never propagates back to the handler — the invite URL is in the
//! HTTP response body regardless, so the user can copy-paste it if
//! email delivery is broken.
//!
//! [`AuditSink`]: crate::audit::AuditSink
//! [`OrgsState::with_mailer`]: ../../../ministr-cloud/src/orgs/routes.rs
//! [`LogOnlyMailSender`]: ../../../ministr-cloud/src/mail.rs

use serde::{Deserialize, Serialize};

/// One transactional-mail payload, ready for a provider to deliver.
///
/// v0 ships a single message kind: invite magic-links. will
/// extend `MailMessage` with a `kind` discriminant once the digest
/// payload lands. Keeping the shape concrete (named fields, not
/// `kind: MessageKind` enum) for now means the trait is testable
/// against literal values; the discriminant comes cheap later.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteMessage {
    /// Recipient address. The handler validates `contains('@')` at
    /// API boundary; the sender treats this as opaque.
    pub to_email: String,
    /// Absolute magic-link URL the recipient clicks. Provider impls
    /// should NOT log this at any level — it grants org membership.
    pub invite_url: String,
    /// Human-readable org name for the subject line / body.
    pub org_name: String,
    /// Display name of the inviter, for the body. `None` falls back
    /// to "An admin" in the message template.
    pub inviter_name: Option<String>,
    /// Role the invite mints (e.g. `"member"`, `"admin"`). Surfaced
    /// in the body so the recipient knows what they're accepting.
    pub role: String,
}

/// Weekly stale-key digest payload. The cron groups stale keys by
/// owner, looks up the owner's email, and fires one digest per owner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaleKeyDigestMessage {
    pub to_email: String,
    pub keys: Vec<StaleKeyEntry>,
    pub threshold_days: u32,
}

/// One stale key inside a [`StaleKeyDigestMessage`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StaleKeyEntry {
    pub name: String,
    pub prefix: String,
}

/// Fire-and-forget sink for outbound mail.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn MailSender>` inside cloud-side state. The method is
/// intentionally sync — the implementation spawns its own async task
/// (provider HTTP call typically) so the handler hot path is never
/// blocked. A delivery failure logs inside the impl but never
/// propagates to the handler; the invite URL is always in the
/// response body so the operator has a manual-paste fallback.
pub trait MailSender: Send + Sync + std::fmt::Debug {
    /// Send a magic-link invite. Returns immediately; the actual
    /// delivery completes asynchronously.
    fn send_invite(&self, message: InviteMessage);

    /// Send a weekly stale-key digest. Default impl is a no-op so
    /// existing implementations don't break.
    fn send_stale_key_digest(&self, _message: StaleKeyDigestMessage) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct StubSender {
        captured: Mutex<Vec<InviteMessage>>,
    }
    impl MailSender for StubSender {
        fn send_invite(&self, message: InviteMessage) {
            self.captured.lock().unwrap().push(message);
        }
    }

    #[test]
    fn invite_message_carries_required_fields() {
        let m = InviteMessage {
            to_email: "bob@example.com".into(),
            invite_url: "https://mcp.ministr.ai/auth/github/start?invite=abc".into(),
            org_name: "Acme".into(),
            inviter_name: Some("Alice".into()),
            role: "member".into(),
        };
        assert_eq!(m.to_email, "bob@example.com");
        assert!(m.invite_url.contains("invite=abc"));
        assert_eq!(m.role, "member");
    }

    #[test]
    fn dyn_dispatch_captures_through_arc() {
        let stub = Arc::new(StubSender::default());
        let sender: Arc<dyn MailSender> = Arc::clone(&stub) as _;
        sender.send_invite(InviteMessage {
            to_email: "carol@example.com".into(),
            invite_url: "https://mcp.ministr.ai/auth/github/start?invite=xyz".into(),
            org_name: "Acme".into(),
            inviter_name: None,
            role: "admin".into(),
        });
        let captured = stub.captured.lock().unwrap();
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].to_email, "carol@example.com");
        assert_eq!(captured[0].role, "admin");
    }
}
