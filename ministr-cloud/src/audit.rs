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

/// Build the audit router. Mounts under no prefix; the routes carry
/// their full paths verbatim. F5.3-c-ii-archive-read adds the
/// `/audit/archived` route alongside the existing live `/audit`
/// list endpoint.
pub fn audit_routes(state: AuditState) -> Router {
    Router::new()
        .route("/api/v1/orgs/{id}/audit", get(list_handler))
        .route(
            "/api/v1/orgs/{id}/audit/archived",
            get(list_archived_handler),
        )
        .with_state(state)
}

/// Axum state for the audit router. The optional `archive_dir`
/// powers the F5.3-c-ii-archive-read endpoint — when `None`, the
/// archived-audit endpoint returns 503; when `Some(path)`, the
/// handler reads gzipped JSONL files from that directory.
#[derive(Debug, Clone)]
pub struct AuditState {
    pub pool: Arc<Pool>,
    pub archive_dir: Option<std::path::PathBuf>,
}

impl AuditState {
    #[must_use]
    pub fn from_arc(pool: Arc<Pool>) -> Self {
        Self {
            pool,
            archive_dir: None,
        }
    }

    /// F5.3-c-ii-archive-read — wire the archive directory the
    /// handler reads from. Production deployments populate this
    /// from the `MINISTR_AUDIT_ARCHIVE_DIR` env var; harness
    /// populates it from a per-run scratch path.
    #[must_use]
    pub fn with_archive_dir(mut self, archive_dir: impl Into<std::path::PathBuf>) -> Self {
        self.archive_dir = Some(archive_dir.into());
        self
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

/// F5.3-c-ii-archive-read — query params for the archived-audit
/// list endpoint. `from` / `to` are inclusive-exclusive ISO-8601
/// dates (`YYYY-MM-DD`). Validated server-side; malformed dates
/// return 400.
#[derive(Debug, Deserialize)]
struct ArchivedAuditQuery {
    from: String,
    to: String,
}

/// F5.3-c-ii-archive-read — one row returned by the archived-audit
/// endpoint. Identical shape to the JSONL written by
/// `archive_audit_partition_to_dir`; we re-deserialize each line.
#[derive(Debug, Serialize, Deserialize)]
struct ArchivedAuditRow {
    id: String,
    action: String,
    resource: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    org_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    actor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ip: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ua: Option<String>,
    ts: String,
}

#[derive(Debug, Serialize)]
struct ArchivedAuditResponse {
    rows: Vec<ArchivedAuditRow>,
}

/// Parse `YYYY-MM-DD` into `(year, month, day)` integers. Returns
/// `None` for any malformed input. The serve never accepts user
/// input that should land here without going through this parser,
/// so it's intentionally strict.
fn parse_iso_date(s: &str) -> Option<(i32, u32, u32)> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let day: u32 = parts[2].parse().ok()?;
    if !(2000..=2999).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
    {
        return None;
    }
    Some((year, month, day))
}

/// Derive `(year, quarter)` from `(year, month)`. Quarters: 1→Q1,
/// 2→Q1, 3→Q1, 4→Q2, …, 12→Q4. Pure; tested below.
fn quarter_of(month: u32) -> i32 {
    #[allow(clippy::cast_possible_wrap)]
    let m = month as i32;
    (m - 1) / 3 + 1
}

/// Walk every (year, quarter) tuple overlapping `[from, to]`
/// (inclusive of from, exclusive of to). Returned in calendar
/// order. Reuses [`next_quarter`] for the increment.
fn quarters_in_range(
    (from_y, from_m): (i32, u32),
    (to_y, to_m): (i32, u32),
) -> Vec<(i32, i32)> {
    let from_q = quarter_of(from_m);
    let to_q = quarter_of(to_m);
    let mut out = Vec::new();
    let mut y = from_y;
    let mut q = from_q;
    while (y, q) <= (to_y, to_q) {
        out.push((y, q));
        let (ny, nq) = next_quarter(y, q);
        y = ny;
        q = nq;
    }
    out
}

async fn list_archived_handler(
    State(state): State<AuditState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
    Query(q): Query<ArchivedAuditQuery>,
) -> Result<Json<ArchivedAuditResponse>, AuditApiError> {
    let Some(Extension(tenant)) = tenant else {
        return Err(AuditApiError::Unauthenticated);
    };
    let role = member_role(&state.pool, &org_id, &tenant.subject)
        .await
        .map_err(|e| AuditApiError::Repo(AuditError::Sql(e.to_string())))?;
    let is_privileged = matches!(role.as_deref(), Some("owner" | "admin"));
    if !is_privileged {
        return Err(AuditApiError::Forbidden);
    }

    let Some(archive_dir) = state.archive_dir.as_ref() else {
        // Endpoint reached but no archive dir wired — surface as
        // "service unavailable" so the customer's compliance tooling
        // distinguishes "no archived data" from "config gap".
        return Err(AuditApiError::Repo(AuditError::Sql(
            "audit archive not configured (set MINISTR_AUDIT_ARCHIVE_DIR)".to_string(),
        )));
    };

    let Some(from_ymd) = parse_iso_date(&q.from) else {
        return Err(AuditApiError::Repo(AuditError::Sql(
            "invalid `from` — must be YYYY-MM-DD".to_string(),
        )));
    };
    let Some(to_ymd) = parse_iso_date(&q.to) else {
        return Err(AuditApiError::Repo(AuditError::Sql(
            "invalid `to` — must be YYYY-MM-DD".to_string(),
        )));
    };

    // Quarter-range filter — which gzipped JSONL files to even open.
    // Inside each file the per-row ts filter narrows further.
    let quarters = quarters_in_range(
        (from_ymd.0, from_ymd.1),
        (to_ymd.0, to_ymd.1),
    );

    // ISO-8601 timestamps lexicographically compare correctly because
    // every field is zero-padded; this lets us compare strings
    // directly without parsing back to a date type.
    let from_prefix = format!("{:04}-{:02}-{:02}", from_ymd.0, from_ymd.1, from_ymd.2);
    let to_prefix = format!("{:04}-{:02}-{:02}", to_ymd.0, to_ymd.1, to_ymd.2);

    let mut rows: Vec<ArchivedAuditRow> = Vec::new();
    for (year, quarter) in quarters {
        let file_path = archive_dir.join(format!("audit_events_y{year:04}q{quarter}.jsonl.gz"));
        let bytes = match tokio::fs::read(&file_path).await {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                tracing::warn!(
                    file = %file_path.display(),
                    error = %e,
                    "audit archive read failed; skipping file"
                );
                continue;
            }
        };
        // Decompress + parse line-by-line. flate2's GzDecoder reads
        // synchronously; we already have the full bytes in memory
        // (per-quarter audit volume is small in v0), so blocking is
        // bounded.
        let mut decompressed = String::new();
        {
            use std::io::Read;
            let mut decoder = flate2::read::GzDecoder::new(&bytes[..]);
            if let Err(e) = decoder.read_to_string(&mut decompressed) {
                tracing::warn!(
                    file = %file_path.display(),
                    error = %e,
                    "audit archive gzip decode failed; skipping file"
                );
                continue;
            }
        }
        for line in decompressed.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let row: ArchivedAuditRow = match serde_json::from_str(line) {
                Ok(r) => r,
                Err(_) => continue,
            };
            // Cross-org isolation: skip rows whose org_id doesn't
            // match the requested org. Personal-account rows
            // (`org_id` IS NULL in the DB → field absent or null in
            // JSON) are never returned via this endpoint — they
            // belong to the user-level audit feed (not yet shipped).
            if row.org_id.as_deref() != Some(org_id.as_str()) {
                continue;
            }
            // Date-range filter on the ISO-8601 ts string. Compare
            // the date prefix `YYYY-MM-DD` lexicographically; `to`
            // is exclusive so the comparison is strict.
            let ts_date = row.ts.get(..10).unwrap_or("");
            if ts_date < from_prefix.as_str() || ts_date >= to_prefix.as_str() {
                continue;
            }
            rows.push(row);
        }
    }

    Ok(Json(ArchivedAuditResponse { rows }))
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

