//! Cross-crate tenant-identity newtype for request extensions.
//!
//! The full `Tenant { subject, org_id, plan }` struct lives in
//! `ministr-mcp::auth::tenant` (MIT) — but `ministr-daemon` cannot
//! import from `ministr-mcp` (the dep arrow points the other way
//! since F1.2 sub-bullet 3 made MCP depend on the daemon's
//! `CorpusRegistry`).
//!
//! [`TenantId`] is the minimal newtype both sides can pass through
//! axum's typed request extensions without crate-coupling: the auth
//! middleware inserts it; the daemon's activity middleware (F1.4 sub-
//! bullet 2) reads it to attribute billable usage events.

use serde::{Deserialize, Serialize};

/// Token subject lifted into a distinct request-extension type.
///
/// axum extensions are keyed on type. A bare `String` would collide
/// with any other `String` extension some other middleware happens to
/// insert; the newtype gives us an unambiguous slot.
///
/// The value is the same as `ministr_mcp::auth::tenant::Tenant.subject`
/// (today the OAuth `client_id`; future SAML/OIDC adapters substitute
/// the issuer's `sub` claim).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TenantId(pub String);

impl TenantId {
    /// View the underlying subject string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for TenantId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for TenantId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let t = TenantId("client-42".to_string());
        let s = serde_json::to_string(&t).unwrap();
        let back: TenantId = serde_json::from_str(&s).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn from_str_and_string_yield_equal_values() {
        assert_eq!(TenantId::from("c"), TenantId::from("c".to_owned()));
    }

    #[test]
    fn as_str_exposes_inner() {
        assert_eq!(TenantId::from("client-7").as_str(), "client-7");
    }
}
