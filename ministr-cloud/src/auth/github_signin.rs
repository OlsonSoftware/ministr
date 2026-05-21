//! `/auth/github/*` — the cloud-side GitHub sign-in flow.
//!
//! Two handlers: `/auth/github/start` (begin flow → redirect to GitHub)
//! and `/auth/github/callback` (GitHub returns here → exchange → upsert
//! → mint bearer token → redirect to caller's loopback).
//!
//! # Threat model
//!
//! - **State CSRF.** Each `start` request generates a fresh inner state
//!   that's used BOTH as the GitHub `state` parameter and as the lookup
//!   key for the pending entry. The caller also supplies its own
//!   `state` which we echo back on the loopback redirect for the
//!   caller's own CSRF check (Tauri side).
//! - **Open redirect.** `loopback_redirect` is the URL we'll send the
//!   bearer token to. We accept ONLY `http://127.0.0.1:<port>/...` or
//!   `http://localhost:<port>/...` — never an externally-resolvable
//!   host. RFC 8252 reserves loopback for native-app PKCE; anything
//!   else here would let an attacker exfiltrate the token.
//! - **Token in the URL.** The bearer ends up as a query parameter on
//!   the loopback. Acceptable per RFC 8252 §7.3 for native-app flows
//!   because the URL never leaves the local machine (the listener is
//!   bound on loopback). Browsers won't include it in `Referer` for the
//!   `127.0.0.1` origin per Fetch policy.
//! - **Replay.** `PendingState` entries are removed on first callback
//!   match; a replayed callback gets `400 unknown state`.
//! - **TTL.** Entries older than [`PENDING_TTL`] are pruned on every
//!   callback. A user who abandons the flow leaves a row that times out
//!   on its own.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    Router,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use deadpool_postgres::Pool;
use ministr_mcp::auth::OAuthStore;
use parking_lot::Mutex;
use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::billing::StripeClient;
use crate::idp::{GitHubIdp, IdentityProvider, IdpError};
use crate::users::{set_stripe_customer_id, upsert_github_user, UserError};

/// Scopes attached to GitHub-IdP-minted bearer tokens. Matches the
/// scope set the Tauri OAuth code-grant flow already requests
/// (`commands_cloud::cloud_authenticate`) so downstream handlers see a
/// uniform scope shape regardless of which sign-in path the user took.
pub const DEFAULT_SIGNIN_SCOPE: &str =
    "ministr:read ministr:write ministr:bundle:read ministr:bundle:write";

/// Pending-state TTL. Long enough to accommodate a user who needs to
/// install the GitHub OAuth App or fumble through 2FA; short enough that
/// abandoned entries don't accumulate forever.
const PENDING_TTL: Duration = Duration::from_secs(600);

/// Path that GitHub redirects back to. The same value goes into the
/// GitHub OAuth App's "Authorization callback URL" config; matching is
/// strict per RFC 6749 §3.1.2.
const GITHUB_CALLBACK_PATH: &str = "/auth/github/callback";

/// State carried alongside every pending GitHub sign-in.
#[derive(Debug)]
struct PendingState {
    /// PKCE verifier supplied to `IdentityProvider::exchange`. Generated
    /// in `start`, replayed on `callback`.
    code_verifier: String,
    /// Where to redirect the user with the minted token. Validated
    /// against the loopback-only allowlist in `start` so we can trust it
    /// in `callback` without re-checking.
    loopback_redirect: String,
    /// State value the CALLER sent us. We echo it back so the loopback
    /// listener can verify its own CSRF nonce.
    client_state: String,
    /// F3.1b-i — optional invite token supplied by the recipient
    /// clicking the magic link. When set, the callback handler calls
    /// `consume_invite` after the user is upserted; success appends a
    /// row to `org_members` so the recipient lands in the org as part
    /// of sign-in. Unknown / expired / already-accepted are logged but
    /// don't block sign-in itself (the recipient still gets a token,
    /// just no membership).
    invite_token: Option<String>,
    /// Used by the TTL pruner.
    created_at: Instant,
}