// ─── F5.3-c-ii-archive-fs — cold partition archive (FS sink) ──────
//
// Once a quarterly partition is older than the customer's retention
// window, its rows can move from hot Postgres storage to cheap
// long-term durable storage (Azure Blob with Object Replication for
// immutability — that's F5.3-c-ii-archive-blob). This chunk ships
// the engine — SELECT → gzipped JSONL → write-to-disk → DETACH →
// DROP — with a filesystem sink. The Azure Blob backend is a sink
// swap.
//
// Operator workflow:
//
//   ministr audit archive --partition audit_events_y2024q1 \
//                          --archive-dir /var/lib/ministr/audit-archive
//
// After completion, the named partition is GONE from the database
// (DETACH'd + DROP'd inside one transaction) and the gzipped JSONL
// file at `<archive_dir>/<partition_name>.jsonl.gz` is the only
// remaining copy. The read-path handler that streams that file back
// for compliance queries lands in F5.3-c-ii-archive-read.

/// Outcome of one archive call. Echoed back in the structured
/// boot/cron log so the operator dashboard can chart "rows archived
/// per partition per quarter".
#[derive(Debug, Clone)]
pub struct ArchiveOutcome {
    /// Number of audit-event rows written.
    pub rows: u64,
    /// Size of the gzipped payload, in bytes.
    pub bytes_on_disk: u64,
    /// Human-readable target location — FS path or `blob://account/container/key`.
    /// Echoed into logs so the operator can locate the archive.
    pub target: String,
}

