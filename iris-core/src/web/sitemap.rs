//! Sitemap XML parser and parallel page fetcher.
//!
//! Parses both `<urlset>` (flat sitemap) and `<sitemapindex>` (nested sitemap
//! index) XML formats, extracting `<loc>` URLs with optional `<lastmod>`
//! timestamps. Supports path prefix filtering, configurable concurrency, and
//! polite rate limiting for crawling.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Semaphore;
use tracing::{debug, instrument, warn};
use url::Url;

use crate::error::WebError;
use crate::parser::html_to_md::html_to_markdown;
use crate::web::HttpClient;
use crate::web::fetcher::FetchedPage;

/// A single entry extracted from a sitemap XML file.
///
/// # Examples
///
/// ```
/// use iris_core::web::sitemap::SitemapEntry;
///
/// let entry = SitemapEntry {
///     url: "https://example.com/docs/guide".into(),
///     lastmod: Some("2026-03-21".into()),
/// };
/// assert_eq!(entry.url, "https://example.com/docs/guide");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SitemapEntry {
    /// The URL from the `<loc>` element.
    pub url: String,
    /// Optional last modification date from the `<lastmod>` element.
    pub lastmod: Option<String>,
}

/// Configuration for sitemap crawling.
///
/// # Examples
///
/// ```
/// use iris_core::web::sitemap::SitemapConfig;
///
/// let config = SitemapConfig::default();
/// assert_eq!(config.max_pages, 50);
/// assert_eq!(config.concurrency, 4);
/// assert_eq!(config.rate_limit_ms, 200);
/// ```
#[derive(Debug, Clone)]
pub struct SitemapConfig {
    /// Only fetch URLs whose path starts with this prefix.
    pub path_filter: Option<String>,
    /// Maximum number of pages to fetch (default: 50).
    pub max_pages: usize,
    /// Maximum concurrent requests (default: 4).
    pub concurrency: usize,
    /// Minimum delay between request starts in milliseconds (default: 200).
    pub rate_limit_ms: u64,
}

impl Default for SitemapConfig {
    fn default() -> Self {
        Self {
            path_filter: None,
            max_pages: 50,
            concurrency: 4,
            rate_limit_ms: 200,
        }
    }
}

/// Parse a sitemap XML string, auto-detecting `<urlset>` vs `<sitemapindex>`.
///
/// For `<urlset>`, returns the URL entries directly.
/// For `<sitemapindex>`, returns the child sitemap URLs as entries (with the
/// sitemap URL in the `url` field).
///
/// # Errors
///
/// Returns [`WebError::SitemapParse`] if the XML is malformed or contains
/// neither `<urlset>` nor `<sitemapindex>`.
///
/// # Examples
///
/// ```
/// use iris_core::web::sitemap::parse_sitemap;
///
/// let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
/// <urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
///   <url><loc>https://example.com/</loc></url>
///   <url><loc>https://example.com/about</loc><lastmod>2026-01-15</lastmod></url>
/// </urlset>"#;
///
/// let entries = parse_sitemap(xml).unwrap();
/// assert_eq!(entries.len(), 2);
/// assert_eq!(entries[0].url, "https://example.com/");
/// assert_eq!(entries[1].lastmod.as_deref(), Some("2026-01-15"));
/// ```
pub fn parse_sitemap(xml: &str) -> Result<Vec<SitemapEntry>, WebError> {
    let trimmed = xml.trim();

    if contains_element(trimmed, "sitemapindex") {
        parse_sitemap_index(trimmed)
    } else if contains_element(trimmed, "urlset") {
        parse_urlset(trimmed)
    } else {
        Err(WebError::SitemapParse {
            reason: "XML contains neither <urlset> nor <sitemapindex>".into(),
        })
    }
}

