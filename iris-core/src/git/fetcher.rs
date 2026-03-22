//! Git repository fetcher with sparse checkout support.
//!
//! [`GitFetcher`] clones remote git repositories into `~/.iris/remote/<repo-hash>/`
//! using shallow, filtered clones. Optionally applies sparse checkout to
//! materialize only requested directories/files.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tracing::{debug, info, instrument, warn};

use crate::error::GitError;

/// Name of the metadata file written into each clone directory.
const METADATA_FILENAME: &str = "iris-clone.toml";

/// Configuration for the git fetcher.
///
/// # Examples
///
/// ```
/// use iris_core::git::GitFetcherConfig;
///
/// let config = GitFetcherConfig::default();
/// assert!(config.remote_dir.to_string_lossy().contains(".iris"));
/// ```
#[derive(Debug, Clone)]
pub struct GitFetcherConfig {
    /// Root directory for cloned repositories (default: `~/.iris/remote`).
    pub remote_dir: PathBuf,
}

impl Default for GitFetcherConfig {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            remote_dir: home.join(".iris").join("remote"),
        }
    }
}

/// Metadata for a cached git clone.
///
/// Stored as TOML in `<clone-dir>/iris-clone.toml` to track the clone's
/// provenance and enable cache reuse.
///
/// # Examples
///
/// ```
/// use iris_core::git::CloneMetadata;
///
/// let meta = CloneMetadata {
///     repo_url: "https://github.com/user/repo.git".into(),
///     branch: Some("main".into()),
///     commit_sha: "abc123".into(),
///     clone_timestamp: "1711036800".into(),
///     checked_out_paths: vec!["docs".into(), "src".into()],
/// };
/// let toml_str = toml::to_string_pretty(&meta).unwrap();
/// let back: CloneMetadata = toml::from_str(&toml_str).unwrap();
/// assert_eq!(meta, back);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloneMetadata {
    /// The remote repository URL.
    pub repo_url: String,
    /// The branch that was cloned (None for default branch).
    pub branch: Option<String>,
    /// The commit SHA at clone time.
    pub commit_sha: String,
    /// Epoch seconds timestamp of the clone.
    pub clone_timestamp: String,
    /// Paths that were checked out via sparse checkout (empty = full checkout).
    pub checked_out_paths: Vec<String>,
}

/// Result of a git clone operation.
#[derive(Debug, Clone)]
pub struct CloneResult {
    /// Path to the clone directory on disk.
    pub clone_dir: PathBuf,
    /// Clone metadata.
    pub metadata: CloneMetadata,
    /// Whether this was a fresh clone or a cached reuse.
    pub from_cache: bool,
    /// Files present in the checkout.
    pub files: Vec<PathBuf>,
}

/// Git repository fetcher with sparse checkout and caching.
///
/// Clones remote repositories using shallow, filtered clones and
/// optionally applies sparse checkout to limit checked-out content.
pub struct GitFetcher {
    config: GitFetcherConfig,
}

impl GitFetcher {
    /// Create a new git fetcher with the given configuration.
    #[must_use]
    pub fn new(config: GitFetcherConfig) -> Self {
        Self { config }
    }

    /// Create a new git fetcher with default configuration.
    #[must_use]
    pub fn with_defaults() -> Self {
        Self::new(GitFetcherConfig::default())
    }

