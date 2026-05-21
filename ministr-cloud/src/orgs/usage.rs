//! F3.3a — org-level usage aggregation.
//!
//! The F1.4 [`crate::billing::endpoint::usage_handler`] returns
//! per-tenant rollups. This module's [`fetch_org_usage`] aggregates
//! the same `usage_rollups` table across every member of an org,
//! producing the per-seat breakdown the F3.3 dashboard renders.
//!
//! `usage_rollups.tenant_id` is keyed by user UUID — every audit
//! event the activity middleware emits stamps the calling user as
//! the tenant. To produce an org view, we join through `org_members`:
//!
//! ```sql
//! SELECT m.user_id, u.email, day, kind, total
//! FROM org_members m
//! JOIN users u ON u.id = m.user_id
//! JOIN usage_rollups r ON r.tenant_id = m.user_id
//! WHERE m.org_id = $1
//! ```
//!
//! # Authz
//!
//! Owner / admin only — members can't see each other's per-seat
//! breakdown. Mirrors [`crate::audit::list_handler`].
//!
//! # F3.3 done criterion alignment
//!
//! The dashboard must "match the live Stripe invoice within ±1".
//! Daily granularity via [`usage_rollups`] is what Stripe meters
//! consume, so a per-day-per-member breakdown sums to the same
//! number Stripe charged. F3.3b/c will render the sparkline and
//! cost-projection on top of this shape.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, Query, State};
use axum::routing::get;
use deadpool_postgres::Pool;
use ministr_mcp::auth::tenant::Tenant;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::orgs::member_role;

/// Default lookback window (days) for the dashboard. The F3.3 spec
/// implies a "this billing cycle" view; 30 days is the right default
/// for a calendar-month Stripe cycle. The query string can override.
pub const DEFAULT_USAGE_DAYS: i32 = 30;

/// One per-day, per-kind rollup row attributed to one member.
/// Same wire shape as the F1.4 [`crate::billing::endpoint::RollupRow`]
/// but with the member's UUID/email attached.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrgRollupRow {
    /// `org_members.user_id` (also `usage_rollups.tenant_id`).
    pub user_id: String,
    /// `users.email` — for human-readable rendering.
    pub email: String,
    /// ISO 8601 `YYYY-MM-DD` (UTC).
    pub day: String,
    /// Wire-format event kind (e.g. `query.served`).
    pub kind: String,
    /// `SUM(usage_events.count)` for that (member, day, kind).
    pub total: i64,
}

/// One per-kind partial row attributed to one member (today's
/// not-yet-rolled-up events).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrgPartialRow {
    pub user_id: String,
    pub email: String,
    pub kind: String,
    pub total: i64,
}

/// `GET /api/v1/orgs/{id}/usage` response. Stable wire shape;
/// F3.3b/c web dashboard + Tauri usage tile both deserialise this.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrgUsageResponse {
    /// Echo of the caller's org context.
    pub org_id: String,
    /// Lookback in days that produced [`Self::rollups`]; echoes the
    /// `?days=N` query string (defaulting to [`DEFAULT_USAGE_DAYS`]).
    pub range_days: i32,
    /// One row per (member, day, kind). Sorted by `email ASC, day
    /// DESC, kind ASC` for stable rendering.
    pub rollups: Vec<OrgRollupRow>,
    /// Today's events that haven't been folded into a rollup yet.
    /// One row per (member, kind).
    pub today_partial: Vec<OrgPartialRow>,
}

/// `?days=N` overrides the default 30-day window. Clamped to
/// `[1, 366]` so a bug in the UI can't fan out a multi-year scan.
#[derive(Debug, Deserialize, Default)]
pub struct OrgUsageQuery {
    pub days: Option<i32>,
}

/// Build the org-usage router. Mounted under no prefix; the route
/// carries its full path verbatim. Owner/admin authz enforced inline.
pub fn org_usage_routes(state: OrgUsageState) -> Router {
    Router::new()
        .route("/api/v1/orgs/{id}/usage", get(list_handler))
        .with_state(state)
}

/// Axum state for the org-usage router.
#[derive(Debug, Clone)]
pub struct OrgUsageState {
    pub pool: Arc<Pool>,
}

impl OrgUsageState {
    #[must_use]
    pub fn from_arc(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

#[derive(Debug)]
enum OrgUsageApiError {
    Unauthenticated,
    Forbidden,
    Repo(String),
}

impl axum::response::IntoResponse for OrgUsageApiError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode as S;
        match self {
            Self::Unauthenticated => (S::UNAUTHORIZED, "unauthenticated").into_response(),
            Self::Forbidden => (S::FORBIDDEN, "forbidden").into_response(),
            Self::Repo(e) => {
                warn!(error = %e, "org usage repo error");
                (S::INTERNAL_SERVER_ERROR, "internal").into_response()
            }
        }
    }
}

