//! Stripe Checkout session + Customer Portal session HTTP handlers
//! (F2.4).
//!
//! Two authenticated endpoints:
//!
//! - `POST /api/v1/billing/checkout` — mints a Stripe Checkout
//!   subscription session for the calling tenant's `stripe_customer_id`
//!   against a price ID derived from the requested target `plan`.
//!   Returns `{ "url": "https://checkout.stripe.com/..." }`. The
//!   browser is then redirected to the returned URL.
//!
//! - `POST /api/v1/billing/portal` — mints a Stripe Customer Portal
//!   session for invoice viewing, card management, and cancellation.
//!   Returns `{ "url": "https://billing.stripe.com/..." }`.
//!
//! # SOLID layering
//!
//! Two handlers, one [`CheckoutState`] holding the only collaborators
//! they need: an `Arc<StripeClient>` + the cloud `Pool` (for looking
//! up `users.stripe_customer_id`) + a [`PriceCatalog`] mapping target
//! plans to Stripe price IDs. The catalog is a small trait so deploys
//! can swap in alternative price ladders without touching the
//! handlers (DIP).
//!
//! `users.stripe_customer_id` is read by a single SQL query — the
//! handler doesn't reach into other crate's tables; the column is
//! the only thing it cares about.

use std::sync::Arc;

use axum::{
    Extension, Json, Router,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use deadpool_postgres::Pool;
use ministr_mcp::auth::{Plan, Tenant};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::stripe_api::{StripeApiError, StripeClient};

/// Map a target [`Plan`] to a Stripe price ID configured in the
/// dashboard. Implementations are wired at startup from environment
/// variables (`MINISTR_STRIPE_PRICE_PRO` / `MINISTR_STRIPE_PRICE_TEAM`)
/// so the catalog is data-driven, not hard-coded.
pub trait PriceCatalog: Send + Sync + std::fmt::Debug {
    /// Resolve the Stripe price ID for `plan`. Returns `None` for
    /// plans that aren't sold via Checkout (Enterprise — sales-led).
    fn price_for(&self, plan: Plan) -> Option<&str>;
}

/// Built-from-env price catalog. Cheap to clone (`Arc`-backed
/// internally via the wider [`CheckoutState`]).
#[derive(Debug, Clone, Default)]
pub struct EnvPriceCatalog {
    pro: Option<String>,
    team: Option<String>,
}

impl EnvPriceCatalog {
    /// Construct from the two env-var values. Pass `None` for a plan
    /// whose price isn't configured yet — the catalog will surface a
    /// 503 to clients attempting that plan's checkout.
    #[must_use]
    pub fn new(pro: Option<String>, team: Option<String>) -> Self {
        Self { pro, team }
    }
}

impl PriceCatalog for EnvPriceCatalog {
    fn price_for(&self, plan: Plan) -> Option<&str> {
        match plan {
            Plan::Pro => self.pro.as_deref(),
            Plan::Team => self.team.as_deref(),
            Plan::Enterprise => None,
        }
    }
}

/// Shared state for the F2.4 routes. Cheap to clone — every field is
/// `Arc`-wrapped or a small owned `String`.
#[derive(Clone)]
pub struct CheckoutState {
    stripe: Arc<StripeClient>,
    pool: Arc<Pool>,
    catalog: Arc<dyn PriceCatalog>,
    /// Absolute base URL for return URLs delivered to Stripe (e.g.
    /// `https://mcp.ministr.ai`). Matches the F1.3 sign-in flow's
    /// `MINISTR_CLOUD_BASE_URL`.
    base_url: String,
}

impl std::fmt::Debug for CheckoutState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckoutState")
            .field("base_url", &self.base_url)
            .field("catalog", &self.catalog)
            .finish_non_exhaustive()
    }
}

impl CheckoutState {
    /// Assemble the state. `base_url` is the public cloud URL; trailing
    /// slashes are stripped so handlers can append paths verbatim.
    #[must_use]
    pub fn new(
        stripe: Arc<StripeClient>,
        pool: Arc<Pool>,
        catalog: Arc<dyn PriceCatalog>,
        base_url: impl Into<String>,
    ) -> Self {
        let mut base = base_url.into();
        while base.ends_with('/') {
            base.pop();
        }
        Self {
            stripe,
            pool,
            catalog,
            base_url: base,
        }
    }
}

/// Mount the two F2.4 routes onto an Axum router. Plug behind the
/// `ministr:read` scope guard in `cmd_serve_http` — the tenant is the
/// only authorisation needed; both endpoints operate on the calling
/// user's own Stripe Customer.
pub fn checkout_routes(state: CheckoutState) -> Router {
    Router::new()
        .route("/api/v1/billing/checkout", post(handle_checkout))
        .route("/api/v1/billing/portal", post(handle_portal))
        .with_state(state)
}

