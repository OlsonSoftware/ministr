//! Stripe outbound API client (F1.5 sub-bullets 1 + 2).
//!
//! Companion to [`crate::billing::stripe`] (the webhook receiver) —
//! this module owns the calls WE make to Stripe. Today:
//!
//! - `POST /v1/customers` — [`StripeClient::create_customer`] on first
//!   sign-in. The returned `cus_…` id is stored in `users.stripe_customer_id`
//!   so future Checkout sessions and Meter events can reference it.
//! - `POST /v1/billing/meter_events` — [`StripeClient::report_meter_event`]
//!   for usage-based billing. The function exists but has no internal
//!   caller in F1.5 ("wired and tolerant"); F2.3 enforcement and F2.4
//!   Checkout will start invoking it once `index.minutes` overage logic
//!   lands.
//!
//! # API conventions
//!
//! - Form-encoded bodies (`application/x-www-form-urlencoded`) — Stripe's
//!   public REST surface accepts only forms, never JSON. Matches the
//!   shape every official Stripe SDK uses internally.
//! - `Idempotency-Key` header on `POST /v1/customers` so the GitHub
//!   sign-in callback can retry without minting duplicate Customers.
//!   The key we send is `cust-create-github-{github_id}` — deterministic
//!   per user, never colliding across users.
//! - `identifier` body field on `POST /v1/billing/meter_events` — same
//!   dedup primitive but server-side; Stripe Meters do not honour
//!   Idempotency-Key, by design (per Meters docs, 2026-02-25 API
//!   version).
//! - All errors return [`StripeApiError`]; transport and protocol are
//!   collapsed at the boundary so the caller doesn't depend on
//!   `reqwest`'s error taxonomy.
//!
//! # Why thin reqwest, not `async-stripe`
//!
//! `async-stripe` pulls a substantial dependency surface (full event
//! type tree, Webhook constructors, hundreds of derives). We use a
//! tiny subset of the Stripe API; a thin reqwest wrapper costs nothing
//! and keeps the binary's audit story compact. Same posture as the
//! `GitHubIdp` client in [`crate::idp::github`].

use std::time::Duration;

use reqwest::header;
use serde::Deserialize;
use tracing::debug;

/// Default base URL of the Stripe REST API. Overridable for tests via
/// [`StripeClient::with_base_url`].
const DEFAULT_BASE_URL: &str = "https://api.stripe.com";

/// Stripe API version pinned at construction time. Mirrors Stripe's
/// "version-locked SDK" pattern — if Stripe ships a breaking change,
/// our existing requests keep getting the old schema until we bump
/// this constant deliberately.
const STRIPE_API_VERSION: &str = "2026-02-25.clover";

/// User-Agent header. Stripe's API requires every request to identify
/// itself (informational; not authentication).
const USER_AGENT: &str = "ministr-cloud-billing/1 (+https://ministr.ai)";

/// Errors surfaced by the Stripe API client. Network and protocol
/// failures are collapsed into [`StripeApiError::Transport`] /
/// [`StripeApiError::Protocol`] so call sites depend on this enum,
/// not on `reqwest::Error`.
#[derive(Debug, thiserror::Error)]
pub enum StripeApiError {
    /// The supplied API key was empty or whitespace-only. Caller forgot
    /// to set `MINISTR_STRIPE_SECRET_KEY`.
    #[error("stripe api: api key is empty")]
    EmptyApiKey,
    /// Network-layer failure (timeout, DNS, TLS).
    #[error("stripe api transport error: {0}")]
    Transport(String),
    /// Stripe returned a 4xx/5xx OR the response body could not be
    /// parsed. The inner string is the body for logging; do not surface
    /// to end users.
    #[error("stripe api protocol error: {0}")]
    Protocol(String),
}

/// Outbound Stripe API client.
///
/// Construct once at cloud startup via [`StripeClient::new`]; share via
/// `Clone` (the inner `reqwest::Client` is `Arc`-backed). The client
/// holds the API key in memory — if rotation is needed, build a fresh
/// instance and atomically swap.
#[derive(Debug, Clone)]
pub struct StripeClient {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl StripeClient {
    /// Build a client bound to the canonical Stripe API host
    /// (`https://api.stripe.com`).
    ///
    /// # Errors
    ///
    /// Returns [`StripeApiError::EmptyApiKey`] when `api_key` is empty or
    /// whitespace-only — distinguishes a misconfigured environment from
    /// a transient transport failure.
    /// Returns [`StripeApiError::Transport`] when the inner HTTP client
    /// fails to build (extremely rare — usually a system-TLS load
    /// failure).
    pub fn new(api_key: impl Into<String>) -> Result<Self, StripeApiError> {
        Self::with_base_url(api_key, DEFAULT_BASE_URL)
    }

