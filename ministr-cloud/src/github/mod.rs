//! GitHub App authentication for the cloud (F2.1).
//!
//! Companion to [`crate::idp::github`] (the user-facing sign-in `IdP`) —
//! this module handles SERVER-to-GitHub authentication where the cloud
//! itself acts as an installed App on a customer's repository.
//!
//! Today's only consumer is the daemon's `clone_repo` handler: when a
//! Pro/Team user clones a private repo, the cloud mints a short-lived
//! installation access token via [`GitHubAppClient`] and splices it
//! into the clone URL. The token never persists server-side — it's
//! re-minted on demand per indexing job (with a brief in-process
//! cache so repeated calls inside the App's 1-hour TTL don't re-sign
//! JWTs).

pub mod app;

pub use app::{GitHubAppClient, GitHubAppError};