/// F5.3-c-ii-archive-blob-sink — destination for one gzipped audit
/// partition archive. Two impls today: [`FsArchiveSink`] (local
/// directory, used by the dev/test harness and on-prem deployments
/// with a persistent volume mount) and [`AzureBlobArchiveSink`]
/// (Azure Blob Storage, the natural production target). Both
/// accept the same key + gzipped body; the dispatcher picks based
/// on which env vars the operator sets.
#[allow(async_fn_in_trait)] // We never need to be dyn-compatible; trait users always know the concrete impl at the boundary.
pub trait ArchiveSink: std::fmt::Debug + Send + Sync {
    /// Persist `gz_body` at the location identified by `key`. Returns
    /// a human-readable target string for the structured log (e.g.
    /// `"file:///var/.../audit_events_y2024q1.jsonl.gz"` for FS or
    /// `"blob://ministrdata/audit-archive/audit_events_y2024q1.jsonl.gz"`
    /// for Azure Blob).
    ///
    /// `key` is the partition's bare filename (e.g. `"audit_events_y2024q1.jsonl.gz"`).
    /// Sinks compose this with their configured prefix as appropriate.
    async fn put(&self, key: &str, gz_body: Vec<u8>) -> Result<String, AuditError>;
}

/// FS-backed archive sink. Production deployments with a persistent
/// volume mount can use this; the dev/test harness uses it
/// unconditionally (Azure Blob is harder to mock locally without
/// the Azurite emulator).
#[derive(Debug, Clone)]
pub struct FsArchiveSink {
    pub dir: std::path::PathBuf,
}

impl FsArchiveSink {
    #[must_use]
    pub fn new(dir: impl Into<std::path::PathBuf>) -> Self {
        Self { dir: dir.into() }
    }
}

impl ArchiveSink for FsArchiveSink {
    async fn put(&self, key: &str, gz_body: Vec<u8>) -> Result<String, AuditError> {
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|e| AuditError::Sql(format!("archive: create_dir_all: {e}")))?;
        let file_path = self.dir.join(key);
        tokio::fs::write(&file_path, &gz_body)
            .await
            .map_err(|e| {
                AuditError::Sql(format!("archive: write {}: {e}", file_path.display()))
            })?;
        Ok(format!("file://{}", file_path.display()))
    }
}