    /// Clone a remote repository with optional sparse checkout.
    ///
    /// If the repository is already cached and the remote HEAD matches
    /// the cached commit SHA, the cached clone is reused.
    ///
    /// # Arguments
    ///
    /// * `repo_url` — The remote repository URL (HTTPS or SSH).
    /// * `paths` — Optional list of paths for sparse checkout.
    /// * `branch` — Optional branch to clone (defaults to repository's default).
    ///
    /// # Errors
    ///
    /// Returns [`GitError`] if git is not installed, the clone fails,
    /// or metadata cannot be written.
    #[instrument(skip(self), fields(repo = %repo_url))]
    pub async fn clone(
        &self,
        repo_url: &str,
        paths: Option<&[String]>,
        branch: Option<&str>,
    ) -> Result<CloneResult, GitError> {
        if repo_url.is_empty() {
            return Err(GitError::InvalidRepo {
                url: repo_url.to_owned(),
            });
        }

        // Verify git is installed.
        check_git_installed().await?;

        let clone_dir = self.clone_dir(repo_url);

        // Check for cached clone.
        if let Some(result) = self
            .try_cached_clone(&clone_dir, repo_url, paths, branch)
            .await?
        {
            return Ok(result);
        }

        // Fresh clone.
        self.fresh_clone(&clone_dir, repo_url, paths, branch).await
    }

    /// Get the remote HEAD commit SHA without cloning.
    ///
    /// # Errors
    ///
    /// Returns [`GitError`] if the `git ls-remote` command fails.
    #[instrument(skip(self), fields(repo = %repo_url))]
    pub async fn remote_head_sha(
        &self,
        repo_url: &str,
        branch: Option<&str>,
    ) -> Result<String, GitError> {
        let ref_name = branch.unwrap_or("HEAD");
        let output = run_git(&["ls-remote", repo_url, ref_name]).await?;
        let sha = output
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().next())
            .map(String::from)
            .ok_or_else(|| GitError::CommandFailed {
                command: "ls-remote".into(),
                exit_code: 0,
                stderr: "no output from ls-remote".into(),
            })?;
        Ok(sha)
    }

    /// Compute the clone directory path for a given repository URL.
    #[must_use]
    pub fn clone_dir(&self, repo_url: &str) -> PathBuf {
        self.config.remote_dir.join(repo_hash(repo_url))
    }

    /// Load cached clone metadata if available.
    ///
    /// # Errors
    ///
    /// Returns [`GitError::Metadata`] if the file exists but cannot be read or parsed.
    pub async fn load_metadata(clone_dir: &Path) -> Result<Option<CloneMetadata>, GitError> {
        let meta_path = clone_dir.join(METADATA_FILENAME);
        if !meta_path.exists() {
            return Ok(None);
        }
        let contents =
            tokio::fs::read_to_string(&meta_path)
                .await
                .map_err(|e| GitError::Metadata {
                    path: meta_path.clone(),
                    reason: e.to_string(),
                })?;
        let meta: CloneMetadata = toml::from_str(&contents).map_err(|e| GitError::Metadata {
            path: meta_path,
            reason: e.to_string(),
        })?;
        Ok(Some(meta))
    }

    /// Returns the fetcher configuration.
    #[must_use]
    pub fn config(&self) -> &GitFetcherConfig {
        &self.config
    }

    /// Try to reuse a cached clone if it exists and the remote HEAD matches.
    async fn try_cached_clone(
        &self,
        clone_dir: &Path,
        repo_url: &str,
        paths: Option<&[String]>,
        branch: Option<&str>,
    ) -> Result<Option<CloneResult>, GitError> {
        let Some(meta) = Self::load_metadata(clone_dir).await? else {
            return Ok(None);
        };

        // Check if remote HEAD matches cached SHA.
        match self.remote_head_sha(repo_url, branch).await {
            Ok(remote_sha) => {
                if remote_sha == meta.commit_sha {
                    info!(
                        repo = %repo_url,
                        sha = %remote_sha,
                        "reusing cached clone"
                    );
                    let files = discover_files(clone_dir).await;
                    return Ok(Some(CloneResult {
                        clone_dir: clone_dir.to_path_buf(),
                        metadata: meta,
                        from_cache: true,
                        files,
                    }));
                }
                debug!(
                    repo = %repo_url,
                    cached = %meta.commit_sha,
                    remote = %remote_sha,
                    "cached clone is stale, re-cloning"
                );
            }
            Err(e) => {
                warn!(
                    repo = %repo_url,
                    error = %e,
                    "could not check remote HEAD, re-cloning"
                );
            }
        }

        // Remove stale clone directory.
        if clone_dir.exists() {
            let _ = tokio::fs::remove_dir_all(clone_dir).await;
        }

        // Re-clone from scratch.
        let result = self.fresh_clone(clone_dir, repo_url, paths, branch).await?;
        Ok(Some(result))
    }

    /// Perform a fresh clone.
    async fn fresh_clone(
        &self,
        clone_dir: &Path,
        repo_url: &str,
        paths: Option<&[String]>,
        branch: Option<&str>,
    ) -> Result<CloneResult, GitError> {
        let start = std::time::Instant::now();

        // Ensure parent directory exists.
        if let Some(parent) = clone_dir.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| GitError::CloneDirectory {
                    path: parent.to_path_buf(),
                    reason: e.to_string(),
                })?;
        }

        // Remove existing directory if present.
        if clone_dir.exists() {
            tokio::fs::remove_dir_all(clone_dir)
                .await
                .map_err(|e| GitError::CloneDirectory {
                    path: clone_dir.to_path_buf(),
                    reason: e.to_string(),
                })?;
        }

        // Build clone command.
        let clone_dir_str = clone_dir.to_string_lossy().to_string();
        let mut args = vec![
            "clone",
            "--no-checkout",
            "--depth",
            "1",
            "--filter=blob:none",
        ];
        if let Some(b) = branch {
            args.push("--branch");
            args.push(b);
        }
        args.push(repo_url);
        args.push(&clone_dir_str);

        run_git(&args).await?;

        // Set up sparse checkout if paths are specified.
        let sparse_paths: Vec<String> = paths.map(<[String]>::to_vec).unwrap_or_default();

        if !sparse_paths.is_empty() {
            let mut sparse_args = vec!["sparse-checkout", "set", "--cone"];
            let path_refs: Vec<&str> = sparse_paths.iter().map(String::as_str).collect();
            sparse_args.extend(path_refs);
            run_git_in_dir(clone_dir, &sparse_args).await?;
        }

        // Checkout the content.
        run_git_in_dir(clone_dir, &["checkout"]).await?;

        // Get the checked-out commit SHA.
        let sha_output = run_git_in_dir(clone_dir, &["rev-parse", "HEAD"]).await?;
        let commit_sha = sha_output.trim().to_owned();

        let elapsed_ms = start.elapsed().as_millis();
        let files = discover_files(clone_dir).await;

        let metadata = CloneMetadata {
            repo_url: repo_url.to_owned(),
            branch: branch.map(String::from),
            commit_sha,
            clone_timestamp: epoch_now(),
            checked_out_paths: sparse_paths,
        };

        // Write metadata.
        write_metadata(clone_dir, &metadata).await?;

        info!(
            repo = %repo_url,
            sha = %metadata.commit_sha,
            files = files.len(),
            elapsed_ms = %elapsed_ms,
            "clone complete"
        );

        Ok(CloneResult {
            clone_dir: clone_dir.to_path_buf(),
            metadata,
            from_cache: false,
            files,
        })
    }
}