    /// Build a client against an arbitrary base URL — only used by
    /// tests pointed at a local mock server. Production code calls
    /// [`Self::new`].
    ///
    /// # Errors
    ///
    /// Same surface as [`Self::new`]:
    /// [`StripeApiError::EmptyApiKey`] for blank credentials,
    /// [`StripeApiError::Transport`] for a failed `reqwest::Client`
    /// build.
    pub fn with_base_url(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Result<Self, StripeApiError> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            return Err(StripeApiError::EmptyApiKey);
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| StripeApiError::Transport(format!("build http: {e}")))?;
        Ok(Self {
            http,
            api_key,
            base_url: trim_trailing_slashes(base_url.into()),
        })
    }

    /// Create a Stripe Customer for a brand-new ministr user.
    ///
    /// `email` is the verified GitHub email; `github_id` is the
    /// GitHub user id (also persisted in `users.github_id`). The
    /// idempotency key is `cust-create-github-{github_id}` so retries
    /// after a crash return the SAME customer instead of minting
    /// duplicates.
    ///
    /// Returns the Stripe customer id (`cus_…`).
    ///
    /// # Errors
    ///
    /// - [`StripeApiError::Transport`] for network failures.
    /// - [`StripeApiError::Protocol`] for non-2xx responses, missing
    ///   `id` field, or malformed JSON.
    pub async fn create_customer(
        &self,
        email: &str,
        github_id: i64,
    ) -> Result<String, StripeApiError> {
        let form = customer_form_body(email, github_id);
        let idempotency_key = format!("cust-create-github-{github_id}");
        let url = format!("{}/v1/customers", self.base_url);

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.api_key, Some(""))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("Stripe-Version", STRIPE_API_VERSION)
            .header("Idempotency-Key", &idempotency_key)
            .body(form)
            .send()
            .await
            .map_err(|e| StripeApiError::Transport(format!("create_customer: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StripeApiError::Protocol(format!(
                "create_customer status {status}: {body}"
            )));
        }
        let parsed: CustomerResponse = resp.json().await.map_err(|e| {
            StripeApiError::Protocol(format!("create_customer parse: {e}"))
        })?;
        debug!(
            customer_id = %parsed.id,
            github_id,
            "stripe customer created"
        );
        Ok(parsed.id)
    }

    /// F3.1c-i — create a Stripe Customer for a brand-new org.
    ///
    /// Mirrors [`Self::create_customer`] but uses org-shaped metadata:
    /// `name` becomes the customer's display name (orgs have rich
    /// names like "Acme Robotics"; users only carry an email), and
    /// the `metadata[ministr_org_id]` ties the cus_… back to the
    /// `orgs.id` UUID. Idempotency key is `cust-create-org-{org_id}`
    /// so a retried org-creation post-Stripe-outage returns the same
    /// Customer rather than minting duplicates.
    ///
    /// `billing_email` is the address Stripe sends invoices to. F3.1a
    /// has the org create endpoint accept no `billing_email` field, so
    /// `cmd_serve_http` derives it from the owner's `users.email` —
    /// matches the desktop UX where the owner sees their own
    /// invoices first.
    ///
    /// Returns the Stripe customer id (`cus_…`).
    ///
    /// # Errors
    ///
    /// - [`StripeApiError::Transport`] for network failures.
    /// - [`StripeApiError::Protocol`] for non-2xx responses, missing
    ///   `id` field, or malformed JSON.
    pub async fn create_org_customer(
        &self,
        org_id: &str,
        name: &str,
        billing_email: &str,
    ) -> Result<String, StripeApiError> {
        let form = org_customer_form_body(org_id, name, billing_email);
        let idempotency_key = format!("cust-create-org-{org_id}");
        let url = format!("{}/v1/customers", self.base_url);

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.api_key, Some(""))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("Stripe-Version", STRIPE_API_VERSION)
            .header("Idempotency-Key", &idempotency_key)
            .body(form)
            .send()
            .await
            .map_err(|e| StripeApiError::Transport(format!("create_org_customer: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StripeApiError::Protocol(format!(
                "create_org_customer status {status}: {body}"
            )));
        }
        let parsed: CustomerResponse = resp.json().await.map_err(|e| {
            StripeApiError::Protocol(format!("create_org_customer parse: {e}"))
        })?;
        debug!(
            customer_id = %parsed.id,
            org_id,
            "stripe customer created for org"
        );
        Ok(parsed.id)
    }

    /// Report a meter event for usage-based billing.
    ///
    /// `event_name` is the meter's name configured in the Stripe
    /// dashboard (e.g. `"index_minutes"`). `customer_id` is the
    /// `cus_…` Stripe id from `users.stripe_customer_id`. `value` is
    /// the quantity attributed to this event (Stripe accumulates it
    /// per billing period). `identifier` is the dedup key — pass a
    /// stable per-event id (e.g. `usage_event_id`) so retries collapse.
    ///
    /// No internal caller in F1.5. F2.3 (quota enforcement) and the
    /// daily rollup will start invoking this once `index.minutes`
    /// overage logic lands.
    ///
    /// # Errors
    ///
    /// Same transport / protocol mapping as
    /// [`Self::create_customer`].
    pub async fn report_meter_event(
        &self,
        event_name: &str,
        customer_id: &str,
        value: f64,
        identifier: &str,
    ) -> Result<(), StripeApiError> {
        let form = meter_event_form_body(event_name, customer_id, value, identifier);
        let url = format!("{}/v1/billing/meter_events", self.base_url);

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.api_key, Some(""))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("Stripe-Version", STRIPE_API_VERSION)
            .body(form)
            .send()
            .await
            .map_err(|e| StripeApiError::Transport(format!("report_meter_event: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StripeApiError::Protocol(format!(
                "report_meter_event status {status}: {body}"
            )));
        }
        debug!(
            event_name,
            customer_id,
            identifier,
            "stripe meter event reported"
        );
        Ok(())
    }

    /// Create a Stripe Checkout session for the upgrade flow (F2.4).
    ///
    /// `customer_id` ties the resulting subscription to the existing
    /// `users.stripe_customer_id`. `price_id` is the Pro/Team price
    /// configured in the Stripe dashboard (per §3 pricing matrix —
    /// `MINISTR_STRIPE_PRICE_PRO` / `MINISTR_STRIPE_PRICE_TEAM`).
    /// `success_url` and `cancel_url` are absolute URLs the browser
    /// returns to after the Stripe-hosted flow.
    ///
    /// Returns the session URL the browser should be redirected to.
    ///
    /// # Errors
    ///
    /// Same surface as [`Self::create_customer`].
    pub async fn create_checkout_session(
        &self,
        customer_id: &str,
        price_id: &str,
        success_url: &str,
        cancel_url: &str,
    ) -> Result<String, StripeApiError> {
        let form = checkout_session_form_body(customer_id, price_id, success_url, cancel_url);
        let url = format!("{}/v1/checkout/sessions", self.base_url);

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.api_key, Some(""))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("Stripe-Version", STRIPE_API_VERSION)
            .body(form)
            .send()
            .await
            .map_err(|e| StripeApiError::Transport(format!("checkout_session: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StripeApiError::Protocol(format!(
                "checkout_session status {status}: {body}"
            )));
        }
        let parsed: SessionUrlResponse = resp.json().await.map_err(|e| {
            StripeApiError::Protocol(format!("checkout_session parse: {e}"))
        })?;
        debug!(customer_id, "stripe checkout session created");
        Ok(parsed.url)
    }

    /// Create a Stripe Customer Portal session (F2.4) so the user can
    /// manage invoices, swap cards, or cancel. `return_url` is where
    /// the browser bounces back to after the portal flow closes (the
    /// Tauri panel deep-link or the docs-next `/billing/manage` page).
    ///
    /// # Errors
    ///
    /// Same surface as [`Self::create_customer`].
    pub async fn create_billing_portal_session(
        &self,
        customer_id: &str,
        return_url: &str,
    ) -> Result<String, StripeApiError> {
        let form = portal_session_form_body(customer_id, return_url);
        let url = format!("{}/v1/billing_portal/sessions", self.base_url);

        let resp = self
            .http
            .post(&url)
            .basic_auth(&self.api_key, Some(""))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("Stripe-Version", STRIPE_API_VERSION)
            .body(form)
            .send()
            .await
            .map_err(|e| StripeApiError::Transport(format!("portal_session: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StripeApiError::Protocol(format!(
                "portal_session status {status}: {body}"
            )));
        }
        let parsed: SessionUrlResponse = resp.json().await.map_err(|e| {
            StripeApiError::Protocol(format!("portal_session parse: {e}"))
        })?;
        debug!(customer_id, "stripe billing portal session created");
        Ok(parsed.url)
    }

    /// F3.1c-iii — cancel a customer's active subscription, used by
    /// the personal-Pro → org-Team transfer path so the user can run
    /// Checkout against the org's Stripe Customer without paying for
    /// both plans simultaneously.
    ///
    /// Two API calls when the customer has an active sub:
    ///   1. `GET /v1/subscriptions?customer={cus_id}&status=active&limit=1`
    ///      — find the current active subscription (returns 0 or 1).
    ///   2. `DELETE /v1/subscriptions/{sub_id}` — immediately cancel.
    ///      We use immediate cancel (not `cancel_at_period_end`) because
    ///      the new Team subscription on the org's Customer mints its
    ///      own proration credit, so the user gets refunded for the
    ///      unused portion via Stripe's invoicing — no double billing.
    ///
    /// Returns [`CancelSubscriptionOutcome::NoSubscription`] when the
    /// customer has nothing to cancel (typical for a Customer that
    /// hasn't run Checkout yet — the transfer endpoint reports this
    /// as a no-op outcome).
    ///
    /// # Errors
    ///
    /// - [`StripeApiError::Transport`] for network failures.
    /// - [`StripeApiError::Protocol`] for non-2xx responses or
    ///   malformed JSON on either call.
    pub async fn cancel_active_subscription_for_customer(
        &self,
        customer_id: &str,
    ) -> Result<CancelSubscriptionOutcome, StripeApiError> {
        let list_url = format!(
            "{}/v1/subscriptions?customer={}&status=active&limit=1",
            self.base_url,
            form_encode(customer_id)
        );
        let resp = self
            .http
            .get(&list_url)
            .basic_auth(&self.api_key, Some(""))
            .header("Stripe-Version", STRIPE_API_VERSION)
            .send()
            .await
            .map_err(|e| StripeApiError::Transport(format!("list_subscriptions: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StripeApiError::Protocol(format!(
                "list_subscriptions status {status}: {body}"
            )));
        }
        let listed: SubscriptionList = resp
            .json()
            .await
            .map_err(|e| StripeApiError::Protocol(format!("list_subscriptions parse: {e}")))?;
        let Some(sub) = listed.data.into_iter().next() else {
            debug!(
                customer_id,
                "no active subscription — cancel-for-transfer is a no-op"
            );
            return Ok(CancelSubscriptionOutcome::NoSubscription);
        };

        let cancel_url = format!("{}/v1/subscriptions/{}", self.base_url, sub.id);
        let resp = self
            .http
            .delete(&cancel_url)
            .basic_auth(&self.api_key, Some(""))
            .header("Stripe-Version", STRIPE_API_VERSION)
            .send()
            .await
            .map_err(|e| StripeApiError::Transport(format!("cancel_subscription: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StripeApiError::Protocol(format!(
                "cancel_subscription status {status}: {body}"
            )));
        }
        debug!(
            customer_id,
            subscription_id = %sub.id,
            "subscription cancelled for personal-to-org transfer",
        );
        Ok(CancelSubscriptionOutcome::Cancelled {
            subscription_id: sub.id,
        })
    }

    /// F3.1c-ii — set the seat quantity on the customer's active
    /// subscription. Idempotent: takes an absolute `target_quantity`
    /// (= `count(org_members)`), so concurrent invite acceptances
    /// converge to the right final value regardless of ordering.
    ///
    /// Two API calls: list active subscriptions for the customer,
    /// then POST the first subscription with the first line item's
    /// quantity updated. Stripe creates a proration credit/charge
    /// at the next invoice cycle (`proration_behavior=create_prorations`).
    ///
    /// Returns a [`SyncSeatOutcome`] so the caller can log + take
    /// action (e.g. surface a warning when no subscription exists
    /// for the customer — F3.1c-i pre-Checkout state).
    ///
    /// # Errors
    ///
    /// - [`StripeApiError::Transport`] for network failures.
    /// - [`StripeApiError::Protocol`] for non-2xx responses or
    ///   malformed JSON on either call.
    pub async fn sync_subscription_seats(
        &self,
        customer_id: &str,
        target_quantity: u64,
    ) -> Result<SyncSeatOutcome, StripeApiError> {
        // Step 1: list active subscriptions for the customer. Cap at
        // 1 — orgs have one Team-tier subscription each in F3.1c-ii;
        // future multi-product setups will need a price-aware lookup.
        let list_url = format!(
            "{}/v1/subscriptions?customer={}&status=active&limit=1",
            self.base_url,
            form_encode(customer_id)
        );
        let resp = self
            .http
            .get(&list_url)
            .basic_auth(&self.api_key, Some(""))
            .header("Stripe-Version", STRIPE_API_VERSION)
            .send()
            .await
            .map_err(|e| StripeApiError::Transport(format!("list_subscriptions: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StripeApiError::Protocol(format!(
                "list_subscriptions status {status}: {body}"
            )));
        }
        let listed: SubscriptionList = resp.json().await.map_err(|e| {
            StripeApiError::Protocol(format!("list_subscriptions parse: {e}"))
        })?;
        let Some(sub) = listed.data.into_iter().next() else {
            debug!(
                customer_id,
                target_quantity, "no active subscription — seat sync no-op"
            );
            return Ok(SyncSeatOutcome::NoSubscription);
        };
        let Some(first_item) = sub.items.data.into_iter().next() else {
            return Err(StripeApiError::Protocol(format!(
                "subscription {} has no items — cannot sync seats",
                sub.id
            )));
        };
        if first_item.quantity == target_quantity {
            debug!(
                customer_id,
                subscription_id = %sub.id,
                target_quantity,
                "subscription already at target seat quantity",
            );
            return Ok(SyncSeatOutcome::AlreadyAtTarget {
                subscription_id: sub.id,
                quantity: target_quantity,
            });
        }

        // Step 2: update the first line item's quantity. We POST to
        // the subscription (not the subscription_item) so Stripe can
        // recompute the whole subscription's billing in one shot.
        let form = seat_quantity_form_body(&first_item.id, target_quantity);
        let idempotency_key = format!(
            "sync-seats-{}-q{target_quantity}",
            sub.id
        );
        let update_url = format!("{}/v1/subscriptions/{}", self.base_url, sub.id);
        let resp = self
            .http
            .post(&update_url)
            .basic_auth(&self.api_key, Some(""))
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("Stripe-Version", STRIPE_API_VERSION)
            .header("Idempotency-Key", &idempotency_key)
            .body(form)
            .send()
            .await
            .map_err(|e| StripeApiError::Transport(format!("update_subscription: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(StripeApiError::Protocol(format!(
                "update_subscription status {status}: {body}"
            )));
        }
        let prior = first_item.quantity;
        debug!(
            customer_id,
            subscription_id = %sub.id,
            prior_quantity = prior,
            new_quantity = target_quantity,
            "seat quantity updated",
        );
        Ok(SyncSeatOutcome::Updated {
            subscription_id: sub.id,
            prior_quantity: prior,
            new_quantity: target_quantity,
        })
    }
}

