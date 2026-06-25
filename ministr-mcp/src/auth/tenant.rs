//! Resolved tenant identity attached to every authenticated request.
//!
//! Handlers read [`Tenant`] from axum's `Extension<Tenant>` after the
//! token-validation middleware ([`super::middleware`]) succeeds. The
//! struct shape: `{ subject, org_id, plan }`.
//!
//! # The open-core seam
//!
//! `Plan` lives here (MIT) rather than in `ministr-cloud` (closed)
//! because [`Tenant`] embeds it and is read by handlers that ship in the
//! MIT-licensed local stack. The cloud crate re-exports `Plan` and
//! adds its own free-standing business logic (`queue_priority`,
//! quota caps) without forcing the local stack to depend on a closed
//! crate.
//!
//! # Resolution lane
//!
//! Self-hosted single-user MCP serve returns [`Tenant::local`] from a
//! token's `client_id` — Pro tier, no org. Cloud deployments replace
//! this with a DB lookup against `users.plan_id` and `org_members` so
//! handlers see the real tenant.

use serde::{Deserialize, Serialize};

/// Billing tier resolved for the requesting tenant.
///
/// Mirrors the §3 pricing matrix exactly: variants serialise to their
/// lowercase public names so logs and API responses are
/// self-documenting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Plan {
    /// Pro — $20/mo. The local stack and self-hosted serve also resolve
    /// here so handlers downstream of the middleware can assume a
    /// non-`None` plan.
    #[default]
    Pro,
    /// Team — $30/seat/mo.
    Team,
    /// Enterprise — contact sales.
    Enterprise,
}

/// Indexing-queue priority for a tier. Higher wins. The pool drains in
/// `ORDER BY priority DESC, enqueued_at ASC`. Customer-driven enqueues
/// stamp this value so Team jumps Pro and Enterprise jumps both.
///
/// Enterprise sits at `4` (not `3`) to leave a dedicated lane in the
/// shared pool ahead of all multi-tenant traffic. `3` is reserved for
/// future per-org boosts (e.g. paid SLA add-on for a Team customer).
///
/// Operator-driven enqueues (GitHub webhooks, Atlas weekly re-index)
/// pass `0` directly — they're not customer-facing and shouldn't jump
/// any paying tier.
///
/// Lives here (MIT) — not on `Plan` itself — so the open-core handler
/// surface can derive priority without depending on `ministr-cloud`.
/// `ministr-cloud` re-exports this verbatim under its own name to keep
/// the existing cloud-side call sites compiling.
#[must_use]
pub const fn queue_priority(plan: Plan) -> i16 {
    match plan {
        Plan::Pro => 1,
        Plan::Team => 2,
        Plan::Enterprise => 4,
    }
}

/// Resolved tenant identity attached to every authenticated request.
///
/// Handlers read this through `axum::Extension<Tenant>`. The
/// token-validation middleware populates it on success; unauthenticated
/// requests never reach handlers and therefore see no `Tenant` in
/// extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    /// Token subject — the OAuth `client_id` today. Future SAML/OIDC
    /// adapters substitute the issuer's `NameID` / `sub` claim.
    pub subject: String,
    /// Organisation membership. `None` for self-hosted, personal-Pro
    /// users, and any request the resolver could not link to an org.
    pub org_id: Option<String>,
    /// Resolved billing tier.
    pub plan: Plan,
}

impl Tenant {
    /// Self-hosted / single-user default: Pro tier, no org, subject =
    /// the token's `client_id`. Cloud-side resolvers replace this with
    /// a DB lookup.
    #[must_use]
    pub fn local(subject: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            org_id: None,
            plan: Plan::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_default_is_pro() {
        assert_eq!(Plan::default(), Plan::Pro);
    }

    #[test]
    fn queue_priority_matches_tier_lanes() {
        // Operator/unscoped enqueues sit at 0; paying tiers climb in
        // order, with Enterprise at `priority=4` (gap at 3 reserved for
        // future per-org boosts on top of a Team subscription).
        assert_eq!(queue_priority(Plan::Pro), 1);
        assert_eq!(queue_priority(Plan::Team), 2);
        assert_eq!(queue_priority(Plan::Enterprise), 4);
    }

    #[test]
    fn plan_serialises_lowercase() {
        assert_eq!(serde_json::to_string(&Plan::Pro).unwrap(), "\"pro\"");
        assert_eq!(serde_json::to_string(&Plan::Team).unwrap(), "\"team\"");
        assert_eq!(
            serde_json::to_string(&Plan::Enterprise).unwrap(),
            "\"enterprise\""
        );
    }

    #[test]
    fn tenant_local_round_trips() {
        let t = Tenant::local("client-42");
        let s = serde_json::to_string(&t).unwrap();
        let back: Tenant = serde_json::from_str(&s).unwrap();
        assert_eq!(back.subject, "client-42");
        assert!(back.org_id.is_none());
        assert_eq!(back.plan, Plan::Pro);
    }
}
