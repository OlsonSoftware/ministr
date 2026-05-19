//! GitHub OAuth 2.0 implementation of [`IdentityProvider`].
//!
//! The cloud-mode default sign-in path at `mcp.ministr.ai`. Self-hosted
//! deployments continue to use the self-issued OAuth issuer in
//! `ministr-mcp::auth`; GitHub is one of several external providers
//! cloud admins can configure once F1.3's GitHub OAuth App is
//! registered on github.com.
//!
//! # Flow recap (RFC 6749 §4.1 + PKCE)
//!
//! 1. The cloud auth handler redirects the user to
//!    `<authorize_url>` with a generated `state` + `code_challenge`.
//! 2. GitHub redirects back to the cloud's callback with `code` +
//!    `state`. The handler verifies `state` and calls
//!    [`GitHubIdp::exchange`].
//! 3. `exchange` POSTs the code to
//!    `https://github.com/login/oauth/access_token`, then GETs
//!    `https://api.github.com/user` (and `/user/emails` for the
//!    verified-primary fallback if the public email is null) and
//!    normalises everything into a [`ResolvedIdentity`].
//!
//! The cloud handler then upserts `users.github_id` (F1.2 schema) so
//! repeat sign-ins land on the same row.

use serde::Deserialize;
use std::time::Duration;
use tracing::debug;

use super::{IdentityProvider, IdpError, ResolvedIdentity};

/// Canonical issuer value persisted alongside `users.github_id`.
pub const GITHUB_ISSUER: &str = "https://github.com";

const AUTHORIZE_URL: &str = "https://github.com/login/oauth/authorize";
const TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const USER_URL: &str = "https://api.github.com/user";
const USER_EMAILS_URL: &str = "https://api.github.com/user/emails";

/// Scopes requested at authorize-time. `read:user` covers profile +
/// account metadata; `user:email` lets us read the verified primary
/// email even when it's not public on the GitHub profile.
const SCOPES: &str = "read:user user:email";

/// User-Agent header — GitHub's REST API requires every request to
/// identify itself.
const USER_AGENT: &str = "ministr-cloud-idp/1 (+https://ministr.ai)";

/// GitHub OAuth App credentials, owned by the cloud auth handler.
///
/// Construct once at cloud start-up from `MINISTR_GITHUB_CLIENT_ID` +
/// `MINISTR_GITHUB_CLIENT_SECRET` (the App registration happens on
/// github.com — outside this crate). The struct holds an HTTP client
/// configured with the right timeout + User-Agent so the rest of the
/// auth path stays transport-agnostic.
#[derive(Debug, Clone)]
pub struct GitHubIdp {
    client_id: String,
    client_secret: String,
    http: reqwest::Client,
}

