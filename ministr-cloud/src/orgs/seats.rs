//! F3.1c-ii — keep Stripe subscription seat quantity in sync with
//! `count(org_members)`.
//!
//! Called from every event that mutates `org_members`:
//!
//! - Org creation (`routes::create_handler` post `create_org`) — the
//!   owner is seat #1. In practice the org has no subscription yet
//!   (Checkout happens later), so the sync resolves to
//!   [`SyncSeatOutcome::NoSubscription`] and noops.
//! - Invite acceptance (`auth::github_signin::handle_github_callback`
//!   after `orgs::consume_invite` returns `Accepted`). Once the
//!   owner has run Checkout, every accepted invite bumps the
//!   subscription's seat quantity to the new member count.
//!
//! Future remove events (F3.2 corpus ACL drop-member, F3.4 service-
//! account revoke) plug into the same helper.
//!
//! # Idempotency
//!
//! We compute `target_quantity = count(org_members)` and ask Stripe
//! to set the line item to that absolute value. Concurrent invite
//! accepts race only on which one issues the last update — both
//! observe the eventually-correct count. The Stripe API's
//! idempotency key on the update side
//! (`sync-seats-{sub_id}-q{N}`) makes a retried sync against the
//! same target a no-op at the wire layer too.

use deadpool_postgres::Pool;

use super::repo::OrgError;
use crate::billing::{StripeClient, SyncSeatOutcome};

/// Read `count(org_members)` for `org_id`.
///
/// # Errors
///
/// [`OrgError::GetConn`] / [`OrgError::Sql`] on connection or query
/// failure.
pub async fn count_org_members(pool: &Pool, org_id: &str) -> Result<u64, OrgError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("count_org_members: {e}")))?;
    let row = conn
        .query_one(
            "SELECT COUNT(*)::bigint AS n FROM org_members WHERE org_id = $1::text::uuid",
            &[&org_id],
        )
        .await
        .map_err(|e| OrgError::Sql(format!("count_org_members: {e}")))?;
    let n: i64 = row.get("n");
    Ok(u64::try_from(n.max(0)).unwrap_or(0))
}

/// F3.1c-ii — orchestrate a one-shot Stripe seat sync for the org.
///
/// Looks up `orgs.stripe_customer_id` (NULL = pre-F3.1c-i row, no
/// Stripe Customer wired) and `count(org_members)`, then calls
/// [`StripeClient::sync_subscription_seats`]. Returns the wire-level
/// [`SyncSeatOutcome`] when Stripe was consulted, or
/// [`SeatsSyncOutcome::NoCustomer`] when the org has no Customer yet
/// (sync is meaningless; the F3.1c-iv backfill job will catch up).
///
/// Caller pattern (best-effort): wrap in `match … { Err(e) =>
/// warn!(error = %e, "seat sync failed"); _ => () }` — a Stripe
/// outage must not unwind the membership-add operation.
///
/// # Errors
///
/// - [`SeatsSyncError::Repo`] on DB failure.
/// - [`SeatsSyncError::Stripe`] on Stripe API failure.
pub async fn sync_org_seats(
    pool: &Pool,
    stripe: &StripeClient,
    org_id: &str,
) -> Result<SeatsSyncOutcome, SeatsSyncError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| SeatsSyncError::Repo(OrgError::GetConn(format!("sync_org_seats: {e}"))))?;
    let row = conn
        .query_one(
            "SELECT stripe_customer_id FROM orgs WHERE id = $1::text::uuid",
            &[&org_id],
        )
        .await
        .map_err(|e| SeatsSyncError::Repo(OrgError::Sql(format!("sync_org_seats lookup: {e}"))))?;
    let cus_id: Option<String> = row.get("stripe_customer_id");
    drop(conn);

    let Some(cus_id) = cus_id else {
        return Ok(SeatsSyncOutcome::NoCustomer);
    };

    let target = count_org_members(pool, org_id)
        .await
        .map_err(SeatsSyncError::Repo)?;

    let stripe_outcome = stripe
        .sync_subscription_seats(&cus_id, target)
        .await
        .map_err(SeatsSyncError::Stripe)?;
    Ok(SeatsSyncOutcome::Synced {
        target_quantity: target,
        stripe: stripe_outcome,
    })
}

/// Result of [`sync_org_seats`]. Distinguishes "no Stripe Customer
/// yet" (pre-F3.1c-i org or Stripe not configured) from "we asked
/// Stripe and here's what happened".
#[derive(Debug, Clone)]
pub enum SeatsSyncOutcome {
    /// Org has no `stripe_customer_id` — typically a pre-F3.1c-i
    /// row, or a deployment without `MINISTR_STRIPE_SECRET_KEY`.
    /// Treated as a noop by callers; F3.1c-iv backfill catches up
    /// the historical rows.
    NoCustomer,
    /// Stripe was consulted. `target_quantity` is what we asked for
    /// (== `count(org_members)`); `stripe` is the wire-level outcome.
    Synced {
        target_quantity: u64,
        stripe: SyncSeatOutcome,
    },
}

/// Errors surfaced by [`sync_org_seats`]. The two arms preserve the
/// underlying error type so handlers can log the actual failure
/// without losing context.
#[derive(Debug, thiserror::Error)]
pub enum SeatsSyncError {
    /// Postgres lookup of `orgs.stripe_customer_id` or member count
    /// failed.
    #[error("repo: {0}")]
    Repo(OrgError),
    /// Stripe API call (list subscriptions or update) failed.
    #[error("stripe: {0}")]
    Stripe(#[from] crate::billing::StripeApiError),
}

#[cfg(test)]
mod tests {
    //! Pure-Rust shape checks. Postgres + Stripe integration is
    //! covered indirectly by the existing routes-level tests; a
    //! dedicated end-to-end seat-sync test would need a Stripe
    //! mock plus the existing `MINISTR_TEST_PG_URL` gate.

    use super::*;

    #[test]
    fn outcome_no_customer_is_clone() {
        // The handler logs this outcome; ensure it stays cheap to
        // pass around (Clone, no allocations beyond the variant tag).
        let _ = SeatsSyncOutcome::NoCustomer.clone();
    }
}