/// F3.1c-iii — outcome of a
/// [`StripeClient::cancel_active_subscription_for_customer`] call.
/// Distinguishes "nothing to cancel" (Customer has no active sub)
/// from "Stripe was actually mutated" so the caller can log + skip
/// downstream actions (e.g. audit emission) when there was no
/// observable change.
#[derive(Debug, Clone)]
pub enum CancelSubscriptionOutcome {
    /// Customer has no active subscription. Common when the user
    /// signed in (which creates a Stripe Customer) but never ran
    /// Checkout. Caller treats this as a no-op transfer.
    NoSubscription,
    /// Stripe state mutated — the named subscription is now `canceled`.
    Cancelled { subscription_id: String },
}

/// F3.1c-ii — outcome of a [`StripeClient::sync_subscription_seats`]
/// call. Distinguishes "nothing to do" (already at target / no
/// subscription) from "Stripe was actually mutated" so the caller can
/// log meaningfully + surface follow-up actions.
#[derive(Debug, Clone)]
pub enum SyncSeatOutcome {
    /// Customer has no active subscription yet. Normal state for an
    /// org whose owner hasn't run Checkout — the sync becomes a no-op
    /// until F2.4's Checkout flow mints the subscription.
    NoSubscription,
    /// The subscription's line item already has the target quantity.
    /// Reached when a concurrent sync race already wrote the value,
    /// or when the caller is re-syncing without an underlying change.
    AlreadyAtTarget {
        subscription_id: String,
        quantity: u64,
    },
    /// Stripe state mutated from `prior_quantity` to `new_quantity`.
    Updated {
        subscription_id: String,
        prior_quantity: u64,
        new_quantity: u64,
    },
}

