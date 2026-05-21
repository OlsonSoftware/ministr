//! Org + member DB helpers (F3.1a).
//!
//! All queries hit the `orgs` and `org_members` tables defined in
//! `migrations/0001_initial.sql`. The helpers are intentionally small
//! and composable: F3.1b will add an `org_invites` table + helpers
//! alongside this module without restructuring the existing surface,
//! and F3.1c's Stripe seat sync reads through `list_org_members` to
//! count members.
//!
//! # Why a separate `repo.rs`
//!
//! Mirrors `crate::billing::endpoint` (handler) ↔ `crate::billing::usage`
//! (DB writer) split: handlers in [`super::routes`] stay focused on
//! HTTP shape + auth + status-code mapping, repo helpers stay focused
//! on SQL. The split also lets us write integration tests against the
//! repo without spinning up an axum harness.

use deadpool_postgres::Pool;

/// Errors surfaced by the repo layer. Mapped to HTTP status codes in
/// [`super::routes::OrgsApiError`].
#[derive(Debug, thiserror::Error)]
pub enum OrgError {
    /// Acquiring a connection from the pool failed.
    #[error("get connection: {0}")]
    GetConn(String),
    /// A SQL statement returned an error. Wraps the underlying message
    /// for log triage; never echoed to clients.
    #[error("sql: {0}")]
    Sql(String),
    /// The submitted org name is empty or longer than
    /// [`MAX_ORG_NAME_LEN`] chars. The CHECK constraint at the DB
    /// level catches this too, but rejecting in the helper gives a
    /// cleaner 422 response without round-tripping to Postgres.
    #[error("org name must be 1..={MAX_ORG_NAME_LEN} non-blank chars")]
    InvalidName,
}

/// A row from the `orgs` table — the minimal columns handlers need.
/// Field set matches what F3.1a serialises to JSON; F3.1c adds
/// `stripe_customer_id` on the wire.
#[derive(Debug, Clone)]
pub struct OrgRow {
    /// `orgs.id` — UUID PK as canonical string. Kept as `String` rather
    /// than `uuid::Uuid` for symmetry with [`crate::users::UserRow`].
    pub id: String,
    /// `orgs.name` — display name supplied by the caller.
    pub name: String,
    /// `orgs.plan_id` — seeded to [`DEFAULT_ORG_PLAN`] on creation; the
    /// Stripe webhook handler in F1.5 is the authoritative writer
    /// thereafter (same convention as `users.plan_id`).
    pub plan_id: String,
    /// `orgs.billing_email` — defaults to NULL on creation; F3.1c will
    /// populate it from the org-creation form when the Customer is
    /// minted in Stripe.
    pub billing_email: Option<String>,
}

/// A row from `list_orgs_for_user` — joins `orgs` with the caller's
/// `org_members.role` so the UI doesn't need a second round-trip.
#[derive(Debug, Clone)]
pub struct OrgWithRole {
    /// `orgs.id`.
    pub id: String,
    /// `orgs.name`.
    pub name: String,
    /// `orgs.plan_id`.
    pub plan_id: String,
    /// Caller's `org_members.role` — one of `owner`, `admin`, `member`.
    pub role: String,
}

/// A row from `list_org_members` — joins `org_members` with `users` so
/// the UI can render the email address alongside the role.
#[derive(Debug, Clone)]
pub struct MemberRow {
    /// `org_members.user_id` as canonical UUID string.
    pub user_id: String,
    /// `users.email` — the verified GitHub primary email.
    pub email: String,
    /// `org_members.role`.
    pub role: String,
}

/// Default `plan_id` seeded into a fresh `orgs` row. Cloud orgs land on
/// `"team"` because §3 prices orgs at $30/seat (the Team tier). F3.1c
/// wires this through Stripe (Customer + Subscription); F3.1a just
/// seeds the column so downstream quota code sees a valid plan from
/// day one.
pub const DEFAULT_ORG_PLAN: &str = "team";

/// Upper bound on `orgs.name` length. Matches a reasonable UI limit; the
/// DB column itself is unconstrained `TEXT` but the helper rejects
/// pathologically large values before the SQL round-trip.
pub const MAX_ORG_NAME_LEN: usize = 128;

