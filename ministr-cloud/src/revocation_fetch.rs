//! F5.4-e-revoke-api-fetch — customer-side HTTP fetcher for the
//! operator's revocation JSONL.
//!
//! The cloud-side counterpart [`crate::admin`-mounted
//! `serve_revocation_list`] (in `ministr-mcp`, F5.4-e-revoke-api-serve)
//! exposes the operator's `revoke-license`-managed list at
//! `/api/v1/license-revocations.jsonl`. This module is what the
//! customer's on-prem serve uses to consume it.
//!
//! # Boot-time flow
//!
//! When the customer sets `MINISTR_LICENSE_REVOCATIONS_URL`:
//!
//! 1. **Fetch** with a short timeout. Write the body to the local
//!    cache path (env `MINISTR_LICENSE_REVOCATIONS_CACHE_PATH`,
//!    defaults to `/tmp/ministr-revocations-cache.jsonl`).
//! 2. **On success**: return the cache path; boot validator reads
//!    it via the existing [`crate::license::is_revoked_by_file`].
//! 3. **On failure WITH a fresh cache** (mtime within
//!    `MINISTR_LICENSE_REVOCATIONS_GRACE_SECS`, default 24 hours):
//!    log a warning, return the cache path. The serve boots under
//!    the slightly-stale list — better than refusing to boot on a
//!    transient portal blip.
//! 4. **On failure WITH a stale or missing cache**: refuse boot
//!    with `LicenseError::RevocationFetchFailed`. The operator
//!    opted into network-fetched revocation; we'd rather refuse
//!    than silently boot a license the operator may have revoked.
//!
//! Grace window cap is the load-bearing tradeoff: too short and a
//! routine portal maintenance window crashes every customer's
//! serve on its next restart; too long and a deliberately
//! unreachable portal leaves stale licenses running for the entire
//! window. 24h is the conservative default; operators tune via env.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tracing::{info, warn};

use crate::license::LicenseError;

/// Default for `MINISTR_LICENSE_REVOCATIONS_CACHE_PATH` — only used
/// when the env var is unset.
pub const DEFAULT_REVOCATION_CACHE_PATH: &str = "/tmp/ministr-revocations-cache.jsonl";

/// Default grace window for cache-fallback when fetch fails. 24
/// hours covers a routine maintenance window without leaving stale
/// licenses running forever.
pub const DEFAULT_REVOCATION_GRACE_SECS: u64 = 86_400;

/// HTTP timeout for the boot fetch. Short enough that a wedged
/// portal doesn't stall serve startup indefinitely.
const FETCH_TIMEOUT_SECS: u64 = 10;

/// F5.4-e-revoke-api-fetch — fetch the operator's revocation list
/// from `url`, write to `cache_path`, return the path the boot
/// validator should consult. Falls back to a within-grace cache on
/// fetch failure; refuses (returns Err) on stale-or-missing cache.
///
/// # Errors
///
/// Returns [`LicenseError::RevocationFetchFailed`] when fetch fails
/// AND no usable cache exists. The error message names the URL +
/// the underlying reason so the operator can diagnose.
pub async fn fetch_revocation_list(
    url: &str,
    cache_path: &Path,
    grace_secs: u64,
) -> Result<PathBuf, LicenseError> {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(FETCH_TIMEOUT_SECS))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return Err(LicenseError::RevocationFetchFailed {
                url: url.to_string(),
                cause: format!("build reqwest client: {e}"),
            });
        }
    };
    match client.get(url).send().await {
        Ok(resp) if resp.status().is_success() => match resp.bytes().await {
            Ok(body) => {
                if let Err(e) = std::fs::write(cache_path, &body) {
                    warn!(
                        error = %e,
                        path = %cache_path.display(),
                        "fetch succeeded but cache write failed; attempting fallback to existing cache"
                    );
                    return fallback_to_cache(url, cache_path, grace_secs, "cache-write-failed");
                }
                info!(
                    url,
                    bytes = body.len(),
                    cache = %cache_path.display(),
                    "fetched revocation list from operator portal"
                );
                Ok(cache_path.to_path_buf())
            }
            Err(e) => fallback_to_cache(
                url,
                cache_path,
                grace_secs,
                &format!("body read failed: {e}"),
            ),
        },
        Ok(resp) => fallback_to_cache(
            url,
            cache_path,
            grace_secs,
            &format!("HTTP {}", resp.status()),
        ),
        Err(e) => fallback_to_cache(url, cache_path, grace_secs, &format!("send: {e}")),
    }
}