/// Parse a `<urlset>` sitemap XML, extracting URL entries.
///
/// # Errors
///
/// Returns [`WebError::SitemapParse`] on malformed XML.
pub fn parse_urlset(xml: &str) -> Result<Vec<SitemapEntry>, WebError> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);

    let mut entries = Vec::new();
    let mut in_url = false;
    let mut in_loc = false;
    let mut in_lastmod = false;
    let mut current_loc = String::new();
    let mut current_lastmod: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                let name = e.name();
                let name_bytes = name.as_ref();
                let local = local_name(name_bytes);
                match local {
                    b"url" => {
                        in_url = true;
                        current_loc.clear();
                        current_lastmod = None;
                    }
                    b"loc" if in_url => in_loc = true,
                    b"lastmod" if in_url => in_lastmod = true,
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let name = e.name();
                let name_bytes = name.as_ref();
                let local = local_name(name_bytes);
                match local {
                    b"url" => {
                        if in_url && !current_loc.is_empty() {
                            entries.push(SitemapEntry {
                                url: current_loc.trim().to_owned(),
                                lastmod: current_lastmod.take(),
                            });
                        }
                        in_url = false;
                    }
                    b"loc" => in_loc = false,
                    b"lastmod" => in_lastmod = false,
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                if in_loc {
                    let text = e.unescape().map_err(|err| WebError::SitemapParse {
                        reason: format!("XML text decode error: {err}"),
                    })?;
                    current_loc.push_str(&text);
                } else if in_lastmod {
                    let text = e.unescape().map_err(|err| WebError::SitemapParse {
                        reason: format!("XML text decode error: {err}"),
                    })?;
                    current_lastmod = Some(text.trim().to_owned());
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(WebError::SitemapParse {
                    reason: format!("XML parse error: {e}"),
                });
            }
            _ => {}
        }
    }

    Ok(entries)
}

/// Parse a `<sitemapindex>` XML, extracting child sitemap URLs as entries.
///
/// # Errors
///
/// Returns [`WebError::SitemapParse`] on malformed XML.
pub fn parse_sitemap_index(xml: &str) -> Result<Vec<SitemapEntry>, WebError> {
    use quick_xml::Reader;
    use quick_xml::events::Event;

    let mut reader = Reader::from_str(xml);

    let mut entries = Vec::new();
    let mut in_sitemap = false;
    let mut in_loc = false;
    let mut in_lastmod = false;
    let mut current_loc = String::new();
    let mut current_lastmod: Option<String> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                let name = e.name();
                let name_bytes = name.as_ref();
                let local = local_name(name_bytes);
                match local {
                    b"sitemap" => {
                        in_sitemap = true;
                        current_loc.clear();
                        current_lastmod = None;
                    }
                    b"loc" if in_sitemap => in_loc = true,
                    b"lastmod" if in_sitemap => in_lastmod = true,
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let name = e.name();
                let name_bytes = name.as_ref();
                let local = local_name(name_bytes);
                match local {
                    b"sitemap" => {
                        if in_sitemap && !current_loc.is_empty() {
                            entries.push(SitemapEntry {
                                url: current_loc.trim().to_owned(),
                                lastmod: current_lastmod.take(),
                            });
                        }
                        in_sitemap = false;
                    }
                    b"loc" => in_loc = false,
                    b"lastmod" => in_lastmod = false,
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                if in_loc {
                    let text = e.unescape().map_err(|err| WebError::SitemapParse {
                        reason: format!("XML text decode error: {err}"),
                    })?;
                    current_loc.push_str(&text);
                } else if in_lastmod {
                    let text = e.unescape().map_err(|err| WebError::SitemapParse {
                        reason: format!("XML text decode error: {err}"),
                    })?;
                    current_lastmod = Some(text.trim().to_owned());
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(WebError::SitemapParse {
                    reason: format!("XML parse error: {e}"),
                });
            }
            _ => {}
        }
    }

    Ok(entries)
}