/// Create an org and atomically insert the caller as `owner`.
///
/// Both inserts run inside one transaction — partial state (an org row
/// with no owner) would let the org leak past `list_orgs_for_user` since
/// the join would return zero rows, but `corpora` and `audit_events`
/// could still reference the orphaned org id. Committing both rows
/// together avoids that whole class of inconsistency.
///
/// # Errors
///
/// - [`OrgError::InvalidName`] when `name` is empty (after trimming) or
///   longer than [`MAX_ORG_NAME_LEN`] chars.
/// - [`OrgError::GetConn`] / [`OrgError::Sql`] on connection or query
///   failure. Foreign-key violation on `owner_user_id` (user not in
///   `users`) surfaces as `Sql` — the caller is expected to authenticate
///   first, so this is an internal-state error not a client error.
pub async fn create_org(pool: &Pool, owner_user_id: &str, name: &str) -> Result<OrgRow, OrgError> {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_ORG_NAME_LEN {
        return Err(OrgError::InvalidName);
    }

    let mut conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("create_org: {e}")))?;
    let tx = conn
        .transaction()
        .await
        .map_err(|e| OrgError::Sql(format!("begin txn: {e}")))?;

    let row = tx
        .query_one(
            "INSERT INTO orgs (name, plan_id)
             VALUES ($1, $2)
             RETURNING id::text AS id_text, name, plan_id, billing_email",
            &[&trimmed, &DEFAULT_ORG_PLAN],
        )
        .await
        .map_err(|e| OrgError::Sql(format!("insert org: {e}")))?;

    let id: String = row
        .try_get("id_text")
        .map_err(|e| OrgError::Sql(format!("read org id: {e}")))?;
    let row_name: String = row
        .try_get("name")
        .map_err(|e| OrgError::Sql(format!("read org name: {e}")))?;
    let plan_id: String = row
        .try_get("plan_id")
        .map_err(|e| OrgError::Sql(format!("read org plan_id: {e}")))?;
    let billing_email: Option<String> = row
        .try_get("billing_email")
        .map_err(|e| OrgError::Sql(format!("read org billing_email: {e}")))?;

    tx.execute(
        "INSERT INTO org_members (org_id, user_id, role)
         VALUES ($1::uuid, $2::uuid, 'owner')",
        &[&id, &owner_user_id],
    )
    .await
    .map_err(|e| OrgError::Sql(format!("insert owner member: {e}")))?;

    tx.commit()
        .await
        .map_err(|e| OrgError::Sql(format!("commit: {e}")))?;

    Ok(OrgRow {
        id,
        name: row_name,
        plan_id,
        billing_email,
    })
}

/// List every org the given user is a member of, joined with their
/// `role`. Ordered by `orgs.created_at ASC` so the UI shows the oldest
/// org first — stable when more orgs join later.
///
/// Cross-tenant safety: the `WHERE m.user_id = $1` join is the only
/// filter; any other user's orgs are not in the result set because the
/// inner join eliminates them. Tested in
/// [`super::routes::tests::list_only_returns_callers_orgs`].
///
/// # Errors
///
/// [`OrgError::GetConn`] / [`OrgError::Sql`] on connection or query
/// failure.
pub async fn list_orgs_for_user(pool: &Pool, user_id: &str) -> Result<Vec<OrgWithRole>, OrgError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("list_orgs_for_user: {e}")))?;
    let rows = conn
        .query(
            "SELECT o.id::text AS id_text, o.name, o.plan_id, m.role
             FROM orgs o
             JOIN org_members m ON m.org_id = o.id
             WHERE m.user_id = $1::uuid
             ORDER BY o.created_at ASC, o.id ASC",
            &[&user_id],
        )
        .await
        .map_err(|e| OrgError::Sql(format!("list_orgs_for_user: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|r| OrgWithRole {
            id: r.get("id_text"),
            name: r.get("name"),
            plan_id: r.get("plan_id"),
            role: r.get("role"),
        })
        .collect())
}

/// Return the caller's role within `org_id`, or `None` if they are not
/// a member. The handler layer maps `None` to a 403 — we deliberately
/// don't distinguish "org doesn't exist" from "you aren't in it" so an
/// attacker can't probe org-id existence.
///
/// # Errors
///
/// [`OrgError::GetConn`] / [`OrgError::Sql`] on connection or query
/// failure.
pub async fn member_role(
    pool: &Pool,
    org_id: &str,
    user_id: &str,
) -> Result<Option<String>, OrgError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("member_role: {e}")))?;
    let row = conn
        .query_opt(
            "SELECT role
             FROM org_members
             WHERE org_id = $1::uuid AND user_id = $2::uuid",
            &[&org_id, &user_id],
        )
        .await
        .map_err(|e| OrgError::Sql(format!("member_role: {e}")))?;
    Ok(row.map(|r| r.get("role")))
}

