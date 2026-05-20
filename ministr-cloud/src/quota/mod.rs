//! Plan-aware quota enforcement (F2.3).
//!
//! Tower-style middleware sitting just inside the scope guards on cloud
//! protected routes. Rejects requests that exceed the calling tenant's
//! tier caps with **HTTP 402 Payment Required** + a JSON body the MCP
//! client + Tauri panel render as an upgrade prompt.
//!
//! # SOLID layout
//!
//! ```text
//!   middleware.rs — axum `from_fn_with_state` glue + 402 response builder
//!         │
//!   rule.rs       — `QuotaRule` trait + concrete `CorpusCountRule`
//!                   and `AtlasAccessRule`. Adding a new cap (queries/day,
//!                   indexing minutes) means writing a new impl — no
//!                   middleware change (OCP).
//!         │
//!   caps.rs       — `PlanCaps` mapping per §3 tier matrix. Pure data;
//!                   no I/O.
//!         │
//!   probe.rs      — `UsageProbe` trait + `RegistryProbe` impl. The
//!                   trait abstracts "how many of X does tenant T own
//!                   right now?" so future Postgres-backed counts
//!                   (once F3 lands multi-tenant corpus ownership) slot
//!                   in without touching the rules.
//! ```
//!
//! # Why a single Tower layer, not many
//!
//! Each request runs the full rule list once; rules cheap-skip via
//! their `matches` predicate when irrelevant. Stacking N layers would
//! pay the request-extraction tax N times.

pub mod caps;
pub mod middleware;
pub mod probe;
pub mod rule;

pub use caps::{caps_for_plan, PlanCaps};
pub use middleware::{quota_middleware, QuotaState};
pub use probe::{ProbeError, RegistryProbe, UsageProbe};
pub use rule::{AtlasAccessRule, CorpusCountRule, Decision, QuotaRule, Violation};