/// Shared state for the `/auth/github/*` handlers.
#[derive(Clone)]
pub struct GitHubSigninState {
    /// The `GitHubIdp` constructed from
    /// `MINISTR_GITHUB_CLIENT_ID`/`MINISTR_GITHUB_CLIENT_SECRET` at
    /// startup. Held in an `Arc` so it can be cheaply cloned into the
    /// async tasks the handlers spawn.
    idp: Arc<GitHubIdp>,
    /// The cloud Postgres pool — same one
    /// `ministr-cloud::PostgresUsageSink` and the billing handler use.
    pool: Pool,
    /// Token-minting store. Cheap-clone façade per
    /// `ministr_mcp::auth::OAuthStore`.
    oauth_store: OAuthStore,
    /// Absolute base URL of the cloud (`https://mcp.ministr.ai` in
    /// prod). Used to build the GitHub `redirect_uri` parameter; must
    /// match the value registered in the GitHub OAuth App.
    cloud_base_url: String,
    /// Optional Stripe client. `None` when
    /// `MINISTR_STRIPE_SECRET_KEY` is unset (self-hosted / pre-billing
    /// deployments). `Some` triggers Customer creation on first
    /// sign-in (F1.5). Failures here are best-effort — they log and
    /// continue rather than blocking sign-in.
    stripe: Option<Arc<StripeClient>>,
    /// Open requests awaiting callback. `parking_lot::Mutex` matches the
    /// workspace convention (no poisoning, smaller, faster).
    pending: Arc<Mutex<HashMap<String, PendingState>>>,
}

impl std::fmt::Debug for GitHubSigninState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubSigninState")
            .field("cloud_base_url", &self.cloud_base_url)
            .field("idp", &"<GitHubIdp>")
            .field("pool", &"<Pool>")
            .field("oauth_store", &"<OAuthStore>")
            .field("stripe_configured", &self.stripe.is_some())
            .field("pending_count", &self.pending.lock().len())
            .finish()
    }
}