async fn list_handler(
    State(state): State<OrgUsageState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
    Query(q): Query<OrgUsageQuery>,
) -> Result<Json<OrgUsageResponse>, OrgUsageApiError> {
    let Some(Extension(tenant)) = tenant else {
        return Err(OrgUsageApiError::Unauthenticated);
    };
    let role = member_role(&state.pool, &org_id, &tenant.subject)
        .await
        .map_err(|e| OrgUsageApiError::Repo(e.to_string()))?;
    if !matches!(role.as_deref(), Some("owner" | "admin")) {
        return Err(OrgUsageApiError::Forbidden);
    }
    let days = q.days.unwrap_or(DEFAULT_USAGE_DAYS).clamp(1, 366);
    let resp = fetch_org_usage(&state.pool, &org_id, days)
        .await
        .map_err(OrgUsageApiError::Repo)?;
    Ok(Json(resp))
}

/// Direct read used by [`list_handler`]. Exposed `pub` so the
/// eventual web `/orgs/{slug}/usage` page can reuse it from the
/// same crate.
///
/// # Errors
///
/// Returns a stringified error on connection or SQL failure.
pub async fn fetch_org_usage(
    pool: &Pool,
    org_id: &str,
    days: i32,
) -> Result<OrgUsageResponse, String> {
    let client = pool
        .get()
        .await
        .map_err(|e| format!("get conn: {e}"))?;

    // Per-day, per-kind, per-member rollups for the last N days.
    // Join through org_members + users so the result carries the
    // human-readable email alongside the rolled-up totals. The
    // existing `idx_org_members_user` (migrations 0001) keeps the
    // join cheap.
    let rollup_rows = client
        .query(
            "SELECT
                 m.user_id::text          AS user_id,
                 u.email                   AS email,
                 to_char(r.day, 'YYYY-MM-DD') AS day,
                 r.kind                    AS kind,
                 r.total                   AS total
             FROM org_members m
             JOIN users u ON u.id = m.user_id
             JOIN usage_rollups r ON r.tenant_id = m.user_id
             WHERE m.org_id = $1::uuid
               AND r.day >= CURRENT_DATE - ($2::int - 1)
             ORDER BY u.email ASC, r.day DESC, r.kind ASC",
            &[&org_id, &days],
        )
        .await
        .map_err(|e| format!("rollups query: {e}"))?;
    let rollups: Vec<OrgRollupRow> = rollup_rows
        .into_iter()
        .map(|row| OrgRollupRow {
            user_id: row.get("user_id"),
            email: row.get("email"),
            day: row.get("day"),
            kind: row.get("kind"),
            total: row.get("total"),
        })
        .collect();

    // Today's not-yet-rolled-up events, summed per (member, kind).
    let partial_rows = client
        .query(
            "SELECT
                 m.user_id::text          AS user_id,
                 u.email                   AS email,
                 e.kind                    AS kind,
                 COALESCE(SUM(e.count), 0)::bigint AS total
             FROM org_members m
             JOIN users u ON u.id = m.user_id
             JOIN usage_events e ON e.tenant_id = m.user_id
             WHERE m.org_id = $1::uuid
               AND e.ts >= CURRENT_DATE::timestamptz
             GROUP BY m.user_id, u.email, e.kind
             ORDER BY u.email ASC, e.kind ASC",
            &[&org_id],
        )
        .await
        .map_err(|e| format!("partial query: {e}"))?;
    let today_partial: Vec<OrgPartialRow> = partial_rows
        .into_iter()
        .map(|row| OrgPartialRow {
            user_id: row.get("user_id"),
            email: row.get("email"),
            kind: row.get("kind"),
            total: row.get("total"),
        })
        .collect();

    Ok(OrgUsageResponse {
        org_id: org_id.to_owned(),
        range_days: days,
        rollups,
        today_partial,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_query_defaults_to_none_then_clamps_to_30() {
        let q = OrgUsageQuery::default();
        assert!(q.days.is_none());
        // Mirror the clamp shape applied in list_handler.
        let days = q.days.unwrap_or(DEFAULT_USAGE_DAYS).clamp(1, 366);
        assert_eq!(days, DEFAULT_USAGE_DAYS);
    }

    #[test]
    fn usage_query_clamps_excessive_inputs() {
        let q = OrgUsageQuery { days: Some(9999) };
        let days = q.days.unwrap_or(DEFAULT_USAGE_DAYS).clamp(1, 366);
        assert_eq!(days, 366);
        let q = OrgUsageQuery { days: Some(0) };
        let days = q.days.unwrap_or(DEFAULT_USAGE_DAYS).clamp(1, 366);
        assert_eq!(days, 1);
    }

    #[test]
    fn usage_response_serialises_stable_field_order() {
        let resp = OrgUsageResponse {
            org_id: "org-uuid".into(),
            range_days: 30,
            rollups: vec![OrgRollupRow {
                user_id: "u1".into(),
                email: "a@x".into(),
                day: "2026-05-21".into(),
                kind: "query.served".into(),
                total: 42,
            }],
            today_partial: vec![OrgPartialRow {
                user_id: "u1".into(),
                email: "a@x".into(),
                kind: "query.served".into(),
                total: 7,
            }],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"org_id\":\"org-uuid\""));
        assert!(json.contains("\"range_days\":30"));
        assert!(json.contains("\"email\":\"a@x\""));
        assert!(json.contains("\"day\":\"2026-05-21\""));
        assert!(json.contains("\"today_partial\":["));
    }
}