/// Filter sitemap entries by path prefix and apply the max page limit.
///
/// # Examples
///
/// ```
/// use iris_core::web::sitemap::{SitemapEntry, SitemapConfig, filter_entries};
///
/// let entries = vec![
///     SitemapEntry { url: "https://example.com/docs/guide".into(), lastmod: None },
///     SitemapEntry { url: "https://example.com/blog/post".into(), lastmod: None },
///     SitemapEntry { url: "https://example.com/docs/api".into(), lastmod: None },
/// ];
/// let config = SitemapConfig {
///     path_filter: Some("/docs/".into()),
///     max_pages: 50,
///     ..SitemapConfig::default()
/// };
/// let filtered = filter_entries(&entries, &config);
/// assert_eq!(filtered.len(), 2);
/// assert!(filtered.iter().all(|e| e.url.contains("/docs/")));
/// ```
#[must_use]
pub fn filter_entries(entries: &[SitemapEntry], config: &SitemapConfig) -> Vec<SitemapEntry> {
    let mut result: Vec<SitemapEntry> = entries
        .iter()
        .filter(|entry| {
            if let Some(ref prefix) = config.path_filter {
                if let Ok(parsed) = Url::parse(&entry.url) {
                    parsed.path().starts_with(prefix.as_str())
                } else {
                    false
                }
            } else {
                true
            }
        })
        .cloned()
        .collect();

    result.truncate(config.max_pages);
    result
}

/// Fetch pages from sitemap entries in parallel with concurrency and rate limiting.
///
/// Uses a semaphore to limit concurrent requests and a delay between request
/// starts for polite crawling.
///
/// Pages that fail to fetch are logged and skipped — the result contains only
/// successfully fetched pages.
///
/// # Errors
///
/// This function does not return errors — individual page failures are logged
/// and the page is skipped.
///
/// # Panics
///
/// Panics if the internal semaphore is unexpectedly closed (should never happen).
#[instrument(skip(client, entries, config), fields(entries = entries.len()))]
pub async fn fetch_sitemap_pages(
    client: &HttpClient,
    entries: &[SitemapEntry],
    config: &SitemapConfig,
) -> Vec<FetchedPage> {
    let semaphore = Arc::new(Semaphore::new(config.concurrency));
    let client = client.clone();
    let rate_limit = Duration::from_millis(config.rate_limit_ms);

    let mut handles = Vec::with_capacity(entries.len());

    for entry in entries {
        // Rate limiting: wait before spawning each request
        if !handles.is_empty() {
            tokio::time::sleep(rate_limit).await;
        }

        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore should not be closed");

        let client = client.clone();
        let url = entry.url.clone();

        handles.push(tokio::spawn(async move {
            let result = fetch_page(&client, &url).await;
            drop(permit);
            result
        }));
    }

    let mut pages = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(Ok(page)) => pages.push(page),
            Ok(Err(e)) => {
                warn!(error = %e, "failed to fetch sitemap page");
            }
            Err(e) => {
                warn!(error = %e, "sitemap fetch task panicked");
            }
        }
    }

    pages
}

/// Fetch a single page and convert HTML to markdown.
async fn fetch_page(client: &HttpClient, url: &str) -> Result<FetchedPage, WebError> {
    let response = client.get(url).await?;
    let content_type = response
        .headers
        .get("content-type")
        .cloned()
        .unwrap_or_default();

    let markdown = if content_type.contains("text/html") {
        html_to_markdown(&response.body)
    } else {
        response.body
    };

    debug!(url = %url, bytes = markdown.len(), "fetched sitemap page");

    Ok(FetchedPage {
        url: url.to_owned(),
        markdown,
    })
}

/// Check if XML contains an element with the given local name, accounting for
/// optional namespace prefixes (e.g. `<sm:urlset>` matches `"urlset"`).
fn contains_element(xml: &str, element: &str) -> bool {
    // Match `<element` or `<prefix:element`
    xml.contains(&format!("<{element}")) || xml.contains(&format!(":{element}"))
}

/// Extract the local name from a potentially namespace-prefixed XML element name.
///
/// For `ns:element`, returns `element`. For `element`, returns `element`.
fn local_name(name: &[u8]) -> &[u8] {
    match name.iter().position(|&b| b == b':') {
        Some(pos) => &name[pos + 1..],
        None => name,
    }
}

