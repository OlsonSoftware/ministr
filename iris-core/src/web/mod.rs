//! HTTP client, URL utilities, and web fetching pipeline.
//!
//! Provides an async [`HttpClient`] with configurable timeout, retry logic,
//! and a User-Agent header identifying iris. Also includes URL normalization
//! and resolution helpers, a [`cache::WebCache`] for persisting fetched content,
//! and a [`fetcher::WebFetcher`] orchestrator for auto-selecting fetch strategies.

pub mod cache;
pub mod fetcher;
pub mod sitemap;

use std::collections::HashMap;
use std::time::Duration;

use url::Url;

use crate::error::WebError;

/// Configuration for [`HttpClient`].
#[derive(Debug, Clone)]
pub struct HttpClientConfig {
    /// Request timeout in seconds (default: 30).
    pub timeout_secs: u64,

    /// Number of retry attempts on transient failures (default: 2).
    pub retry_count: u32,

    /// User-Agent header value.
    pub user_agent: String,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            timeout_secs: 30,
            retry_count: 2,
            user_agent: format!("iris/{}", env!("CARGO_PKG_VERSION")),
        }
    }
}

/// Response from an HTTP fetch.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,

    /// Response body as a string.
    pub body: String,

    /// Selected response headers (lowercased keys).
    pub headers: HashMap<String, String>,
}

/// Result of a conditional HTTP staleness check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StalenessResult {
    /// The resource has not been modified (HTTP 304).
    Fresh,
    /// The resource has been modified — includes the new `ETag` and `Last-Modified` if present.
    Stale {
        /// New `ETag` value, if the server returned one.
        new_etag: Option<String>,
        /// New `Last-Modified` value, if the server returned one.
        new_last_modified: Option<String>,
    },
}

/// Async HTTP client with retry logic and configurable timeouts.
#[derive(Debug, Clone)]
pub struct HttpClient {
    inner: reqwest::Client,
    config: HttpClientConfig,
}

impl HttpClient {
    /// Create a new HTTP client with the given configuration.
    ///
    /// # Errors
    ///
    /// Returns [`WebError::Request`] if the underlying client cannot be built.
    pub fn new(config: HttpClientConfig) -> Result<Self, WebError> {
        let inner = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .user_agent(&config.user_agent)
            .build()?;

        Ok(Self { inner, config })
    }

    /// Create a new HTTP client with default configuration.
    ///
    /// # Errors
    ///
    /// Returns [`WebError::Request`] if the underlying client cannot be built.
    pub fn with_defaults() -> Result<Self, WebError> {
        Self::new(HttpClientConfig::default())
    }

    /// Fetch a URL with GET, retrying on transient failures (5xx, timeouts).
    ///
    /// # Errors
    ///
    /// Returns [`WebError::InvalidUrl`] if the URL is malformed,
    /// [`WebError::TooManyRetries`] if all retry attempts fail on transient errors,
    /// or [`WebError::HttpStatus`] for non-retryable HTTP error codes (4xx).
    #[tracing::instrument(skip(self), fields(url = %url))]
    pub async fn get(&self, url: &str) -> Result<HttpResponse, WebError> {
        let parsed = normalize_url(url)?;
        let url_str = parsed.as_str();

        let mut last_error = String::new();
        let max_attempts = self.config.retry_count + 1;

        for attempt in 1..=max_attempts {
            match self.inner.get(url_str).send().await {
                Ok(response) => {
                    let status = response.status().as_u16();

                    if response.status().is_success() {
                        let headers = extract_headers(&response);
                        let body = response.text().await?;
                        return Ok(HttpResponse {
                            status,
                            body,
                            headers,
                        });
                    }

                    if !response.status().is_server_error() {
                        // Client errors (4xx) are not retryable
                        return Err(WebError::HttpStatus {
                            url: url_str.to_owned(),
                            status,
                        });
                    }

                    last_error = format!("HTTP {status}");
                    if attempt < max_attempts {
                        let backoff = Duration::from_secs(u64::from(attempt));
                        tokio::time::sleep(backoff).await;
                    }
                }
                Err(e) => {
                    last_error = e.to_string();
                    if !is_retryable(&e) {
                        return Err(WebError::Request { source: e });
                    }
                    if attempt < max_attempts {
                        let backoff = Duration::from_secs(u64::from(attempt));
                        tokio::time::sleep(backoff).await;
                    }
                }
            }
        }

        Err(WebError::TooManyRetries {
            url: url_str.to_owned(),
            attempts: max_attempts,
            reason: last_error,
        })
    }

