//! F5.4-e-audit-db — DB-backed mirror of the F5.4-e-audit JSONL
//! mint log.
//!
//! The JSONL log is operator-local-by-design: each operator's
//! `mint-license --audit-log PATH` appends to their own file. Great
//! single-host. Doesn't compose when several operators issue
//! licenses from different machines — each one's history is invisible
//! to the others.
//!
//! This module ships the dual-write story: when the operator opts
//! in via `mint-license --pg-url URL` (or `MINISTR_PG_URL` env var),
//! every successful mint also lands one row in `license_issuances`.
//! `list-licenses --pg-url URL` reads from the unified DB instead
//! of the local JSONL. Both backends carry identical fields so the
//! human-readable table view renders the same regardless of source.
//!
//! Idempotency: `persist_issuance` uses `INSERT ... ON CONFLICT
//! DO NOTHING` against the table's `UNIQUE(jwt_id_hash)` constraint.
//! An operator re-running mint-license after a transient blip is
//! a no-op on the DB side; the second JSONL line might land (no
//! UNIQUE there) but `list-licenses` from PG sees one row, not two.

use deadpool_postgres::Pool;

/// One row in `license_issuances` — mirrors the JSONL field shape.
/// Sourced from the same `LicenseClaims` + `valid_days` + `jwt`
/// inputs the JSONL writer consumes, so the two backends never drift.
#[derive(Debug, Clone)]
pub struct LicenseIssuance {
    /// ISO-8601 UTC timestamp of the mint moment.
    pub ts_iso: String,
    /// Unix-seconds timestamp (same moment as `ts_iso`).
    pub ts_unix: u64,
    /// Human-readable customer label.
    pub enterprise_id: String,
    /// Seat-cap stamped on the JWT.
    pub seat_count: u32,
    /// `--valid-days` arg the operator passed.
    pub valid_days: u32,
    /// Unix-seconds JWT expiry (now + `valid_days * 86400`).
    pub exp: u64,
    /// First 16 hex of `sha256(jwt)` — same as
    /// [`crate::license::license_jwt_id_hash`].
    pub jwt_id_hash: String,
}

/// Errors surfaced by [`persist_issuance`] + [`list_issuances`].
#[derive(Debug, thiserror::Error)]
pub enum LicenseIssuanceError {
    /// Could not acquire a connection from the pool.
    #[error("license issuance get_conn: {0}")]
    GetConn(String),
    /// The statement was rejected by the backend.
    #[error("license issuance sql: {0}")]
    Sql(String),
}

/// F5.4-e-audit-db — append one row to `license_issuances`.
///
/// Idempotent on `jwt_id_hash` via `ON CONFLICT DO NOTHING` —
/// re-running `mint-license` against the same JWT is a no-op,
/// which is what the operator wants under retry. Returns `true`
/// when a new row landed, `false` when the hash was already there
/// (caller can decide whether to log "duplicate" vs "fresh").
///
/// # Errors
///
/// Returns [`LicenseIssuanceError::GetConn`] when the pool can't
/// hand out a connection, [`LicenseIssuanceError::Sql`] when the
/// INSERT fails for any reason other than the UNIQUE violation
/// (which is silently absorbed).
pub async fn persist_issuance(
    pool: &Pool,
    issuance: &LicenseIssuance,
) -> Result<bool, LicenseIssuanceError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| LicenseIssuanceError::GetConn(e.to_string()))?;
    // INTEGER fits up to ~21M for seat_count and ~5800 days
    // (~16 years) for valid_days — comfortably more than any
    // realistic license.
    let seat_i32 = i32::try_from(issuance.seat_count).unwrap_or(i32::MAX);
    let valid_i32 = i32::try_from(issuance.valid_days).unwrap_or(i32::MAX);
    let ts_i64 = i64::try_from(issuance.ts_unix).unwrap_or(i64::MAX);
    let exp_i64 = i64::try_from(issuance.exp).unwrap_or(i64::MAX);
    let rows = conn
        .execute(
            "INSERT INTO license_issuances
                 (ts_unix, ts_iso, enterprise_id, seat_count, valid_days, exp, jwt_id_hash)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (jwt_id_hash) DO NOTHING",
            &[
                &ts_i64,
                &issuance.ts_iso,
                &issuance.enterprise_id,
                &seat_i32,
                &valid_i32,
                &exp_i64,
                &issuance.jwt_id_hash,
            ],
        )
        .await
        .map_err(|e| LicenseIssuanceError::Sql(e.to_string()))?;
    // execute() returns the affected row count — 1 if INSERTed, 0 if
    // ON CONFLICT triggered (duplicate hash).
    Ok(rows == 1)
}