/// Subset of `Checkout.Session` / `BillingPortal.Session` — both
/// surface a `url` field the browser is redirected to.
#[derive(Debug, Deserialize)]
struct SessionUrlResponse {
    url: String,
}

/// Subset of Stripe's `Customer` object the client reads. Stripe
/// returns many more fields; we only need the id.
#[derive(Debug, Deserialize)]
struct CustomerResponse {
    id: String,
}

/// F3.1c-ii — `GET /v1/subscriptions` list response. Stripe paginates
/// with `data[]`; we only ever need the first row (the active
/// subscription) so we don't follow `has_more`.
#[derive(Debug, Deserialize)]
struct SubscriptionList {
    data: Vec<SubscriptionListItem>,
}

#[derive(Debug, Deserialize)]
struct SubscriptionListItem {
    id: String,
    items: SubscriptionItems,
}

#[derive(Debug, Deserialize)]
struct SubscriptionItems {
    data: Vec<SubscriptionLineItem>,
}

#[derive(Debug, Deserialize)]
struct SubscriptionLineItem {
    id: String,
    #[serde(default)]
    quantity: u64,
}

/// Build the form-encoded body for `POST /v1/customers`. Pulled out so
/// the encoding can be unit-tested without an HTTP round-trip.
fn customer_form_body(email: &str, github_id: i64) -> String {
    let mut body = String::new();
    body.push_str("email=");
    body.push_str(&form_encode(email));
    body.push_str("&metadata[github_id]=");
    body.push_str(&form_encode(&github_id.to_string()));
    body.push_str("&metadata[source]=ministr-cloud-signin");
    body
}