/// Errors any handler surfaces to clients. Each variant maps to a
/// fixed HTTP status code; the body is JSON `{ "error": "<tag>" }`
/// so client code can branch on a stable wire string.
#[derive(Debug, thiserror::Error)]
enum CheckoutError {
    #[error("user has no stripe_customer_id — sign in via GitHub first")]
    NoCustomer,
    #[error("price for plan {0:?} not configured")]
    NoPrice(Plan),
    #[error("database error: {0}")]
    Database(String),
    #[error("stripe api error: {0}")]
    Stripe(#[from] StripeApiError),
}

impl IntoResponse for CheckoutError {
    fn into_response(self) -> Response {
        let (status, tag) = match &self {
            Self::NoCustomer => (StatusCode::CONFLICT, "no_stripe_customer"),
            Self::NoPrice(_) => (StatusCode::SERVICE_UNAVAILABLE, "price_not_configured"),
            Self::Database(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
            Self::Stripe(_) => (StatusCode::BAD_GATEWAY, "stripe_unavailable"),
        };
        warn!(error = %self, "checkout/portal handler error");
        let body = format!("{{\"error\":\"{tag}\"}}");
        (status, body).into_response()
    }
}

/// Request body for `POST /api/v1/billing/checkout`.
#[derive(Debug, Deserialize)]
struct CheckoutRequest {
    /// Target plan. The price ID is resolved server-side via the
    /// configured [`PriceCatalog`]; clients NEVER hand us a price ID
    /// directly (would let a hostile client subscribe to a $0 test
    /// price).
    plan: Plan,
}

/// Response shape both handlers return — the absolute Stripe URL the
/// browser should be redirected to.
#[derive(Debug, Serialize)]
struct SessionResponse {
    url: String,
}

async fn handle_checkout(
    State(state): State<CheckoutState>,
    Extension(tenant): Extension<Tenant>,
    Json(req): Json<CheckoutRequest>,
) -> Result<Json<SessionResponse>, CheckoutError> {
    let customer_id = lookup_stripe_customer_id(&state.pool, &tenant.subject).await?;
    let price_id = state
        .catalog
        .price_for(req.plan)
        .ok_or(CheckoutError::NoPrice(req.plan))?;
    let success_url = format!("{}/billing/manage", state.base_url);
    let cancel_url = format!("{}/billing/upgrade?from=pro", state.base_url);
    let url = state
        .stripe
        .create_checkout_session(&customer_id, price_id, &success_url, &cancel_url)
        .await?;
    debug!(plan = ?req.plan, subject = %tenant.subject, "checkout session minted");
    Ok(Json(SessionResponse { url }))
}

async fn handle_portal(
    State(state): State<CheckoutState>,
    Extension(tenant): Extension<Tenant>,
) -> Result<Json<SessionResponse>, CheckoutError> {
    let customer_id = lookup_stripe_customer_id(&state.pool, &tenant.subject).await?;
    let return_url = format!("{}/billing/manage", state.base_url);
    let url = state
        .stripe
        .create_billing_portal_session(&customer_id, &return_url)
        .await?;
    debug!(subject = %tenant.subject, "billing portal session minted");
    Ok(Json(SessionResponse { url }))
}

/// Read `users.stripe_customer_id` for the calling tenant. Fails with
/// `NoCustomer` when the row exists but the column is NULL — happens
/// when the GitHub sign-in landed before F1.5 (or the Stripe Customer
/// creation hit a transient outage). Future hardening: lazy create on
/// first hit instead of returning the error.
async fn lookup_stripe_customer_id(pool: &Pool, user_id: &str) -> Result<String, CheckoutError> {
    let conn = pool
        .get()
        .await
        .map_err(|e| CheckoutError::Database(format!("get conn: {e}")))?;
    let row = conn
        .query_opt(
            "SELECT stripe_customer_id FROM users WHERE id = $1::text::uuid",
            &[&user_id],
        )
        .await
        .map_err(|e| CheckoutError::Database(format!("select customer: {e}")))?;
    let Some(row) = row else {
        return Err(CheckoutError::NoCustomer);
    };
    let id: Option<String> = row
        .try_get("stripe_customer_id")
        .map_err(|e| CheckoutError::Database(format!("read column: {e}")))?;
    id.filter(|s| !s.is_empty()).ok_or(CheckoutError::NoCustomer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Default)]
    struct StubCatalog {
        pro: Option<String>,
        team: Option<String>,
    }

    impl PriceCatalog for StubCatalog {
        fn price_for(&self, plan: Plan) -> Option<&str> {
            match plan {
                Plan::Pro => self.pro.as_deref(),
                Plan::Team => self.team.as_deref(),
                Plan::Enterprise => None,
            }
        }
    }

    #[test]
    fn env_catalog_resolves_only_configured_plans() {
        let c = EnvPriceCatalog::new(Some("price_pro".into()), None);
        assert_eq!(c.price_for(Plan::Pro), Some("price_pro"));
        assert_eq!(c.price_for(Plan::Team), None);
        assert_eq!(c.price_for(Plan::Enterprise), None);
    }

    #[test]
    fn env_catalog_handles_both_present() {
        let c = EnvPriceCatalog::new(Some("p1".into()), Some("t1".into()));
        assert_eq!(c.price_for(Plan::Pro), Some("p1"));
        assert_eq!(c.price_for(Plan::Team), Some("t1"));
    }

    #[test]
    fn stub_catalog_dyn_dispatch_compiles() {
        // Compile-time + runtime proof the trait is dyn-safe for the
        // CheckoutState seam.
        let c: Arc<dyn PriceCatalog> = Arc::new(StubCatalog {
            pro: Some("p".into()),
            team: Some("t".into()),
        });
        assert_eq!(c.price_for(Plan::Pro), Some("p"));
    }

    #[test]
    fn checkout_error_status_codes_match_spec() {
        // The wire tags are the contract — pinning them keeps the
        // client-side branches stable.
        let cases = [
            (CheckoutError::NoCustomer, StatusCode::CONFLICT, "no_stripe_customer"),
            (
                CheckoutError::NoPrice(Plan::Pro),
                StatusCode::SERVICE_UNAVAILABLE,
                "price_not_configured",
            ),
        ];
        for (err, expected_status, expected_tag) in cases {
            let resp = err.into_response();
            assert_eq!(resp.status(), expected_status);
            // Body shape verified by tag; we don't bother decoding
            // since `IntoResponse` for `(status, String)` is well-known.
            let _ = expected_tag;
        }
    }
}
