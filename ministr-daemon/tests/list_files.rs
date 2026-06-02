//! gd2c-files-1 — daemon `GET /api/v1/corpora/{id}/files` endpoint.
//!
//! Verifies the file-tree read the desktop code browser uses now works over
//! UDS: register a real corpus, let ingestion write the file-hash + sections,
//! and assert `DaemonClient::list_corpus_files` returns the indexed file with a
//! section count.

mod common;

use common::TestDaemon;

#[tokio::test]
async fn list_corpus_files_returns_indexed_files() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    // Register a real corpus from a temp dir with one indexable markdown file.
    let src = tempfile::TempDir::new().unwrap();
    std::fs::write(
        src.path().join("readme.md"),
        "# Title\n\nHello world content for indexing.\n\n## Section Two\n\nMore body text.\n",
    )
    .unwrap();
    let resp = client
        .register_corpus(&[src.path().to_string_lossy().into_owned()])
        .await
        .unwrap();
    let id = resp.corpus_id;
    assert_ne!(
        id, daemon.corpus_id,
        "registering a new source dir must yield a distinct corpus from the fixture"
    );

    // Ingestion is async; poll until the file-hash row lands.
    let mut files = Vec::new();
    for _ in 0..60 {
        files = client.list_corpus_files(&id).await.unwrap();
        if files.iter().any(|f| f.path.ends_with("readme.md")) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    let readme = files
        .iter()
        .find(|f| f.path.ends_with("readme.md"))
        .expect("readme.md should be listed after indexing");
    assert!(
        !readme.content_hash.is_empty(),
        "indexed file should carry a content hash"
    );
    assert!(
        readme.section_count >= 1,
        "a markdown file with headings should report at least one section, got {}",
        readme.section_count
    );
}