/// F3.1c-ii — build the form-encoded body for `POST /v1/subscriptions/{id}`
/// when updating the first line item's seat quantity. Stripe expects
/// `items[0][id]=<si_…>&items[0][quantity]=<N>` plus a
/// `proration_behavior` so mid-cycle changes credit/charge fairly at
/// the next invoice. Extracted so the encoding can be asserted in
/// unit tests without an HTTP round-trip.
fn seat_quantity_form_body(line_item_id: &str, quantity: u64) -> String {
    let mut body = String::new();
    body.push_str("items[0][id]=");
    body.push_str(&form_encode(line_item_id));
    body.push_str("&items[0][quantity]=");
    body.push_str(&form_encode(&quantity.to_string()));
    body.push_str("&proration_behavior=create_prorations");
    body
}

/// F3.1c-i — build the form-encoded body for `POST /v1/customers`
/// against an org. Same SOLID shape as [`customer_form_body`] —
/// extracted so the encoding can be exercised in unit tests without
/// an HTTP round-trip.
fn org_customer_form_body(org_id: &str, name: &str, billing_email: &str) -> String {
    let mut body = String::new();
    body.push_str("name=");
    body.push_str(&form_encode(name));
    body.push_str("&email=");
    body.push_str(&form_encode(billing_email));
    body.push_str("&metadata[ministr_org_id]=");
    body.push_str(&form_encode(org_id));
    body.push_str("&metadata[source]=ministr-cloud-org-create");
    body
}

