//! Web content cache for storing fetched pages on disk.
//!
//! Persists fetched web pages under `~/.iris/web/<url-hash>/` with a metadata
//! file tracking the source URL, fetch timestamp, `ETag`, and content hash.
//! This enables incremental re-fetching and freshness checks.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::WebError;

/// Metadata for a cached web page.
///
/// # Examples
///
/// ```
/// use iris_core::web::cache::WebPageMeta;
///
/// let meta = WebPageMeta {
///     source_url: "https://example.com/docs/".into(),
///     fetched_at: "2026-03-21T12:00:00Z".into(),
///     etag: Some("\"abc123\"".into()),
///     last_modified: Some("Fri, 20 Mar 2026 10:00:00 GMT".into()),
///     content_hash: "deadbeef".into(),
///     content_type: Some("text/html".into()),
/// };
/// assert_eq!(meta.source_url, "https://example.com/docs/");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebPageMeta {
    /// The original source URL.
    pub source_url: String,
    /// ISO 8601 timestamp of when the page was fetched.
    pub fetched_at: String,
    /// HTTP `ETag` header for conditional re-fetching.
    pub etag: Option<String>,
    /// HTTP `Last-Modified` header for conditional re-fetching.
    #[serde(default)]
    pub last_modified: Option<String>,
    /// SHA-256 hex digest of the markdown content.
    pub content_hash: String,
    /// Content-Type from the HTTP response.
    pub content_type: Option<String>,
}

/// Disk-based cache for fetched web pages.
///
/// Each page is stored in `{cache_dir}/<url-hash>/` with two files:
/// - `content.md` — the converted markdown content
/// - `meta.json` — the [`WebPageMeta`] metadata
///
/// # Examples
///
/// ```no_run
/// use iris_core::web::cache::WebCache;
/// use std::path::Path;
///
/// let cache = WebCache::new(Path::new("/tmp/iris-test/web"));
/// ```
pub struct WebCache {
    /// Root directory for the web cache.
    cache_dir: PathBuf,
}

impl WebCache {
    /// Create a new web cache at the given directory.
    #[must_use]
    pub fn new(cache_dir: &Path) -> Self {
        Self {
            cache_dir: cache_dir.to_path_buf(),
        }
    }

    /// Store a fetched page's markdown content and metadata.
    ///
    /// Creates the cache directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns [`WebError::CacheIo`] if the directory cannot be created or files
    /// cannot be written.
    pub async fn store_page(
        &self,
        url: &str,
        markdown: &str,
        meta: &WebPageMeta,
    ) -> Result<PathBuf, WebError> {
        let page_dir = self.page_dir(url);
        tokio::fs::create_dir_all(&page_dir)
            .await
            .map_err(|e| WebError::CacheIo {
                path: page_dir.clone(),
                reason: e.to_string(),
            })?;

        let content_path = page_dir.join("content.md");
        tokio::fs::write(&content_path, markdown)
            .await
            .map_err(|e| WebError::CacheIo {
                path: content_path,
                reason: e.to_string(),
            })?;

        let meta_path = page_dir.join("meta.json");
        let meta_json = serde_json::to_string_pretty(meta).map_err(|e| WebError::CacheIo {
            path: meta_path.clone(),
            reason: e.to_string(),
        })?;
        tokio::fs::write(&meta_path, meta_json)
            .await
            .map_err(|e| WebError::CacheIo {
                path: meta_path,
                reason: e.to_string(),
            })?;

        Ok(page_dir)
    }

