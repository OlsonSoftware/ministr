//! Integration tests for the web fetch pipeline.
//!
//! Tests the end-to-end flow: raw markdown content → ingestion pipeline →
//! storage (sections, claims searchable). Also tests the web cache and
//! the `ingest_content` method.

use iris_core::ingestion::IngestionPipeline;
use iris_core::parser::ParserKind;
use iris_core::storage::{SqliteStorage, Storage};
use iris_core::web::cache::{WebCache, WebPageMeta, url_hash};
use iris_core::web::fetcher::web_source_path;

/// Sample markdown content simulating a fetched web page.
const SAMPLE_MARKDOWN: &str = "\
# Getting Started with Iris

Iris is a context cache controller for LLM agents.

## Installation

Install iris using cargo:

```bash
cargo install iris
```

The binary is available on crates.io.

## Configuration

Create a configuration file at `~/.iris/config.toml`:

- `data_dir` — root data directory
- `default_model` — embedding model name
- `corpus_paths` — list of directories to index

## Quick Start

Run iris with a corpus directory:

```bash
iris serve --corpus ./docs
```

The MCP server starts on stdio by default.
";

#[tokio::test]
async fn ingest_content_creates_document_and_sections() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let pipeline = IngestionPipeline::new();

    let source_path = "web://example.com/docs/getting-started";
    let stats = pipeline
        .ingest_content(source_path, SAMPLE_MARKDOWN, ParserKind::Markdown, &storage)
        .await
        .unwrap();

    assert!(!stats.skipped);
    assert!(stats.sections > 0, "should extract sections");
    assert!(stats.claims > 0, "should extract claims");

    // Verify document was stored
    let docs = storage.list_documents().await.unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].source_path, source_path);
    assert!(docs[0].summary.is_some());

    // Verify sections were stored
    let sections = storage.list_sections(&docs[0].id).await.unwrap();
    assert!(
        sections.len() >= 3,
        "should have at least 3 sections (Installation, Configuration, Quick Start)"
    );

    // Verify claims were stored on at least one section
    let mut total_claims = 0;
    for section in &sections {
        let claims = storage.list_claims(&section.id).await.unwrap();
        total_claims += claims.len();
    }
    assert!(total_claims > 0, "should have claims in storage");
}

#[tokio::test]
async fn ingest_content_skips_unchanged() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let pipeline = IngestionPipeline::new();

    let source_path = "web://example.com/docs/guide";
    let content = "# Guide\n\nThis is a guide with enough content to generate claims. It covers installation, configuration, and basic usage of the tool.\n";

    // First ingestion
    let stats1 = pipeline
        .ingest_content(source_path, content, ParserKind::Markdown, &storage)
        .await
        .unwrap();
    assert!(!stats1.skipped);

    // Second ingestion with same content — should skip
    let stats2 = pipeline
        .ingest_content(source_path, content, ParserKind::Markdown, &storage)
        .await
        .unwrap();
    assert!(stats2.skipped);
}

#[tokio::test]
async fn ingest_content_reindexes_changed() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let pipeline = IngestionPipeline::new();

    let source_path = "web://example.com/docs/api";

    let content_v1 = "# API v1\n\n## Endpoints\n\nGET /users returns a list of users. POST /users creates a new user.\n";
    let stats1 = pipeline
        .ingest_content(source_path, content_v1, ParserKind::Markdown, &storage)
        .await
        .unwrap();
    assert!(!stats1.skipped);

    let content_v2 = "# API v2\n\n## Endpoints\n\nGET /users returns a paginated list of users. POST /users creates a new user. DELETE /users/:id removes a user.\n";
    let stats2 = pipeline
        .ingest_content(source_path, content_v2, ParserKind::Markdown, &storage)
        .await
        .unwrap();
    assert!(!stats2.skipped);

    // Should still have exactly one document
    let docs = storage.list_documents().await.unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].title, "API v2");
}

#[tokio::test]
async fn web_cache_store_and_retrieve() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = WebCache::new(tmp.path());

    let url = "https://docs.example.com/guide";
    let markdown = SAMPLE_MARKDOWN;
    let meta = WebPageMeta {
        source_url: url.into(),
        fetched_at: "1711036800".into(),
        etag: Some("\"v1\"".into()),
        content_hash: url_hash(markdown),
        content_type: Some("text/html".into()),
    };

    // Store
    cache.store_page(url, markdown, &meta).await.unwrap();
    assert!(cache.has_page(url));

    // Retrieve
    let (loaded_md, loaded_meta) = cache.get_page(url).await.unwrap().unwrap();
    assert_eq!(loaded_md, markdown);
    assert_eq!(loaded_meta.source_url, url);
    assert_eq!(loaded_meta.etag.as_deref(), Some("\"v1\""));
}

#[tokio::test]
async fn web_source_path_generation() {
    assert_eq!(
        web_source_path("https://example.com/docs/guide"),
        "web://example.com/docs/guide"
    );
    assert_eq!(
        web_source_path("http://example.com/api"),
        "web://example.com/api"
    );
}

#[tokio::test]
async fn ingest_multiple_web_pages() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let pipeline = IngestionPipeline::new();

    let pages = vec![
        (
            "web://example.com/docs/intro",
            "# Introduction\n\nIris provides a multi-resolution index for LLM context management. It supports document summaries, section text, and atomic claims.\n",
        ),
        (
            "web://example.com/docs/api",
            "# API Reference\n\n## Survey\n\nThe iris_survey tool searches the corpus for relevant content. It returns ranked results with scores.\n\n## Read\n\nThe iris_read tool retrieves full section text by section ID.\n",
        ),
    ];

    for (source_path, content) in &pages {
        let stats = pipeline
            .ingest_content(source_path, content, ParserKind::Markdown, &storage)
            .await
            .unwrap();
        assert!(!stats.skipped);
    }

    let docs = storage.list_documents().await.unwrap();
    assert_eq!(docs.len(), 2);
}
