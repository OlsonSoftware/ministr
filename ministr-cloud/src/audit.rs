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

use std::collections::HashSet;
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

// ─── F5.3-c-ii — boot-time partition extension ─────────────────────
//
// Migration 0013 seeded 16 quarterly partitions covering 2024-Q1
// through 2027-Q4. Without an extender, the system silently fails
// `INSERT INTO audit_events` once `now()` crosses into Q1 2028.
// `ensure_audit_partitions` runs at pod boot, finds the highest
// existing partition, and CREATEs new quarterly partitions out to
// `now() + lookahead_quarters` so the forward edge stays ahead of
// real time.

/// Default lookahead — 8 quarters = 2 years of runway from the
/// current quarter. Picked so a pod that boots once and doesn't
/// reboot for 18 months still has ≥6 months of headroom.
pub const DEFAULT_PARTITION_LOOKAHEAD_QUARTERS: u32 = 8;

/// Outcome of one [`ensure_audit_partitions`] call. Reports both
/// what existed and what was just created so the boot log can
/// distinguish "warm start, nothing to do" from "cold start, just
/// created N quarters of headroom".
#[derive(Debug, Clone, Copy)]
pub struct EnsurePartitionsOutcome {
    /// Total partitions on `audit_events` BEFORE this call.
    pub existing: u32,
    /// New partitions created by this call.
    pub created: u32,
    /// Target end-of-runway: roughly `quarter_start(now()) +
    /// lookahead_quarters * 3 months`. Echoed back so the caller's
    /// structured log can record "covered through Y-MM-DD".
    pub target_end_year: i32,
    pub target_end_quarter: i32,
}

/// Parse an audit-events partition name back into `(year, quarter)`.
/// Returns `None` for names that don't match the
/// `audit_events_y{YYYY}q{N}` pattern from migration 0013. Pure —
/// pulled out for unit-testability without spinning up Postgres.
#[must_use]
pub fn parse_audit_partition_name(relname: &str) -> Option<(i32, i32)> {
    let rest = relname.strip_prefix("audit_events_y")?;
    let (year_s, rest) = rest.split_once('q')?;
    let year: i32 = year_s.parse().ok()?;
    let quarter: i32 = rest.parse().ok()?;
    if !(1..=4).contains(&quarter) {
        return None;
    }
    if !(2000..=2999).contains(&year) {
        return None;
    }
    Some((year, quarter))
}

/// Walk one quarter forward from `(year, quarter)`. Q4 → next year's Q1.
#[must_use]
pub fn next_quarter(year: i32, quarter: i32) -> (i32, i32) {
    if quarter >= 4 {
        (year + 1, 1)
    } else {
        (year, quarter + 1)
    }
}

/// First day of a quarter, in UTC, as a Postgres `timestamptz`
/// literal. Q1 starts in January, Q2 in April, Q3 in July, Q4 in
/// October.
#[must_use]
pub fn quarter_start_literal(year: i32, quarter: i32) -> String {
    let month = match quarter {
        1 => 1,
        2 => 4,
        3 => 7,
        _ => 10,
    };
    format!("{year:04}-{month:02}-01 00:00:00+00")
}

