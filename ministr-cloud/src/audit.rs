//! F3.7a — audit-light backend.
//!
//! Implements [`ministr_api::AuditSink`] backed by Postgres. The
//! `audit_events` table shipped with F1.2's `0001_initial.sql`; this
//! module is the first consumer.
//!
//! # Fire-and-forget posture
//!
//! Mirrors [`crate::PostgresUsageSink`]: the trait method enqueues
//! via `tokio::spawn` and returns immediately. A storage hiccup logs
//! but never propagates to the handler. Mounted on `OrgsState` +
//! `ApiKeysState` via `with_audit_sink`; self-hosted serve never
//! constructs the sink.
//!
//! # F3.7a list endpoint
//!
//! `GET /api/v1/orgs/{id}/audit` — owner / admin only. Returns the
//! most recent rows for the org, paginated by `limit` (default 50,
//! max 200) and `before_id` (cursor for fetch-older). User-level
//! actions (`org_id IS NULL`) are not surfaced — they live in the
//! user-level audit feed that will land later.

use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::{Extension, Path, Query, State};
use axum::routing::get;
use deadpool_postgres::Pool;
use ministr_api::{AuditEntry, AuditSink};
use ministr_mcp::auth::tenant::Tenant;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::orgs::member_role;

/// Errors surfaced by the audit list endpoint. Mirrors `OrgError`.
#[derive(Debug, thiserror::Error)]
pub enum AuditError {
    /// Pool acquisition failed.
    #[error("get connection: {0}")]
    GetConn(String),
    /// SQL error from a query / execute.
    #[error("sql: {0}")]
    Sql(String),
}

/// Postgres-backed implementation of [`AuditSink`].
///
/// Per call: spawns a tokio task that inserts one row into
/// `audit_events`. The hot path is one `Arc` clone + a channel-free
/// task spawn (~µs). The actual INSERT happens off the request thread.
#[derive(Debug, Clone)]
pub struct PostgresAuditSink {
    pool: Arc<Pool>,
}

impl PostgresAuditSink {
    /// Bind a sink to a shared pool.
    #[must_use]
    pub fn from_arc(pool: Arc<Pool>) -> Self {
        Self { pool }
    }

    /// Convenience: wrap as `Arc<dyn AuditSink>` for state wiring.
    #[must_use]
    pub fn into_dyn(self) -> Arc<dyn AuditSink> {
        Arc::new(self)
    }
}

impl AuditSink for PostgresAuditSink {
    fn record(&self, entry: AuditEntry) {
        let pool = Arc::clone(&self.pool);
        tokio::spawn(async move {
            let client = match pool.get().await {
                Ok(c) => c,
                Err(e) => {
                    warn!(error = %e, "audit sink: failed to acquire pool connection");
                    return;
                }
            };
            // ip column is INET; we accept it as text and let Postgres
            // parse via the implicit cast. user-agent stored verbatim.
            let res = client
                .execute(
                    "INSERT INTO audit_events
                       (org_id, actor, action, resource, ip, ua)
                     VALUES (
                         CASE WHEN $1::text IS NULL THEN NULL ELSE $1::uuid END,
                         CASE WHEN $2::text IS NULL THEN NULL ELSE $2::uuid END,
                         $3, $4,
                         CASE WHEN $5::text IS NULL THEN NULL ELSE $5::inet END,
                         $6
                     )",
                    &[
                        &entry.org_id,
                        &entry.actor,
                        &entry.action,
                        &entry.resource,
                        &entry.ip,
                        &entry.user_agent,
                    ],
                )
                .await;
            if let Err(e) = res {
                warn!(
                    error = %e,
                    action = %entry.action,
                    "audit sink: insert failed",
                );
            }
        });
    }
}

// ── List endpoint ──────────────────────────────────────────────────────────

/// One row from `audit_events`, shaped for the list response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditRow {
    /// Stable sequential id; serialise as a string so JS clients
    /// don't lose precision on large bigserials.
    pub id: String,
    pub org_id: Option<String>,
    pub actor: Option<String>,
    pub action: String,
    pub resource: String,
    /// ISO-8601 UTC.
    pub ts: String,
    pub ip: Option<String>,
    pub ua: Option<String>,
}

/// `GET /api/v1/orgs/{id}/audit` query string.
#[derive(Debug, Deserialize, Default)]
pub struct AuditListQuery {
    /// Page size cap. Default 50, max 200.
    pub limit: Option<i64>,
    /// Fetch older — return rows with `id < before_id`. Cursor pattern
    /// is stable across new inserts.
    pub before_id: Option<i64>,
}

