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
    /// F3.7b — exact-match filter on `audit_events.action`
    /// (e.g. `"corpus.created"`, `"share.granted"`).
    pub action: Option<String>,
    /// F3.7b — exact-match filter on `audit_events.actor` (UUID
    /// string of the acting user). Useful for "what did X do?"
    /// admin investigations.
    pub actor: Option<String>,
    /// F3.7b — lower bound on `audit_events.ts` (ISO-8601 UTC).
    /// Inclusive — rows with `ts >= from_ts` admit.
    pub from_ts: Option<String>,
    /// F3.7b — upper bound on `audit_events.ts` (ISO-8601 UTC).
    /// Exclusive — rows with `ts < to_ts` admit, mirroring the
    /// half-open `[from, to)` convention in `usage_rollups`.
    pub to_ts: Option<String>,
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
    let rows = list_org_audit(&state.pool, &org_id, limit, &q)
        .await
        .map_err(AuditApiError::Repo)?;
    Ok(Json(AuditListResponse { rows }))
}

/// Default audit-retention window in days. The F3.7c cron drops
/// rows older than this on a daily schedule. Per §3 of ROADMAP.md
/// Team-tier audit ships at "90-day retention"; the F5.3 immutable
/// audit-log feature inherits the same `audit_events` shape but
/// retains forever.
pub const DEFAULT_AUDIT_RETENTION_DAYS: u32 = 90;

/// Canonical wire-shape plan id whose rows are exempt from the
/// F3.7c prune. Mirrors `ministr_mcp::auth::store::parse_plan_id`'s
/// "enterprise" arm — kept as a `const` here so the SQL string the
/// pruner emits and the parser's accepted form stay in lockstep.
const ENTERPRISE_PLAN_ID: &str = "enterprise";

/// Outcome of one [`prune_audit_events`] pass.
#[derive(Debug, Clone, Copy)]
pub struct PruneOutcome {
    /// Rows deleted by this pass.
    pub deleted: u64,
    /// How long the DELETE took. Tracked so the cron's structured-log
    /// dashboard can alarm on a runaway pass (large catch-up after a
    /// missed run, etc.).
    pub elapsed: std::time::Duration,
    /// Days kept (the cutoff). Echoes the caller's input so the cron's
    /// log message is self-describing without a second lookup.
    pub retention_days: u32,
}

/// F3.7c — drop `audit_events` rows older than `retention_days`. The
/// daily Container Apps Job invokes this via the `ministr audit prune`
/// CLI subcommand; manual local runs are equivalent.
///
/// **F5.3-a tier-aware retention**: rows whose `org_id` belongs to an
/// org with `plan_id = 'enterprise'` are EXEMPT from this prune. Per
/// §3 of ROADMAP.md the Enterprise tier ships immutable audit with
/// unlimited retention; F3.7c's daily DELETE would otherwise clobber
/// it after 90 days. Rows with `org_id IS NULL` (user-level actions
/// on personal accounts — pre-org sign-up flows, API-key actions on
/// personal accounts) DO get pruned because Enterprise's promise
/// covers org-scoped actions only.
///
/// Implemented as a single statement so Postgres can stream the
/// delete plan without a temp result set. The partial index on `ts`
/// is not created today (audit volume is tiny in v0); add one when
/// daily prune wall-clock exceeds a few hundred ms.
///
/// # Errors
///
/// [`AuditError::GetConn`] when the pool refuses a connection,
/// [`AuditError::Sql`] when the DELETE itself fails.
pub async fn prune_audit_events(
    pool: &Pool,
    retention_days: u32,
) -> Result<PruneOutcome, AuditError> {
    let client = pool
        .get()
        .await
        .map_err(|e| AuditError::GetConn(format!("prune_audit_events: {e}")))?;
    let started = std::time::Instant::now();
    // `make_interval(days => $1::integer)` is Postgres' interval-from-int
    // helper; using it (rather than concatenating into a literal) keeps
    // the value strictly parameterised so even a bogus retention number
    // can never inject SQL.
    //
    // The tier-skip clause uses a NOT EXISTS subquery rather than a
    // LEFT JOIN so the plan can drive off audit_events' (small) row
    // set; orgs is read once per audit row but the lookup is on the
    // PK and PG caches it. NULL org_id rows fall through to DELETE
    // because the subquery returns NULL → false in WHERE-context.
    let deleted = client
        .execute(
            "DELETE FROM audit_events ae
             WHERE ae.ts < now() - make_interval(days => $1::integer)
               AND (
                 ae.org_id IS NULL
                 OR NOT EXISTS (
                   SELECT 1 FROM orgs o
                   WHERE o.id = ae.org_id AND o.plan_id = $2
                 )
               )",
            &[
                &i32::try_from(retention_days).unwrap_or(i32::MAX),
                &ENTERPRISE_PLAN_ID,
            ],
        )
        .await
        .map_err(|e| AuditError::Sql(format!("prune audit_events: {e}")))?;
    Ok(PruneOutcome {
        deleted,
        elapsed: started.elapsed(),
        retention_days,
    })
}

