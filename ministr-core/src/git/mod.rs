//! Git repository cloning with sparse checkout support.
//!
//! Provides [`GitFetcher`] for cloning remote repositories into a local cache
//! directory (`~/.ministr/remote/<repo-hash>/`) using shallow, filtered clones
//! with optional sparse checkout. Clone metadata is tracked in TOML files
//! to enable cache reuse and staleness detection.

use std::path::Path;
use std::process::Command;

pub mod blame;
pub mod diff;
pub mod fetcher;

pub use blame::{BlameAuthor, BlameError, BlameSummary};
pub use diff::{ChangedFile, ChangedRange, DiffError};
pub use fetcher::{CloneMetadata, CloneResult, GitFetcher, GitFetcherConfig, GitStalenessResult};

/// Get the HEAD commit SHA for a directory inside a git repository.
///
/// Runs `git rev-parse HEAD` as a subprocess. Returns `None` if the
/// directory is not inside a git work tree or if git is not installed.
///
/// # Examples
///
/// ```no_run
/// use ministr_core::git::local_head_sha;
/// use std::path::Path;
///
/// if let Some(sha) = local_head_sha(Path::new(".")) {
///     println!("HEAD is at {sha}");
/// }
/// ```
#[must_use]
pub fn local_head_sha(dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let sha = String::from_utf8(output.stdout).ok()?;
    let trimmed = sha.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn local_head_sha_not_a_repo() {
        let tmp = TempDir::new().unwrap();
        assert!(local_head_sha(tmp.path()).is_none());
    }

    #[test]
    fn local_head_sha_on_real_repo() {
        // This test runs against the ministr-rs repo itself.
        let sha = local_head_sha(Path::new(env!("CARGO_MANIFEST_DIR")));
        assert!(sha.is_some(), "expected SHA from the ministr-rs git repo");
        let sha = sha.unwrap();
        assert!(
            sha.len() >= 40,
            "SHA should be at least 40 hex chars, got: {sha}"
        );
        assert!(
            sha.chars().all(|c| c.is_ascii_hexdigit()),
            "SHA should be hex: {sha}"
        );
    }
}