impl GitHubSigninState {
    /// Assemble the state. `cloud_base_url` must NOT carry a trailing
    /// slash — we append paths verbatim. The trailing-slash invariant
    /// is enforced at construction so handlers can stay
    /// allocation-light.
    #[must_use]
    pub fn new(
        idp: Arc<GitHubIdp>,
        pool: Pool,
        oauth_store: OAuthStore,
        cloud_base_url: impl Into<String>,
    ) -> Self {
        Self {
            idp,
            pool,
            oauth_store,
            cloud_base_url: trim_trailing_slashes(cloud_base_url.into()),
            stripe: None,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Wire a [`StripeClient`] for F1.5 Customer-creation on first
    /// sign-in. When set, the callback handler creates a Stripe Customer
    /// for newly-inserted user rows and persists the `cus_…` id into
    /// `users.stripe_customer_id`. Failures are logged and tolerated —
    /// sign-in succeeds even when Stripe is unreachable.
    #[must_use]
    pub fn with_stripe(mut self, stripe: Arc<StripeClient>) -> Self {
        self.stripe = Some(stripe);
        self
    }
}

fn trim_trailing_slashes(mut s: String) -> String {
    while s.ends_with('/') {
        s.pop();
    }
    s
}

/// Errors surfaced by the GitHub sign-in handlers. Internal variants are
/// mapped to HTTP statuses in [`IntoResponse`]; the body never leaks
/// secret material.
#[derive(Debug, thiserror::Error)]
pub enum GitHubSigninError {
    /// Caller supplied a `loopback_redirect` that wasn't on
    /// `127.0.0.1` / `localhost`.
    #[error("invalid loopback_redirect: must be http://127.0.0.1 or http://localhost")]
    InvalidLoopback,
    /// Callback `state` did not match any open pending entry.
    #[error("unknown or expired state")]
    UnknownState,
    /// GitHub rejected the code or returned a malformed response.
    #[error("github exchange failed: {0}")]
    Exchange(#[from] IdpError),
    /// `upsert_github_user` failed (DB issue or missing required fields).
    #[error("persist user: {0}")]
    PersistUser(#[from] UserError),
    /// `OAuthStore::issue_bearer_token` failed (storage backend issue).
    #[error("mint token: {0}")]
    MintToken(String),
}

impl IntoResponse for GitHubSigninError {
    fn into_response(self) -> Response {
        let (code, body) = match &self {
            Self::InvalidLoopback => (StatusCode::BAD_REQUEST, self.to_string()),
            Self::UnknownState => (StatusCode::BAD_REQUEST, "unknown state".to_string()),
            Self::Exchange(_) => (
                StatusCode::BAD_GATEWAY,
                "github sign-in failed".to_string(),
            ),
            Self::PersistUser(_) | Self::MintToken(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "sign-in temporarily unavailable".to_string(),
            ),
        };
        warn!(error = %self, "github sign-in error");
        (code, body).into_response()
    }
}

/// Build the axum sub-router that exposes the two handlers. Mount on
/// the public protected-router merge in `cmd_serve_http`; the
/// handlers are deliberately UN-protected (sign-in MUST be reachable
/// without an existing token).
pub fn github_signin_routes(state: GitHubSigninState) -> Router {
    Router::new()
        .route(GITHUB_CALLBACK_PATH, get(handle_github_callback))
        .route("/auth/github/start", get(handle_github_start))
        .with_state(state)
}

/// Query string accepted by `/auth/github/start`.
#[derive(Debug, Deserialize)]
struct StartQuery {
    /// Loopback URL we'll redirect the user to with `?token=...&state=...`
    /// once GitHub sign-in completes.
    loopback_redirect: String,
    /// Caller's CSRF nonce. Echoed back on the loopback redirect so
    /// the listener can verify it.
    state: String,
    /// F3.1b-i — optional org-invite token. When the recipient clicks
    /// `…/auth/github/start?invite=<raw>&loopback_redirect=…&state=…`,
    /// the raw token rides through `PendingState` and gets consumed
    /// after user upsert. Empty / missing means a regular sign-in.
    #[serde(default)]
    invite: Option<String>,
}

/// Query string accepted by `/auth/github/callback`.
#[derive(Debug, Deserialize)]
struct CallbackQuery {
    /// GitHub's authorization code. Single-use; exchanged for an access
    /// token at the GitHub token endpoint.
    code: Option<String>,
    /// Our inner state value — also the key into the pending map.
    state: Option<String>,
    /// Present when the user denied access on github.com (e.g.
    /// `access_denied`). We surface this to the loopback as `?error=...`
    /// so the desktop UI can show a friendly message.
    error: Option<String>,
}

async fn handle_github_start(
    State(state): State<GitHubSigninState>,
    Query(query): Query<StartQuery>,
) -> Result<Response, GitHubSigninError> {
    if !is_loopback_redirect(&query.loopback_redirect) {
        return Err(GitHubSigninError::InvalidLoopback);
    }

    // Inner PKCE materials — fresh per attempt. The verifier survives in
    // PendingState only; never sent out except as part of the
    // server-to-GitHub token-exchange POST.
    let code_verifier = crate::auth::github_signin::random_url_safe_id(64);
    let code_challenge = pkce_s256(&code_verifier);
    let inner_state = crate::auth::github_signin::random_url_safe_id(32);

    {
        let mut pending = state.pending.lock();
        prune_expired(&mut pending);
        pending.insert(
            inner_state.clone(),
            PendingState {
                code_verifier,
                loopback_redirect: query.loopback_redirect,
                client_state: query.state,
                invite_token: query.invite.filter(|s| !s.is_empty()),
                created_at: Instant::now(),
            },
        );
        debug!(
            pending_count = pending.len(),
            "registered github sign-in attempt"
        );
    }

    let callback_url = format!("{}{GITHUB_CALLBACK_PATH}", state.cloud_base_url);
    let authorize_url = state
        .idp
        .authorize_url(&inner_state, &callback_url, &code_challenge);
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, authorize_url)],
    )
        .into_response())
}

