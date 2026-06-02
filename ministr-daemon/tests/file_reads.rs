//! gd2c-files-2 — daemon file-content + occurrences read endpoints.
//!
//! Registers a real corpus, lets ingestion index a source file, then exercises
//! `DaemonClient::read_file_content` (full contents + symbol spans) and
//! `DaemonClient::file_occurrences` over UDS.

mod common;

use common::TestDaemon;

#[tokio::test]
async fn read_file_and_occurrences_over_uds() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let src = tempfile::TempDir::new().unwrap();
    std::fs::write(
        src.path().join("readme.md"),
        "# Title\n\nHello world content for indexing.\n",
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

    // Wait for indexing to register the file, then use its exact stored path.
    let mut file_path = None;
    for _ in 0..60 {
        let files = client.list_corpus_files(&id).await.unwrap();
        if let Some(f) = files.iter().find(|f| f.path.ends_with("readme.md")) {
            file_path = Some(f.path.clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let file_path = file_path.expect("readme.md should be indexed");

    // read_file_content returns the full file text (+ any symbol spans).
    let content = client
        .read_file_content(&id, file_path.clone())
        .await
        .unwrap();
    assert!(
        content.content.contains("Hello world"),
        "file content should round-trip the indexed source text"
    );

    // file_occurrences is reachable and well-formed (empty unless occurrence
    // indexing was enabled at index time, which the test corpus does not do).
    let occ = client.file_occurrences(&id, file_path).await.unwrap();
    assert!(
        occ.is_empty(),
        "no occurrence index expected for the test corpus, got {} entries",
        occ.len()
    );
}
