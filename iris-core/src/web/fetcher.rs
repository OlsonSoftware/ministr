//! Web content fetcher with automatic strategy selection.
//!
//! [`WebFetcher`] orchestrates fetching web content by trying strategies in
//! priority order: `llms-full.txt` → `llms.txt` link list → direct page fetch.
//! Each strategy produces clean markdown that can be piped into the ingestion
//! pipeline.

use std::path::Path;

use tracing::{debug, info, instrument, warn};
use url::Url;

use crate::error::WebError;
use crate::ingestion::IngestionPipeline;
use crate::llms_txt::{LlmsTxtContent, fetch_llms_txt};
use crate::parser::ParserKind;
use crate::parser::html_to_md::html_to_markdown;
use crate::storage::traits::Storage;
use crate::token::count_tokens;
use crate::web::cache::{WebCache, WebPageMeta, url_hash};
use crate::web::{HttpClient, normalize_url};

/// The strategy used to fetch content from a URL.
///
/// # Examples
///
/// ```
/// use iris_core::web::fetcher::FetchStrategy;
///
/// let strategy = FetchStrategy::LlmsFullTxt;
/// assert_eq!(strategy.to_string(), "llms_full_txt");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchStrategy {
    /// Content came from `llms-full.txt` (single markdown file).
    LlmsFullTxt,
    /// Content came from following links in `llms.txt`.
    LlmsTxtLinks,
    /// Content was fetched directly from the given URL.
    DirectFetch,
}

impl std::fmt::Display for FetchStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LlmsFullTxt => f.write_str("llms_full_txt"),
            Self::LlmsTxtLinks => f.write_str("llms_txt_links"),
            Self::DirectFetch => f.write_str("direct_fetch"),
        }
    }
}

/// A single fetched page with its markdown content.
#[derive(Debug, Clone)]
pub struct FetchedPage {
    /// The source URL of the page.
    pub url: String,
    /// Clean markdown content.
    pub markdown: String,
}

/// Result of a web fetch operation.
///
/// # Examples
///
/// ```
/// use iris_core::web::fetcher::{FetchResult, FetchStrategy};
///
/// let result = FetchResult {
///     pages: vec![],
///     strategy: FetchStrategy::DirectFetch,
///     sections_indexed: 0,
///     claims_extracted: 0,
///     tokens_added: 0,
/// };
/// assert_eq!(result.pages_fetched(), 0);
/// ```
#[derive(Debug, Clone)]
pub struct FetchResult {
    /// The pages that were fetched.
    pub pages: Vec<FetchedPage>,
    /// The strategy that was used.
    pub strategy: FetchStrategy,
    /// Total sections indexed across all pages.
    pub sections_indexed: usize,
    /// Total claims extracted across all pages.
    pub claims_extracted: usize,
    /// Total tokens added to the corpus from fetched content.
    pub tokens_added: usize,
}

impl FetchResult {
    /// Number of pages fetched.
    #[must_use]
    pub fn pages_fetched(&self) -> usize {
        self.pages.len()
    }
}

/// Configuration for the web fetcher.
///
/// # Examples
///
/// ```
/// use iris_core::web::fetcher::WebFetcherConfig;
///
/// let config = WebFetcherConfig::default();
/// assert_eq!(config.max_pages, 50);
/// ```
#[derive(Debug, Clone)]
pub struct WebFetcherConfig {
    /// Maximum number of pages to fetch when following links (default: 50).
    pub max_pages: usize,
    /// Optional path prefix filter — only fetch URLs whose path starts with this.
    pub path_filter: Option<String>,
}

impl Default for WebFetcherConfig {
    fn default() -> Self {
        Self {
            max_pages: 50,
            path_filter: None,
        }
    }
}

/// Web content fetcher with automatic strategy selection.
///
/// Tries fetch strategies in priority order:
/// 1. `llms-full.txt` — returns raw markdown directly
/// 2. `llms.txt` — parses the link list and fetches each page
/// 3. Direct page fetch — fetches the URL and converts HTML to markdown
///
/// Fetched content is cached on disk and can be piped into the ingestion
/// pipeline for indexing.
pub struct WebFetcher {
    client: HttpClient,
    cache: WebCache,
    config: WebFetcherConfig,
}