#[allow(clippy::too_many_lines)]
async fn handle_github_callback(
    State(state): State<GitHubSigninState>,
    Query(query): Query<CallbackQuery>,
) -> Result<Response, GitHubSigninError> {
    // Look up and remove the pending entry up-front so a replayed
    // callback cannot reuse the state.
    let Some(inner_state) = query.state else {
        return Err(GitHubSigninError::UnknownState);
    };
    let pending = {
        let mut map = state.pending.lock();
        prune_expired(&mut map);
        map.remove(&inner_state)
    };
    let Some(pending) = pending else {
        return Err(GitHubSigninError::UnknownState);
    };

    // GitHub bounces back with `?error=access_denied` when the user
    // denied the App on the consent screen — surface that to the
    // loopback so the desktop UI can render an explanation.
    if let Some(err) = query.error {
        let redirect = append_query(
            &pending.loopback_redirect,
            &[("error", &err), ("state", &pending.client_state)],
        );
        info!(error = %err, "github sign-in declined by user");
        return Ok((StatusCode::FOUND, [(header::LOCATION, redirect)]).into_response());
    }
    let Some(code) = query.code else {
        return Err(GitHubSigninError::UnknownState);
    };

    let callback_url = format!("{}{GITHUB_CALLBACK_PATH}", state.cloud_base_url);
    let identity = state
        .idp
        .exchange(&code, &callback_url, &pending.code_verifier)
        .await?;

    let user = upsert_github_user(&state.pool, &identity).await?;
    debug!(
        user_id = %user.id,
        email = %user.email,
        inserted = user.inserted,
        "github sign-in user upserted"
    );

    // F3.1b-i — apply the invite if one was threaded through. We
    // intentionally tolerate failure: an expired / unknown / already-
    // accepted invite logs and proceeds rather than aborting the
    // entire sign-in. The recipient still gets a bearer token; the
    // missing membership is recoverable (the inviter can mint a new
    // link). Hard-failing here would block sign-in for unrelated
    // reasons — a worse UX than landing them in the panel with no
    // orgs and a clear "invite expired" toast in F3.1b-ii's email
    // half. The result is observable via tracing today.
    if let Some(raw_invite) = pending.invite_token.as_deref() {
        match crate::orgs::consume_invite(&state.pool, raw_invite, &user.id).await {
            Ok(crate::orgs::ConsumeOutcome::Accepted { org_id, role }) => {
                info!(
                    user_id = %user.id,
                    org_id = %org_id,
                    role = %role,
                    "org invite accepted as part of github sign-in"
                );

                // F3.1c-ii — bump the Stripe subscription's seat
                // quantity to match the new member count. Best-
                // effort: a Stripe outage must not unwind the
                // membership insert that consume_invite just
                // committed; failures log + continue (F3.1c-iv
                // backfill job will catch up). Skipped when no
                // Stripe client is wired (self-hosted serve / no
                // MINISTR_STRIPE_SECRET_KEY).
                if let Some(stripe) = state.stripe.as_ref() {
                    match crate::orgs::sync_org_seats(&state.pool, stripe, &org_id).await {
                        Ok(outcome) => {
                            debug!(
                                user_id = %user.id,
                                org_id = %org_id,
                                ?outcome,
                                "seat sync after invite accept"
                            );
                        }
                        Err(e) => {
                            warn!(
                                user_id = %user.id,
                                org_id = %org_id,
                                error = %e,
                                "seat sync after invite accept failed — sign-in proceeds"
                            );
                        }
                    }
                }
            }
            Ok(other) => {
                warn!(
                    user_id = %user.id,
                    outcome = ?other,
                    "org invite present but not applied (expired / unknown / already accepted)"
                );
            }
            Err(e) => {
                warn!(
                    user_id = %user.id,
                    error = %e,
                    "org invite lookup failed — sign-in proceeds without membership"
                );
            }
        }
    }

    // F1.5 — create a Stripe Customer on the user's FIRST sign-in.
    // Best-effort: a Stripe outage must not block the user from
    // landing in the Tauri panel. The Customer is also the prerequisite
    // for F2.4 Checkout, so a follow-up sync job will fill any rows
    // that race past this hook with `stripe_customer_id IS NULL`.
    if user.inserted
        && let Some(stripe) = state.stripe.as_ref()
        && let Some(github_id) = user.github_id
    {
        match stripe.create_customer(&user.email, github_id).await {
            Ok(customer_id) => {
                if let Err(e) =
                    set_stripe_customer_id(&state.pool, &user.id, &customer_id).await
                {
                    warn!(error = %e, user_id = %user.id, "persist stripe_customer_id failed");
                } else {
                    info!(
                        user_id = %user.id,
                        stripe_customer_id = %customer_id,
                        "stripe customer created for new sign-in"
                    );
                }
            }
            Err(e) => {
                warn!(
                    error = %e,
                    user_id = %user.id,
                    "stripe customer creation failed — sign-in proceeds, follow-up sync will retry"
                );
            }
        }
    }

    let token = state
        .oauth_store
        .issue_bearer_token(&user.id, DEFAULT_SIGNIN_SCOPE)
        .await
        .map_err(|e| GitHubSigninError::MintToken(e.to_string()))?;

    info!(
        user_id = %user.id,
        plan = %user.plan_id,
        "github sign-in completed; bearer token issued"
    );

    let redirect = append_query(
        &pending.loopback_redirect,
        &[("token", &token), ("state", &pending.client_state)],
    );
    Ok((StatusCode::FOUND, [(header::LOCATION, redirect)]).into_response())
}