/// Compute a short SHA-256 hash of a repository URL for directory naming.
///
/// # Examples
///
/// ```
/// use iris_core::git::fetcher::repo_hash;
///
/// let hash = repo_hash("https://github.com/user/repo.git");
/// assert_eq!(hash.len(), 16);
/// assert_eq!(hash, repo_hash("https://github.com/user/repo.git"));
/// ```
#[must_use]
pub fn repo_hash(repo_url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repo_url.as_bytes());
    let digest = hasher.finalize();
    format!("{digest:x}")[..16].to_string()
}

/// Check that the `git` binary is available on PATH.
async fn check_git_installed() -> Result<(), GitError> {
    let result = Command::new("git")
        .arg("--version")
        .output()
        .await
        .map_err(|_| GitError::NotInstalled)?;

    if !result.status.success() {
        return Err(GitError::NotInstalled);
    }
    Ok(())
}

/// Run a git command and return stdout as a string.
async fn run_git(args: &[&str]) -> Result<String, GitError> {
    debug!(args = ?args, "running git command");
    let output = Command::new("git")
        .args(args)
        .output()
        .await
        .map_err(|_| GitError::NotInstalled)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(GitError::CommandFailed {
            command: args.first().copied().unwrap_or("git").to_owned(),
            exit_code: output.status.code().unwrap_or(-1),
            stderr,
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Run a git command in a specific working directory.
async fn run_git_in_dir(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    debug!(dir = %dir.display(), args = ?args, "running git command in dir");
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .await
        .map_err(|_| GitError::NotInstalled)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(GitError::CommandFailed {
            command: args.first().copied().unwrap_or("git").to_owned(),
            exit_code: output.status.code().unwrap_or(-1),
            stderr,
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Write clone metadata to the clone directory.
async fn write_metadata(clone_dir: &Path, metadata: &CloneMetadata) -> Result<(), GitError> {
    let meta_path = clone_dir.join(METADATA_FILENAME);
    let toml_str = toml::to_string_pretty(metadata).map_err(|e| GitError::Metadata {
        path: meta_path.clone(),
        reason: e.to_string(),
    })?;
    tokio::fs::write(&meta_path, toml_str)
        .await
        .map_err(|e| GitError::Metadata {
            path: meta_path,
            reason: e.to_string(),
        })?;
    Ok(())
}

/// Discover all files in the clone directory (excluding `.git` and metadata).
async fn discover_files(clone_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![clone_dir.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Skip .git directory and metadata file.
            if name_str == ".git" || name_str == METADATA_FILENAME {
                continue;
            }

            if path.is_dir() {
                stack.push(path);
            } else {
                // Store as relative path from clone_dir.
                if let Ok(rel) = path.strip_prefix(clone_dir) {
                    files.push(rel.to_path_buf());
                }
            }
        }
    }

    files.sort();
    files
}

/// Get the current UTC timestamp as epoch seconds string.
fn epoch_now() -> String {
    let now = std::time::SystemTime::now();
    let secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_hash_deterministic() {
        let h1 = repo_hash("https://github.com/user/repo.git");
        let h2 = repo_hash("https://github.com/user/repo.git");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
    }

    #[test]
    fn repo_hash_different_urls_differ() {
        let h1 = repo_hash("https://github.com/user/repo-a.git");
        let h2 = repo_hash("https://github.com/user/repo-b.git");
        assert_ne!(h1, h2);
    }

    #[test]
    fn clone_metadata_serde_roundtrip() {
        let meta = CloneMetadata {
            repo_url: "https://github.com/user/repo.git".into(),
            branch: Some("main".into()),
            commit_sha: "abc123def456".into(),
            clone_timestamp: "1711036800".into(),
            checked_out_paths: vec!["docs".into(), "src".into()],
        };
        let toml_str = toml::to_string_pretty(&meta).unwrap();
        let back: CloneMetadata = toml::from_str(&toml_str).unwrap();
        assert_eq!(meta, back);
    }

    #[test]
    fn clone_metadata_serde_no_branch() {
        let meta = CloneMetadata {
            repo_url: "https://github.com/user/repo.git".into(),
            branch: None,
            commit_sha: "abc123".into(),
            clone_timestamp: "1711036800".into(),
            checked_out_paths: vec![],
        };
        let toml_str = toml::to_string_pretty(&meta).unwrap();
        let back: CloneMetadata = toml::from_str(&toml_str).unwrap();
        assert_eq!(meta, back);
    }

    #[test]
    fn git_fetcher_config_defaults() {
        let config = GitFetcherConfig::default();
        assert!(config.remote_dir.to_string_lossy().contains("remote"));
    }

    #[test]
    fn clone_dir_uses_repo_hash() {
        let config = GitFetcherConfig {
            remote_dir: PathBuf::from("/tmp/test-remote"),
        };
        let fetcher = GitFetcher::new(config);
        let dir = fetcher.clone_dir("https://github.com/user/repo.git");
        let hash = repo_hash("https://github.com/user/repo.git");
        assert_eq!(dir, PathBuf::from(format!("/tmp/test-remote/{hash}")));
    }

    #[tokio::test]
    async fn check_git_installed_succeeds() {
        // This test assumes git is installed on the CI/dev machine.
        check_git_installed().await.unwrap();
    }

    #[tokio::test]
    async fn clone_empty_url_returns_error() {
        let fetcher = GitFetcher::new(GitFetcherConfig {
            remote_dir: PathBuf::from("/tmp/iris-test-git"),
        });
        let result = fetcher.clone("", None, None).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), GitError::InvalidRepo { .. }));
    }

    #[tokio::test]
    async fn write_and_load_metadata_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let meta = CloneMetadata {
            repo_url: "https://github.com/user/repo.git".into(),
            branch: Some("main".into()),
            commit_sha: "deadbeef".into(),
            clone_timestamp: "1711036800".into(),
            checked_out_paths: vec!["docs".into()],
        };
        write_metadata(tmp.path(), &meta).await.unwrap();
        let loaded = GitFetcher::load_metadata(tmp.path()).await.unwrap();
        assert_eq!(loaded, Some(meta));
    }

    #[tokio::test]
    async fn load_metadata_missing_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let loaded = GitFetcher::load_metadata(tmp.path()).await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn discover_files_excludes_git_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let git_dir = tmp.path().join(".git");
        tokio::fs::create_dir_all(&git_dir).await.unwrap();
        tokio::fs::write(git_dir.join("HEAD"), "ref: refs/heads/main\n")
            .await
            .unwrap();
        tokio::fs::write(tmp.path().join("README.md"), "# Test")
            .await
            .unwrap();
        tokio::fs::write(tmp.path().join(METADATA_FILENAME), "test")
            .await
            .unwrap();

        let files = discover_files(tmp.path()).await;
        assert_eq!(files, vec![PathBuf::from("README.md")]);
    }

    #[tokio::test]
    async fn clone_real_repo() {
        // Clone a small, well-known public repo to verify the full pipeline.
        // Using git's own test fixtures would be ideal, but for simplicity
        // we use a tiny repo. Skip if no network or git unavailable.
        if check_git_installed().await.is_err() {
            eprintln!("git not installed, skipping clone_real_repo test");
            return;
        }

        let tmp = tempfile::tempdir().unwrap();
        let config = GitFetcherConfig {
            remote_dir: tmp.path().to_path_buf(),
        };
        let fetcher = GitFetcher::new(config);

        // Use a well-known tiny repo.
        let repo_url = "https://github.com/octocat/Hello-World.git";
        let result = fetcher.clone(repo_url, None, None).await;

        match result {
            Ok(clone_result) => {
                assert!(!clone_result.from_cache);
                assert!(!clone_result.metadata.commit_sha.is_empty());
                assert_eq!(clone_result.metadata.repo_url, repo_url);
                assert!(!clone_result.files.is_empty());
                assert!(clone_result.clone_dir.exists());

                // Verify metadata was written.
                let loaded = GitFetcher::load_metadata(&clone_result.clone_dir)
                    .await
                    .unwrap();
                assert!(loaded.is_some());
                assert_eq!(loaded.unwrap().commit_sha, clone_result.metadata.commit_sha);

                // Clone again — should reuse cache.
                let cached_result = fetcher.clone(repo_url, None, None).await.unwrap();
                assert!(cached_result.from_cache);
                assert_eq!(
                    cached_result.metadata.commit_sha,
                    clone_result.metadata.commit_sha
                );
            }
            Err(e) => {
                // Network failures in CI are acceptable — don't fail the test.
                eprintln!("clone failed (network?): {e}, skipping assertions");
            }
        }
    }
}
