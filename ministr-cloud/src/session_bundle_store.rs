//! F6.2-c — Azure Blob-backed session bundle store.
//!
//! Implements [`ministr_api::SessionBundleStore`]. Uploads exported
//! session bundles under `sessions/{tenant}/{session_id}/{ts}.tar`
//! inside a dedicated container and mints a short-lived signed URL
//! the caller hands to the user.
//!
//! # URL scheme
//!
//! ```text
//! {cloud_base}/api/v1/sessions/bundles/{blob_path}?expires={unix}&sig={hex}
//! ```
//!
//! Where `sig = hex(HMAC-SHA256(secret, blob_path + "\n" + expires_unix))`.
//! HMAC-SHA256 is reused from the F3.5 webhook dispatcher and the
//! Stripe-inbound verifier — same shape, same `hmac` + `sha2` deps.
//!
//! # Why not Azure SAS
//!
//! The official Azure Rust SDK (v1, GA'd May 2026) does not yet expose
//! user-delegation SAS minting. We could fall back to shared-key SAS,
//! but production pods authenticate via Managed Identity and don't
//! have the account key. Self-signed HMAC tokens served by our own
//! download route avoid the entire SAS-vs-MI mismatch, give us hooks
//! for metering / audit in a future iteration, and keep the bytes
//! flowing through the same TLS termination as the rest of the API.
//! Bundle sizes are small (KBs) so the proxy hop is cheap.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use azure_core::credentials::TokenCredential;
use azure_core::http::{RequestContent, Url};
use azure_storage_blob::{BlobContainerClient, BlobServiceClient};
use hmac::{Hmac, Mac};
use ministr_api::{
    PutAndSignFuture, SessionBundleStore, SessionBundleStoreError, SignedBundleUrl,
    VerifyAndGetFuture,
};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use tracing::debug;

use crate::blob::{BlobError, BlobResult};

type HmacSha256 = Hmac<Sha256>;

/// Default container name. Operators can override via the wiring
/// factory; the deploy infra creates it idempotently.
pub const DEFAULT_SESSION_BUNDLE_CONTAINER: &str = "ministr-session-bundles";

/// Signed-URL TTL — 24 hours per the F6.2-c roadmap spec.
pub const DEFAULT_SIGNED_URL_TTL_SECS: u64 = 24 * 60 * 60;

/// Azure Blob-backed session bundle store.
///
/// Holds its own [`BlobContainerClient`] (the SDK type is not `Clone`,
/// so we wrap in `Arc` at the caller site if multiple tasks need a
/// handle). Distinct container from `CorpusBlobStore` — different
/// data lifecycles, different ACLs.
pub struct CloudSessionBundleStore {
    container: BlobContainerClient,
    secret: Vec<u8>,
    cloud_base_url: String,
    ttl_secs: u64,
}

impl std::fmt::Debug for CloudSessionBundleStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudSessionBundleStore")
            .field("ttl_secs", &self.ttl_secs)
            .finish_non_exhaustive()
    }
}

impl CloudSessionBundleStore {
    /// Construct a store using a caller-supplied credential. Mirrors
    /// [`crate::blob::CorpusBlobStore::with_credential`]'s shape.
    ///
    /// `cloud_base_url` is the absolute base URL the signed link
    /// resolves through — e.g. `https://mcp.ministr.ai` in prod.
    /// `secret` must be at least 32 bytes; shorter secrets refuse to
    /// construct.
    ///
    /// # Errors
    ///
    /// Fails when `account_name` is malformed, when `secret` is shorter
    /// than the 32-byte floor, or when the service-client construction
    /// errors.
    pub fn with_credential(
        account_name: &str,
        container_name: &str,
        credential: Arc<dyn TokenCredential>,
        cloud_base_url: impl Into<String>,
        secret: Vec<u8>,
    ) -> BlobResult<Self> {
        if secret.len() < 32 {
            return Err(BlobError::Azure(azure_core::Error::with_message(
                azure_core::error::ErrorKind::Other,
                "session bundle signing secret must be at least 32 bytes",
            )));
        }
        let service_url =
            Url::parse(&format!("https://{account_name}.blob.core.windows.net/")).map_err(|e| {
                BlobError::Azure(azure_core::Error::with_message(
                    azure_core::error::ErrorKind::Other,
                    e.to_string(),
                ))
            })?;
        let service = BlobServiceClient::new(service_url, Some(credential), None)?;
        let container = service.blob_container_client(container_name);
        debug!(
            account = account_name,
            container = container_name,
            "opened session bundle blob store"
        );
        Ok(Self {
            container,
            secret,
            cloud_base_url: cloud_base_url.into(),
            ttl_secs: DEFAULT_SIGNED_URL_TTL_SECS,
        })
    }