/// Direct read used by the list handler. Exposed `pub` for the
/// eventual /orgs/{slug}/audit web page to call from the same crate.
///
/// All filters in `query` are AND-combined. Empty / `None` filters
/// admit every row.
///
/// # Errors
///
/// [`AuditError::GetConn`] / [`AuditError::Sql`] on DB issues.
pub async fn list_org_audit(
    pool: &Pool,
    org_id: &str,
    limit: i64,
    query: &AuditListQuery,
) -> Result<Vec<AuditRow>, AuditError> {
    let client = pool
        .get()
        .await
        .map_err(|e| AuditError::GetConn(format!("list_audit: {e}")))?;

    // Build the WHERE clause incrementally. `org_id` is always
    // present; subsequent filters use `COALESCE` so we can pass `None`
    // as a single canonical wire-shape parameter without conditional
    // SQL string assembly. Postgres optimiser handles the constant
    // `IS NULL` predicates efficiently.
    let action: Option<String> = query.action.clone();
    let actor: Option<String> = query.actor.clone();
    let before_id: Option<i64> = query.before_id;
    let from_ts: Option<String> = query.from_ts.clone();
    let to_ts: Option<String> = query.to_ts.clone();

    let sql = "SELECT
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
           AND ($2::bigint IS NULL OR id < $2::bigint)
           AND ($3::text   IS NULL OR action = $3::text)
           AND ($4::text   IS NULL OR actor::text = $4::text)
           AND ($5::text   IS NULL OR ts >= $5::timestamptz)
           AND ($6::text   IS NULL OR ts <  $6::timestamptz)
         ORDER BY id DESC
         LIMIT $7";

    let rows = client
        .query(
            sql,
            &[&org_id, &before_id, &action, &actor, &from_ts, &to_ts, &limit],
        )
        .await
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
        assert!(q.action.is_none());
        assert!(q.actor.is_none());
        assert!(q.from_ts.is_none());
        assert!(q.to_ts.is_none());
    }

    #[test]
    fn audit_query_deserialises_filters_from_json() {
        // Verify the serde wire-shape of every filter field. axum's
        // Query extractor uses serde_urlencoded under the hood; the
        // field names + Option<…> shapes must match for any caller's
        // `?action=…&from_ts=…` to land in the expected variant.
        let raw = serde_json::json!({
            "limit": 25,
            "before_id": 42,
            "action": "corpus.created",
            "actor": "00000000-0000-0000-0000-000000000001",
            "from_ts": "2026-01-01T00:00:00Z",
            "to_ts": "2026-02-01T00:00:00Z"
        });
        let q: AuditListQuery = serde_json::from_value(raw).unwrap();
        assert_eq!(q.limit, Some(25));
        assert_eq!(q.before_id, Some(42));
        assert_eq!(q.action.as_deref(), Some("corpus.created"));
        assert_eq!(
            q.actor.as_deref(),
            Some("00000000-0000-0000-0000-000000000001")
        );
        assert_eq!(q.from_ts.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(q.to_ts.as_deref(), Some("2026-02-01T00:00:00Z"));
    }

    #[test]
    fn audit_entry_round_trip_through_sink_trait() {
        // Compile-time check that PostgresAuditSink implements AuditSink.
        fn assert_impl<T: AuditSink>() {}
        assert_impl::<PostgresAuditSink>();
    }

    #[test]
    fn default_retention_is_team_tier_window() {
        // ROADMAP §3 Team tier ships "audit-light, 90-day retention".
        // If this constant ever changes, the F5.3 immutable-audit feature
        // (which inherits the same audit_events shape) must also be
        // re-checked.
        assert_eq!(DEFAULT_AUDIT_RETENTION_DAYS, 90);
    }

    #[test]
    fn enterprise_plan_id_matches_parse_plan_id_lowercase() {
        // F5.3-a — the prune SQL filters on `plan_id = 'enterprise'`.
        // If the wire-shape canonical string ever shifts (e.g. someone
        // mixed-cases to "Enterprise"), the SQL stops matching even
        // though `parse_plan_id` would still admit it. Lock the
        // constant to the lowercase form `parse_plan_id` produces.
        assert_eq!(ENTERPRISE_PLAN_ID, "enterprise");
        assert_eq!(ENTERPRISE_PLAN_ID, ENTERPRISE_PLAN_ID.to_ascii_lowercase());
    }

    #[test]
    fn prune_outcome_serialises_durations_safely() {
        // Compile-time guard: PruneOutcome's elapsed is a Duration —
        // converting to_ms() via `u64::try_from(d.as_millis())` is the
        // canonical pattern used by `cmd_audit_prune` for tracing.
        // This test catches a regression where someone replaces
        // `Duration` with a type that loses the ::as_millis() API.
        let o = PruneOutcome {
            deleted: 7,
            elapsed: std::time::Duration::from_millis(42),
            retention_days: 90,
        };
        let ms = u64::try_from(o.elapsed.as_millis()).unwrap();
        assert_eq!(ms, 42);
        assert_eq!(o.deleted, 7);
        assert_eq!(o.retention_days, 90);
    }
}