/// `GET /api/v1/orgs/{id}/audit` response.
#[derive(Debug, Serialize)]
struct AuditListResponse {
    rows: Vec<AuditRow>,
}

/// Build the audit router. Mounts under no prefix; the route carries
/// its full path verbatim.
pub fn audit_routes(state: AuditState) -> Router {
    Router::new()
        .route("/api/v1/orgs/{id}/audit", get(list_handler))
        .with_state(state)
}

/// Axum state for the audit router.
#[derive(Debug, Clone)]
pub struct AuditState {
    pub pool: Arc<Pool>,
}

impl AuditState {
    #[must_use]
    pub fn from_arc(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

#[derive(Debug)]
enum AuditApiError {
    Unauthenticated,
    Forbidden,
    Repo(AuditError),
}

impl axum::response::IntoResponse for AuditApiError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode as S;
        match self {
            Self::Unauthenticated => (S::UNAUTHORIZED, "unauthenticated").into_response(),
            Self::Forbidden => (S::FORBIDDEN, "forbidden").into_response(),
            Self::Repo(e) => {
                warn!(error = %e, "audit list repo error");
                (S::INTERNAL_SERVER_ERROR, "internal").into_response()
            }
        }
    }
}

async fn list_handler(
    State(state): State<AuditState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
    Query(q): Query<AuditListQuery>,
) -> Result<Json<AuditListResponse>, AuditApiError> {
    let Some(Extension(tenant)) = tenant else {
        return Err(AuditApiError::Unauthenticated);
    };
    // Authz: owner / admin only. Member can NOT see audit — the
    // GitLab convention (owners + maintainers see audit) maps cleanly
    // onto our owner/admin pair.
    let role = member_role(&state.pool, &org_id, &tenant.subject)
        .await
        .map_err(|e| AuditApiError::Repo(AuditError::Sql(e.to_string())))?;
    let is_privileged = matches!(role.as_deref(), Some("owner" | "admin"));
    if !is_privileged {
        return Err(AuditApiError::Forbidden);
    }

    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let rows = list_org_audit(&state.pool, &org_id, limit, q.before_id)
        .await
        .map_err(AuditApiError::Repo)?;
    Ok(Json(AuditListResponse { rows }))
}

/// Direct read used by the list handler. Exposed `pub` for the
/// eventual /orgs/{slug}/audit web page to call from the same crate.
///
/// # Errors
///
/// [`AuditError::GetConn`] / [`AuditError::Sql`] on DB issues.
pub async fn list_org_audit(
    pool: &Pool,
    org_id: &str,
    limit: i64,
    before_id: Option<i64>,
) -> Result<Vec<AuditRow>, AuditError> {
    let client = pool
        .get()
        .await
        .map_err(|e| AuditError::GetConn(format!("list_audit: {e}")))?;
    let rows = if let Some(before) = before_id {
        client
            .query(
                "SELECT
                   id::text                AS id_text,
                   org_id::text            AS org_id_text,
                   actor::text             AS actor_text,
                   action,
                   resource,
                   to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS ts_iso,
                   host(ip)                AS ip_text,
                   ua
                 FROM audit_events
                 WHERE org_id = $1::uuid AND id < $2
                 ORDER BY id DESC
                 LIMIT $3",
                &[&org_id, &before, &limit],
            )
            .await
    } else {
        client
            .query(
                "SELECT
                   id::text                AS id_text,
                   org_id::text            AS org_id_text,
                   actor::text             AS actor_text,
                   action,
                   resource,
                   to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') AS ts_iso,
                   host(ip)                AS ip_text,
                   ua
                 FROM audit_events
                 WHERE org_id = $1::uuid
                 ORDER BY id DESC
                 LIMIT $2",
                &[&org_id, &limit],
            )
            .await
    }
    .map_err(|e| AuditError::Sql(format!("list audit_events: {e}")))?;

    Ok(rows
        .into_iter()
        .map(|r| AuditRow {
            id: r.get("id_text"),
            org_id: r.try_get("org_id_text").ok().flatten(),
            actor: r.try_get("actor_text").ok().flatten(),
            action: r.get("action"),
            resource: r.get("resource"),
            ts: r.get("ts_iso"),
            ip: r.try_get("ip_text").ok().flatten(),
            ua: r.try_get("ua").ok().flatten(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_query_defaults() {
        let q = AuditListQuery::default();
        assert!(q.limit.is_none());
        assert!(q.before_id.is_none());
    }

    #[test]
    fn audit_entry_round_trip_through_sink_trait() {
        // Compile-time check that PostgresAuditSink implements AuditSink.
        fn assert_impl<T: AuditSink>() {}
        assert_impl::<PostgresAuditSink>();
    }
}
