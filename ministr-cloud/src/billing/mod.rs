//! Billing namespace for ministr-cloud.
//!
//! Houses the multi-tenant billing surface. The first occupant is
//! [`usage`] (F1.4 sub-bullet 1), the write path for `usage_events`
//! rows that feed the daily aggregator and the `/api/v1/billing/usage`
//! endpoint. Future occupants:
//!
//! | Phase | Module |
//! |---|---|
//! | F1.4 | `usage::rollup` — daily aggregator cron |
//! | F1.4 | `usage::endpoint` — `GET /api/v1/billing/usage` |
//! | F1.5 | `stripe` — Stripe Meters + webhook receiver |
//! | F2.4 | `checkout` — Stripe Checkout session creation |

pub mod usage;

pub use usage::{record_usage, UsageEventKind};