/// F5.4-e-audit-db — read all issuances, sorted most-recent first.
/// Returns `Vec<LicenseIssuance>` matching the JSONL list-view shape
/// so the CLI renders identically regardless of source.
///
/// `limit` of `None` means no LIMIT clause (return all rows);
/// `Some(n)` caps at n. Operator dashboards typically pass `Some(100)`
/// or `Some(1000)`; CLI defaults to all.
///
/// # Errors
///
/// Returns [`LicenseIssuanceError::GetConn`] / `Sql` on failure.
pub async fn list_issuances(
    pool: &Pool,
    limit: Option<i64>,
) -> Result<Vec<LicenseIssuance>, LicenseIssuanceError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| LicenseIssuanceError::GetConn(e.to_string()))?;
    // Construct the query with optional LIMIT — keeps a single
    // prepared statement per shape.
    let rows = if let Some(n) = limit {
        conn.query(
            "SELECT ts_unix, ts_iso, enterprise_id, seat_count, valid_days, exp, jwt_id_hash
             FROM license_issuances
             ORDER BY ts_unix DESC
             LIMIT $1",
            &[&n],
        )
        .await
    } else {
        conn.query(
            "SELECT ts_unix, ts_iso, enterprise_id, seat_count, valid_days, exp, jwt_id_hash
             FROM license_issuances
             ORDER BY ts_unix DESC",
            &[],
        )
        .await
    }
    .map_err(|e| LicenseIssuanceError::Sql(e.to_string()))?;
    let out = rows
        .into_iter()
        .map(|row| {
            let ts_i64: i64 = row.get("ts_unix");
            let seat_i32: i32 = row.get("seat_count");
            let valid_i32: i32 = row.get("valid_days");
            let exp_i64: i64 = row.get("exp");
            LicenseIssuance {
                ts_unix: u64::try_from(ts_i64).unwrap_or(0),
                ts_iso: row.get("ts_iso"),
                enterprise_id: row.get("enterprise_id"),
                seat_count: u32::try_from(seat_i32).unwrap_or(0),
                valid_days: u32::try_from(valid_i32).unwrap_or(0),
                exp: u64::try_from(exp_i64).unwrap_or(0),
                jwt_id_hash: row.get::<_, String>("jwt_id_hash").trim().to_string(),
            }
        })
        .collect();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{connect, run_migrations};

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn persist_then_list_round_trips() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let pool = connect(&url).expect("open pool");
        run_migrations(&pool).await.expect("migrate");

        // Unique-per-test-run hash so concurrent CI invocations don't
        // collide on the UNIQUE constraint.
        let hash = format!("{:016x}", std::process::id());
        let issuance = LicenseIssuance {
            ts_iso: "2026-05-23T10:00:00Z".to_string(),
            ts_unix: 1_779_522_000,
            enterprise_id: "test-acme".to_string(),
            seat_count: 25,
            valid_days: 365,
            exp: 1_811_058_000,
            jwt_id_hash: hash.clone(),
        };

        let inserted = persist_issuance(&pool, &issuance)
            .await
            .expect("persist");
        assert!(inserted, "first persist should INSERT");

        // Second call with same hash is a no-op (idempotency).
        let again = persist_issuance(&pool, &issuance)
            .await
            .expect("persist again");
        assert!(!again, "second persist should be a no-op ON CONFLICT");

        // Read back.
        let rows = list_issuances(&pool, Some(100)).await.expect("list");
        let matched = rows.iter().find(|r| r.jwt_id_hash == hash);
        let found = matched.expect("our row is in the list");
        assert_eq!(found.enterprise_id, "test-acme");
        assert_eq!(found.seat_count, 25);
        assert_eq!(found.valid_days, 365);

        // Clean up.
        let conn = pool.get().await.expect("get conn");
        conn.execute(
            "DELETE FROM license_issuances WHERE jwt_id_hash = $1",
            &[&hash],
        )
        .await
        .expect("cleanup");
    }
}