    /// Load a cached page's markdown content and metadata.
    ///
    /// Returns `None` if the page is not in the cache.
    ///
    /// # Errors
    ///
    /// Returns [`WebError::CacheIo`] if the cache files exist but cannot be read
    /// or the metadata cannot be deserialized.
    pub async fn get_page(&self, url: &str) -> Result<Option<(String, WebPageMeta)>, WebError> {
        let page_dir = self.page_dir(url);
        let content_path = page_dir.join("content.md");
        let meta_path = page_dir.join("meta.json");

        if !content_path.exists() || !meta_path.exists() {
            return Ok(None);
        }

        let markdown = tokio::fs::read_to_string(&content_path)
            .await
            .map_err(|e| WebError::CacheIo {
                path: content_path,
                reason: e.to_string(),
            })?;

        let meta_json =
            tokio::fs::read_to_string(&meta_path)
                .await
                .map_err(|e| WebError::CacheIo {
                    path: meta_path.clone(),
                    reason: e.to_string(),
                })?;

        let meta: WebPageMeta =
            serde_json::from_str(&meta_json).map_err(|e| WebError::CacheIo {
                path: meta_path,
                reason: e.to_string(),
            })?;

        Ok(Some((markdown, meta)))
    }

    /// Check whether a URL is already cached.
    #[must_use]
    pub fn has_page(&self, url: &str) -> bool {
        let page_dir = self.page_dir(url);
        page_dir.join("content.md").exists() && page_dir.join("meta.json").exists()
    }

    /// Compute the cache directory for a URL.
    fn page_dir(&self, url: &str) -> PathBuf {
        let hash = url_hash(url);
        self.cache_dir.join(hash)
    }

    /// Returns the root cache directory.
    #[must_use]
    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }
}

/// Compute a short SHA-256 hash of a URL for use as a directory name.
///
/// Uses the first 16 hex characters (64 bits) to balance uniqueness and
/// filesystem friendliness.
///
/// # Examples
///
/// ```
/// use iris_core::web::cache::url_hash;
///
/// let hash = url_hash("https://example.com/docs/");
/// assert_eq!(hash.len(), 16);
/// // Same URL always produces the same hash
/// assert_eq!(hash, url_hash("https://example.com/docs/"));
/// ```
#[must_use]
pub fn url_hash(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    let digest = hasher.finalize();
    format!("{digest:x}")[..16].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_hash_deterministic() {
        let h1 = url_hash("https://example.com/docs/");
        let h2 = url_hash("https://example.com/docs/");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
    }

    #[test]
    fn url_hash_different_urls_differ() {
        let h1 = url_hash("https://example.com/docs/");
        let h2 = url_hash("https://example.com/api/");
        assert_ne!(h1, h2);
    }

    #[test]
    fn web_page_meta_serialize_roundtrip() {
        let meta = WebPageMeta {
            source_url: "https://example.com/".into(),
            fetched_at: "2026-03-21T12:00:00Z".into(),
            etag: Some("\"abc\"".into()),
            last_modified: None,
            content_hash: "deadbeef".into(),
            content_type: Some("text/html".into()),
        };
        let json = serde_json::to_string(&meta).unwrap();
        let back: WebPageMeta = serde_json::from_str(&json).unwrap();
        assert_eq!(meta, back);
    }

    #[tokio::test]
    async fn store_and_retrieve_page() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = WebCache::new(tmp.path());

        let url = "https://example.com/docs/guide";
        let markdown = "# Guide\n\nWelcome to the guide.\n";
        let meta = WebPageMeta {
            source_url: url.into(),
            fetched_at: "2026-03-21T12:00:00Z".into(),
            etag: None,
            last_modified: None,
            content_hash: "abc123".into(),
            content_type: None,
        };

        let page_dir = cache.store_page(url, markdown, &meta).await.unwrap();
        assert!(page_dir.exists());
        assert!(cache.has_page(url));

        let (loaded_md, loaded_meta) = cache.get_page(url).await.unwrap().unwrap();
        assert_eq!(loaded_md, markdown);
        assert_eq!(loaded_meta, meta);
    }

    #[tokio::test]
    async fn get_nonexistent_page_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = WebCache::new(tmp.path());

        let result = cache.get_page("https://example.com/missing").await.unwrap();
        assert!(result.is_none());
        assert!(!cache.has_page("https://example.com/missing"));
    }
}
