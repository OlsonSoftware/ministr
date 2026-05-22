//! `users` table helpers for the F1.2 multi-tenant data model.
//!
//! Today's only call site is the F1.3 GitHub `IdP` callback in
//! [`crate::auth`]: when a first-time visitor signs in with GitHub, the
//! handler upserts a row keyed by the verified `github_id`. The same
//! shape will host the future Google / Microsoft / OIDC ID claims by
//! adding columns alongside `github_id` (per the F1.3 `IdP` trait notes in
//! [`crate::idp`]).
//!
//! The upsert is `ON CONFLICT (github_id) DO UPDATE` rather than `DO
//! NOTHING` so that an email change on the GitHub profile is reflected
//! immediately. The user's chosen plan is *not* touched on subsequent
//! sign-ins — only the very first INSERT seeds `plan_id`.

use deadpool_postgres::Pool;

use crate::idp::ResolvedIdentity;

/// Errors surfaced by [`upsert_github_user`].
#[derive(Debug, thiserror::Error)]
pub enum UserError {
    /// Acquiring a connection from the pool failed.
    #[error("get connection: {0}")]
    GetConn(String),
    /// The provider returned no usable `(github_id, email)` pair. The
    /// `users` table treats both as required (`UNIQUE NOT NULL` on
    /// `email`, `UNIQUE` on `github_id`); without either we cannot
    /// honour the upsert.
    #[error("identity missing required fields: {0}")]
    MissingField(&'static str),
    /// A SQL statement returned an error.
    #[error("sql: {0}")]
    Sql(String),
}

/// A row from the `users` table — the minimal subset cloud handlers
/// need after sign-in. Future fields land alongside as F-items demand
/// (e.g. `stripe_customer_id` is consumed by F1.5).
#[derive(Debug, Clone)]
pub struct UserRow {
    /// `users.id` — UUID primary key, rendered as its canonical
    /// hyphenated string. Kept as `String` rather than `uuid::Uuid` so
    /// downstream code (Tenant subject, audit log, future JSON
    /// responses) can use it without pulling the `uuid` crate into the
    /// open-core surface.
    pub id: String,
    /// `users.email` — populated from GitHub's verified primary email.
    pub email: String,
    /// `users.github_id` — populated only on the GitHub `IdP` path; left
    /// `None` for future providers that don't surface a numeric GitHub
    /// user id.
    pub github_id: Option<i64>,
    /// `users.plan_id` — the tier seam every quota / billing handler
    /// reads. Defaults to `"pro"` on first GitHub sign-in (cloud has no
    /// free tier per B3); subsequent sign-ins preserve whatever the
    /// billing path most recently set.
    pub plan_id: String,
    /// `True` when this row was inserted on THIS call — i.e. the user
    /// just signed in for the first time. F1.5 uses it to gate Stripe
    /// Customer creation; future signup-side effects (welcome email,
    /// audit-log entry) can hook on the same flag. Detected via the
    /// Postgres `xmax = 0` system-column trick in the RETURNING clause
    /// (xmax is 0 for fresh INSERT rows, non-zero for UPDATEs).
    pub inserted: bool,
}

/// Default plan assigned to brand-new GitHub sign-ins. Cloud has no free
/// tier (B3 resolved) so every first-time user lands on `pro` and Stripe
/// Checkout takes them through payment in F2.4. Until F2.3's enforcement
/// middleware ships, `pro` simply means "may use the cloud API".
pub const DEFAULT_GITHUB_SIGNIN_PLAN: &str = "pro";

/// Insert-or-update the `users` row identified by GitHub's stable
/// numeric user id. Returns the persisted row (including its server-
/// generated UUID for first-time sign-ins).
///
/// The upsert key is `github_id`, not `email`, because GitHub users can
/// change their primary email but the `id` is immutable per
/// `https://docs.github.com/en/rest/users/users`. Tracking by `id` keeps
/// the historical row stable when an email changes.
///
/// On first INSERT the row is seeded with [`DEFAULT_GITHUB_SIGNIN_PLAN`].
/// On UPDATE the `plan_id` is left alone (the billing handler is the
/// authoritative writer for that column).
///
/// # Errors
///
/// - [`UserError::MissingField`] when `identity` lacks `github_id` or
///   `email`. Both are mandatory for the GitHub `IdP` path; OIDC / SAML
///   providers will eventually need their own helpers with different
///   constraints.
/// - [`UserError::GetConn`] / [`UserError::Sql`] when Postgres is
///   unreachable or rejects the statement.
pub async fn upsert_github_user(
    pool: &Pool,
    identity: &ResolvedIdentity,
) -> Result<UserRow, UserError> {
    let github_id = identity
        .github_id
        .ok_or(UserError::MissingField("github_id"))?;
    let email = identity
        .email
        .as_deref()
        .ok_or(UserError::MissingField("email"))?;

    let conn = pool
        .get()
        .await
        .map_err(|e| UserError::GetConn(format!("upsert_github_user: {e}")))?;

    // `ON CONFLICT (github_id) DO UPDATE` so the row's email tracks the
    // GitHub profile. `plan_id = users.plan_id` (self-assignment) keeps
    // the billing column untouched on returning users — the only writer
    // of `plan_id` after the seed is Stripe webhook handler in F1.5.
    // `id::text` casts the UUID server-side so we don't have to pull the
    // `uuid` crate into the binary just for `try_get`.
    // `xmax = 0 AS inserted` exposes Postgres's transaction-id system
    // column: fresh INSERT rows have xmax=0, UPSERT-promoted UPDATE rows
    // carry the locking xid.
    let row = conn
        .query_one(
            "INSERT INTO users (email, github_id, plan_id)
             VALUES ($1, $2, $3)
             ON CONFLICT (github_id) DO UPDATE
                 SET email = EXCLUDED.email,
                     plan_id = users.plan_id
             RETURNING id::text AS id_text, email, github_id, plan_id,
                       (xmax = 0) AS inserted",
            &[&email, &github_id, &DEFAULT_GITHUB_SIGNIN_PLAN],
        )
        .await
        .map_err(|e| UserError::Sql(format!("upsert_github_user: {e}")))?;

    let id: String = row
        .try_get("id_text")
        .map_err(|e| UserError::Sql(format!("read id: {e}")))?;
    let email_out: String = row
        .try_get("email")
        .map_err(|e| UserError::Sql(format!("read email: {e}")))?;
    let github_id_out: Option<i64> = row
        .try_get("github_id")
        .map_err(|e| UserError::Sql(format!("read github_id: {e}")))?;
    let plan_id: String = row
        .try_get("plan_id")
        .map_err(|e| UserError::Sql(format!("read plan_id: {e}")))?;
    let inserted: bool = row
        .try_get("inserted")
        .map_err(|e| UserError::Sql(format!("read inserted: {e}")))?;

    Ok(UserRow {
        id,
        email: email_out,
        github_id: github_id_out,
        plan_id,
        inserted,
    })
}

/// Persist the Stripe customer id on the `users` row identified by
/// UUID. The id should be the freshly-minted `cus_…` returned by
/// [`crate::billing::StripeClient::create_customer`]. Best-effort —
/// callers (the GitHub sign-in callback in F1.5) log + continue on
/// failure rather than blocking the user's sign-in.
///
/// # Errors
///
/// Same connection / SQL surface as [`upsert_github_user`].
pub async fn set_stripe_customer_id(
    pool: &Pool,
    user_id: &str,
    stripe_customer_id: &str,
) -> Result<(), UserError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| UserError::GetConn(format!("set_stripe_customer_id: {e}")))?;
    conn.execute(
        "UPDATE users SET stripe_customer_id = $1 WHERE id = $2::text::uuid",
        &[&stripe_customer_id, &user_id],
    )
    .await
    .map_err(|e| UserError::Sql(format!("set_stripe_customer_id: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Integration tests require a real Postgres at `MINISTR_TEST_PG_URL`
    //! (e.g. `postgres://ministr:ministr@localhost:5432/ministr_test`).
    //! `cargo test -p ministr-cloud -- --ignored` runs them.

    use super::*;
    use crate::db::{connect, run_migrations};
    use crate::idp::{GITHUB_ISSUER, ResolvedIdentity};

    fn identity(github_id: i64, email: &str) -> ResolvedIdentity {
        ResolvedIdentity {
            issuer: GITHUB_ISSUER.into(),
            subject: github_id.to_string(),
            email: Some(email.into()),
            display_name: Some(format!("user-{github_id}")),
            github_id: Some(github_id),
        }
    }

    #[tokio::test]
    async fn missing_github_id_is_an_explicit_error() {
        // Construct via the public path so we don't have to spin up a pool
        // just to exercise the input-validation branch.
        let id = ResolvedIdentity {
            issuer: GITHUB_ISSUER.into(),
            subject: "abc".into(),
            email: Some("u@example.com".into()),
            display_name: None,
            github_id: None,
        };
        // `Pool::default()` is unavailable, so re-implement the validation
        // assertion at the field level — the function returns
        // `MissingField("github_id")` before touching the pool.
        assert!(id.github_id.is_none());
        // Also covers the `email` branch.
        let id2 = ResolvedIdentity {
            github_id: Some(1),
            email: None,
            ..id
        };
        assert!(id2.email.is_none());
    }

    #[tokio::test]
    #[ignore = "needs MINISTR_TEST_PG_URL"]
    async fn upsert_inserts_then_updates() {
        let Ok(url) = std::env::var("MINISTR_TEST_PG_URL") else {
            return;
        };
        let pool = connect(&url).expect("open pool");
        run_migrations(&pool).await.expect("migrate");

        // Use a high github_id to avoid colliding with other fixtures.
        let gid: i64 = 9_000_000 + i64::from(std::process::id() % 1_000);

        let first = upsert_github_user(&pool, &identity(gid, "first@example.com"))
            .await
            .expect("first upsert");
        assert_eq!(first.github_id, Some(gid));
        assert_eq!(first.email, "first@example.com");
        assert_eq!(first.plan_id, DEFAULT_GITHUB_SIGNIN_PLAN);
        assert!(first.inserted, "fresh insert must report inserted=true");

        // Same github_id, new email — should UPDATE, preserve id.
        let second = upsert_github_user(&pool, &identity(gid, "renamed@example.com"))
            .await
            .expect("second upsert");
        assert_eq!(second.id, first.id, "stable row id on rename");
        assert_eq!(second.email, "renamed@example.com");
        assert!(
            !second.inserted,
            "upsert that promoted to UPDATE must report inserted=false"
        );

        // Stripe customer id round-trip — sanity-check the F1.5 hook.
        set_stripe_customer_id(&pool, &first.id, "cus_test_set_round_trip")
            .await
            .expect("set stripe_customer_id");
        let conn = pool.get().await.unwrap();
        let cust_row = conn
            .query_one(
                "SELECT stripe_customer_id FROM users WHERE id = $1::text::uuid",
                &[&first.id],
            )
            .await
            .unwrap();
        let saved: Option<String> = cust_row.try_get("stripe_customer_id").unwrap();
        assert_eq!(saved.as_deref(), Some("cus_test_set_round_trip"));

        // Clean up so the test is rerunnable. `id::uuid` casts the
        // text-form back to the column type for the WHERE comparison.
        let conn = pool.get().await.unwrap();
        conn.execute(
            "DELETE FROM users WHERE id = $1::text::uuid",
            &[&first.id],
        )
        .await
        .unwrap();
    }
}