fn is_loopback_redirect(url: &str) -> bool {
    // RFC 8252 §7.3 — native-app PKCE flows MUST use loopback. Anything
    // else here is a token-exfiltration attempt.
    url.starts_with("http://127.0.0.1:")
        || url.starts_with("http://[::1]:")
        || url.starts_with("http://localhost:")
}

fn prune_expired(pending: &mut HashMap<String, PendingState>) {
    pending.retain(|_, v| v.created_at.elapsed() <= PENDING_TTL);
}

fn append_query(base: &str, params: &[(&str, &str)]) -> String {
    let mut out = String::from(base);
    let mut first = !base.contains('?');
    for (k, v) in params {
        out.push(if first { '?' } else { '&' });
        first = false;
        out.push_str(&url_percent_encode(k));
        out.push('=');
        out.push_str(&url_percent_encode(v));
    }
    out
}

/// PKCE code-challenge = base64url(sha256(verifier)).
fn pkce_s256(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    base64_url_no_pad(&hasher.finalize())
}

/// Generate a URL-safe identifier from `bytes` cryptographically-random
/// bytes. `getrandom::fill` delegates to the platform CSPRNG.
///
/// # Panics
///
/// Panics if the OS RNG is unreachable. Sign-in cannot proceed safely
/// without a real RNG; failing loudly here beats silently emitting a
/// predictable verifier.
fn random_url_safe_id(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS RNG must be available for PKCE");
    base64_url_no_pad(&buf)
}

fn base64_url_no_pad(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((data.len() * 4).div_ceil(3));
    let mut i = 0;
    while i < data.len() {
        let b0 = u32::from(data[i]);
        let b1 = if i + 1 < data.len() {
            u32::from(data[i + 1])
        } else {
            0
        };
        let b2 = if i + 2 < data.len() {
            u32::from(data[i + 2])
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < data.len() {
            out.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        }
        if i + 2 < data.len() {
            out.push(ALPHABET[(triple & 0x3F) as usize] as char);
        }
        i += 3;
    }
    out
}

fn url_percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let allowed = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if allowed {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_nibble(b >> 4));
            out.push(hex_nibble(b & 0x0f));
        }
    }
    out
}

