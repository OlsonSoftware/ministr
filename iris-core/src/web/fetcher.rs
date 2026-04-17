//! Web content fetcher with automatic strategy selection.
//!
//! [`WebFetcher`] orchestrates fetching web content by trying strategies in
//! priority order: `llms-full.txt` → `llms.txt` link list → direct page fetch.
//! Each strategy produces clean markdown that can be piped into the ingestion
//! pipeline.

use std::path::Path;

use futures::stream::{self, StreamExt};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, instrument, warn};
use url::Url;

use crate::error::WebError;
use crate::ingestion::IngestionPipeline;
use crate::llms_txt::{LlmsTxtContent, fetch_llms_txt};
use crate::parser::ParserKind;
use crate::parser::html_to_md::html_to_markdown;
use crate::storage::traits::{Storage, WebCacheRecord};
use crate::token::count_tokens;
use crate::web::cache::{WebCache, WebPageMeta, url_hash};
use crate::web::sitemap::{
    SitemapConfig, fetch_sitemap_pages, filter_entries, is_sitemap_url, parse_sitemap,
};
use crate::web::{HttpClient, StalenessResult, normalize_url};

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
    /// Content was crawled from a sitemap.xml.
    Sitemap,
}

impl std::fmt::Display for FetchStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LlmsFullTxt => f.write_str("llms_full_txt"),
            Self::LlmsTxtLinks => f.write_str("llms_txt_links"),
            Self::DirectFetch => f.write_str("direct_fetch"),
            Self::Sitemap => f.write_str("sitemap"),
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
    /// Time-to-live in seconds before a cached URL is considered stale (default: 3600 = 1 hour).
    pub staleness_ttl_secs: u64,
    /// Maximum concurrent URL staleness checks during refresh (default: 4).
    pub refresh_concurrency: usize,
}

impl Default for WebFetcherConfig {
    fn default() -> Self {
        Self {
            max_pages: 50,
            path_filter: None,
            staleness_ttl_secs: 3600,
            refresh_concurrency: 4,
        }
    }
}

/// Status of a single URL after a refresh check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshUrlStatus {
    /// The URL content has not changed.
    Unchanged,
    /// The URL content was updated and re-indexed.
    Updated,
    /// The staleness check or re-fetch failed.
    Failed(String),
}

impl std::fmt::Display for RefreshUrlStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unchanged => f.write_str("unchanged"),
            Self::Updated => f.write_str("updated"),
            Self::Failed(reason) => write!(f, "failed: {reason}"),
        }
    }
}

/// Result of a single URL refresh operation.
#[derive(Debug, Clone)]
pub struct RefreshUrlDetail {
    /// The URL that was checked.
    pub url: String,
    /// The outcome of the check.
    pub status: RefreshUrlStatus,
}

/// Aggregate result of a refresh operation across multiple URLs.
#[derive(Debug, Clone)]
pub struct RefreshResult {
    /// Number of URLs checked.
    pub urls_checked: usize,
    /// Number of URLs that had new content and were re-indexed.
    pub urls_refreshed: usize,
    /// Number of URLs that were unchanged (304 or content hash match).
    pub urls_unchanged: usize,
    /// Number of URLs where the check failed.
    pub urls_failed: usize,
    /// Per-URL details.
    pub details: Vec<RefreshUrlDetail>,
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

        // Strategy 0: Sitemap URL — parse and crawl
        if is_sitemap_url(parsed.as_str()) {
            return self.fetch_via_sitemap(&parsed).await;
        }

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
            last_modified: response.headers.get("last-modified").cloned(),
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