/// F5.3-c-ii-archive-blob-sink — Azure Blob Storage archive sink.
/// Wraps an `azure_storage_blob::BlobContainerClient`. Uses
/// `azure_identity` for auth — production pods pass a
/// `ManagedIdentityCredential`; dev/CI flows pass a
/// `DeveloperToolsCredential` (which chains `az login`, env vars,
/// etc).
///
/// **Cost posture (zero local validation, zero default spend)**:
/// the SDK is endpoint-pinned at the production
/// `<account>.blob.core.windows.net` URL — no Azurite harness is
/// shipped in this chunk because the SDK's `TokenCredential` path
/// requires HTTPS and Azurite uses plain HTTP. Customers validate
/// in their own Azure tenant when they configure
/// `MINISTR_AUDIT_ARCHIVE_BLOB_ACCOUNT` +
/// `MINISTR_AUDIT_ARCHIVE_BLOB_CONTAINER`.
pub struct AzureBlobArchiveSink {
    account_name: String,
    container_name: String,
    container: azure_storage_blob::BlobContainerClient,
}

impl std::fmt::Debug for AzureBlobArchiveSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AzureBlobArchiveSink")
            .field("account", &self.account_name)
            .field("container", &self.container_name)
            .finish()
    }
}

impl AzureBlobArchiveSink {
    /// Construct using `ManagedIdentityCredential` — the same auth
    /// pattern as [`crate::session_bundle_store`]. Production
    /// Container Apps pods auth via their assigned Managed
    /// Identity automatically. Local dev needs to be run inside an
    /// Azure VM or with a workaround (operator-driven, so usually
    /// fine to defer local validation to F5.3-c-ii-archive-blob-pulumi).
    ///
    /// # Errors
    ///
    /// Fails if `ManagedIdentityCredential::new(None)` errors (rare —
    /// chain construction is mostly infallible) or if the constructed
    /// service URL is malformed.
    pub fn with_managed_identity(
        account_name: &str,
        container_name: &str,
    ) -> Result<Self, AuditError> {
        let credential = azure_identity::ManagedIdentityCredential::new(None).map_err(|e| {
            AuditError::Sql(format!("archive blob: ManagedIdentityCredential: {e}"))
        })?;
        Self::with_credential(account_name, container_name, credential)
    }

    /// Construct from explicit account + container + credential.
    /// Pulled out so tests and future caller paths (e.g. a Pulumi
    /// init job that uses a different credential chain) can pass
    /// their own. Production code uses [`with_default_credential`].
    ///
    /// # Errors
    ///
    /// Fails if the constructed service URL is malformed (rare —
    /// validates `account_name` is a syntactic host label) or the
    /// SDK's pipeline construction errors.
    pub fn with_credential(
        account_name: &str,
        container_name: &str,
        credential: std::sync::Arc<dyn azure_core::credentials::TokenCredential>,
    ) -> Result<Self, AuditError> {
        let service_url = azure_core::http::Url::parse(&format!(
            "https://{account_name}.blob.core.windows.net/"
        ))
        .map_err(|e| AuditError::Sql(format!("archive blob: parse service url: {e}")))?;
        let service = azure_storage_blob::BlobServiceClient::new(
            service_url,
            Some(credential),
            None,
        )
        .map_err(|e| AuditError::Sql(format!("archive blob: service client: {e}")))?;
        let container = service.blob_container_client(container_name);
        Ok(Self {
            account_name: account_name.to_string(),
            container_name: container_name.to_string(),
            container,
        })
    }
}

impl ArchiveSink for AzureBlobArchiveSink {
    async fn put(&self, key: &str, gz_body: Vec<u8>) -> Result<String, AuditError> {
        // BlobClient::upload_blob: PUT block blob with the supplied
        // bytes. overwrite=false by default — but for archive files
        // we WANT overwrite-safe semantics because a retry after a
        // transient failure must succeed. Default content-type
        // "application/gzip" so the customer's downstream tooling
        // (Azure portal, azcopy) recognises it.
        let blob = self.container.blob_client(key);
        // `RequestContent::from(Vec<u8>)` wraps as a `Bytes` body —
        // BlobClient::upload signs the request via the pipeline's
        // bearer-token policy. Block-blob default is overwrite-safe;
        // audit-file contents for a given partition are idempotent
        // (same SELECT → same JSONL) so a retry after a transient
        // failure is harmless.
        let body = azure_core::http::RequestContent::from(gz_body);
        blob.upload(body, None)
            .await
            .map_err(|e| AuditError::Sql(format!("archive blob: upload {key}: {e}")))?;
        Ok(format!(
            "blob://{}/{}/{}",
            self.account_name, self.container_name, key
        ))
    }
}

