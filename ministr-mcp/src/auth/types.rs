//! Record types that move across the OAuth storage boundary.
//!
//! These are `pub(crate)` so storage backends (`InMemory`, Cosmos) and handlers
//! share a single representation. `Serialize`/`Deserialize` lets backends
//! persist them as JSON without an intermediate DTO layer.

use serde::{Deserialize, Serialize};

/// A client registered via Dynamic Client Registration (RFC 7591).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)] // some fields are persisted but not yet read by handlers
pub(crate) struct RegisteredClient {
    pub(crate) client_id: String,
    pub(crate) client_secret: Option<String>,
    pub(crate) redirect_uris: Vec<String>,
    pub(crate) client_name: Option<String>,
    pub(crate) scope: String,
    pub(crate) registered_at: u64,
}

/// An issued authorization code awaiting exchange.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub(crate) struct AuthorizationCode {
    pub(crate) code: String,
    pub(crate) client_id: String,
    pub(crate) redirect_uri: String,
    pub(crate) scope: String,
    pub(crate) code_challenge: String,
    pub(crate) code_challenge_method: String,
    pub(crate) expires_at: u64,
}

/// An active bearer access token.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub(crate) struct AccessToken {
    pub(crate) token: String,
    pub(crate) client_id: String,
    pub(crate) scope: String,
    pub(crate) expires_at: u64,
}