    /// Fetch content via a sitemap.xml URL.
    ///
    /// Fetches the sitemap XML, parses it, applies path filtering and max page
    /// limits, then fetches all matching pages in parallel.
    ///
    /// For `<sitemapindex>` documents, fetches and parses each child sitemap
    /// to build the full URL list before crawling.
    #[instrument(skip(self), fields(url = %url))]
    async fn fetch_via_sitemap(&self, url: &Url) -> Result<FetchResult, WebError> {
        let response = self.client.get(url.as_str()).await?;
        let mut entries = parse_sitemap(&response.body)?;

        // If entries look like child sitemaps (sitemapindex), resolve them
        let is_index = entries.iter().all(|e| is_sitemap_url(&e.url));
        if is_index && !entries.is_empty() {
            let child_urls: Vec<String> = entries.iter().map(|e| e.url.clone()).collect();
            entries.clear();
            for child_url in &child_urls {
                match self.client.get(child_url).await {
                    Ok(child_response) => match parse_sitemap(&child_response.body) {
                        Ok(child_entries) => entries.extend(child_entries),
                        Err(e) => {
                            warn!(url = %child_url, error = %e, "failed to parse child sitemap");
                        }
                    },
                    Err(e) => {
                        warn!(url = %child_url, error = %e, "failed to fetch child sitemap");
                    }
                }
            }
        }

        let sitemap_config = SitemapConfig {
            path_filter: self.config.path_filter.clone(),
            max_pages: self.config.max_pages,
            ..SitemapConfig::default()
        };

        let filtered = filter_entries(&entries, &sitemap_config);
        let pages = fetch_sitemap_pages(&self.client, &filtered, &sitemap_config).await;

        // Cache each fetched page
        for page in &pages {
            let now = chrono_now();
            let meta = WebPageMeta {
                source_url: page.url.clone(),
                fetched_at: now,
                etag: None,
                last_modified: None,
                content_hash: url_hash(&page.markdown),
                content_type: Some("text/html".into()),
            };
            if let Err(e) = self
                .cache
                .store_page(&page.url, &page.markdown, &meta)
                .await
            {
                warn!(url = %page.url, error = %e, "failed to cache sitemap page");
            }
        }

        info!(
            url = %url,
            total_entries = entries.len(),
            filtered = filtered.len(),
            fetched = pages.len(),
            "fetched via sitemap"
        );

        Ok(FetchResult {
            pages,
            strategy: FetchStrategy::Sitemap,
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
                    last_modified: None,
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
                    if let Some(ref filter) = self.config.path_filter
                        && let Ok(parsed) = Url::parse(&link.url)
                        && !parsed.path().starts_with(filter.as_str())
                    {
                        debug!(url = %link.url, filter = %filter, "skipping: path filter mismatch");
                        continue;
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
            last_modified: response.headers.get("last-modified").cloned(),
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

            // Upsert web_cache record for staleness tracking
            let disk_meta = self.cache.get_page(&page.url).await.ok().flatten();
            let now = chrono_now();
            let web_cache_record = WebCacheRecord {
                source_url: page.url.clone(),
                fetch_timestamp: now,
                etag: disk_meta.as_ref().and_then(|(_, m)| m.etag.clone()),
                last_modified: disk_meta
                    .as_ref()
                    .and_then(|(_, m)| m.last_modified.clone()),
                content_hash: url_hash(&page.markdown),
                content_type: disk_meta.as_ref().and_then(|(_, m)| m.content_type.clone()),
            };
            if let Err(e) = storage.upsert_web_cache(&web_cache_record).await {
                warn!(url = %page.url, error = %e, "failed to upsert web cache record");
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
    #[instrument(skip(self, pipeline, storage, embedder, index, ct), fields(url = %url))]
    pub async fn fetch_and_ingest_with_embeddings<S, E, I>(
        &self,
        url: &str,
        pipeline: &IngestionPipeline,
        storage: &S,
        embedder: &E,
        index: &I,
        ct: Option<&CancellationToken>,
    ) -> Result<FetchResult, WebError>
    where
        S: Storage + ?Sized,
        E: crate::embedding::Embedder + ?Sized,
        I: crate::index::VectorIndex + ?Sized,
    {
        // Check cancellation before starting the fetch.
        if ct.is_some_and(CancellationToken::is_cancelled) {
            return Err(WebError::Cancelled);
        }

        let mut result = self.fetch(url).await?;

        let mut total_sections = 0;
        let mut total_claims = 0;
        let mut total_tokens = 0;

        for page in &result.pages {
            // Check cancellation between page ingestions.
            if ct.is_some_and(CancellationToken::is_cancelled) {
                return Err(WebError::Cancelled);
            }
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

            // Upsert web_cache record for staleness tracking
            let disk_meta = self.cache.get_page(&page.url).await.ok().flatten();
            let now = chrono_now();
            let web_cache_record = WebCacheRecord {
                source_url: page.url.clone(),
                fetch_timestamp: now,
                etag: disk_meta.as_ref().and_then(|(_, m)| m.etag.clone()),
                last_modified: disk_meta
                    .as_ref()
                    .and_then(|(_, m)| m.last_modified.clone()),
                content_hash: url_hash(&page.markdown),
                content_type: disk_meta.as_ref().and_then(|(_, m)| m.content_type.clone()),
            };
            if let Err(e) = storage.upsert_web_cache(&web_cache_record).await {
                warn!(url = %page.url, error = %e, "failed to upsert web cache record");
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

    /// Returns the fetcher configuration.
    #[must_use]
    pub fn config(&self) -> &WebFetcherConfig {
        &self.config
    }

    /// Check whether a cached URL is stale using conditional HTTP requests.
    ///
    /// Sends an HTTP HEAD with `If-None-Match` and/or `If-Modified-Since`
    /// headers based on cached metadata. Returns whether the URL needs
    /// re-fetching.
    ///
    /// # Errors
    ///
    /// Returns [`WebError`] if the HTTP request fails.
    #[instrument(skip(self), fields(url = %url))]
    pub async fn check_staleness(
        &self,
        url: &str,
        cached: &WebCacheRecord,
    ) -> Result<StalenessResult, WebError> {
        // Check TTL first — if within TTL, skip the HTTP check
        if let Ok(fetch_time) = parse_timestamp(&cached.fetch_timestamp) {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let age_secs = now.saturating_sub(fetch_time);
            if age_secs < self.config.staleness_ttl_secs {
                debug!(url = %url, age_secs, ttl = self.config.staleness_ttl_secs, "within TTL, skipping HTTP check");
                return Ok(StalenessResult::Fresh);
            }
        }

        self.client
            .head_conditional(url, cached.etag.as_deref(), cached.last_modified.as_deref())
            .await
    }

    /// Refresh a single URL: check staleness, re-fetch if stale, re-ingest with embeddings.
    ///
    /// Returns the refresh status for this URL.
    #[instrument(skip(self, pipeline, storage, embedder, index), fields(url = %url))]
    pub async fn refresh_url<S, E, I>(
        &self,
        url: &str,
        cached: &WebCacheRecord,
        pipeline: &IngestionPipeline,
        storage: &S,
        embedder: &E,
        index: &I,
    ) -> RefreshUrlDetail
    where
        S: Storage + ?Sized,
        E: crate::embedding::Embedder + ?Sized,
        I: crate::index::VectorIndex + ?Sized,
    {
        match self.check_staleness(url, cached).await {
            Ok(StalenessResult::Fresh) => {
                debug!(url = %url, "URL is fresh, skipping re-fetch");
                RefreshUrlDetail {
                    url: url.to_owned(),
                    status: RefreshUrlStatus::Unchanged,
                }
            }
            Ok(StalenessResult::Stale { .. }) => {
                // Re-fetch and re-ingest
                match self
                    .fetch_and_ingest_with_embeddings(url, pipeline, storage, embedder, index, None)
                    .await
                {
                    Ok(result) => {
                        // Update the web_cache record
                        let now = chrono_now();
                        let new_hash = result
                            .pages
                            .first()
                            .map(|p| url_hash(&p.markdown))
                            .unwrap_or_default();

                        if new_hash == cached.content_hash {
                            debug!(url = %url, "content unchanged despite 200 response");
                            // Still update the timestamp so TTL resets
                            let updated = WebCacheRecord {
                                source_url: url.to_owned(),
                                fetch_timestamp: now,
                                etag: cached.etag.clone(),
                                last_modified: cached.last_modified.clone(),
                                content_hash: cached.content_hash.clone(),
                                content_type: cached.content_type.clone(),
                            };
                            if let Err(e) = storage.upsert_web_cache(&updated).await {
                                warn!(url = %url, error = %e, "failed to update web cache timestamp");
                            }
                            RefreshUrlDetail {
                                url: url.to_owned(),
                                status: RefreshUrlStatus::Unchanged,
                            }
                        } else {
                            info!(url = %url, sections = result.sections_indexed, "refreshed stale URL");
                            RefreshUrlDetail {
                                url: url.to_owned(),
                                status: RefreshUrlStatus::Updated,
                            }
                        }
                    }
                    Err(e) => {
                        warn!(url = %url, error = %e, "failed to re-fetch stale URL");
                        RefreshUrlDetail {
                            url: url.to_owned(),
                            status: RefreshUrlStatus::Failed(e.to_string()),
                        }
                    }
                }
            }
            Err(e) => {
                warn!(url = %url, error = %e, "staleness check failed");
                RefreshUrlDetail {
                    url: url.to_owned(),
                    status: RefreshUrlStatus::Failed(e.to_string()),
                }
            }
        }
    }

    /// Refresh all cached web URLs, or a single URL if specified.
    ///
    /// Checks each cached URL for staleness and re-fetches/re-indexes any
    /// that have changed.
    ///
    /// # Errors
    ///
    /// Returns [`WebError`] if the URL is not found in the cache or the
    /// storage layer fails to list cached URLs.
    #[instrument(skip(self, pipeline, storage, embedder, index))]
    pub async fn refresh_all<S, E, I>(
        &self,
        url_filter: Option<&str>,
        pipeline: &IngestionPipeline,
        storage: &S,
        embedder: &E,
        index: &I,
    ) -> Result<RefreshResult, WebError>
    where
        S: Storage + ?Sized,
        E: crate::embedding::Embedder + ?Sized,
        I: crate::index::VectorIndex + ?Sized,
    {
        let records = if let Some(url) = url_filter {
            match storage.get_web_cache(url).await {
                Ok(Some(record)) => vec![record],
                Ok(None) => {
                    return Err(WebError::InvalidUrl {
                        url: url.to_owned(),
                        reason: "URL not found in web cache".into(),
                    });
                }
                Err(e) => {
                    return Err(WebError::CacheIo {
                        path: std::path::PathBuf::from("web_cache"),
                        reason: e.to_string(),
                    });
                }
            }
        } else {
            storage
                .list_web_cache()
                .await
                .map_err(|e| WebError::CacheIo {
                    path: std::path::PathBuf::from("web_cache"),
                    reason: e.to_string(),
                })?
        };

        let concurrency = self.config.refresh_concurrency;
        let num_records = records.len();
        let details: Vec<RefreshUrlDetail> = stream::iter(records)
            .map(|record| async move {
                self.refresh_url(
                    &record.source_url,
                    &record,
                    pipeline,
                    storage,
                    embedder,
                    index,
                )
                .await
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;

        let mut refreshed = 0;
        let mut unchanged = 0;
        let mut failed = 0;
        for detail in &details {
            match &detail.status {
                RefreshUrlStatus::Updated => refreshed += 1,
                RefreshUrlStatus::Unchanged => unchanged += 1,
                RefreshUrlStatus::Failed(_) => failed += 1,
            }
        }

        info!(
            checked = num_records,
            refreshed, unchanged, failed, "refresh complete"
        );

        Ok(RefreshResult {
            urls_checked: num_records,
            urls_refreshed: refreshed,
            urls_unchanged: unchanged,
            urls_failed: failed,
            details,
        })
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

/// Get the current UTC timestamp as epoch seconds string.
fn chrono_now() -> String {
    let now = std::time::SystemTime::now();
    let duration = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    format!("{secs}")
}

/// Parse a timestamp string (epoch seconds or ISO 8601) to epoch seconds.
///
/// Returns `Err` if the string is not a valid number.
fn parse_timestamp(ts: &str) -> Result<u64, std::num::ParseIntError> {
    ts.parse::<u64>()
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
        assert_eq!(FetchStrategy::Sitemap.to_string(), "sitemap");
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
        assert_eq!(config.staleness_ttl_secs, 3600);
    }

    #[test]
    fn refresh_url_status_display() {
        assert_eq!(RefreshUrlStatus::Unchanged.to_string(), "unchanged");
        assert_eq!(RefreshUrlStatus::Updated.to_string(), "updated");
        assert_eq!(
            RefreshUrlStatus::Failed("timeout".into()).to_string(),
            "failed: timeout"
        );
    }

    #[test]
    fn parse_timestamp_valid() {
        assert_eq!(parse_timestamp("1711036800").unwrap(), 1_711_036_800);
    }

    #[test]
    fn parse_timestamp_invalid() {
        assert!(parse_timestamp("not-a-number").is_err());
    }

    #[test]
    fn refresh_result_counts() {
        let result = RefreshResult {
            urls_checked: 3,
            urls_refreshed: 1,
            urls_unchanged: 1,
            urls_failed: 1,
            details: vec![
                RefreshUrlDetail {
                    url: "https://a.com".into(),
                    status: RefreshUrlStatus::Updated,
                },
                RefreshUrlDetail {
                    url: "https://b.com".into(),
                    status: RefreshUrlStatus::Unchanged,
                },
                RefreshUrlDetail {
                    url: "https://c.com".into(),
                    status: RefreshUrlStatus::Failed("404".into()),
                },
            ],
        };
        assert_eq!(result.urls_checked, 3);
        assert_eq!(result.urls_refreshed, 1);
        assert_eq!(result.urls_unchanged, 1);
        assert_eq!(result.urls_failed, 1);
    }
}