/// Check if a URL looks like a sitemap URL.
///
/// Returns `true` if the URL path ends with `sitemap.xml`, `sitemap.xml.gz`,
/// or similar sitemap patterns.
///
/// # Examples
///
/// ```
/// use iris_core::web::sitemap::is_sitemap_url;
///
/// assert!(is_sitemap_url("https://example.com/sitemap.xml"));
/// assert!(is_sitemap_url("https://example.com/sitemap_index.xml"));
/// assert!(!is_sitemap_url("https://example.com/docs/guide"));
/// ```
#[must_use]
pub fn is_sitemap_url(url: &str) -> bool {
    if let Ok(parsed) = Url::parse(url) {
        let path = parsed.path().to_lowercase();
        let has_xml_ext = std::path::Path::new(&path)
            .extension()
            .is_some_and(|ext| ext == "xml" || ext == "gz");
        has_xml_ext && path.contains("sitemap")
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_urlset tests --

    #[test]
    fn parse_simple_urlset() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url>
    <loc>https://example.com/</loc>
  </url>
  <url>
    <loc>https://example.com/about</loc>
    <lastmod>2026-01-15</lastmod>
  </url>
  <url>
    <loc>https://example.com/docs/guide</loc>
    <lastmod>2026-03-21T10:00:00+00:00</lastmod>
    <changefreq>weekly</changefreq>
    <priority>0.8</priority>
  </url>
</urlset>"#;

        let entries = parse_sitemap(xml).unwrap();
        assert_eq!(entries.len(), 3);

        assert_eq!(entries[0].url, "https://example.com/");
        assert!(entries[0].lastmod.is_none());

        assert_eq!(entries[1].url, "https://example.com/about");
        assert_eq!(entries[1].lastmod.as_deref(), Some("2026-01-15"));

        assert_eq!(entries[2].url, "https://example.com/docs/guide");
        assert_eq!(
            entries[2].lastmod.as_deref(),
            Some("2026-03-21T10:00:00+00:00")
        );
    }

    #[test]
    fn parse_urlset_with_namespace_prefix() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<sm:urlset xmlns:sm="http://www.sitemaps.org/schemas/sitemap/0.9">
  <sm:url>
    <sm:loc>https://example.com/page1</sm:loc>
    <sm:lastmod>2026-02-01</sm:lastmod>
  </sm:url>
</sm:urlset>"#;

        let entries = parse_sitemap(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, "https://example.com/page1");
        assert_eq!(entries[0].lastmod.as_deref(), Some("2026-02-01"));
    }

    // -- parse_sitemap_index tests --

    #[test]
    fn parse_simple_sitemap_index() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <sitemap>
    <loc>https://example.com/sitemap1.xml</loc>
    <lastmod>2026-01-01</lastmod>
  </sitemap>
  <sitemap>
    <loc>https://example.com/sitemap2.xml</loc>
    <lastmod>2026-02-01</lastmod>
  </sitemap>
</sitemapindex>"#;

        let entries = parse_sitemap(xml).unwrap();
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].url, "https://example.com/sitemap1.xml");
        assert_eq!(entries[0].lastmod.as_deref(), Some("2026-01-01"));

        assert_eq!(entries[1].url, "https://example.com/sitemap2.xml");
        assert_eq!(entries[1].lastmod.as_deref(), Some("2026-02-01"));
    }

    #[test]
    fn parse_sitemap_index_without_lastmod() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <sitemap>
    <loc>https://example.com/sitemap-docs.xml</loc>
  </sitemap>