/// Build the form-encoded body for `POST /v1/checkout/sessions`.
/// Stripe's subscription Checkout mode requires `mode=subscription`,
/// at least one `line_items[0][price]` line, and the customer / return
/// URLs. `payment_method_types[]=card` keeps the surface narrow to
/// cards for now; future ACH / Link is a follow-on knob.
fn checkout_session_form_body(
    customer_id: &str,
    price_id: &str,
    success_url: &str,
    cancel_url: &str,
) -> String {
    let mut body = String::new();
    body.push_str("mode=subscription");
    body.push_str("&customer=");
    body.push_str(&form_encode(customer_id));
    body.push_str("&line_items[0][price]=");
    body.push_str(&form_encode(price_id));
    body.push_str("&line_items[0][quantity]=1");
    body.push_str("&success_url=");
    body.push_str(&form_encode(success_url));
    body.push_str("&cancel_url=");
    body.push_str(&form_encode(cancel_url));
    body.push_str("&payment_method_types[]=card");
    body
}

/// Build the form-encoded body for `POST /v1/billing_portal/sessions`.
fn portal_session_form_body(customer_id: &str, return_url: &str) -> String {
    let mut body = String::new();
    body.push_str("customer=");
    body.push_str(&form_encode(customer_id));
    body.push_str("&return_url=");
    body.push_str(&form_encode(return_url));
    body
}

/// Build the form-encoded body for `POST /v1/billing/meter_events`.
/// Stripe nests payload fields under `payload[...]` per the form-
/// encoding convention their SDKs use for sub-objects.
fn meter_event_form_body(
    event_name: &str,
    customer_id: &str,
    value: f64,
    identifier: &str,
) -> String {
    let mut body = String::new();
    body.push_str("event_name=");
    body.push_str(&form_encode(event_name));
    body.push_str("&payload[stripe_customer_id]=");
    body.push_str(&form_encode(customer_id));
    body.push_str("&payload[value]=");
    body.push_str(&form_encode(&value.to_string()));
    body.push_str("&identifier=");
    body.push_str(&form_encode(identifier));
    body
}

/// Form-encode a single value. Stripe accepts the standard
/// application/x-www-form-urlencoded encoding — same RFC 3986
/// unreserved alphabet as the rest of the workspace's encoders.
fn form_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b == b' ' {
            out.push('+');
        } else if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_nibble(b >> 4));
            out.push(hex_nibble(b & 0x0f));
        }
    }
    out
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'A' + (n - 10)) as char,
        _ => '0',
    }
}

