//! Cloud-side authentication surface — F1.3 GitHub sign-in flow.
//!
//! The cloud (`mcp.ministr.ai`) federates sign-in through GitHub via
//! [`crate::idp::GitHubIdp`] and persists the resolved identity into the
//! F1.2 `users` table via [`crate::users::upsert_github_user`]. The
//! resulting bearer token is minted through
//! [`ministr_mcp::auth::OAuthStore::issue_bearer_token`] so it validates
//! through the same middleware as the existing OAuth code-grant tokens —
//! API handlers downstream (corpora, billing, MCP tools) need no
//! changes.
//!
//! # Why this lives in `ministr-cloud`, not `ministr-mcp::auth`
//!
//! The self-issued OAuth issuer in `ministr-mcp::auth` is the open
//! local-stack default (auto-approves on `/oauth/authorize` because a
//! single-user CLI has no other user to consent on behalf of). Cloud
//! deployments stack GitHub-IdP federation ON TOP of the same
//! `OAuthStore` — the routes here are an additional sign-in pathway,
//! not a replacement.
//!
//! # Routes
//!
//! | Route | Verb | Purpose |
//! |---|---|---|
//! | `/auth/github/start`    | GET | Begin sign-in — bounce to GitHub |
//! | `/auth/github/callback` | GET | GitHub's redirect target — finishes the flow |
//!
//! The Tauri desktop client (or any RFC 8252 native-app loopback flow)
//! drives the sequence:
//!
//! 1. Bind a one-shot `127.0.0.1:0` listener.
//! 2. Open `<cloud>/auth/github/start?loopback_redirect=...&state=...`
//!    in the system browser.
//! 3. The cloud redirects to `github.com/login/oauth/authorize` with
//!    its OWN PKCE materials; receives the GitHub callback at
//!    `/auth/github/callback`.
//! 4. The cloud exchanges + upserts the user, mints a bearer token, and
//!    redirects to `<loopback_redirect>?token=...&state=...`.
//! 5. The loopback listener verifies `state` and stores the token in the
//!    OS keychain.

pub mod github_signin;

pub use github_signin::{
    github_signin_routes, GitHubSigninError, GitHubSigninState, DEFAULT_SIGNIN_SCOPE,
};
