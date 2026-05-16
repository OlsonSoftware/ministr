//! Filesystem helpers hardened for cross-platform reliability.

use std::path::Path;
use std::time::Duration;

/// Remove a directory tree, retrying transient Windows failures.
///
/// `std::fs::remove_dir_all` is notoriously unreliable on Windows: a file
/// handle that another task hasn't closed yet (an indexer mid-write, a
/// directory watcher, a just-dropped SQLite connection whose OS handle
/// lingers briefly) makes the call fail with a sharing violation /
/// `ERROR_DIR_NOT_EMPTY`, and `DeleteFileW` only *marks* files for
/// deletion so a parent-dir removal can momentarily observe a
/// not-yet-empty directory. These are racy and clear within tens of
/// milliseconds, so a bounded exponential backoff turns a spurious
/// one-shot failure into a reliable delete.
///
/// Symlink safety is inherited from `std::fs::remove_dir_all`, which since
/// the CVE-2022-21658 fix does not traverse symlinked directories.
///
/// Returns `Ok(())` if the directory is already gone. Any *non-transient*
/// error (e.g. permission denied that never clears) is returned after the
/// retry budget is exhausted, so callers can surface it instead of
/// silently reporting success.
///
/// # Errors
///
/// Returns the last [`std::io::Error`] if the tree could not be removed
/// within the retry budget.
pub async fn remove_dir_all_robust(path: &Path) -> std::io::Result<()> {
    // ~0 + 50 + 100 + 200 + 400 + 800 ms ≈ 1.55s total worst case —
    // long enough to outlast handle-close races, short enough that a
    // genuinely stuck delete fails fast and visibly.
    const MAX_ATTEMPTS: u32 = 6;

    for attempt in 0..MAX_ATTEMPTS {
        match std::fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            // Already gone (or never existed) — the postcondition holds.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                if attempt + 1 == MAX_ATTEMPTS {
                    return Err(e);
                }
                if !is_transient(&e) {
                    return Err(e);
                }
                let backoff = Duration::from_millis(50u64 << attempt);
                tracing::debug!(
                    path = %path.display(),
                    attempt = attempt + 1,
                    error = %e,
                    "remove_dir_all transient failure; retrying"
                );
                tokio::time::sleep(backoff).await;
            }
        }
    }
    Ok(())
}

/// Heuristic for "this will probably succeed if we wait a moment".
///
/// Covers the Windows handle-close / mark-for-delete races. `PermissionDenied`
/// is included because Windows surfaces a sharing violation that way; a
/// truly permanent permission problem still fails once the retry budget
/// is exhausted, just a second or so later.
fn is_transient(e: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        e.kind(),
        ErrorKind::PermissionDenied | ErrorKind::DirectoryNotEmpty | ErrorKind::ResourceBusy
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn removes_existing_tree() {
        let tmp = std::env::temp_dir().join(format!("ministr-fsutil-{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("a/b/c")).unwrap();
        std::fs::write(tmp.join("a/b/c/f.txt"), b"x").unwrap();
        remove_dir_all_robust(&tmp).await.unwrap();
        assert!(!tmp.exists());
    }

    #[tokio::test]
    async fn missing_dir_is_ok() {
        let missing = std::env::temp_dir().join("ministr-fsutil-does-not-exist-zzz");
        remove_dir_all_robust(&missing).await.unwrap();
    }
}