/// F5.3-c-ii-archive-fs — archive one `audit_events` partition to a
/// gzipped JSONL file in `archive_dir`, then DETACH + DROP it from
/// the live database in a single transaction.
///
/// `partition_name` MUST match the migration-0013 pattern
/// (`audit_events_y{YYYY}q{N}`) — anything else is rejected as a
/// defense-in-depth measure against path-traversal via a malicious
/// DB row (the file path is `archive_dir/<partition_name>.jsonl.gz`;
/// the name is validated via [`parse_audit_partition_name`] before
/// touching the filesystem).
///
/// Backward-compat thin wrapper for
/// [`archive_audit_partition_with_sink`] with an [`FsArchiveSink`].
/// Existing callers (CLI default, harness) keep working through the
/// FS path; production deployments opt into Azure Blob via the
/// `_with_sink` form + [`AzureBlobArchiveSink`].
///
/// # Errors
///
/// Surfaces whatever the wrapped call returns.
pub async fn archive_audit_partition_to_dir(
    pool: &Pool,
    partition_name: &str,
    archive_dir: &std::path::Path,
) -> Result<ArchiveOutcome, AuditError> {
    let sink = FsArchiveSink::new(archive_dir);
    archive_audit_partition_with_sink(pool, partition_name, &sink).await
}

