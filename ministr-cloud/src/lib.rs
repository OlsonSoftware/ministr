//! ministr-cloud — proprietary multi-tenant cloud surface for ministr.
//!
//! This crate is the home of every piece of code that only exists because
//! ministr is run as a managed service at `mcp.ministr.ai`. The local
//! open-core stack (`ministr-core`, `ministr-api`, `ministr-daemon`,
//! `ministr-mcp`, `ministr-cli`, `ministr-app/src-tauri`) is MIT-licensed
//! and works without anything in this crate.
//!
//! See `STEWARDSHIP.md` at the repo root for the open-core split, and
//! `ROADMAP.md` §2 for the principles that decide what lands here vs.
//! upstream in the MIT crates.
//!
//! # Current contents
//!
//! The crate is intentionally minimal until the rest of F1.1 lands. The
//! [`Plan`] enum is the seam every downstream cloud feature (quota,
//! billing, Atlas access) reads through; it ships now so the closed-source
//! marker has a real type to host.
//!
//! Future phases extend the crate as follows:
//!
//! | Phase | Modules added |
//! |---|---|
//! | F1.2 | `tenant::Tenant`, Postgres-backed schema migrations |
//! | F1.3 | `idp::IdentityProvider` trait + GitHub/Google/Microsoft impls |
//! | F1.4 | `billing::usage` daily rollup + `/api/v1/billing/usage` |
//! | F1.5 | `billing::stripe` Stripe Meters + webhook receiver |
//! | F2.1 | `github::app` installation-token minter |
//! | F2.3 | `quota` plan-aware tower middleware |
//! | F2.4 | `billing::checkout` Stripe Checkout sessions |
//! | F3.1 | `orgs` org CRUD + magic-link invites |
//! | F3.2 | `acl` corpus ACL middleware |
//! | F3.4 | `api_keys` service-account keys |
//! | F3.5 | `webhooks::outbound` Slack/Discord/HMAC delivery |
//! | F6.2 | `sessions::export` session bundle ZIP |

#![deny(unsafe_code)]

pub mod blob;
pub mod db;

pub use blob::{BlobError, BlobResult, CorpusBlobStore};
pub use db::{connect, run_migrations, DbError};

use serde::{Deserialize, Serialize};

/// Billing tier attached to every resolved tenant.
///
/// The seam that makes the §3 tier matrix enforceable in code. Every
/// quota check, every Atlas access gate, and every billing-portal handler
/// reads through `Plan` rather than re-deriving "what can this tenant
/// do?" from raw subscription state.
///
/// Variants intentionally mirror the public tier names verbatim so logs
/// and API responses are self-documenting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Plan {
    /// Pro — $20/mo. Hosted fast lane, 10 corpora, Atlas reads.
    Pro,
    /// Team — $30/seat/mo. Priority queue, 50 corpora, ACL, dashboard,
    /// audit-light, bridge visualizer.
    Team,
    /// Enterprise — contact sales. Dedicated pool, SSO/SAML, immutable
    /// audit, on-prem option, CMK.
    Enterprise,
}

impl Plan {
    /// Indexing-queue priority. Higher wins. The pool drains in
    /// `ORDER BY priority DESC, enqueued_at ASC`. F2.2 wires this into
    /// `JobQueue::enqueue`; F5.5 reserves `priority = 4` for the
    /// dedicated Enterprise pool.
    #[must_use]
    pub const fn queue_priority(self) -> i16 {
        match self {
            Self::Pro => 1,
            Self::Team => 2,
            Self::Enterprise => 3,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_order_pro_team_enterprise() {
        assert!(Plan::Enterprise.queue_priority() > Plan::Team.queue_priority());
        assert!(Plan::Team.queue_priority() > Plan::Pro.queue_priority());
    }

    #[test]
    fn plan_serialises_lowercase() {
        let s = serde_json::to_string(&Plan::Pro).unwrap();
        assert_eq!(s, "\"pro\"");
    }
}
