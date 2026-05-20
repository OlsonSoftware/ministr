//! GitHub App installation-token minting seam.
//!
//! Open-core boundary that lets the daemon's `clone_repo` handler
//! accept a `github_installation_id` without depending on the closed
//! `ministr-cloud` crate. The cloud crate ships a
//! `GitHubAppClient` that implements this trait by RS256-signing a JWT
//! as the App and exchanging it at
//! `POST /app/installations/{id}/access_tokens`.
//!
//! # Why async — and why `Pin<Box<dyn Future>>`
//!
//! Mirrors the [`crate::usage::UsageSink`] rationale upside-down. That
//! trait is fire-and-forget so it can stay sync + `dyn`-safe; THIS
//! trait genuinely needs to await the GitHub token endpoint before the
//! caller can proceed with the git clone. We pay the boxed-future
//! allocation explicitly (one alloc per clone) instead of pulling in
//! `async-trait`, matching the workspace convention of "no surprise
//! dependencies for one-off async-dyn calls".

use std::future::Future;
use std::pin::Pin;

/// Errors a [`InstallationTokenMinter`] implementation can surface.
/// Kept narrow and string-based so the open-core trait doesn't depend
/// on the cloud crate's `reqwest` / `jsonwebtoken` error taxonomy.
#[derive(Debug, thiserror::Error)]
pub enum MintError {
    /// Local-side failure before the network call (bad key, JWT
    /// construction failed, etc.).
    #[error("installation token mint setup: {0}")]
    Setup(String),
    /// Network-layer failure talking to GitHub.
    #[error("installation token mint transport: {0}")]
    Transport(String),
    /// GitHub returned a non-2xx response or a malformed body.
    #[error("installation token mint protocol: {0}")]
    Protocol(String),
}

/// Mints short-lived installation access tokens for GitHub Apps.
///
/// Cloud deployments wire `ministr_cloud::github::GitHubAppClient` into
/// the daemon's `AppState` via [`with_installation_minter`]; self-hosted
/// serve leaves the field `None` and the PAT-in-URL path stays in
/// effect.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn InstallationTokenMinter>` inside the daemon state.
///
/// [`with_installation_minter`]: trait.InstallationTokenMinter.html
pub trait InstallationTokenMinter: Send + Sync + std::fmt::Debug {
    /// Mint a fresh installation access token for the GitHub App's
    /// installation `installation_id`. Implementations are encouraged
    /// to cache tokens internally — GitHub-issued tokens last about an
    /// hour, so caching dramatically reduces JWT-signing overhead on
    /// the clone hot path.
    ///
    /// Returns the opaque token string; the daemon splices it into the
    /// clone URL as `https://x-access-token:<token>@github.com/...`.
    ///
    /// # Errors
    ///
    /// Returns [`MintError::Setup`] for local-side preparation issues,
    /// [`MintError::Transport`] for network failures, and
    /// [`MintError::Protocol`] for malformed / unauthorised GitHub
    /// responses.
    fn mint<'a>(
        &'a self,
        installation_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, MintError>> + Send + 'a>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Debug)]
    struct StubMinter {
        tag: &'static str,
    }

    impl InstallationTokenMinter for StubMinter {
        fn mint<'a>(
            &'a self,
            installation_id: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<String, MintError>> + Send + 'a>> {
            Box::pin(async move {
                if installation_id.is_empty() {
                    Err(MintError::Protocol("empty installation_id".into()))
                } else {
                    Ok(format!("{}-{installation_id}", self.tag))
                }
            })
        }
    }

    #[tokio::test]
    async fn trait_is_dyn_compatible_and_awaitable() {
        let m: Arc<dyn InstallationTokenMinter> = Arc::new(StubMinter { tag: "tok" });
        let t = m.mint("123").await.expect("stub minter returns Ok");
        assert_eq!(t, "tok-123");
        let err = m.mint("").await.expect_err("empty id rejected");
        assert!(matches!(err, MintError::Protocol(_)));
    }
}
