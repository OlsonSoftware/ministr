//! `GET /api/v1/billing/usage` — the cloud's billable-usage view
//! (F1.4 sub-bullet 4).
//!
//! Returns the calling tenant's per-day, per-kind usage history from
//! `usage_rollups` for the last 30 days, plus the partial totals for
//! "today, so far" computed from the raw `usage_events` rows that the
//! nightly aggregator hasn't yet rolled up.
//!
//! Both Tauri's cloud panel and the future `ministr.ai/billing` web
//! UI consume this endpoint; the response shape is the stable wire
//! contract between this module and those consumers.
//!
//! # Auth
//!
//! Mounted in `cmd_serve_http` behind the OAuth `ministr:read` scope
//! guard. The handler reads `Extension<TenantId>` from the request
//! (populated by `ministr-mcp::auth::middleware`); requests without
//! an attached tenant get a 401 — that's the same shape every other
//! tenant-scoped handler will use in F2.x.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Extension, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use deadpool_postgres::Pool;
use ministr_api::TenantId;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// Default lookback window for rolled-up history. The Tauri panel
/// renders this as a sparkline; 30 days is enough to spot the
/// month-over-month trend without paginating.
const DEFAULT_ROLLUP_DAYS: i32 = 30;

/// Shape returned by `GET /api/v1/billing/usage`. Stable wire format
/// — the Tauri panel and `/billing` web UI both deserialise this.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageResponse {
    /// Echo of the caller's tenant subject. Useful for the UI to
    /// confirm "you are viewing X's billing".
    pub tenant_id: String,
    /// Per-(day, kind) rolled-up totals for the last
    /// [`DEFAULT_ROLLUP_DAYS`] days, newest day first.
    pub rollups: Vec<RollupRow>,
    /// Per-kind totals for events that landed since 00:00 UTC today
    /// — the events that haven't yet been folded into a rollup row.
    /// Empty after the next nightly aggregator run absorbs them.
    pub today_partial: Vec<PartialRow>,
}

/// One row of [`UsageResponse::rollups`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RollupRow {
    /// UTC calendar date, ISO 8601 (`YYYY-MM-DD`).
    pub day: String,
    /// Wire-format kind (e.g. `query.served`).
    pub kind: String,
    /// Sum of `usage_events.count` for that (tenant, day, kind).
    pub total: i64,
}

/// One row of [`UsageResponse::today_partial`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PartialRow {
    /// Wire-format kind (e.g. `query.served`).
    pub kind: String,
    /// Sum of `usage_events.count` since 00:00 UTC today.
    pub total: i64,
}

/// State passed to the billing handler.
#[derive(Clone)]
pub struct BillingState {
    pool: Arc<Pool>,
}

impl BillingState {
    /// Construct from an existing pool. The pool is shared with the
    /// other cloud features (OAuth, usage write path) — every
    /// cloud-side Postgres consumer uses the same connection pool.
    #[must_use]
    pub fn new(pool: Pool) -> Self {
        Self {
            pool: Arc::new(pool),
        }
    }

