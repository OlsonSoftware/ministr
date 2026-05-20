//! Per-IP + per-tenant rate limits (F2.2 sub-bullet 3).
//!
//! Mitigates the indexing-abuse cost spiral on `POST /api/v1/corpora`
//! and the clone endpoint. A leaked Pro API key can still hammer the
//! cloud — without a free tier (B3 resolved) we have no automatic
//! "stop indexing" backstop except the bill the attacker would rack
//! up, so we enforce the wall server-side.
//!
//! # SOLID layering
//!
//! ```text
//!   middleware.rs   — axum from_fn_with_state glue
//!         │
//!   layer.rs        — RateLimitConfig + KeyExtractor composition
//!         │
//!   key.rs          — IpKey / TenantKey strategies (DIP: each is a
//!                     fn pointer, future "(ip,tenant)" key impls slot
//!                     in alongside)
//!         │
//!   bucket.rs       — TokenBucket trait (ISP) + InMemoryBucket impl.
//!                     Future Redis-backed bucket lands as a sibling
//!                     impl without touching the layer/middleware.
//! ```
//!
//! Each file owns ONE responsibility — bucket math doesn't know about
//! HTTP, the middleware doesn't know about refill rates, the key
//! extractors don't know about Tenant taxonomy beyond the public
//! `ministr_mcp::auth::Tenant` type.
//!
//! # Why in-process, not Redis (today)
//!
//! Pro-tier ACA Container Apps run as few-pod scale-outs; per-pod
//! token buckets are slightly leaky across pods but the per-IP / per-
//! tenant caps we ship are loose enough that the cross-pod skew is
//! immaterial. F5 Enterprise's stricter SLA, when it ships, may
//! upgrade to a Redis-backed `TokenBucket` impl — the seam is in
//! place.

pub mod bucket;
pub mod key;
pub mod layer;
pub mod middleware;

pub use bucket::{InMemoryBucket, RateLimitDecision, TokenBucket};
pub use key::{ip_key, tenant_key};
pub use layer::RateLimitConfig;
pub use middleware::rate_limit_middleware;
