//! Single-instance-per-repo coordinator.
//!
//! Ensures only one iris process owns the HNSW index for a given corpus.
//! The first process becomes the **primary** (owns the index, serves stdio +
//! HTTP). Subsequent processes become **secondaries** that proxy their stdio
//! MCP traffic to the primary's HTTP endpoint.
//!
//! Uses `std::fs::File::try_lock()` (Rust 1.89+) for cross-platform file
//! locking. The lock is automatically released when the process exits.

use std::fs::{self, File, TryLockError};
use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::{Path, PathBuf};

use miette::{IntoDiagnostic, WrapErr};
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

/// Holds the exclusive lock for the primary instance.
///
/// The file lock (`flock` on Unix, `LockFileEx` on Windows) is released
/// automatically when this struct is dropped. The port file is also cleaned
/// up so secondaries don't connect to a stale endpoint.
pub struct PrimaryLock {
    _lock_file: File,
    port_file: PathBuf,
    /// The HTTP port the primary listens on for secondary connections.
    pub http_port: u16,
}

impl Drop for PrimaryLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.port_file);
    }
}

/// The role this iris instance should assume.
pub enum InstanceRole {
    /// First instance — owns the index, serves stdio + HTTP.
    Primary(PrimaryLock),
    /// Subsequent instance — proxies stdio to the primary's HTTP endpoint.
    Secondary { mcp_url: String },
}

/// Determine whether this process should be the primary or a secondary.
///
/// Tries to acquire an exclusive file lock on `{corpus_dir}/iris.lock`.
/// If acquired, computes a deterministic HTTP port from the corpus paths,
/// binds a TCP listener to confirm availability, writes the port to
/// `{corpus_dir}/iris.port`, and returns [`InstanceRole::Primary`].
///
/// If the lock is already held, reads the port file and returns
/// [`InstanceRole::Secondary`].
pub fn acquire_role(corpus_dir: &Path, corpus_paths: &[String]) -> miette::Result<InstanceRole> {
    let lock_path = corpus_dir.join("iris.lock");
    let port_path = corpus_dir.join("iris.port");

    let lock_file = File::create(&lock_path)
        .into_diagnostic()
        .wrap_err("failed to create iris.lock")?;

    // File::try_lock is stable since Rust 1.89; our installed rustc is 1.94.
    #[allow(clippy::incompatible_msrv)]
    match lock_file.try_lock() {
        Ok(()) => {
            // We are the primary.
            let port = resolve_port(corpus_paths, &port_path)?;
            info!(port, "acquired primary lock");

            Ok(InstanceRole::Primary(PrimaryLock {
                _lock_file: lock_file,
                port_file: port_path,
                http_port: port,
            }))
        }
        Err(TryLockError::WouldBlock) => {
            // Another instance holds the lock — become a secondary.
            if let Ok(mcp_url) = read_port_file_with_retry(&port_path) {
                return Ok(InstanceRole::Secondary { mcp_url });
            }

            // Port file missing or primary not responding.
            // The primary likely crashed — the flock was released on
            // death but the port file may be stale. Clean up and retry.
            warn!("primary appears dead — cleaning stale files and retrying lock");
            let _ = fs::remove_file(&port_path);
            let _ = fs::remove_file(&lock_path);

            let lock_file = File::create(&lock_path)
                .into_diagnostic()
                .wrap_err("failed to re-create iris.lock")?;
            #[allow(clippy::incompatible_msrv)]
            lock_file
                .try_lock()
                .into_diagnostic()
                .wrap_err("failed to acquire lock after stale cleanup")?;

            let port = resolve_port(corpus_paths, &port_path)?;
            info!(port, "promoted to primary after stale cleanup");

            Ok(InstanceRole::Primary(PrimaryLock {
                _lock_file: lock_file,
                port_file: port_path,
                http_port: port,
            }))
        }
        Err(TryLockError::Error(e)) => Err(e)
            .into_diagnostic()
            .wrap_err("failed to acquire iris.lock"),
    }
}

/// Compute a deterministic port, verify it's available, write to file.
fn resolve_port(corpus_paths: &[String], port_path: &Path) -> miette::Result<u16> {
    let preferred = deterministic_port(corpus_paths);

    // Try the deterministic port first; fall back to OS-assigned.
    let listener = TcpListener::bind(("127.0.0.1", preferred))
        .or_else(|_| {
            debug!(preferred, "preferred port occupied, using OS-assigned");
            TcpListener::bind(("127.0.0.1", 0u16))
        })
        .into_diagnostic()
        .wrap_err("failed to bind HTTP listener port")?;

    let port = listener
        .local_addr()
        .into_diagnostic()
        .wrap_err("failed to read bound port")?
        .port();

    // Drop the listener — the actual HTTP server will rebind in
    // `spawn_http_listener`. There's a tiny race window, but on localhost
    // with a deterministic port it's practically zero.
    drop(listener);

    fs::write(port_path, port.to_string())
        .into_diagnostic()
        .wrap_err("failed to write iris.port")?;

    Ok(port)
}

/// Derive a stable port in the ephemeral range (49152–65535) from corpus paths.
fn deterministic_port(corpus_paths: &[String]) -> u16 {
    let mut hasher = Sha256::new();
    for p in corpus_paths {
        hasher.update(p.as_bytes());
        hasher.update(b"\0");
    }
    let hash = hasher.finalize();
    let raw = u16::from_be_bytes([hash[0], hash[1]]);
    49152 + (raw % 16384)
}

/// Read `iris.port` with retries (the primary may still be writing it).
fn read_port_file_with_retry(port_path: &Path) -> miette::Result<String> {
    for attempt in 0..10 {
        match fs::read_to_string(port_path) {
            Ok(contents) => {
                let port_str = contents.trim();
                if let Ok(port) = port_str.parse::<u16>() {
                    return Ok(format!("http://127.0.0.1:{port}/mcp"));
                }
                // File exists but content is invalid — primary still writing.
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                // Primary hasn't written the port file yet.
            }
            Err(e) => {
                warn!(error = %e, "failed to read iris.port");
            }
        }

        if attempt < 9 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    miette::bail!("timed out waiting for primary to write iris.port — primary may have crashed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_port_is_stable() {
        let paths = vec!["src".to_string(), "docs".to_string()];
        let p1 = deterministic_port(&paths);
        let p2 = deterministic_port(&paths);
        assert_eq!(p1, p2);
        assert!(p1 >= 49152);
    }

    #[test]
    fn deterministic_port_differs_for_different_paths() {
        let a = deterministic_port(&["a".to_string()]);
        let b = deterministic_port(&["b".to_string()]);
        // Could theoretically collide but astronomically unlikely.
        assert_ne!(a, b);
    }

    #[test]
    fn primary_lock_lifecycle() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = vec!["test".to_string()];

        // First acquire → Primary
        let role = acquire_role(tmp.path(), &paths).unwrap();
        assert!(matches!(role, InstanceRole::Primary(_)));

        // While held → Secondary
        let role2 = acquire_role(tmp.path(), &paths).unwrap();
        assert!(matches!(role2, InstanceRole::Secondary { .. }));

        // Drop primary → next acquire becomes Primary
        drop(role);
        let role3 = acquire_role(tmp.path(), &paths).unwrap();
        assert!(matches!(role3, InstanceRole::Primary(_)));
    }
}