/// Try the within-grace cache after a fetch failure. Returns the
/// cache path on success or `LicenseError::RevocationFetchFailed`
/// when no fresh cache is available.
fn fallback_to_cache(
    url: &str,
    cache_path: &Path,
    grace_secs: u64,
    fetch_reason: &str,
) -> Result<PathBuf, LicenseError> {
    let Ok(meta) = std::fs::metadata(cache_path) else {
        return Err(LicenseError::RevocationFetchFailed {
            url: url.to_string(),
            cause: format!(
                "{fetch_reason}; no cache at {} to fall back to",
                cache_path.display()
            ),
        });
    };
    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let age = SystemTime::now()
        .duration_since(mtime)
        .map_or(u64::MAX, |d| d.as_secs());
    if age <= grace_secs {
        warn!(
            url,
            cache = %cache_path.display(),
            age_secs = age,
            fetch_reason,
            "revocation fetch failed; falling back to within-grace cache"
        );
        Ok(cache_path.to_path_buf())
    } else {
        Err(LicenseError::RevocationFetchFailed {
            url: url.to_string(),
            cause: format!(
                "{fetch_reason}; cache at {} is {age}s old (grace window is {grace_secs}s)",
                cache_path.display()
            ),
        })
    }
}

/// Default refresh interval for the background re-fetch task. 1
/// hour is the production default; operators tune via
/// `MINISTR_LICENSE_REVOCATIONS_REFRESH_SECS`.
pub const DEFAULT_REVOCATION_REFRESH_SECS: u64 = 3_600;

/// Shared state between the refresh task and the server.
///
/// When revocation is detected, the task sets `revoked` and notifies
/// `shutdown` so the server can drain gracefully instead of
/// `process::exit`.
#[derive(Clone)]
pub struct RevocationShutdownHandle {
    pub revoked: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub shutdown: std::sync::Arc<tokio::sync::Notify>,
}

impl RevocationShutdownHandle {
    #[must_use]
    pub fn new() -> Self {
        Self {
            revoked: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            shutdown: std::sync::Arc::new(tokio::sync::Notify::new()),
        }
    }

    #[must_use]
    pub fn is_revoked(&self) -> bool {
        self.revoked.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn trigger(&self) {
        self.revoked
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.shutdown.notify_one();
    }
}

impl Default for RevocationShutdownHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for RevocationShutdownHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RevocationShutdownHandle")
            .field("revoked", &self.is_revoked())
            .finish()
    }
}

/// F5.4-e-revoke-api-refresh — spawn a tokio task that periodically
/// re-fetches the revocation list to keep the local cache warm.
///
/// When `shutdown_handle` is `Some`, revocation triggers a graceful
/// shutdown signal instead of `process::exit(1)`. When `None`, falls
/// back to the brutal exit for backward compatibility.
pub fn spawn_refresh_task(
    url: String,
    cache_path: PathBuf,
    refresh_secs: u64,
    grace_secs: u64,
    current_jwt_id_hash: String,
    shutdown_handle: Option<RevocationShutdownHandle>,
) {
    let interval = if refresh_secs == 0 {
        DEFAULT_REVOCATION_REFRESH_SECS
    } else {
        refresh_secs
    };
    info!(
        url,
        cache = %cache_path.display(),
        refresh_secs = interval,
        boot_jwt_id_hash = %current_jwt_id_hash,
        "license revocation refresh task spawned (F5.4-e-revoke-api-refresh + mid-flight)"
    );
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(interval));
        // First tick is immediate per tokio docs; swallow to avoid
        // doubling up with the boot-time fetch that already ran.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let fetch_result =
                fetch_revocation_list(&url, &cache_path, grace_secs).await;
            match fetch_result {
                Ok(path) => {
                    info!(
                        url,
                        cache = %path.display(),
                        "background revocation refresh succeeded"
                    );
                    // F5.4-e-revoke-mid-flight — re-check the current
                    // license's hash against the freshly-written
                    // cache. If revoked, exit the process so the
                    // orchestrator restarts and the boot validator
                    // refuses the now-revoked license. Errors during
                    // the check log warn but don't exit (the cache
                    // read could be transient).
                    match crate::license::is_revoked_by_file(&path, &current_jwt_id_hash) {
                        Ok(Some(record)) => {
                            tracing::error!(
                                jwt_id_hash = %current_jwt_id_hash,
                                reason = %record.reason,
                                "running license has been REVOKED by the operator — initiating graceful shutdown"
                            );
                            if let Some(ref handle) = shutdown_handle {
                                handle.trigger();
                                return;
                            }
                            std::process::exit(1);
                        }
                        Ok(None) => {
                            // Not revoked; continue running.
                        }
                        Err(e) => {
                            warn!(
                                error = %e,
                                "mid-flight revocation check failed reading the just-refreshed cache; will retry next tick"
                            );
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        url,
                        cache = %cache_path.display(),
                        error = %e,
                        "background revocation refresh failed; cache may go stale"
                    );
                }
            }
        }
    });
}