</sitemapindex>"#;

        let entries = parse_sitemap(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, "https://example.com/sitemap-docs.xml");
        assert!(entries[0].lastmod.is_none());
    }

    // -- filter_entries tests --

    #[test]
    fn filter_by_path_prefix() {
        let entries = vec![
            SitemapEntry {
                url: "https://example.com/docs/guide".into(),
                lastmod: None,
            },
            SitemapEntry {
                url: "https://example.com/blog/post1".into(),
                lastmod: None,
            },
            SitemapEntry {
                url: "https://example.com/docs/api".into(),
                lastmod: None,
            },
            SitemapEntry {
                url: "https://example.com/about".into(),
                lastmod: None,
            },
        ];

        let config = SitemapConfig {
            path_filter: Some("/docs/".into()),
            max_pages: 50,
            ..SitemapConfig::default()
        };

        let filtered = filter_entries(&entries, &config);
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].url, "https://example.com/docs/guide");
        assert_eq!(filtered[1].url, "https://example.com/docs/api");
    }

    #[test]
    fn filter_respects_max_pages() {
        let entries: Vec<SitemapEntry> = (0..10)
            .map(|i| SitemapEntry {
                url: format!("https://example.com/page{i}"),
                lastmod: None,
            })
            .collect();

        let config = SitemapConfig {
            max_pages: 3,
            ..SitemapConfig::default()
        };

        let filtered = filter_entries(&entries, &config);
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn filter_no_filter_returns_all_up_to_max() {
        let entries = vec![
            SitemapEntry {
                url: "https://example.com/a".into(),
                lastmod: None,
            },
            SitemapEntry {
                url: "https://example.com/b".into(),
                lastmod: None,
            },
        ];

        let config = SitemapConfig::default();
        let filtered = filter_entries(&entries, &config);
        assert_eq!(filtered.len(), 2);
    }

    // -- parse error tests --

    #[test]
    fn parse_invalid_xml_returns_error() {
        let xml = "this is not xml at all";
        let result = parse_sitemap(xml);
        assert!(result.is_err());
    }

    #[test]
    fn parse_empty_urlset() {
        let xml = r#"<?xml version="1.0"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
</urlset>"#;

        let entries = parse_sitemap(xml).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_urlset_skips_empty_loc() {
        let xml = r#"<?xml version="1.0"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url><loc></loc></url>
  <url><loc>https://example.com/valid</loc></url>
</urlset>"#;

        let entries = parse_sitemap(xml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, "https://example.com/valid");
    }

    // -- is_sitemap_url tests --

    #[test]
    fn sitemap_url_detection() {
        assert!(is_sitemap_url("https://example.com/sitemap.xml"));
        assert!(is_sitemap_url("https://example.com/sitemap_index.xml"));
        assert!(is_sitemap_url("https://example.com/docs/sitemap-docs.xml"));
        assert!(!is_sitemap_url("https://example.com/docs/guide"));
        assert!(!is_sitemap_url("https://example.com/"));
    }

    // -- SitemapConfig default tests --

    #[test]
    fn sitemap_config_defaults() {
        let config = SitemapConfig::default();
        assert_eq!(config.max_pages, 50);
        assert_eq!(config.concurrency, 4);
        assert_eq!(config.rate_limit_ms, 200);
        assert!(config.path_filter.is_none());
    }

    // -- lastmod parsing tests --

    #[test]
    fn parse_various_lastmod_formats() {
        let xml = r#"<?xml version="1.0"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
  <url>
    <loc>https://example.com/a</loc>
    <lastmod>2026-03-21</lastmod>
  </url>
  <url>
    <loc>https://example.com/b</loc>
    <lastmod>2026-03-21T14:30:00+00:00</lastmod>
  </url>
  <url>
    <loc>https://example.com/c</loc>
    <lastmod>2026-03-21T14:30:00Z</lastmod>
  </url>
</urlset>"#;

        let entries = parse_sitemap(xml).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].lastmod.as_deref(), Some("2026-03-21"));
        assert_eq!(
            entries[1].lastmod.as_deref(),
            Some("2026-03-21T14:30:00+00:00")
        );
        assert_eq!(entries[2].lastmod.as_deref(), Some("2026-03-21T14:30:00Z"));
    }
}