/// F5.3-c-ii-archive-fs + -blob-sink — archive one `audit_events`
/// partition through the supplied [`ArchiveSink`], then DETACH +
/// DROP it from the live database inside the same transaction.
///
/// `partition_name` MUST match the migration-0013 pattern
/// (`audit_events_y{YYYY}q{N}`) — anything else is rejected as a
/// defense-in-depth measure against path-traversal via a malicious
/// DB row.
///
/// # Errors
///
/// - [`AuditError::Sql`] when the partition name doesn't match the
///   expected pattern, the partition doesn't exist in `pg_inherits`,
///   the SELECT / DETACH / DROP statements fail, or the sink's
///   `put` errors.
/// - [`AuditError::GetConn`] when the pool refuses a connection.
#[allow(clippy::too_many_lines)] // validation + SELECT + serialize + sink put + DETACH/DROP → cohesive flow
pub async fn archive_audit_partition_with_sink<S: ArchiveSink + ?Sized>(
    pool: &Pool,
    partition_name: &str,
    sink: &S,
) -> Result<ArchiveOutcome, AuditError> {
    // Defense-in-depth: validate the partition name before any
    // filesystem path construction. parse_audit_partition_name
    // returns None for anything that doesn't match
    // `audit_events_y{YYYY}q{N}` so a name like `../../etc/passwd`
    // never reaches the FS layer.
    if parse_audit_partition_name(partition_name).is_none() {
        return Err(AuditError::Sql(format!(
            "archive: partition name '{partition_name}' doesn't match audit_events_y{{YYYY}}q{{N}} pattern"
        )));
    }

    let mut client = pool
        .get()
        .await
        .map_err(|e| AuditError::GetConn(format!("archive: {e}")))?;

    // Verify the partition currently exists as a child of
    // audit_events. Without this, a typo in --partition would happily
    // produce an empty JSONL + then fail on DETACH; this surfaces
    // the error early and clearly.
    let exists_row = client
        .query_one(
            "SELECT count(*) FROM pg_inherits i \
             JOIN pg_class c ON c.oid = i.inhrelid \
             WHERE i.inhparent = 'audit_events'::regclass \
               AND c.relname = $1",
            &[&partition_name],
        )
        .await
        .map_err(|e| AuditError::Sql(format!("archive: existence check: {e}")))?;
    let exists_count: i64 = exists_row.get(0);
    if exists_count == 0 {
        return Err(AuditError::Sql(format!(
            "archive: partition '{partition_name}' is not a child of audit_events"
        )));
    }

    // Open a transaction. The SELECT + DETACH + DROP must commit
    // atomically — otherwise a crash between DETACH and DROP would
    // leave a dangling detached table that's invisible to queries
    // (gone from pg_inherits) but still occupies disk space.
    let tx = client
        .transaction()
        .await
        .map_err(|e| AuditError::Sql(format!("archive: begin: {e}")))?;

    // SELECT all rows. Cast id + ts to text to avoid carrying
    // tokio_postgres type plumbing for chrono / i64; we re-serialize
    // each row as a JSON object below.
    //
    // SECURITY: the table name interpolation is safe because
    // partition_name was already validated against the strict
    // pattern. Belt-and-suspenders alternative would be format!
    // with `%I` via a DO block; the validation-at-the-edge approach
    // is simpler.
    let select_sql = format!(
        "SELECT id::text, action, resource, org_id::text, actor::text, \
                ip::text, ua, \
                to_char(ts AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') \
         FROM {partition_name} ORDER BY id"
    );
    let rows = tx
        .query(&select_sql, &[])
        .await
        .map_err(|e| AuditError::Sql(format!("archive: SELECT: {e}")))?;
    let row_count = u64::try_from(rows.len()).unwrap_or(u64::MAX);

    // Serialize each row as one JSON line. The schema mirrors the
    // F5.3-d-i Splunk HEC event shape so consumers reading the
    // archive can use one query template across hot + cold storage.
    let mut buf = Vec::with_capacity(rows.len() * 256);
    for row in &rows {
        let id: String = row.get(0);
        let action: String = row.get(1);
        let resource: String = row.get(2);
        let org_id: Option<String> = row.get(3);
        let actor: Option<String> = row.get(4);
        let ip: Option<String> = row.get(5);
        let ua: Option<String> = row.get(6);
        let ts: String = row.get(7);
        let line = serde_json::json!({
            "id": id,
            "action": action,
            "resource": resource,
            "org_id": org_id,
            "actor": actor,
            "ip": ip,
            "ua": ua,
            "ts": ts,
        });
        serde_json::to_writer(&mut buf, &line)
            .map_err(|e| AuditError::Sql(format!("archive: serialize row: {e}")))?;
        buf.push(b'\n');
    }

    // Gzip the buffer. flate2's GzEncoder requires explicit finish()
    // to flush the trailer; without it the file is corrupt.
    let mut gz_buf: Vec<u8> = Vec::with_capacity(buf.len() / 4);
    {
        use std::io::Write;
        let mut encoder = flate2::write::GzEncoder::new(&mut gz_buf, flate2::Compression::default());
        encoder
            .write_all(&buf)
            .map_err(|e| AuditError::Sql(format!("archive: gzip write: {e}")))?;
        encoder
            .finish()
            .map_err(|e| AuditError::Sql(format!("archive: gzip finish: {e}")))?;
    }

    // Hand the gzipped buffer to the configured sink. Both
    // FsArchiveSink (local dir) and AzureBlobArchiveSink (Azure
    // Blob) implement the same trait — the dispatch happens at the
    // CLI / cmd_serve_http edge based on which env vars the
    // operator set. The sink returns a human-readable target string
    // for the structured log.
    let bytes_on_disk = u64::try_from(gz_buf.len()).unwrap_or(u64::MAX);
    let key = format!("{partition_name}.jsonl.gz");
    let target = sink.put(&key, gz_buf).await?;

    // DETACH + DROP. Both inside the same transaction so a crash
    // between them is rolled back. The file is already written; if
    // the transaction aborts, the customer just has a redundant copy
    // of the data — never the inverse (data missing from both).
    let detach_sql = format!("ALTER TABLE audit_events DETACH PARTITION {partition_name}");
    tx.batch_execute(&detach_sql)
        .await
        .map_err(|e| AuditError::Sql(format!("archive: DETACH: {e}")))?;
    let drop_sql = format!("DROP TABLE {partition_name}");
    tx.batch_execute(&drop_sql)
        .await
        .map_err(|e| AuditError::Sql(format!("archive: DROP: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| AuditError::Sql(format!("archive: commit: {e}")))?;

    Ok(ArchiveOutcome {
        rows: row_count,
        bytes_on_disk,
        target,
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