    /// Idempotently create the underlying container. Safe to call on
    /// every pod boot — an `already-exists` response is swallowed.
    ///
    /// # Errors
    ///
    /// Surfaces any non-409 error from the storage service.
    pub async fn ensure_container(&self) -> BlobResult<()> {
        match self.container.create(None).await {
            Ok(_) => Ok(()),
            Err(e) if is_already_exists(&e) => {
                debug!("session bundle container already exists; reusing");
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Override the signed URL TTL. Default 24h matches the F6.2-c
    /// spec; tests use shorter values.
    #[must_use]
    pub fn with_ttl_secs(mut self, ttl_secs: u64) -> Self {
        self.ttl_secs = ttl_secs;
        self
    }

    /// Compute the blob path for a tenant + session + timestamp.
    ///
    /// Format: `sessions/{tenant}/{session_id}/{ts}.tar`.
    fn build_blob_path(tenant_id: &str, session_id: &str, unix_secs: u64) -> String {
        format!("sessions/{tenant_id}/{session_id}/{unix_secs}.tar")
    }

    /// Compute the HMAC-SHA256 signature for a blob path + expiry.
    /// Hex-encoded so it round-trips through URL query parameters.
    fn sign(&self, blob_path: &str, expires_unix: u64) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.secret)
            .expect("HMAC-SHA256 accepts any key length");
        mac.update(blob_path.as_bytes());
        mac.update(b"\n");
        mac.update(expires_unix.to_string().as_bytes());
        let bytes = mac.finalize().into_bytes();
        let mut hex = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            use std::fmt::Write;
            let _ = write!(&mut hex, "{b:02x}");
        }
        hex
    }
}

/// Best-effort check for the "already exists" / 409 conflict shape.
/// Copied from `crate::blob::is_already_exists` (private there).
fn is_already_exists(e: &azure_core::Error) -> bool {
    e.to_string()
        .to_ascii_lowercase()
        .contains("containeralreadyexists")
}

fn is_not_found(e: &azure_core::Error) -> bool {
    let s = e.to_string().to_ascii_lowercase();
    s.contains("blobnotfound") || s.contains("not found")
}

impl SessionBundleStore for CloudSessionBundleStore {
    fn put_and_sign<'a>(
        &'a self,
        tenant_id: &'a str,
        session_id: &'a str,
        bytes: Vec<u8>,
    ) -> PutAndSignFuture<'a> {
        Box::pin(async move {
            let now_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default();
            let blob_path = Self::build_blob_path(tenant_id, session_id, now_secs);
            let blob = self.container.blob_client(&blob_path);
            blob.upload(RequestContent::from(bytes), None)
                .await
                .map_err(|e| SessionBundleStoreError::Storage(e.to_string()))?;
            let expires_unix = now_secs.saturating_add(self.ttl_secs);
            let sig = self.sign(&blob_path, expires_unix);
            let url = format!(
                "{base}/api/v1/sessions/bundles/{blob_path}?expires={expires_unix}&sig={sig}",
                base = self.cloud_base_url.trim_end_matches('/'),
            );
            Ok(SignedBundleUrl {
                url,
                expires_at: iso8601_from_secs(expires_unix),
            })
        })
    }

    fn verify_and_get<'a>(
        &'a self,
        blob_path: &'a str,
        token: &'a str,
    ) -> VerifyAndGetFuture<'a> {
        Box::pin(async move {
            let (expires_unix, sig) = parse_token(token)
                .ok_or(SessionBundleStoreError::InvalidToken)?;
            let now_secs = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or_default();
            if now_secs >= expires_unix {
                return Err(SessionBundleStoreError::InvalidToken);
            }
            let expected = self.sign(blob_path, expires_unix);
            // Constant-time compare; mismatched-length sigs collapse to inequality.
            if expected.as_bytes().ct_eq(sig.as_bytes()).unwrap_u8() != 1 {
                return Err(SessionBundleStoreError::InvalidToken);
            }
            let blob = self.container.blob_client(blob_path);
            let response = blob
                .download(None)
                .await
                .map_err(|e| {
                    if is_not_found(&e) {
                        SessionBundleStoreError::NotFound
                    } else {
                        SessionBundleStoreError::Storage(e.to_string())
                    }
                })?;
            let bytes = response
                .body
                .collect()
                .await
                .map_err(|e| SessionBundleStoreError::Storage(e.to_string()))?;
            Ok(bytes.to_vec())
        })
    }
}