    /// Send a conditional HTTP HEAD request to check for staleness.
    ///
    /// Includes `If-None-Match` (from `etag`) and/or `If-Modified-Since`
    /// (from `last_modified`) headers when available. Returns [`StalenessResult::Fresh`]
    /// on HTTP 304, or [`StalenessResult::Stale`] on HTTP 200.
    ///
    /// # Errors
    ///
    /// Returns [`WebError`] if the request fails or the URL is invalid.
    #[tracing::instrument(skip(self), fields(url = %url))]
    pub async fn head_conditional(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<StalenessResult, WebError> {
        let parsed = normalize_url(url)?;
        let url_str = parsed.as_str();

        let mut request = self.inner.head(url_str);
        if let Some(etag_val) = etag {
            request = request.header("If-None-Match", etag_val);
        }
        if let Some(lm_val) = last_modified {
            request = request.header("If-Modified-Since", lm_val);
        }

        let response = request.send().await?;
        let status = response.status().as_u16();

        if status == 304 {
            return Ok(StalenessResult::Fresh);
        }

        if response.status().is_success() {
            let headers = extract_headers(&response);
            return Ok(StalenessResult::Stale {
                new_etag: headers.get("etag").cloned(),
                new_last_modified: headers.get("last-modified").cloned(),
            });
        }

        Err(WebError::HttpStatus {
            url: url_str.to_owned(),
            status,
        })
    }

    /// Returns the client configuration.
    #[must_use]
    pub fn config(&self) -> &HttpClientConfig {
        &self.config
    }
}

/// Normalize a URL string: parse, validate scheme, strip fragment, normalize trailing slash.
///
/// # Errors
///
/// Returns [`WebError::InvalidUrl`] if the URL cannot be parsed or uses
/// an unsupported scheme (only `http` and `https` are accepted).
///
/// # Examples
///
/// ```
/// use iris_core::web::normalize_url;
///
/// let url = normalize_url("https://example.com/docs/#section").unwrap();
/// assert_eq!(url.as_str(), "https://example.com/docs/");
/// assert!(url.fragment().is_none());
/// ```
pub fn normalize_url(raw: &str) -> Result<Url, WebError> {
    let mut url = Url::parse(raw).map_err(|e| WebError::InvalidUrl {
        url: raw.to_owned(),
        reason: e.to_string(),
    })?;

    let scheme = url.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(WebError::InvalidUrl {
            url: raw.to_owned(),
            reason: format!("unsupported scheme: {scheme}"),
        });
    }

    // Strip fragment
    url.set_fragment(None);

    // Normalize trailing slash: ensure paths without a file extension end with /
    let path = url.path().to_owned();
    if !path.ends_with('/')
        && !path
            .rsplit('/')
            .next()
            .is_some_and(|last| last.contains('.'))
    {
        url.set_path(&format!("{path}/"));
    }

    Ok(url)
}

/// Resolve a relative URL against a base URL.
///
/// # Errors
///
/// Returns [`WebError::InvalidUrl`] if the result cannot be resolved
/// or the resolved URL uses an unsupported scheme.
///
/// # Examples
///
/// ```
/// use iris_core::web::{normalize_url, resolve_url};
///
/// let base = normalize_url("https://example.com/docs/").unwrap();
/// let resolved = resolve_url(&base, "getting-started").unwrap();
/// assert_eq!(resolved.as_str(), "https://example.com/docs/getting-started/");
/// ```
pub fn resolve_url(base: &Url, relative: &str) -> Result<Url, WebError> {
    let resolved = base.join(relative).map_err(|e| WebError::InvalidUrl {
        url: relative.to_owned(),
        reason: e.to_string(),
    })?;

    // Validate the resolved URL through normalize_url
    normalize_url(resolved.as_str())
}

/// Check if a reqwest error is transient and worth retrying.
fn is_retryable(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request()
}

