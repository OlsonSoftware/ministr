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
//! | F1.2 | `db` (Postgres-backed schema migrations); `Tenant` itself lives in `ministr-mcp::auth::tenant` (MIT) so handlers in the local stack can read it without depending on this closed crate |
//! | F1.3 | `idp::IdentityProvider` trait (landed); GitHub/Google/Microsoft impls plug in via the same trait |
//! | F1.4 | `billing::usage` write path (landed); daily rollup + `/api/v1/billing/usage` plug in alongside |
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

pub mod auth;
pub mod billing;
pub mod blob;
pub mod db;
pub mod github;
pub mod idp;
pub mod ratelimit;
pub mod users;

pub use billing::{
    billing_routes, record_usage, rollup_day, stripe_webhook_routes, BillingState, PartialRow,
    PostgresUsageSink, RollupRow, StripeApiError, StripeClient, StripeWebhookError,
    StripeWebhookState, UsageEventKind, UsageResponse,
};
pub use auth::{
    github_signin_routes, GitHubSigninError, GitHubSigninState, DEFAULT_SIGNIN_SCOPE,
};
pub use blob::{BlobError, BlobResult, CorpusBlobStore};
pub use db::{connect, run_migrations, DbError};
pub use github::{GitHubAppClient, GitHubAppError};
pub use idp::{GitHubIdp, IdentityProvider, IdpError, ResolvedIdentity, GITHUB_ISSUER};
pub use ratelimit::{
    ip_key, rate_limit_middleware, tenant_key, InMemoryBucket, RateLimitConfig,
    RateLimitDecision, TokenBucket,
};
pub use users::{
    set_stripe_customer_id, upsert_github_user, UserError, UserRow, DEFAULT_GITHUB_SIGNIN_PLAN,
};

/// Re-exported from `ministr-mcp` (MIT) so the auth middleware in the
/// local stack can attach a [`Plan`]-bearing `Tenant` to every request
/// extension without depending on this closed crate. Cloud-only code
/// (quota, billing, Atlas access) keeps reading `ministr_cloud::Plan`
/// — both paths see the same enum.
pub use ministr_mcp::auth::Plan;

/// Re-exported from `ministr-mcp` (MIT). Lives there so the open-core
/// handler surface can derive priority without depending on this
/// closed crate; cloud-side callers (F1.4 metering, F2.4 Checkout) keep
/// reading `ministr_cloud::queue_priority` so the public surface stays
/// stable.
pub use ministr_mcp::auth::queue_priority;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_order_pro_team_enterprise() {
        assert!(queue_priority(Plan::Enterprise) > queue_priority(Plan::Team));
        assert!(queue_priority(Plan::Team) > queue_priority(Plan::Pro));
    }

    #[test]
    fn plan_serialises_lowercase() {
        let s = serde_json::to_string(&Plan::Pro).unwrap();
        assert_eq!(s, "\"pro\"");
    }
}