impl WebFetcher {
    /// Create a new web fetcher.
    ///
    /// # Arguments
    ///
    /// * `client` — HTTP client for making requests.
    /// * `cache_dir` — Directory for storing cached web content.
    /// * `config` — Fetcher configuration.
    #[must_use]
    pub fn new(client: HttpClient, cache_dir: &Path, config: WebFetcherConfig) -> Self {
        Self {
            client,
            cache: WebCache::new(cache_dir),
            config,
        }
    }

    /// Fetch content from a URL using automatic strategy selection.
    ///
    /// Tries strategies in order until one succeeds:
    /// 1. Try `llms-full.txt` / `llms.txt` for the domain
    /// 2. Fall back to direct page fetch
    ///
    /// # Errors
    ///
    /// Returns [`WebError`] if all strategies fail.
    #[instrument(skip(self), fields(url = %url))]
    pub async fn fetch(&self, url: &str) -> Result<FetchResult, WebError> {
        let parsed = normalize_url(url)?;
        let domain = parsed
            .host_str()
            .ok_or_else(|| WebError::InvalidUrl {
                url: url.to_owned(),
                reason: "no host in URL".into(),
            })?
            .to_owned();

        // Strategy 1: Try llms-full.txt / llms.txt
        match self.try_llms_txt(&domain).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                debug!(domain = %domain, error = %e, "llms.txt strategy failed, trying direct fetch");
            }
        }

        // Strategy 2: Direct page fetch
        self.fetch_single_page(&parsed).await
    }

    /// Fetch a single page and convert to markdown.
    ///
    /// # Errors
    ///
    /// Returns [`WebError`] if the page cannot be fetched.
    #[instrument(skip(self), fields(url = %url))]
    pub async fn fetch_single_page(&self, url: &Url) -> Result<FetchResult, WebError> {
        let url_str = url.as_str();

        let response = self.client.get(url_str).await?;
        let content_type = response
            .headers
            .get("content-type")
            .cloned()
            .unwrap_or_default();

        let markdown = if content_type.contains("text/html") {
            html_to_markdown(&response.body)
        } else {
            // Assume markdown/plain text
            response.body.clone()
        };

        // Cache the fetched page
        let now = chrono_now();
        let meta = WebPageMeta {
            source_url: url_str.to_owned(),
            fetched_at: now,
            etag: response.headers.get("etag").cloned(),
            content_hash: url_hash(&markdown),
            content_type: Some(content_type),
        };
        self.cache.store_page(url_str, &markdown, &meta).await?;

        info!(url = %url_str, bytes = markdown.len(), "fetched page via direct fetch");

        Ok(FetchResult {
            pages: vec![FetchedPage {
                url: url_str.to_owned(),
                markdown,
            }],
            strategy: FetchStrategy::DirectFetch,
            sections_indexed: 0,
            claims_extracted: 0,
            tokens_added: 0,
        })
    }

    /// Try the llms.txt strategy for a domain.
    async fn try_llms_txt(&self, domain: &str) -> Result<FetchResult, WebError> {
        let content = fetch_llms_txt(&self.client, domain).await?;

        match content {
            LlmsTxtContent::Full(markdown) => {
                let url = format!("https://{domain}/llms-full.txt");
                let now = chrono_now();
                let meta = WebPageMeta {
                    source_url: url.clone(),
                    fetched_at: now,
                    etag: None,
                    content_hash: url_hash(&markdown),
                    content_type: Some("text/plain".into()),
                };
                self.cache.store_page(&url, &markdown, &meta).await?;

                info!(domain = %domain, bytes = markdown.len(), "fetched via llms-full.txt");

                Ok(FetchResult {
                    pages: vec![FetchedPage { url, markdown }],
                    strategy: FetchStrategy::LlmsFullTxt,
                    sections_indexed: 0,
                    claims_extracted: 0,
                    tokens_added: 0,
                })
            }
            LlmsTxtContent::Parsed(llms_txt) => {
                let mut pages = Vec::new();

                // Collect all links from non-optional sections
                let links: Vec<_> = llms_txt
                    .sections
                    .iter()
                    .filter(|s| !s.is_optional)
                    .flat_map(|s| &s.links)
                    .collect();

                let max = self.config.max_pages.min(links.len());
                for link in links.iter().take(max) {
                    if let Some(ref filter) = self.config.path_filter {
                        if let Ok(parsed) = Url::parse(&link.url) {
                            if !parsed.path().starts_with(filter.as_str()) {
                                debug!(url = %link.url, filter = %filter, "skipping: path filter mismatch");
                                continue;
                            }
                        }
                    }

                    match self.fetch_link_page(&link.url).await {
                        Ok(page) => pages.push(page),
                        Err(e) => {
                            warn!(url = %link.url, error = %e, "failed to fetch linked page");
                        }
                    }
                }

                info!(
                    domain = %domain,
                    pages_fetched = pages.len(),
                    total_links = links.len(),
                    "fetched via llms.txt links"
                );

                Ok(FetchResult {
                    pages,
                    strategy: FetchStrategy::LlmsTxtLinks,
                    sections_indexed: 0,
                    claims_extracted: 0,
                    tokens_added: 0,
                })
            }
        }
    }

    /// Fetch a single linked page and convert to markdown.
    async fn fetch_link_page(&self, url: &str) -> Result<FetchedPage, WebError> {
        let response = self.client.get(url).await?;
        let content_type = response
            .headers
            .get("content-type")
            .cloned()
            .unwrap_or_default();

        let markdown = if content_type.contains("text/html") {
            html_to_markdown(&response.body)
        } else {
            response.body.clone()
        };

        // Cache
        let now = chrono_now();
        let meta = WebPageMeta {
            source_url: url.to_owned(),
            fetched_at: now,
            etag: response.headers.get("etag").cloned(),
            content_hash: url_hash(&markdown),
            content_type: Some(content_type),
        };
        self.cache.store_page(url, &markdown, &meta).await?;

        debug!(url = %url, bytes = markdown.len(), "fetched linked page");

        Ok(FetchedPage {
            url: url.to_owned(),
            markdown,
        })
    }

    /// Fetch content and ingest it into storage.
    ///
    /// Combines [`fetch`](Self::fetch) with ingestion via [`IngestionPipeline`].
    /// Each fetched page is ingested as a separate document with a virtual source
    /// path of `web://{url}`.
    ///
    /// # Errors
    ///
    /// Returns [`WebError`] if fetching fails, or [`WebError::IngestionFailed`]
    /// if ingestion fails.
    #[instrument(skip(self, pipeline, storage), fields(url = %url))]
    pub async fn fetch_and_ingest<S: Storage>(
        &self,
        url: &str,
        pipeline: &IngestionPipeline,
        storage: &S,
    ) -> Result<FetchResult, WebError> {
        let mut result = self.fetch(url).await?;

        let mut total_sections = 0;
        let mut total_claims = 0;
        let mut total_tokens = 0;

        for page in &result.pages {
            let source_path = web_source_path(&page.url);
            total_tokens += count_tokens(&page.markdown);
            let stats = pipeline
                .ingest_content(&source_path, &page.markdown, ParserKind::Markdown, storage)
                .await
                .map_err(|e| WebError::IngestionFailed {
                    reason: e.to_string(),
                })?;

            if !stats.skipped {
                total_sections += stats.sections;
                total_claims += stats.claims;
            }
        }

        result.sections_indexed = total_sections;
        result.claims_extracted = total_claims;
        result.tokens_added = total_tokens;

        info!(
            url = %url,
            pages = result.pages_fetched(),
            sections = total_sections,
            claims = total_claims,
            strategy = %result.strategy,
            "fetch and ingest complete"
        );

        Ok(result)
    }

    /// Fetch content and ingest it into storage with embeddings.
    ///
    /// Like [`fetch_and_ingest`](Self::fetch_and_ingest), but also generates
    /// embeddings and inserts them into the vector index so the content is
    /// immediately searchable via `iris_survey`.
    ///
    /// # Errors
    ///
    /// Returns [`WebError`] if fetching fails, or [`WebError::IngestionFailed`]
    /// if ingestion or embedding fails.
    #[instrument(skip(self, pipeline, storage, embedder, index), fields(url = %url))]
    pub async fn fetch_and_ingest_with_embeddings<S, E, I>(
        &self,
        url: &str,
        pipeline: &IngestionPipeline,
        storage: &S,
        embedder: &E,
        index: &I,
    ) -> Result<FetchResult, WebError>
    where
        S: Storage + ?Sized,
        E: crate::embedding::Embedder + ?Sized,
        I: crate::index::VectorIndex + ?Sized,
    {
        let mut result = self.fetch(url).await?;

        let mut total_sections = 0;
        let mut total_claims = 0;
        let mut total_tokens = 0;

        for page in &result.pages {
            let source_path = web_source_path(&page.url);
            total_tokens += count_tokens(&page.markdown);
            let stats = pipeline
                .ingest_content_with_embeddings(
                    &source_path,
                    &page.markdown,
                    ParserKind::Markdown,
                    storage,
                    embedder,
                    index,
                )
                .await
                .map_err(|e| WebError::IngestionFailed {
                    reason: e.to_string(),
                })?;

            if !stats.skipped {
                total_sections += stats.sections;
                total_claims += stats.claims;
            }
        }

        result.sections_indexed = total_sections;
        result.claims_extracted = total_claims;
        result.tokens_added = total_tokens;

        info!(
            url = %url,
            pages = result.pages_fetched(),
            sections = total_sections,
            claims = total_claims,
            strategy = %result.strategy,
            "fetch and ingest with embeddings complete"
        );

        Ok(result)
    }

    /// Returns a reference to the underlying HTTP client.
    #[must_use]
    pub fn client(&self) -> &HttpClient {
        &self.client
    }

    /// Returns a reference to the web cache.
    #[must_use]
    pub fn cache(&self) -> &WebCache {
        &self.cache
    }
}