/// Extract useful headers from a response (lowercased keys).
fn extract_headers(response: &reqwest::Response) -> HashMap<String, String> {
    let keys = ["content-type", "etag", "last-modified", "content-length"];
    let mut headers = HashMap::new();
    for key in keys {
        if let Some(value) = response.headers().get(key) {
            if let Ok(v) = value.to_str() {
                headers.insert(key.to_owned(), v.to_owned());
            }
        }
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- URL normalization tests --

    #[test]
    fn normalize_strips_fragment() {
        let url = normalize_url("https://example.com/docs/#section").unwrap();
        assert!(url.fragment().is_none());
        assert_eq!(url.as_str(), "https://example.com/docs/");
    }

    #[test]
    fn normalize_adds_trailing_slash_to_path() {
        let url = normalize_url("https://example.com/docs").unwrap();
        assert_eq!(url.path(), "/docs/");
    }

    #[test]
    fn normalize_preserves_trailing_slash() {
        let url = normalize_url("https://example.com/docs/").unwrap();
        assert_eq!(url.path(), "/docs/");
    }

    #[test]
    fn normalize_preserves_file_extension_without_trailing_slash() {
        let url = normalize_url("https://example.com/page.html").unwrap();
        assert_eq!(url.path(), "/page.html");
    }

    #[test]
    fn normalize_rejects_ftp_scheme() {
        let err = normalize_url("ftp://example.com/file").unwrap_err();
        assert!(err.to_string().contains("unsupported scheme"));
    }

    #[test]
    fn normalize_rejects_invalid_url() {
        let err = normalize_url("not a url at all").unwrap_err();
        assert!(matches!(err, WebError::InvalidUrl { .. }));
    }

    #[test]
    fn normalize_preserves_query_params() {
        let url = normalize_url("https://example.com/search?q=rust&page=1").unwrap();
        assert_eq!(url.query(), Some("q=rust&page=1"));
    }

    #[test]
    fn normalize_http_scheme_accepted() {
        let url = normalize_url("http://example.com/").unwrap();
        assert_eq!(url.scheme(), "http");
    }

    #[test]
    fn normalize_strips_fragment_preserves_query() {
        let url = normalize_url("https://example.com/docs?v=2#heading").unwrap();
        assert_eq!(url.as_str(), "https://example.com/docs/?v=2");
    }

    // -- URL resolution tests --

    #[test]
    fn resolve_relative_path() {
        let base = normalize_url("https://example.com/docs/").unwrap();
        let resolved = resolve_url(&base, "getting-started").unwrap();
        assert_eq!(
            resolved.as_str(),
            "https://example.com/docs/getting-started/"
        );
    }

    #[test]
    fn resolve_absolute_path() {
        let base = normalize_url("https://example.com/docs/").unwrap();
        let resolved = resolve_url(&base, "/api/v2").unwrap();
        assert_eq!(resolved.as_str(), "https://example.com/api/v2/");
    }

    #[test]
    fn resolve_full_url_ignores_base() {
        let base = normalize_url("https://example.com/docs/").unwrap();
        let resolved = resolve_url(&base, "https://other.com/page").unwrap();
        assert_eq!(resolved.as_str(), "https://other.com/page/");
    }

    #[test]
    fn resolve_parent_path() {
        let base = normalize_url("https://example.com/docs/v1/").unwrap();
        let resolved = resolve_url(&base, "../v2/intro").unwrap();
        assert_eq!(resolved.as_str(), "https://example.com/docs/v2/intro/");
    }

    // -- HttpClient construction tests --

    #[test]
    fn client_default_config() {
        let config = HttpClientConfig::default();
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.retry_count, 2);
        assert!(config.user_agent.starts_with("iris/"));
    }

    #[test]
    fn client_custom_config() {
        let config = HttpClientConfig {
            timeout_secs: 10,
            retry_count: 5,
            user_agent: "test-agent/1.0".into(),
        };
        let client = HttpClient::new(config.clone()).unwrap();
        assert_eq!(client.config().timeout_secs, 10);
        assert_eq!(client.config().retry_count, 5);
        assert_eq!(client.config().user_agent, "test-agent/1.0");
    }

    #[test]
    fn client_with_defaults_builds() {
        let client = HttpClient::with_defaults();
        assert!(client.is_ok());
    }

    // -- StalenessResult tests --

    #[test]
    fn staleness_result_fresh_eq() {
        assert_eq!(StalenessResult::Fresh, StalenessResult::Fresh);
    }

    #[test]
    fn staleness_result_stale_with_etag() {
        let result = StalenessResult::Stale {
            new_etag: Some("\"v2\"".into()),
            new_last_modified: None,
        };
        assert_ne!(result, StalenessResult::Fresh);
        if let StalenessResult::Stale {
            new_etag,
            new_last_modified,
        } = result
        {
            assert_eq!(new_etag.as_deref(), Some("\"v2\""));
            assert!(new_last_modified.is_none());
        }
    }
}
