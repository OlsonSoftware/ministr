//! Session bundle blob storage — open-core seam (F6.2-c).
//!
//! Cloud impl uploads the tar artefact under
//! `sessions/{tenant}/{session_id}/{ts}.tar` in blob storage and
//! returns a short-lived signed URL. The trait lives in `ministr-api`
//! (MIT) so the open-core `handle_export` route in `ministr-mcp` can
//! upload-and-sign via a `dyn`-typed seam without depending on
//! `ministr-cloud`.
//!
//! # Why separate from [`SessionStorage`] and [`DropsLedger`]
//!
//! [`SessionStorage`] persists the session's live state (budget,
//! coherence). [`DropsLedger`] is an append-only eviction log. This
//! seam persists the *exported bundle artefact* — a packaged tar
//! representing one point-in-time export.
//!
//! Concretely: an export call uploads N bytes and returns a URL the
//! user can fetch within the TTL. Distinct lifecycle from the live
//! session state.
//!
//! [`SessionStorage`]: crate::session_storage::SessionStorage
//! [`DropsLedger`]: crate::drops_ledger::DropsLedger
//!
//! # Download path
//!
//! The store also exposes [`SessionBundleStore::verify_and_get`] so a
//! download route can validate the signed URL and stream the bytes
//! back. Implementations decide the URL scheme (HMAC over path +
//! expiry, Azure user-delegation SAS, etc.); the trait stays opaque
//! about it.

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};

/// Errors a [`SessionBundleStore`] implementation can surface.
#[derive(Debug, thiserror::Error)]
pub enum SessionBundleStoreError {
    /// Storage layer rejected the call (network, schema drift, etc.).
    #[error("session bundle store: {0}")]
    Storage(String),
    /// The signed URL's token was invalid, expired, or malformed.
    /// `verify_and_get` callers map this to HTTP 401/403.
    #[error("session bundle store: invalid token")]
    InvalidToken,
    /// The blob the signed URL points at was not found (already GC'd,
    /// wrong path).
    #[error("session bundle store: blob not found")]
    NotFound,
}

/// One uploaded session bundle and its short-lived signed URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedBundleUrl {
    /// Absolute URL the client follows to download the bundle.
    pub url: String,
    /// Wall-clock ISO-8601 UTC timestamp when the signed URL expires.
    pub expires_at: String,
}

/// Returned future shape for [`SessionBundleStore::put_and_sign`].
pub type PutAndSignFuture<'a> =
    Pin<Box<dyn Future<Output = Result<SignedBundleUrl, SessionBundleStoreError>> + Send + 'a>>;

/// Returned future shape for [`SessionBundleStore::verify_and_get`].
pub type VerifyAndGetFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<u8>, SessionBundleStoreError>> + Send + 'a>>;

/// Blob-backed store for exported session bundles.
///
/// Implementations must be `Send + Sync` so they can be stored as
/// `Arc<dyn SessionBundleStore>` inside `SessionExportState`.
pub trait SessionBundleStore: Send + Sync + std::fmt::Debug {
    /// Upload `bytes` for the `(tenant_id, session_id)` pair and mint
    /// a signed URL the caller can hand to the user.
    fn put_and_sign<'a>(
        &'a self,
        tenant_id: &'a str,
        session_id: &'a str,
        bytes: Vec<u8>,
    ) -> PutAndSignFuture<'a>;

    /// Verify the signed token against `blob_path`, fetch the blob,
    /// and return the bytes. Returns [`SessionBundleStoreError::InvalidToken`]
    /// for any signature / expiry failure.
    ///
    /// `blob_path` is the path component (e.g.
    /// `sessions/{tenant}/{session_id}/{ts}.tar`) — already
    /// URL-decoded by the axum extractor. `token` carries the signing
    /// material and expiry.
    fn verify_and_get<'a>(&'a self, blob_path: &'a str, token: &'a str) -> VerifyAndGetFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Default)]
    struct StubStore {
        blobs: Mutex<Vec<(String, Vec<u8>)>>,
    }

    impl SessionBundleStore for StubStore {
        fn put_and_sign<'a>(
            &'a self,
            tenant_id: &'a str,
            session_id: &'a str,
            bytes: Vec<u8>,
        ) -> PutAndSignFuture<'a> {
            Box::pin(async move {
                let path = format!("sessions/{tenant_id}/{session_id}/stub.tar");
                self.blobs.lock().unwrap().push((path.clone(), bytes));
                Ok(SignedBundleUrl {
                    url: format!("https://example.test/{path}?token=stub"),
                    expires_at: "2026-05-22T00:00:00Z".into(),
                })
            })
        }
        fn verify_and_get<'a>(
            &'a self,
            blob_path: &'a str,
            token: &'a str,
        ) -> VerifyAndGetFuture<'a> {
            Box::pin(async move {
                if token != "stub" {
                    return Err(SessionBundleStoreError::InvalidToken);
                }
                let blobs = self.blobs.lock().unwrap();
                blobs
                    .iter()
                    .find(|(p, _)| p == blob_path)
                    .map(|(_, b)| b.clone())
                    .ok_or(SessionBundleStoreError::NotFound)
            })
        }
    }

    #[tokio::test]
    async fn put_then_verify_round_trips_through_dyn() {
        let stub = Arc::new(StubStore::default());
        let store: Arc<dyn SessionBundleStore> = Arc::clone(&stub) as _;
        let signed = store
            .put_and_sign("tenant-1", "sess-1", b"tar-bytes".to_vec())
            .await
            .unwrap();
        assert!(signed.url.contains("sessions/tenant-1/sess-1/"));
        assert!(signed.url.ends_with("?token=stub"));
        let bytes = store
            .verify_and_get("sessions/tenant-1/sess-1/stub.tar", "stub")
            .await
            .unwrap();
        assert_eq!(bytes, b"tar-bytes".to_vec());
    }

    #[tokio::test]
    async fn invalid_token_is_rejected() {
        let stub = Arc::new(StubStore::default());
        let store: Arc<dyn SessionBundleStore> = Arc::clone(&stub) as _;
        store
            .put_and_sign("t1", "s1", b"data".to_vec())
            .await
            .unwrap();
        let err = store
            .verify_and_get("sessions/t1/s1/stub.tar", "wrong")
            .await
            .unwrap_err();
        assert!(matches!(err, SessionBundleStoreError::InvalidToken));
    }

    #[tokio::test]
    async fn unknown_blob_path_returns_not_found() {
        let stub = Arc::new(StubStore::default());
        let store: Arc<dyn SessionBundleStore> = Arc::clone(&stub) as _;
        let err = store
            .verify_and_get("sessions/t/s/never-uploaded.tar", "stub")
            .await
            .unwrap_err();
        assert!(matches!(err, SessionBundleStoreError::NotFound));
    }
}