/// Compute the virtual source path for a web-fetched document.
///
/// # Examples
///
/// ```
/// use iris_core::web::fetcher::web_source_path;
///
/// assert_eq!(
///     web_source_path("https://example.com/docs/guide"),
///     "web://example.com/docs/guide"
/// );
/// ```
#[must_use]
pub fn web_source_path(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("https://") {
        format!("web://{rest}")
    } else if let Some(rest) = url.strip_prefix("http://") {
        format!("web://{rest}")
    } else {
        format!("web://{url}")
    }
}

/// Get the current UTC timestamp as ISO 8601.
fn chrono_now() -> String {
    // Use a simple approach without pulling in the chrono crate
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Format as a simple ISO-ish timestamp
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_source_path_strips_https() {
        assert_eq!(
            web_source_path("https://example.com/docs/guide"),
            "web://example.com/docs/guide"
        );
    }

    #[test]
    fn web_source_path_strips_http() {
        assert_eq!(
            web_source_path("http://example.com/api"),
            "web://example.com/api"
        );
    }

    #[test]
    fn web_source_path_no_scheme() {
        assert_eq!(
            web_source_path("example.com/page"),
            "web://example.com/page"
        );
    }

    #[test]
    fn fetch_strategy_display() {
        assert_eq!(FetchStrategy::LlmsFullTxt.to_string(), "llms_full_txt");
        assert_eq!(FetchStrategy::LlmsTxtLinks.to_string(), "llms_txt_links");
        assert_eq!(FetchStrategy::DirectFetch.to_string(), "direct_fetch");
    }

    #[test]
    fn fetch_result_pages_fetched() {
        let result = FetchResult {
            pages: vec![
                FetchedPage {
                    url: "https://a.com".into(),
                    markdown: "# A".into(),
                },
                FetchedPage {
                    url: "https://b.com".into(),
                    markdown: "# B".into(),
                },
            ],
            strategy: FetchStrategy::LlmsTxtLinks,
            sections_indexed: 0,
            claims_extracted: 0,
            tokens_added: 0,
        };
        assert_eq!(result.pages_fetched(), 2);
    }

    #[test]
    fn web_fetcher_config_defaults() {
        let config = WebFetcherConfig::default();
        assert_eq!(config.max_pages, 50);
        assert!(config.path_filter.is_none());
    }
}