impl GitHubIdp {
    /// Build the `IdP` with the App's credentials. Both inputs must be
    /// non-empty; an empty `client_id` means the App is not yet
    /// registered, in which case the cloud handler should refuse to
    /// mount the GitHub sign-in route.
    ///
    /// # Errors
    ///
    /// Returns [`IdpError::Protocol`] if either credential is blank or
    /// the underlying reqwest client cannot be built.
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Result<Self, IdpError> {
        let client_id = client_id.into();
        let client_secret = client_secret.into();
        if client_id.trim().is_empty() {
            return Err(IdpError::Protocol(
                "github idp: client_id is empty".into(),
            ));
        }
        if client_secret.trim().is_empty() {
            return Err(IdpError::Protocol(
                "github idp: client_secret is empty".into(),
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(USER_AGENT)
            .build()
            .map_err(|e| IdpError::Transport(format!("github idp: http client: {e}")))?;
        Ok(Self {
            client_id,
            client_secret,
            http,
        })
    }
}

/// Minimal subset of GitHub's `/user` response the `IdP` consumes.
#[derive(Debug, Deserialize)]
struct GitHubUser {
    id: i64,
    login: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
}

/// Minimal subset of GitHub's `/user/emails` response. The primary
/// verified address is what we persist when the user has elected to
/// keep their profile email private.
#[derive(Debug, Deserialize)]
struct GitHubEmail {
    email: String,
    primary: bool,
    verified: bool,
}

/// Minimal subset of GitHub's `/login/oauth/access_token` response.
#[derive(Debug, Deserialize)]
struct GitHubTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

impl IdentityProvider for GitHubIdp {
    fn name(&self) -> &'static str {
        "github"
    }

    fn authorize_url(&self, state: &str, redirect_uri: &str, code_challenge: &str) -> String {
        // Manual percent-encoding keeps the crate's dep footprint
        // smaller than pulling in `url` for one call site; the inputs
        // are bounded (caller-generated UUIDs / IDs).
        format!(
            "{AUTHORIZE_URL}?response_type=code&client_id={cid}&redirect_uri={ru}\
             &scope={sc}&state={st}&code_challenge={cc}&code_challenge_method=S256",
            cid = percent_encode(&self.client_id),
            ru = percent_encode(redirect_uri),
            sc = percent_encode(SCOPES),
            st = percent_encode(state),
            cc = percent_encode(code_challenge),
        )
    }

    async fn exchange(
        &self,
        code: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> Result<ResolvedIdentity, IdpError> {
        // Token exchange. GitHub accepts both `application/json` and
        // form-encoded; JSON keeps the parsing path uniform.
        let token: GitHubTokenResponse = self
            .http
            .post(TOKEN_URL)
            .header("accept", "application/json")
            .json(&serde_json::json!({
                "client_id":     self.client_id,
                "client_secret": self.client_secret,
                "code":          code,
                "redirect_uri":  redirect_uri,
                "code_verifier": code_verifier,
            }))
            .send()
            .await
            .map_err(|e| IdpError::Transport(format!("github token: {e}")))?
            .error_for_status()
            .map_err(|e| IdpError::Protocol(format!("github token status: {e}")))?
            .json()
            .await
            .map_err(|e| IdpError::Protocol(format!("github token parse: {e}")))?;
        if let Some(err) = token.error {
            return Err(IdpError::Protocol(format!(
                "github token error {err}: {}",
                token.error_description.unwrap_or_default()
            )));
        }
        let access_token = token.access_token.ok_or_else(|| {
            IdpError::Protocol("github token response missing access_token".into())
        })?;
        debug!("github idp: token exchange succeeded");

        // Profile fetch.
        let user: GitHubUser = self
            .http
            .get(USER_URL)
            .bearer_auth(&access_token)
            .header("accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| IdpError::Transport(format!("github user: {e}")))?
            .error_for_status()
            .map_err(|e| IdpError::Protocol(format!("github user status: {e}")))?
            .json()
            .await
            .map_err(|e| IdpError::Protocol(format!("github user parse: {e}")))?;

        // Fall back to /user/emails when the profile email is null —
        // GitHub users who keep their email private still have a
        // verified primary that the `user:email` scope grants us.
        let email = if user.email.is_some() {
            user.email
        } else {
            primary_verified_email(&self.http, &access_token).await?
        };

        Ok(ResolvedIdentity {
            issuer: GITHUB_ISSUER.into(),
            subject: user.id.to_string(),
            email,
            display_name: user.name.or_else(|| Some(user.login.clone())),
            github_id: Some(user.id),
        })
    }
}

async fn primary_verified_email(
    http: &reqwest::Client,
    access_token: &str,
) -> Result<Option<String>, IdpError> {
    let list: Vec<GitHubEmail> = http
        .get(USER_EMAILS_URL)
        .bearer_auth(access_token)
        .header("accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| IdpError::Transport(format!("github emails: {e}")))?
        .error_for_status()
        .map_err(|e| IdpError::Protocol(format!("github emails status: {e}")))?
        .json()
        .await
        .map_err(|e| IdpError::Protocol(format!("github emails parse: {e}")))?;
    Ok(list
        .into_iter()
        .find(|e| e.primary && e.verified)
        .map(|e| e.email))
}

/// RFC 3986 unreserved set: `A-Za-z0-9-_.~`. Mirrors the helper in
/// `ministr-app/src-tauri/src/commands_cloud.rs::url_encode`; copied so
/// this crate doesn't grow a `url`-crate dep for one call site.
fn percent_encode(s: &str) -> String {
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

    fn idp() -> GitHubIdp {
        GitHubIdp::new("test-client-id", "test-secret").expect("construct GitHubIdp")
    }

    #[test]
    fn new_rejects_empty_credentials() {
        assert!(GitHubIdp::new("", "secret").is_err());
        assert!(GitHubIdp::new("client", "").is_err());
        assert!(GitHubIdp::new("   ", "secret").is_err());
    }

    #[test]
    fn name_is_stable() {
        assert_eq!(idp().name(), "github");
    }

    #[test]
    fn authorize_url_includes_required_parameters() {
        let url = idp().authorize_url(
            "state-nonce",
            "https://cloud.example/callback",
            "challenge-x",
        );
        assert!(url.starts_with(AUTHORIZE_URL), "wrong base: {url}");
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=test-client-id"));
        assert!(url.contains("state=state-nonce"));
        assert!(url.contains("code_challenge=challenge-x"));
        assert!(url.contains("code_challenge_method=S256"));
        // Scopes percent-encoded — space becomes %20 under RFC 3986.
        assert!(url.contains("scope=read%3Auser%20user%3Aemail"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Fcloud.example%2Fcallback"));
    }

    #[test]
    fn percent_encode_preserves_unreserved_chars() {
        assert_eq!(percent_encode("abcDEF-_.~012"), "abcDEF-_.~012");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("a:b"), "a%3Ab");
        assert_eq!(percent_encode("a/b"), "a%2Fb");
    }

    #[test]
    fn github_user_parses_minimum_fields() {
        let payload = serde_json::json!({
            "id": 42_i64,
            "login": "octocat",
            "name": "The Octocat",
            "email": "octo@example.com"
        });
        let parsed: GitHubUser = serde_json::from_value(payload).unwrap();
        assert_eq!(parsed.id, 42);
        assert_eq!(parsed.login, "octocat");
        assert_eq!(parsed.email.as_deref(), Some("octo@example.com"));
    }

    #[test]
    fn github_user_tolerates_missing_optionals() {
        let payload = serde_json::json!({ "id": 7, "login": "ghost" });
        let parsed: GitHubUser = serde_json::from_value(payload).unwrap();
        assert!(parsed.name.is_none());
        assert!(parsed.email.is_none());
    }

    #[test]
    fn github_token_response_handles_error_shape() {
        let payload = serde_json::json!({
            "error": "bad_verification_code",
            "error_description": "The code passed is incorrect or expired."
        });
        let parsed: GitHubTokenResponse = serde_json::from_value(payload).unwrap();
        assert!(parsed.access_token.is_none());
        assert_eq!(parsed.error.as_deref(), Some("bad_verification_code"));
    }
}