/// Parse `expires={unix}&sig={hex}` (also tolerates reversed order).
/// Returns `(expires_unix, sig_hex)` on success.
fn parse_token(token: &str) -> Option<(u64, String)> {
    let mut expires: Option<u64> = None;
    let mut sig: Option<String> = None;
    for pair in token.split('&') {
        let (k, v) = pair.split_once('=')?;
        match k {
            "expires" => expires = v.parse::<u64>().ok(),
            "sig" => sig = Some(v.to_owned()),
            _ => {}
        }
    }
    Some((expires?, sig?))
}

// `iso8601_from_secs` is re-exported from `ministr-mcp::task` so the
// signed-URL `expires_at` field stays byte-identical to the in-bundle
// `opened_at` / `exported_at` strings the F6.2-a manifest emits.
use ministr_mcp::task::iso8601_from_secs;

/// Env-var selector. Mirrors `blob_backend::build_from_env`'s shape.
///
/// | Trigger | Result |
/// |---|---|
/// | `MINISTR_SESSION_BUNDLE_SIGNING_SECRET` + `MINISTR_BLOB_STORE_KIND=azure` + `MINISTR_BLOB_AZURE_ACCOUNT` + `MINISTR_CLOUD_BASE_URL` | `Some(CloudSessionBundleStore)` |
/// | any of the above missing | `None` |
///
/// We reuse the existing `MINISTR_BLOB_AZURE_ACCOUNT` rather than
/// introducing a new account env var — the corpus bundle store and
/// the session bundle store live in the same account, different
/// containers. The container name defaults to
/// [`DEFAULT_SESSION_BUNDLE_CONTAINER`]; override via
/// `MINISTR_SESSION_BUNDLE_CONTAINER`.
///
/// `MINISTR_SESSION_BUNDLE_SIGNING_SECRET` is the HMAC-SHA256 key;
/// must be at least 32 bytes after trimming. Operators generate this
/// with `openssl rand -base64 32` (the base64 string is the env value;
/// we hash over its UTF-8 bytes, not over the decoded bytes — keeps the
/// env shape opaque to operators).
///
/// # Errors
///
/// Returns [`BlobError::Azure`] when the env signals "build" but the
/// credential or service-client construction fails.
pub fn build_from_env(cloud_base_url: Option<&str>) -> BlobResult<Option<CloudSessionBundleStore>> {
    let trimmed = |k: &str| -> Option<String> {
        std::env::var(k)
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
    };
    let secret = trimmed("MINISTR_SESSION_BUNDLE_SIGNING_SECRET");
    let Some(secret) = secret else {
        return Ok(None);
    };
    if secret.len() < 32 {
        tracing::warn!(
            "MINISTR_SESSION_BUNDLE_SIGNING_SECRET is set but shorter than 32 bytes — \
             session bundle store disabled (use `openssl rand -base64 32` to generate)"
        );
        return Ok(None);
    }
    let Some(base) = cloud_base_url else {
        tracing::warn!(
            "MINISTR_SESSION_BUNDLE_SIGNING_SECRET is set but MINISTR_CLOUD_BASE_URL is not — \
             session bundle store disabled (signed URLs need an absolute base)"
        );
        return Ok(None);
    };
    let kind = std::env::var("MINISTR_BLOB_STORE_KIND")
        .ok()
        .map(|s| s.trim().to_ascii_lowercase());
    if kind.as_deref() != Some("azure") {
        // Filesystem / no-kind deployments don't have an Azure account
        // to upload to. Self-hosted serve falls through to inline tar.
        return Ok(None);
    }
    let Some(account) = trimmed("MINISTR_BLOB_AZURE_ACCOUNT") else {
        tracing::warn!(
            "MINISTR_BLOB_STORE_KIND=azure but MINISTR_BLOB_AZURE_ACCOUNT is missing — \
             session bundle store disabled"
        );
        return Ok(None);
    };
    let container = trimmed("MINISTR_SESSION_BUNDLE_CONTAINER")
        .unwrap_or_else(|| DEFAULT_SESSION_BUNDLE_CONTAINER.to_owned());
    tracing::info!(
        account = account.as_str(),
        container = container.as_str(),
        "constructing session bundle store credential via ManagedIdentityCredential"
    );
    let cred: Arc<dyn TokenCredential> =
        azure_identity::ManagedIdentityCredential::new(None).map_err(BlobError::Azure)?;
    let store = CloudSessionBundleStore::with_credential(
        &account,
        &container,
        cred,
        base,
        secret.into_bytes(),
    )?;
    Ok(Some(store))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store(secret: Vec<u8>) -> CloudSessionBundleStore {
        // We can't easily construct a real BlobContainerClient in a
        // unit test without Azure creds. The sign / parse / verify
        // path is what's worth covering; the upload/download path is
        // covered by the integration tests gated on MINISTR_TEST_AZURE_*.
        // For these tests we sidestep by building a struct value with
        // a stub container — we never call container.* in these tests.
        let creds = azure_identity::DeveloperToolsCredential::new(None)
            .expect("DeveloperToolsCredential should construct without env state");
        CloudSessionBundleStore::with_credential(
            "fakestorageacct",
            "test-container",
            creds,
            "https://example.test",
            secret,
        )
        .expect("store builds with a 32-byte secret")
    }

    #[test]
    fn rejects_short_secret() {
        let creds = azure_identity::DeveloperToolsCredential::new(None).unwrap();
        let err = CloudSessionBundleStore::with_credential(
            "fakestorageacct",
            "test-container",
            creds,
            "https://example.test",
            b"too-short".to_vec(),
        )
        .expect_err("31-byte secret should refuse");
        assert!(format!("{err}").contains("32 bytes"), "got: {err}");
    }

    #[test]
    fn build_blob_path_includes_tenant_session_ts() {
        let p = CloudSessionBundleStore::build_blob_path("tenant-uuid", "sess-1", 1_700_000_000);
        assert_eq!(p, "sessions/tenant-uuid/sess-1/1700000000.tar");
    }

    #[test]
    fn sign_is_deterministic() {
        let store = make_store(vec![0u8; 32]);
        let a = store.sign("sessions/t/s/1.tar", 1_700_000_000);
        let b = store.sign("sessions/t/s/1.tar", 1_700_000_000);
        assert_eq!(a, b);
        assert_eq!(a.len(), 64, "sha256 hex is 64 chars");
    }

    #[test]
    fn sign_diverges_on_path_or_expiry() {
        let store = make_store(vec![0u8; 32]);
        let base = store.sign("sessions/t/s/1.tar", 1_700_000_000);
        let alt_path = store.sign("sessions/t/s/2.tar", 1_700_000_000);
        let alt_exp = store.sign("sessions/t/s/1.tar", 1_700_000_001);
        assert_ne!(base, alt_path);
        assert_ne!(base, alt_exp);
    }

    #[test]
    fn sign_diverges_on_secret() {
        let a = make_store(vec![1u8; 32]).sign("sessions/t/s/1.tar", 1_700_000_000);
        let b = make_store(vec![2u8; 32]).sign("sessions/t/s/1.tar", 1_700_000_000);
        assert_ne!(a, b);
    }

    #[test]
    fn parse_token_round_trips_two_orders() {
        let (e, s) = parse_token("expires=123&sig=abcd").unwrap();
        assert_eq!(e, 123);
        assert_eq!(s, "abcd");
        let (e2, s2) = parse_token("sig=abcd&expires=123").unwrap();
        assert_eq!(e2, 123);
        assert_eq!(s2, "abcd");
    }

    #[test]
    fn parse_token_rejects_missing_pair() {
        assert!(parse_token("expires=123").is_none());
        assert!(parse_token("sig=abcd").is_none());
        assert!(parse_token("").is_none());
    }

    #[test]
    fn iso8601_from_secs_matches_unix_epoch_shape() {
        assert_eq!(iso8601_from_secs(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601_from_secs(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn store_debug_does_not_leak_secret() {
        let store = make_store(vec![0u8; 32]);
        let s = format!("{store:?}");
        assert!(s.contains("ttl_secs"));
        assert!(!s.contains("secret"), "Debug must omit the secret key");
    }
}
