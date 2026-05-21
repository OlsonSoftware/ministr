//! F3.1c-iii — transfer a user's personal Pro subscription out so they
//! can run Checkout for a Team subscription on the org's Stripe Customer.
//!
//! Stripe doesn't support cross-Customer subscription transfers, so the
//! flow is cancel-then-recreate:
//!
//! 1. This orchestrator looks up the user's `stripe_customer_id`,
//!    finds their active subscription, and cancels it via
//!    [`StripeClient::cancel_active_subscription_for_customer`].
//! 2. The caller (typically the Tauri "Convert to Team org" UI) then
//!    redirects the owner to the existing F2.4 Checkout flow against
//!    `orgs.stripe_customer_id`, where they mint a Team subscription
//!    seated for the org's current member count.
//!
//! The proration credit Stripe issues on cancel means the user is
//! refunded for the unused portion of their Pro period; the new Team
//! sub's invoice will net against that credit when finalised.
//!
//! # Idempotency
//!
//! Re-calling the orchestrator on a user whose personal sub is already
//! cancelled is a no-op: the Stripe list step returns no active subs
//! and the orchestrator returns [`TransferPersonalOutcome::NoActiveSubscription`].
//! Callers can safely retry on transient failures.
//!
//! # Auth scope
//!
//! The orchestrator does NOT verify org membership — that's a route-
//! handler concern (see `routes::transfer_personal_handler`). This
//! module is the wire-shape + Stripe-orchestration layer; the route
//! enforces "caller must be owner/admin of the target org".

use deadpool_postgres::Pool;

use super::repo::OrgError;
use crate::billing::{CancelSubscriptionOutcome, StripeClient};

/// Result of [`transfer_personal_to_org`]. Distinguishes three states:
/// no Customer (Stripe not configured for the user yet), no active sub
/// to cancel (idempotent re-call or already-Team user), or successful
/// cancel.
#[derive(Debug, Clone)]
pub enum TransferPersonalOutcome {
    /// User has no `stripe_customer_id`. Typically the user signed in
    /// against a deployment without `MINISTR_STRIPE_SECRET_KEY` — no
    /// Stripe state to mutate. UI surfaces this as "no Stripe setup
    /// detected".
    NoPersonalCustomer,
    /// Customer exists but has no active subscription. Either the user
    /// never ran personal Pro Checkout, or they already transferred.
    /// Idempotent re-call lands here.
    NoActiveSubscription,
    /// Personal subscription cancelled. The named `subscription_id` is
    /// now `canceled` in Stripe; the user can immediately run Checkout
    /// for the org's Team plan without double-billing.
    Cancelled { subscription_id: String },
}

/// Errors surfaced by [`transfer_personal_to_org`]. Mirrors the
/// [`super::seats::SeatsSyncError`] taxonomy: separate variants for
/// DB and Stripe so handlers can log the actual failure context.
#[derive(Debug, thiserror::Error)]
pub enum TransferPersonalError {
    /// Postgres lookup of `users.stripe_customer_id` failed.
    #[error("repo: {0}")]
    Repo(OrgError),
    /// Stripe API call (list-active or cancel) failed.
    #[error("stripe: {0}")]
    Stripe(#[from] crate::billing::StripeApiError),
}

/// F3.1c-iii — cancel `user_id`'s personal Pro subscription in
/// preparation for them moving to a Team subscription on `org_id`'s
/// Stripe Customer.
///
/// The `org_id` parameter is informational here — it's used only for
/// the eventual audit row context; ownership of the org is verified
/// by the route handler before calling this. Passing the wrong org
/// id won't change Stripe state.
///
/// # Errors
///
/// - [`TransferPersonalError::Repo`] on DB lookup failure.
/// - [`TransferPersonalError::Stripe`] on Stripe API failure.
pub async fn transfer_personal_to_org(
    pool: &Pool,
    stripe: &StripeClient,
    user_id: &str,
    _org_id: &str,
) -> Result<TransferPersonalOutcome, TransferPersonalError> {
    let cus_id = lookup_user_stripe_customer_id(pool, user_id)
        .await
        .map_err(TransferPersonalError::Repo)?;
    let Some(cus_id) = cus_id else {
        return Ok(TransferPersonalOutcome::NoPersonalCustomer);
    };

    let outcome = stripe
        .cancel_active_subscription_for_customer(&cus_id)
        .await
        .map_err(TransferPersonalError::Stripe)?;
    match outcome {
        CancelSubscriptionOutcome::NoSubscription => {
            Ok(TransferPersonalOutcome::NoActiveSubscription)
        }
        CancelSubscriptionOutcome::Cancelled { subscription_id } => {
            Ok(TransferPersonalOutcome::Cancelled { subscription_id })
        }
    }
}

/// Single-statement helper — `SELECT stripe_customer_id FROM users
/// WHERE id = $1::uuid`. Returns `None` when the column is NULL
/// (user signed in before F1.5 wired the Customer-on-signin hook) or
/// when the row doesn't exist (defensive; the route's auth gate
/// guarantees the caller is a real user).
async fn lookup_user_stripe_customer_id(
    pool: &Pool,
    user_id: &str,
) -> Result<Option<String>, OrgError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| OrgError::GetConn(format!("lookup_user_stripe_customer_id: {e}")))?;
    let row = conn
        .query_opt(
            "SELECT stripe_customer_id FROM users WHERE id = $1::uuid",
            &[&user_id],
        )
        .await
        .map_err(|e| {
            OrgError::Sql(format!("lookup_user_stripe_customer_id: {e}"))
        })?;
    Ok(row.and_then(|r| r.get::<_, Option<String>>("stripe_customer_id")))
}

#[cfg(test)]
mod tests {
    //! Pure-Rust shape checks. Postgres + Stripe integration is covered
    //! by the routes-level tests with `MINISTR_TEST_PG_URL` + Stripe
    //! mock fixtures (deferred — same posture as
    //! [`super::seats`] which doesn't ship integration tests either).

    use super::*;

    #[test]
    fn outcome_no_personal_customer_is_clone() {
        // Handler logs this; ensure it stays cheap.
        let _ = TransferPersonalOutcome::NoPersonalCustomer.clone();
    }

    #[test]
    fn outcome_no_active_subscription_is_clone() {
        let _ = TransferPersonalOutcome::NoActiveSubscription.clone();
    }

    #[test]
    fn outcome_cancelled_carries_subscription_id() {
        let o = TransferPersonalOutcome::Cancelled {
            subscription_id: "sub_abc".into(),
        };
        match o.clone() {
            TransferPersonalOutcome::Cancelled { subscription_id } => {
                assert_eq!(subscription_id, "sub_abc");
            }
            other => panic!("expected Cancelled, got {other:?}"),
        }
    }
}
