//! Identity-provider abstraction for the cloud surface.
//!
//! The seam every cloud-mode external `IdP` plugs into. The self-issued
//! issuer in `ministr-mcp/src/auth/` (the local-stack default) sits
//! OUTSIDE this trait — it's the path taken when no `IdentityProvider`
//! is configured. Cloud deployments at `mcp.ministr.ai` configure one
//! or more `IdP`s (GitHub for Pro, OIDC for Enterprise SSO) and route the
//! sign-in flow through them.
//!
//! # Variants land as they're needed
//!
//! - **F1.3** — `GitHubIdp` (this F-item, separate sub-bullet) — public
//!   GitHub OAuth app for the cloud's main sign-in button. Populates
//!   `users.github_id` on first sign-in.
//! - **F5.1** — SAML — its own `AssertionConsumer` surface (NOT this
//!   trait); the `IdP`-initiated and SP-initiated flows don't fit a
//!   single `authorize_url -> exchange` shape.
//! - **F5.2** — `OidcIdp` — generic OIDC discovery via
//!   `.well-known/openid-configuration`, JWKS-validated ID tokens.
//!
//! # SOLID layering
//!
//! Mirrors `ministr-mcp::auth::storage`: the `IdentityProvider` trait
//! defines the contract, concrete providers (under `idp::github`,
//! `idp::oidc`, …) implement it. Cloud handlers depend on the trait,
//! not concrete types, so adding a provider doesn't touch any handler.

use serde::{Deserialize, Serialize};

/// The cloud-mode identity surface. Implementations cover the OAuth
/// 2.0 authorize-code + PKCE shape that GitHub and OIDC providers
/// share.
///
/// Methods follow the project convention of returning `impl Future +
/// Send` (same as `OAuthStorage` in `ministr-mcp::auth::storage`) so
/// static dispatch monomorphises cleanly. A concrete dispatch enum
/// modeled on `OAuthBackend` lands once a second `IdP` exists; for the
/// single-provider case the cloud handler can hold an
/// `Arc<GitHubIdp>` directly.
pub trait IdentityProvider: Send + Sync + std::fmt::Debug {
    /// Short stable provider name, e.g. `"github"`, `"google"`,
    /// `"microsoft"`, `"oidc:keycloak"`. Persisted in audit logs and
    /// referenced when joining to provider-specific user columns
    /// (`users.github_id`, future `users.google_id`).
    fn name(&self) -> &str;

    /// Build the authorize-URL the user is redirected to. `state` and
    /// `code_challenge` are caller-generated (the cloud auth handler
    /// owns both — same flow as the desktop client). The trait is
    /// transport-agnostic so SP-initiated and `IdP`-initiated wiring
    /// stay separate concerns.
    fn authorize_url(&self, state: &str, redirect_uri: &str, code_challenge: &str) -> String;

    /// Exchange the authorization code for a resolved identity. Wraps
    /// the provider's token-endpoint POST plus whatever user-info call
    /// (or ID-token claim extraction) is required to populate
    /// [`ResolvedIdentity`].
    ///
    /// # Errors
    ///
    /// Returns [`IdpError::Transport`] for network failures and
    /// [`IdpError::Protocol`] for malformed or unauthorised responses
    /// from the provider.
    fn exchange(
        &self,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> impl Future<Output = Result<ResolvedIdentity, IdpError>> + Send;
}

/// Provider-resolved identity, normalised across `IdP` dialects.
///
/// `subject` is the stable per-provider identifier (GitHub user ID,
/// Google `sub` claim, etc.); the cloud's `users` table joins on
/// `(issuer, subject)`. Provider-specific FK targets — currently just
/// `github_id` — sit alongside as optional fields so future providers
/// don't add new required columns to existing rows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolvedIdentity {
    /// Provider issuer URL or short name. Stable per provider; e.g.
    /// `"https://github.com"` for GitHub, the OIDC `issuer` claim for
    /// OIDC providers.
    pub issuer: String,
    /// Provider-stable subject identifier — the `sub` claim for OIDC,
    /// GitHub's numeric user id stringified, etc.
    pub subject: String,
    /// Verified email address when the provider supplies one. None
    /// when the user has no public/verified email or the scope was
    /// not requested.
    pub email: Option<String>,
    /// Display name (`name` claim or GitHub `name`/`login`). UI-only;
    /// never used for joining.
    pub display_name: Option<String>,
    /// GitHub user id, populated only by `GitHubIdp`. Future
    /// `google_id`/`microsoft_id` fields slot in alongside as their
    /// providers land — leaving them `None` here keeps the row shape
    /// stable for non-GitHub `IdP`s.
    pub github_id: Option<i64>,
}

