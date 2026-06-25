//! Audit-log emission hook.
//!
//! service-account API keys, org invites, ACL shares, and
//! org create all emit one `audit_events` row per write. The trait
//! lives here (MIT) so the open-core handlers in `ministr-cloud` can
//! call into a `dyn`-typed sink; the concrete `PostgresAuditSink`
//! lives in `ministr-cloud`. Self-hosted serve never wires a sink so
//! audit emission is compiled out.
//!
//! # Shape parallels [`UsageSink`]
//!
//! Same fire-and-forget posture: the trait method takes the entry by
//! value and returns nothing. Implementations spawn their own
//! `tokio::spawn` so the handler hot path is never blocked by audit
//! writes. A storage hiccup logs but never fails the user's request.
//!
//! [`UsageSink`]: crate::usage::UsageSink

use serde::{Deserialize, Serialize};

/// One audit entry, ready for insertion into `audit_events`.
///
/// `org_id` and `actor` are optional because user-level actions
/// (`api_key.created` on a personal account, `org.created`) have no
/// existing org id yet — for the create case, `resource` carries the
/// new org's id and `org_id` mirrors it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Canonical wire-format action name. Examples:
    /// `"api_key.created"`, `"api_key.revoked"`, `"share.granted"`,
    /// `"share.revoked"`, `"org.created"`, `"invite.created"`,
    /// `"member.added"`.
    pub action: String,
    /// Stable identifier of the affected resource. Most actions store
    /// a single UUID string; multi-resource actions (e.g.
    /// `share.granted`) join the participating ids with `:` so the
    /// list endpoint can render them without parsing.
    pub resource: String,
    /// The org this action affects, when applicable. `None` for
    /// user-level actions that pre-date or sidestep an org context.
    pub org_id: Option<String>,
    /// The user whose token authenticated the request (UUID string).
    /// `None` for actions taken by a service-account API key (the
    /// `api_keys` row carries its own owner; the actor here is the
    /// human who minted the key, but at audit time we only know the
    /// resolved tenant subject which equals the key's owner — so
    /// effectively the actor IS recorded; left optional for the rare
    /// case where the auth layer couldn't resolve a UUID).
    pub actor: Option<String>,
    /// Source IP, if the handler has it. Optional because some
    /// handlers run server-side without a client request context
    /// (none yet, but reserved for future cron-driven actions).
    pub ip: Option<String>,
    /// Client user-agent string, if present. Truncated to a sane
    /// length by the sink implementation.
    pub user_agent: Option<String>,
}

impl AuditEntry {
    /// Build an entry with required fields; everything else nullable.
    #[must_use]
    pub fn new(action: impl Into<String>, resource: impl Into<String>) -> Self {
        Self {
            action: action.into(),
            resource: resource.into(),
            org_id: None,
            actor: None,
            ip: None,
            user_agent: None,
        }
    }

    /// Builder: set the org id this action affects.
    #[must_use]
    pub fn with_org(mut self, org_id: impl Into<String>) -> Self {
        self.org_id = Some(org_id.into());
        self
    }

    /// Builder: set the actor (resolved tenant subject UUID).
    #[must_use]
    pub fn with_actor(mut self, actor: impl Into<String>) -> Self {
        self.actor = Some(actor.into());
        self
    }
}

/// Fire-and-forget sink for [`AuditEntry`] writes.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn AuditSink>` inside cloud-side state (`OrgsState`,
/// `ApiKeysState`). The method is intentionally sync — the
/// implementation spawns its own async task. A storage hiccup logs
/// inside the impl but never propagates to the handler.
pub trait AuditSink: Send + Sync + std::fmt::Debug {
    /// Record an [`AuditEntry`]. Returns immediately; the actual
    /// write completes asynchronously.
    fn record(&self, entry: AuditEntry);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct StubSink {
        captured: Mutex<Vec<AuditEntry>>,
    }
    impl AuditSink for StubSink {
        fn record(&self, entry: AuditEntry) {
            self.captured.lock().unwrap().push(entry);
        }
    }

    #[test]
    fn audit_entry_builder_chains() {
        let e = AuditEntry::new("api_key.created", "key-uuid")
            .with_org("org-uuid")
            .with_actor("user-uuid");
        assert_eq!(e.action, "api_key.created");
        assert_eq!(e.resource, "key-uuid");
        assert_eq!(e.org_id.as_deref(), Some("org-uuid"));
        assert_eq!(e.actor.as_deref(), Some("user-uuid"));
    }

    #[test]
    fn dyn_dispatch_captures_through_arc() {
        // Keep the concrete handle to inspect; pass an Arc<dyn> into a
        // function that records through the trait surface to verify
        // dyn dispatch works.
        let stub = Arc::new(StubSink::default());
        let sink: Arc<dyn AuditSink> = Arc::clone(&stub) as _;
        sink.record(AuditEntry::new("share.granted", "corpus:org"));
        sink.record(AuditEntry::new("share.revoked", "corpus:org"));
        assert_eq!(stub.captured.lock().unwrap().len(), 2);
    }
}