/// F5.3-c-ii — ensure `audit_events` has partitions covering up to
/// `lookahead_quarters` past the current calendar quarter. Walks
/// `pg_inherits` to find existing partitions, parses their names to
/// determine the highest existing `(year, quarter)`, then issues
/// `CREATE TABLE … PARTITION OF audit_events` for each missing
/// quarter through the target.
///
/// Idempotent: a second call with the same lookahead is a no-op
/// (returns `created = 0`). Cheap on warm starts (one `pg_inherits`
/// query + one `now()` query).
///
/// # Errors
///
/// [`AuditError::GetConn`] when the pool refuses a connection;
/// [`AuditError::Sql`] when any of the DDL statements fail.
pub async fn ensure_audit_partitions(
    pool: &Pool,
    lookahead_quarters: u32,
) -> Result<EnsurePartitionsOutcome, AuditError> {
    let client = pool
        .get()
        .await
        .map_err(|e| AuditError::GetConn(format!("ensure_audit_partitions: {e}")))?;

    // 1. List existing partitions of audit_events.
    let rows = client
        .query(
            "SELECT c.relname
             FROM pg_inherits i
             JOIN pg_class c ON c.oid = i.inhrelid
             WHERE i.inhparent = 'audit_events'::regclass",
            &[],
        )
        .await
        .map_err(|e| AuditError::Sql(format!("list partitions: {e}")))?;

    let existing_set: HashSet<(i32, i32)> = rows
        .iter()
        .filter_map(|r| parse_audit_partition_name(r.get::<_, &str>(0)))
        .collect();
    let existing = u32::try_from(existing_set.len()).unwrap_or(u32::MAX);

    // 2. Get current (year, quarter) from Postgres so timezone /
    // calendar arithmetic stays consistent with the partitioning
    // bounds (PG returns UTC quarter on a timestamptz).
    let now_row = client
        .query_one(
            "SELECT extract(year FROM now() AT TIME ZONE 'UTC')::int AS y,
                    extract(quarter FROM now() AT TIME ZONE 'UTC')::int AS q",
            &[],
        )
        .await
        .map_err(|e| AuditError::Sql(format!("now-quarter: {e}")))?;
    let cur_y: i32 = now_row.get("y");
    let cur_q: i32 = now_row.get("q");

    // 3. Target quarter = current + lookahead. Walk forward
    // lookahead_quarters times.
    let lookahead_i32 = i32::try_from(lookahead_quarters).unwrap_or(i32::MAX);
    let mut target_y = cur_y;
    let mut target_q = cur_q;
    for _ in 0..lookahead_i32 {
        let (ny, nq) = next_quarter(target_y, target_q);
        target_y = ny;
        target_q = nq;
    }

    // 4. Walk the FULL range [current quarter .. target quarter]
    // inclusive. Skip any quarter that already exists; CREATE the
    // rest. This fills gaps left by a manual DROP TABLE on a future
    // partition AND extends the forward edge in the same pass —
    // strictly more robust than "next_quarter(highest)" which can
    // skip gaps when a future partition exists past a hole.
    let mut walk_y = cur_y;
    let mut walk_q = cur_q;
    let mut created: u32 = 0;
    while (walk_y, walk_q) <= (target_y, target_q) {
        if !existing_set.contains(&(walk_y, walk_q)) {
            let (end_y, end_q) = next_quarter(walk_y, walk_q);
            let from_lit = quarter_start_literal(walk_y, walk_q);
            let to_lit = quarter_start_literal(end_y, end_q);
            let pname = format!("audit_events_y{walk_y}q{walk_q}");
            // `IF NOT EXISTS` defensive against a concurrent pod
            // racing to create the same partition. PG 11+ supports
            // it on `CREATE TABLE … PARTITION OF`.
            let sql = format!(
                "CREATE TABLE IF NOT EXISTS {pname} PARTITION OF audit_events \
                 FOR VALUES FROM ('{from_lit}') TO ('{to_lit}')"
            );
            client
                .batch_execute(&sql)
                .await
                .map_err(|e| AuditError::Sql(format!("create {pname}: {e}")))?;
            created = created.saturating_add(1);
        }
        let (ny, nq) = next_quarter(walk_y, walk_q);
        walk_y = ny;
        walk_q = nq;
    }

    Ok(EnsurePartitionsOutcome {
        existing,
        created,
        target_end_year: target_y,
        target_end_quarter: target_q,
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
    fn parse_audit_partition_name_extracts_year_quarter() {
        assert_eq!(parse_audit_partition_name("audit_events_y2024q1"), Some((2024, 1)));
        assert_eq!(parse_audit_partition_name("audit_events_y2026q3"), Some((2026, 3)));
        assert_eq!(parse_audit_partition_name("audit_events_y2027q4"), Some((2027, 4)));
    }

    #[test]
    fn parse_audit_partition_name_rejects_garbage() {
        assert_eq!(parse_audit_partition_name("audit_events"), None);
        assert_eq!(parse_audit_partition_name("audit_events_y2026"), None);
        assert_eq!(parse_audit_partition_name("audit_events_y2026q5"), None);
        assert_eq!(parse_audit_partition_name("audit_events_y2026q0"), None);
        assert_eq!(parse_audit_partition_name("audit_events_yfooq1"), None);
        assert_eq!(parse_audit_partition_name("audit_events_y1999q1"), None);
        assert_eq!(parse_audit_partition_name("audit_events_y3000q1"), None);
        assert_eq!(parse_audit_partition_name("something_else"), None);
    }

    #[test]
    fn next_quarter_wraps_q4_to_next_year_q1() {
        assert_eq!(next_quarter(2026, 1), (2026, 2));
        assert_eq!(next_quarter(2026, 2), (2026, 3));
        assert_eq!(next_quarter(2026, 3), (2026, 4));
        assert_eq!(next_quarter(2026, 4), (2027, 1));
        assert_eq!(next_quarter(2099, 4), (2100, 1));
    }

    #[test]
    fn quarter_start_literal_matches_migration_bounds() {
        // These four strings must match migration 0013's bounds
        // byte-for-byte so a partition created by the helper sits
        // adjacent to the seeded ones with no gap or overlap.
        assert_eq!(quarter_start_literal(2026, 1), "2026-01-01 00:00:00+00");
        assert_eq!(quarter_start_literal(2026, 2), "2026-04-01 00:00:00+00");
        assert_eq!(quarter_start_literal(2026, 3), "2026-07-01 00:00:00+00");
        assert_eq!(quarter_start_literal(2026, 4), "2026-10-01 00:00:00+00");
        assert_eq!(quarter_start_literal(2028, 1), "2028-01-01 00:00:00+00");
    }

    #[test]
    fn default_partition_lookahead_is_two_years() {
        // Doc comment promises "8 quarters = 2 years of runway".
        assert_eq!(DEFAULT_PARTITION_LOOKAHEAD_QUARTERS, 8);
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