/// Read the customer-side env vars and return the chosen
/// (`url`, `cache_path`, `grace_secs`) triple, or `None` when the
/// URL env var is unset (operator hasn't opted into network-fetched
/// revocation).
#[must_use]
pub fn revocation_url_config() -> Option<(String, PathBuf, u64)> {
    let url = std::env::var("MINISTR_LICENSE_REVOCATIONS_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())?;
    let cache_path = std::env::var("MINISTR_LICENSE_REVOCATIONS_CACHE_PATH")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map_or_else(
            || PathBuf::from(DEFAULT_REVOCATION_CACHE_PATH),
            PathBuf::from,
        );
    let grace_secs = std::env::var("MINISTR_LICENSE_REVOCATIONS_GRACE_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_REVOCATION_GRACE_SECS);
    Some((url, cache_path, grace_secs))
}

/// F5.4-e-revoke-api-refresh — read the refresh interval env var.
/// Returns the configured value or [`DEFAULT_REVOCATION_REFRESH_SECS`].
#[must_use]
pub fn revocation_refresh_secs() -> u64 {
    std::env::var("MINISTR_LICENSE_REVOCATIONS_REFRESH_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_REVOCATION_REFRESH_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn fetch_returns_error_when_url_is_unreachable_and_no_cache() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let cache = tmp.path().to_path_buf();
        std::fs::remove_file(&cache).unwrap(); // ensure no cache exists
        let err = fetch_revocation_list(
            "http://127.0.0.1:1/this-port-is-not-listening",
            &cache,
            DEFAULT_REVOCATION_GRACE_SECS,
        )
        .await
        .expect_err("must fail without cache");
        assert!(matches!(err, LicenseError::RevocationFetchFailed { .. }));
    }

    #[tokio::test]
    async fn fetch_falls_back_to_fresh_cache_when_unreachable() {
        // Pre-create a "fresh" cache file with a tiny revocation
        // payload; the helper should return its path even though
        // the URL is unreachable.
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(
            tmp,
            r#"{{"ts_iso":"2026-05-23T00:00:00Z","ts_unix":1779494400,"enterprise_id":"x","jwt_id_hash":"0000000000000000","reason":"test"}}"#
        )
        .unwrap();
        tmp.flush().unwrap();
        let cache = tmp.path().to_path_buf();
        let path = fetch_revocation_list(
            "http://127.0.0.1:1/this-port-is-not-listening",
            &cache,
            3600, // 1h grace, file is fresh
        )
        .await
        .expect("must fall back to cache");
        assert_eq!(path, cache);
    }

    #[tokio::test]
    async fn fetch_fails_when_cache_is_stale() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "{{}}").unwrap();
        tmp.flush().unwrap();
        let cache = tmp.path().to_path_buf();
        // Grace window of 0 means the file must be < 1 second old.
        // tokio sleep of 1s + grace=0 forces stale-cache rejection.
        tokio::time::sleep(Duration::from_millis(1100)).await;
        let err = fetch_revocation_list(
            "http://127.0.0.1:1/this-port-is-not-listening",
            &cache,
            0,
        )
        .await
        .expect_err("stale cache must not satisfy grace=0");
        assert!(matches!(err, LicenseError::RevocationFetchFailed { .. }));
    }

    // Note: `revocation_url_config` reads an env var; testing the
    // unset branch would require `std::env::remove_var` which is
    // unsafe under Rust 2024. The crate forbids unsafe_code. The
    // unset path is exercised end-to-end in the harness's existing
    // license boot tests where MINISTR_LICENSE_REVOCATIONS_URL is
    // never set by default.
}