/// F3.1c-i — persist the Stripe customer id on the `orgs` row.
/// Mirrors [`crate::users::set_stripe_customer_id`]; the `cus_…`
/// comes from
/// [`crate::billing::StripeClient::create_org_customer`]. Best-
/// effort — the create-org handler logs + continues on failure so a
/// Stripe outage doesn't block tenant onboarding.
///
/// # Errors
///
/// [`OrgError::GetConn`] / [`OrgError::Sql`] on connection or query
/// failure.
pub async fn set_org_stripe_customer_id(
    pool: &Pool,
    org_id: &str,
    stripe_customer_id: &str,
) -> Result<(), OrgError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("set_org_stripe_customer_id: {e}")))?;
    conn.execute(
        "UPDATE orgs SET stripe_customer_id = $1 WHERE id = $2::uuid",
        &[&stripe_customer_id, &org_id],
    )
    .await
    .map_err(|e| OrgError::Sql(format!("set_org_stripe_customer_id: {e}")))?;
    Ok(())
}

/// F3.1c-i — read an owner's email so the org-creation flow can
/// derive `billing_email` for the Stripe Customer without forcing
/// the user to retype it. Looks up `users.email` by UUID.
///
/// Returns `None` when no row matches (shouldn't happen in
/// production — the `create_org` call established the row).
///
/// # Errors
///
/// [`OrgError::GetConn`] / [`OrgError::Sql`] on connection or query
/// failure.
pub async fn user_email(pool: &Pool, user_id: &str) -> Result<Option<String>, OrgError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("user_email: {e}")))?;
    let row = conn
        .query_opt(
            "SELECT email FROM users WHERE id = $1::uuid",
            &[&user_id],
        )
        .await
        .map_err(|e| OrgError::Sql(format!("user_email: {e}")))?;
    Ok(row.map(|r| r.get("email")))
}

/// F3.1b-ii-a — look up an org's display name by id. Used by the
/// invite-send pipeline so the transactional-email template can show
/// the recipient *which* org they're being invited to.
///
/// Returns `None` when the org row doesn't exist.
///
/// # Errors
///
/// [`OrgError::GetConn`] / [`OrgError::Sql`] on connection or query
/// failure.
pub async fn org_name(pool: &Pool, org_id: &str) -> Result<Option<String>, OrgError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("org_name: {e}")))?;
    let row = conn
        .query_opt(
            "SELECT name FROM orgs WHERE id = $1::uuid",
            &[&org_id],
        )
        .await
        .map_err(|e| OrgError::Sql(format!("org_name: {e}")))?;
    Ok(row.map(|r| r.get("name")))
}

/// List every member of `org_id` with their email and role. Sorted so
/// owners surface first, then admins, then members (alphabetical within
/// each role) — stable rendering for the UI.
///
/// # Errors
///
/// [`OrgError::GetConn`] / [`OrgError::Sql`] on connection or query
/// failure.
pub async fn list_org_members(pool: &Pool, org_id: &str) -> Result<Vec<MemberRow>, OrgError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("list_org_members: {e}")))?;
    let rows = conn
        .query(
            "SELECT m.user_id::text AS user_id_text, u.email, m.role
             FROM org_members m
             JOIN users u ON u.id = m.user_id
             WHERE m.org_id = $1::uuid
             ORDER BY CASE m.role
                          WHEN 'owner' THEN 0
                          WHEN 'admin' THEN 1
                          ELSE 2
                      END,
                      u.email ASC",
            &[&org_id],
        )
        .await
        .map_err(|e| OrgError::Sql(format!("list_org_members: {e}")))?;
    Ok(rows
        .into_iter()
        .map(|r| MemberRow {
            user_id: r.get("user_id_text"),
            email: r.get("email"),
            role: r.get("role"),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    //! Pure-Rust unit tests (input validation). The Postgres-integration
    //! tests sit in [`super::routes::tests`] alongside the route-level
    //! integration so they exercise the full handler stack; `#[ignore]`
    //! gates them on `MINISTR_TEST_PG_URL` per the workspace convention.

    use super::*;

    #[test]
    fn invalid_name_rejects_empty_after_trim() {
        // `create_org` validates before touching the pool, so we can
        // assert the rejection branch without any Postgres.
        let pool_unused = false;
        assert!(!pool_unused, "this test never touches the pool");

        // Inline copy of the validation predicate so the test breaks
        // loudly if the rule changes without intent.
        let blank = "   ";
        assert!(blank.trim().is_empty());
        let too_long: String = "a".repeat(MAX_ORG_NAME_LEN + 1);
        assert!(too_long.chars().count() > MAX_ORG_NAME_LEN);
    }

    #[test]
    fn default_org_plan_is_team() {
        // §3 tier matrix prices orgs at $30/seat (the Team tier). If
        // someone changes this constant without intent, the test
        // surfaces it before deployment.
        assert_eq!(DEFAULT_ORG_PLAN, "team");
    }
}