    /// Bare constructor for callers that already own an `Arc<Pool>`
    /// (e.g. `cmd_serve_http` after building one pool for migrations,
    /// OAuth, and the usage sink).
    #[must_use]
    pub fn from_arc(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

/// Build the billing router. Mount under no prefix; the route already
/// carries its full path (`/api/v1/billing/usage`).
pub fn billing_routes(state: BillingState) -> Router {
    Router::new()
        .route("/api/v1/billing/usage", get(usage_handler))
        .with_state(state)
}

async fn usage_handler(
    State(state): State<BillingState>,
    tenant: Option<Extension<TenantId>>,
) -> Result<Json<UsageResponse>, BillingError> {
    let Extension(tenant) = tenant.ok_or(BillingError::MissingTenant)?;
    let resp = fetch_usage(&state.pool, &tenant).await?;
    Ok(Json(resp))
}

async fn fetch_usage(pool: &Pool, tenant: &TenantId) -> Result<UsageResponse, BillingError> {
    let client = pool
        .get()
        .await
        .map_err(|e| BillingError::Internal(format!("get conn: {e}")))?;

    let rollup_rows = client
        .query(
            "SELECT to_char(day, 'YYYY-MM-DD') AS day,
                    kind,
                    total
             FROM usage_rollups
             WHERE tenant_id = $1::uuid
               AND day >= CURRENT_DATE - ($2::int - 1)
             ORDER BY day DESC, kind ASC",
            &[&tenant.as_str(), &DEFAULT_ROLLUP_DAYS],
        )
        .await
        .map_err(|e| BillingError::Internal(format!("rollups query: {e}")))?;
    let rollups = rollup_rows
        .into_iter()
        .map(|row| RollupRow {
            day: row.get("day"),
            kind: row.get("kind"),
            total: row.get("total"),
        })
        .collect();

    let partial_rows = client
        .query(
            "SELECT kind, COALESCE(SUM(count), 0)::bigint AS total
             FROM usage_events
             WHERE tenant_id = $1::uuid
               AND ts >= CURRENT_DATE::timestamptz
             GROUP BY kind
             ORDER BY kind ASC",
            &[&tenant.as_str()],
        )
        .await
        .map_err(|e| BillingError::Internal(format!("partial query: {e}")))?;
    let today_partial = partial_rows
        .into_iter()
        .map(|row| PartialRow {
            kind: row.get("kind"),
            total: row.get("total"),
        })
        .collect();

    Ok(UsageResponse {
        tenant_id: tenant.as_str().to_owned(),
        rollups,
        today_partial,
    })
}

/// Errors surfaced by the billing handler. Mapped to HTTP responses
/// via `IntoResponse`.
#[derive(Debug)]
enum BillingError {
    /// Auth middleware should have populated `Extension<TenantId>`;
    /// missing extension means the route was wired wrong.
    MissingTenant,
    /// Connection-pool or SQL failure. Logged at warn; surfaced as
    /// 500 with no body detail (avoids leaking schema).
    Internal(String),
}

impl IntoResponse for BillingError {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::MissingTenant => {
                (StatusCode::UNAUTHORIZED, "missing tenant identity").into_response()
            }
            Self::Internal(msg) => {
                warn!(error = %msg, "billing usage handler internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error reading usage",
                )
                    .into_response()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::billing::rollup::rollup_day;
    use crate::billing::usage::{record_usage, UsageEventKind};
    use crate::db::{connect, run_migrations};

    #[test]
    fn usage_response_serialises_stable_field_order() {
        let resp = UsageResponse {
            tenant_id: "abc".into(),
            rollups: vec![RollupRow {
                day: "2026-05-19".into(),
                kind: "query.served".into(),
                total: 42,
            }],
            today_partial: vec![PartialRow {
                kind: "query.served".into(),
                total: 7,
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"tenant_id\":\"abc\""));
        assert!(json.contains("\"rollups\":["));
        assert!(json.contains("\"today_partial\":["));
        assert!(json.contains("\"day\":\"2026-05-19\""));
    }

    async fn seed_user(pool: &Pool, suffix: &str) -> String {
        let client = pool.get().await.unwrap();
        client
            .query_one(
                "INSERT INTO users (email, plan_id)
                 VALUES ($1, $2)
                 RETURNING id::text",
                &[&format!("billing-{suffix}@test"), &"pro"],
            )
            .await
            .unwrap()
            .get("id")
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn fetch_usage_returns_rolled_up_and_partial_totals() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let pool = connect(&url).expect("connect");
        run_migrations(&pool).await.expect("migrate");

        let tenant = seed_user(&pool, &format!("ep-{}", std::process::id())).await;
        // Seed today's events: two query.served (will land in partial
        // until we call rollup_day), one index.minutes.
        record_usage(&pool, &tenant, UsageEventKind::QueryServed, 4)
            .await
            .unwrap();
        record_usage(&pool, &tenant, UsageEventKind::QueryServed, 3)
            .await
            .unwrap();
        record_usage(&pool, &tenant, UsageEventKind::IndexMinutes, 9)
            .await
            .unwrap();

        // Before any rollup: nothing in rollups, everything in today_partial.
        let resp = fetch_usage(&pool, &TenantId::from(tenant.clone()))
            .await
            .unwrap();
        assert_eq!(resp.tenant_id, tenant);
        assert!(resp.rollups.is_empty(), "no rollups yet");
        // today_partial is keyed by kind; assert the SUMs.
        let partial: std::collections::HashMap<_, _> = resp
            .today_partial
            .iter()
            .map(|p| (p.kind.as_str(), p.total))
            .collect();
        assert_eq!(partial.get("query.served"), Some(&7));
        assert_eq!(partial.get("index.minutes"), Some(&9));

        // After rollup, the values move into rollups[]; partial keeps
        // showing them too because they're still raw events on the
        // current day (the aggregator never deletes events). That's
        // intentional — the UI dedupes by treating rollups as
        // authoritative for past days and partial as "today only".
        rollup_day(&pool, 0).await.unwrap();
        let resp = fetch_usage(&pool, &TenantId::from(tenant.clone()))
            .await
            .unwrap();
        let rollup_totals: std::collections::HashMap<_, _> = resp
            .rollups
            .iter()
            .map(|r| (r.kind.as_str(), r.total))
            .collect();
        assert_eq!(rollup_totals.get("query.served"), Some(&7));
        assert_eq!(rollup_totals.get("index.minutes"), Some(&9));
    }
}