fn trim_trailing_slashes(mut s: String) -> String {
    while s.ends_with('/') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_empty_api_key() {
        assert!(matches!(
            StripeClient::new(""),
            Err(StripeApiError::EmptyApiKey)
        ));
        assert!(matches!(
            StripeClient::new("   "),
            Err(StripeApiError::EmptyApiKey)
        ));
    }

    #[test]
    fn new_accepts_realistic_key_shape() {
        // Stripe test keys are prefixed `sk_test_`; live keys `sk_live_`.
        // The client doesn't validate the prefix (Stripe accepts either
        // against the right base URL) — it only checks non-emptiness.
        let c = StripeClient::new("sk_test_xyz").expect("non-empty key accepted");
        assert!(c.base_url.starts_with("https://api.stripe.com"));
    }

    #[test]
    fn customer_form_body_encodes_email_and_github_id() {
        let body = customer_form_body("user+plus@example.com", 42);
        assert!(body.contains("email=user%2Bplus%40example.com"), "got {body}");
        assert!(body.contains("metadata[github_id]=42"), "got {body}");
        assert!(body.contains("metadata[source]=ministr-cloud-signin"));
    }

    #[test]
    fn seat_quantity_form_body_carries_item_id_quantity_and_proration() {
        // F3.1c-ii — Stripe wants `items[0][id]` + `items[0][quantity]`
        // (not the bare quantity), and a proration_behavior so mid-
        // cycle changes credit/charge fairly at the next invoice.
        let body = seat_quantity_form_body("si_test123", 3);
        assert!(body.contains("items[0][id]=si_test123"), "got {body}");
        assert!(body.contains("items[0][quantity]=3"), "got {body}");
        assert!(
            body.contains("proration_behavior=create_prorations"),
            "got {body}"
        );
    }

    #[test]
    fn seat_quantity_form_body_url_encodes_line_item_id() {
        // Line item ids are well-shaped (`si_…`) in practice, but
        // belt-and-braces — if Stripe ever changes the shape we
        // shouldn't generate a malformed body. `form_encode` uses
        // `+` for spaces (matches application/x-www-form-urlencoded;
        // mirrors the F3.1c-i comment in routes.rs).
        let body = seat_quantity_form_body("si test", 1);
        assert!(body.contains("items[0][id]=si+test"), "got {body}");
    }

    #[test]
    fn org_customer_form_body_carries_org_metadata() {
        // F3.1c-i — org Customers ship a display `name`, an `email`
        // for invoice delivery, and an `ministr_org_id` metadata
        // entry that lets a future webhook handler resolve cus_… back
        // to an `orgs.id` without a DB round-trip.
        let body = org_customer_form_body(
            "0190f000-0000-7000-8000-000000000001",
            "Acme Robotics",
            "billing+admin@acme.example",
        );
        // Stripe-style form encoding uses `+` for the space (the
        // shared `form_encode` helper matches application/x-www-form-
        // urlencoded). %20 would also be a valid encoding but is not
        // what we emit.
        assert!(body.contains("name=Acme+Robotics"), "got {body}");
        assert!(
            body.contains("email=billing%2Badmin%40acme.example"),
            "got {body}"
        );
        assert!(
            body.contains("metadata[ministr_org_id]=0190f000-0000-7000-8000-000000000001"),
            "got {body}"
        );
        assert!(body.contains("metadata[source]=ministr-cloud-org-create"));
        // The user-create source string MUST NOT appear in an org's
        // form body — they're distinct surfaces and a Stripe-side
        // report grouping by `metadata[source]` should keep them
        // disjoint.
        assert!(!body.contains("metadata[source]=ministr-cloud-signin"));
    }

    #[test]
    fn checkout_session_form_body_carries_required_fields() {
        let body = checkout_session_form_body(
            "cus_abc",
            "price_pro_monthly",
            "https://ministr.ai/billing/success",
            "https://ministr.ai/billing/cancel",
        );
        assert!(body.contains("mode=subscription"), "{body}");
        assert!(body.contains("customer=cus_abc"), "{body}");
        assert!(body.contains("line_items[0][price]=price_pro_monthly"), "{body}");
        assert!(body.contains("line_items[0][quantity]=1"), "{body}");
        assert!(
            body.contains("success_url=https%3A%2F%2Fministr.ai%2Fbilling%2Fsuccess"),
            "{body}"
        );
        assert!(
            body.contains("cancel_url=https%3A%2F%2Fministr.ai%2Fbilling%2Fcancel"),
            "{body}"
        );
        assert!(body.contains("payment_method_types[]=card"), "{body}");
    }

    #[test]
    fn portal_session_form_body_carries_customer_and_return_url() {
        let body = portal_session_form_body(
            "cus_abc",
            "https://mcp.ministr.ai/billing/portal-return",
        );
        assert!(body.contains("customer=cus_abc"), "{body}");
        assert!(
            body.contains(
                "return_url=https%3A%2F%2Fmcp.ministr.ai%2Fbilling%2Fportal-return"
            ),
            "{body}"
        );
    }

    #[test]
    fn meter_event_form_body_nests_payload_fields() {
        let body = meter_event_form_body("index_minutes", "cus_abc", 12.5, "evt-1");
        assert!(body.contains("event_name=index_minutes"));
        assert!(body.contains("payload[stripe_customer_id]=cus_abc"));
        assert!(body.contains("payload[value]=12.5"));
        assert!(body.contains("identifier=evt-1"));
    }

    #[test]
    fn form_encode_preserves_unreserved_chars() {
        assert_eq!(form_encode("abcDEF-_.~012"), "abcDEF-_.~012");
        assert_eq!(form_encode("a b"), "a+b");
        assert_eq!(form_encode("a:b"), "a%3Ab");
        assert_eq!(form_encode("a@b"), "a%40b");
    }

    #[test]
    fn trim_trailing_slashes_normalises_base_url() {
        assert_eq!(
            trim_trailing_slashes("https://api.stripe.com///".into()),
            "https://api.stripe.com"
        );
    }

    /// Read enough bytes from `stream` to capture the full HTTP request
    /// (headers + body). reqwest can split the request across multiple
    /// TCP segments, so a single `read` would miss part of it. We loop
    /// until we see the headers terminator AND enough body bytes to
    /// satisfy `Content-Length`, then return the accumulated string.
    async fn drain_http_request(
        stream: &mut tokio::net::TcpStream,
    ) -> String {
        use tokio::io::AsyncReadExt as _;
        let mut buf = Vec::with_capacity(4096);
        let mut tmp = [0u8; 1024];
        loop {
            let n = stream.read(&mut tmp).await.unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
            let text = String::from_utf8_lossy(&buf);
            if let Some(idx) = text.find("\r\n\r\n") {
                let headers = &text[..idx];
                let content_length = headers
                    .lines()
                    .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                    .and_then(|l| l.split(':').nth(1))
                    .and_then(|v| v.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if buf.len() >= idx + 4 + content_length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&buf).to_string()
    }

    #[tokio::test]
    async fn create_customer_round_trips_against_local_mock() {
        // Spin up a single-shot HTTP server that responds with a
        // canned Customer JSON. Verifies the wire shape end-to-end
        // (URL, method, body, headers) without contacting Stripe.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            let (mut stream, _) = listener.accept().await.unwrap();
            let req = drain_http_request(&mut stream).await;
            let resp = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 28\r\nConnection: close\r\n\r\n{\"id\":\"cus_test_round_trip\"}";
            stream.write_all(resp.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            req
        });

        let base = format!("http://{addr}");
        let client = StripeClient::with_base_url("sk_test_dummy", base).unwrap();
        let id = client
            .create_customer("u@example.com", 99)
            .await
            .expect("create_customer succeeds against mock");
        assert_eq!(id, "cus_test_round_trip");

        let req = server.await.unwrap();
        let req_lower = req.to_ascii_lowercase();
        assert!(req.starts_with("POST /v1/customers"), "request line: {req}");
        assert!(
            req_lower.contains("stripe-version: 2026-02-25.clover"),
            "request: {req}"
        );
        assert!(
            req_lower.contains("idempotency-key: cust-create-github-99"),
            "request: {req}"
        );
        assert!(req.contains("email=u%40example.com"), "request: {req}");
        assert!(req.contains("metadata[github_id]=99"), "request: {req}");
    }

    #[tokio::test]
    async fn create_customer_surfaces_protocol_error_on_4xx() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = drain_http_request(&mut stream).await;
            let body = "{\"error\":{\"message\":\"bad key\"}}";
            let resp = format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
        });

        let base = format!("http://{addr}");
        let client = StripeClient::with_base_url("sk_test_dummy", base).unwrap();
        let err = client
            .create_customer("u@example.com", 100)
            .await
            .expect_err("4xx should surface as Protocol");
        assert!(matches!(err, StripeApiError::Protocol(ref m) if m.contains("401")));
    }

    #[tokio::test]
    async fn create_checkout_session_returns_url_from_stripe() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            let (mut stream, _) = listener.accept().await.unwrap();
            let req = drain_http_request(&mut stream).await;
            let body = "{\"url\":\"https://checkout.stripe.com/c/pay/cs_test_123\"}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            req
        });

        let base = format!("http://{addr}");
        let client = StripeClient::with_base_url("sk_test_dummy", base).unwrap();
        let url = client
            .create_checkout_session(
                "cus_abc",
                "price_pro_monthly",
                "https://ministr.ai/billing/success",
                "https://ministr.ai/billing/cancel",
            )
            .await
            .expect("create_checkout_session succeeds");
        assert_eq!(url, "https://checkout.stripe.com/c/pay/cs_test_123");

        let req = server.await.unwrap();
        assert!(req.starts_with("POST /v1/checkout/sessions"), "request: {req}");
        assert!(req.contains("mode=subscription"), "{req}");
        assert!(req.contains("customer=cus_abc"), "{req}");
    }

    #[tokio::test]
    async fn create_billing_portal_session_returns_url() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            let (mut stream, _) = listener.accept().await.unwrap();
            let req = drain_http_request(&mut stream).await;
            let body = "{\"url\":\"https://billing.stripe.com/p/session/test_xyz\"}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            req
        });

        let base = format!("http://{addr}");
        let client = StripeClient::with_base_url("sk_test_dummy", base).unwrap();
        let url = client
            .create_billing_portal_session("cus_abc", "https://mcp.ministr.ai/billing/portal-return")
            .await
            .expect("create_billing_portal_session succeeds");
        assert_eq!(url, "https://billing.stripe.com/p/session/test_xyz");

        let req = server.await.unwrap();
        assert!(
            req.starts_with("POST /v1/billing_portal/sessions"),
            "request: {req}"
        );
        assert!(req.contains("customer=cus_abc"), "{req}");
    }

    #[tokio::test]
    async fn report_meter_event_posts_expected_body() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt as _;
            let (mut stream, _) = listener.accept().await.unwrap();
            let req = drain_http_request(&mut stream).await;
            let body = "{\"object\":\"billing.meter_event\"}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            req
        });

        let base = format!("http://{addr}");
        let client = StripeClient::with_base_url("sk_test_dummy", base).unwrap();
        client
            .report_meter_event("index_minutes", "cus_abc", 3.0, "evt-42")
            .await
            .expect("meter event posts");

        let req = server.await.unwrap();
        let req_lower = req.to_ascii_lowercase();
        assert!(
            req.starts_with("POST /v1/billing/meter_events"),
            "request line: {req}"
        );
        assert!(req.contains("event_name=index_minutes"), "request: {req}");
        assert!(
            req.contains("payload[stripe_customer_id]=cus_abc"),
            "request: {req}"
        );
        assert!(req.contains("payload[value]=3"), "request: {req}");
        assert!(req.contains("identifier=evt-42"), "request: {req}");
        // Meter events do NOT use Idempotency-Key; dedup is via the
        // `identifier` body field.
        assert!(
            !req_lower.contains("idempotency-key:"),
            "meter events must not carry Idempotency-Key: {req}"
        );
    }
}