/// Errors surfaced by [`IdentityProvider::exchange`].
#[derive(Debug, thiserror::Error)]
pub enum IdpError {
    /// Network-layer failure (timeout, DNS, TLS, etc.).
    #[error("identity provider transport error: {0}")]
    Transport(String),
    /// Provider returned a response we couldn't parse or that
    /// indicates the flow is invalid (unauthorised, expired code,
    /// CSRF state mismatch, missing required claim, etc.).
    #[error("identity provider protocol error: {0}")]
    Protocol(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-test impl used only to prove the trait surface compiles +
    /// is implementable from a downstream crate without surprises. A
    /// real provider lives in a sibling module (e.g. `idp::github`)
    /// once F1.3's GitHub sub-bullet lands.
    #[derive(Debug)]
    struct StubIdp {
        name: &'static str,
        identity: ResolvedIdentity,
    }

    impl IdentityProvider for StubIdp {
        fn name(&self) -> &str {
            self.name
        }
        fn authorize_url(
            &self,
            state: &str,
            redirect_uri: &str,
            code_challenge: &str,
        ) -> String {
            format!(
                "https://stub.example/authorize?state={state}&redirect_uri={redirect_uri}&code_challenge={code_challenge}"
            )
        }
        async fn exchange(
            &self,
            _code: &str,
            _redirect_uri: &str,
            _code_verifier: &str,
        ) -> Result<ResolvedIdentity, IdpError> {
            Ok(self.identity.clone())
        }
    }

    fn sample_identity() -> ResolvedIdentity {
        ResolvedIdentity {
            issuer: "https://stub.example".into(),
            subject: "user-42".into(),
            email: Some("user@example.com".into()),
            display_name: Some("Test User".into()),
            github_id: None,
        }
    }

    #[test]
    fn resolved_identity_round_trips() {
        let original = sample_identity();
        let s = serde_json::to_string(&original).unwrap();
        let back: ResolvedIdentity = serde_json::from_str(&s).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn resolved_identity_omits_optional_fields_when_none() {
        let r = ResolvedIdentity {
            issuer: "https://x".into(),
            subject: "s".into(),
            email: None,
            display_name: None,
            github_id: None,
        };
        let s = serde_json::to_string(&r).unwrap();
        // We do NOT skip None — keeping all keys present makes the
        // shape stable across providers. The future quota / audit
        // middleware can rely on every field existing in the JSON.
        for key in ["issuer", "subject", "email", "display_name", "github_id"] {
            assert!(s.contains(key), "missing {key} in {s}");
        }
    }

    #[tokio::test]
    async fn trait_is_implementable_and_usable() {
        let provider = StubIdp {
            name: "stub",
            identity: sample_identity(),
        };
        assert_eq!(provider.name(), "stub");
        let url = provider.authorize_url("nonce-1", "http://127.0.0.1/cb", "challenge-1");
        assert!(url.contains("state=nonce-1"));
        assert!(url.contains("code_challenge=challenge-1"));
        let identity = provider
            .exchange("dummy-code", "http://127.0.0.1/cb", "verifier-1")
            .await
            .expect("stub never errors");
        assert_eq!(identity.subject, "user-42");
    }

    #[test]
    fn idp_error_renders_human_readable_messages() {
        let t = IdpError::Transport("dns timeout".into());
        let p = IdpError::Protocol("missing access_token field".into());
        assert!(t.to_string().contains("transport"));
        assert!(p.to_string().contains("protocol"));
    }
}