fn hex_nibble(b: u8) -> char {
    match b {
        0..=9 => (b'0' + b) as char,
        10..=15 => (b'A' + (b - 10)) as char,
        _ => '0',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_redirect_accepts_127_and_localhost() {
        assert!(is_loopback_redirect("http://127.0.0.1:53219/cb"));
        assert!(is_loopback_redirect("http://localhost:53219/cb"));
        assert!(is_loopback_redirect("http://[::1]:53219/cb"));
    }

    #[test]
    fn loopback_redirect_rejects_external_hosts() {
        assert!(!is_loopback_redirect("https://evil.example/cb"));
        assert!(!is_loopback_redirect("http://example.com/cb"));
        // Schemes other than `http://` are rejected — RFC 8252 §7.3
        // recommends http (over loopback) for native apps.
        assert!(!is_loopback_redirect("https://127.0.0.1:53219/cb"));
        // Bare loopback without a port is also rejected — the listener
        // always picks a kernel-assigned port.
        assert!(!is_loopback_redirect("http://127.0.0.1/cb"));
    }

    #[test]
    fn append_query_adds_separator_when_url_has_no_query() {
        let out = append_query("http://127.0.0.1:9/cb", &[("token", "abc"), ("state", "x")]);
        assert_eq!(out, "http://127.0.0.1:9/cb?token=abc&state=x");
    }

    #[test]
    fn append_query_appends_when_url_already_has_query() {
        let out = append_query("http://127.0.0.1:9/cb?foo=bar", &[("token", "abc")]);
        assert_eq!(out, "http://127.0.0.1:9/cb?foo=bar&token=abc");
    }

    #[test]
    fn append_query_percent_encodes_reserved_chars() {
        let out = append_query("http://x/cb", &[("token", "a/b c")]);
        assert!(out.contains("token=a%2Fb%20c"), "wrong: {out}");
    }

    #[test]
    fn random_url_safe_id_uses_pkce_alphabet_and_minimum_length() {
        let id = random_url_safe_id(32);
        assert!(
            id.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        );
        // RFC 7636 §4.1: verifier MUST be 43-128 base64url chars.
        assert!((43..=128).contains(&id.len()), "len {}", id.len());
    }

    #[test]
    fn pending_ttl_prunes_old_entries() {
        let mut map: HashMap<String, PendingState> = HashMap::new();
        map.insert(
            "fresh".into(),
            PendingState {
                code_verifier: "v".into(),
                loopback_redirect: "http://127.0.0.1:1/cb".into(),
                client_state: "s".into(),
                invite_token: None,
                created_at: Instant::now(),
            },
        );
        // Use a created_at safely past the TTL by subtracting the TTL
        // plus a comfortable margin.
        let aged = Instant::now()
            .checked_sub(PENDING_TTL + Duration::from_secs(1))
            .expect("test clock supports backdating");
        map.insert(
            "stale".into(),
            PendingState {
                code_verifier: "v".into(),
                loopback_redirect: "http://127.0.0.1:1/cb".into(),
                client_state: "s".into(),
                invite_token: None,
                created_at: aged,
            },
        );
        prune_expired(&mut map);
        assert!(map.contains_key("fresh"));
        assert!(!map.contains_key("stale"));
    }

    #[test]
    fn trim_trailing_slashes_normalises_base_urls() {
        assert_eq!(
            trim_trailing_slashes("https://mcp.ministr.ai///".into()),
            "https://mcp.ministr.ai"
        );
        assert_eq!(
            trim_trailing_slashes("https://mcp.ministr.ai".into()),
            "https://mcp.ministr.ai"
        );
        // A bare `/` is degenerate but shouldn't panic — the loop just
        // empties the string.
        assert_eq!(trim_trailing_slashes("/".into()), "");
    }
}
