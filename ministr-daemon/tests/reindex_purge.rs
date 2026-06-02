//! gd3a — daemon-side reindex + delete-with-purge endpoints.
//!
//! Verifies that the daemon (which owns the corpus data directory) handles
//! the on-disk purge, so the desktop GUI can drive reindex / remove as pure
//! `DaemonClient` calls without ever touching `~/.ministr/corpora`.

mod common;

use common::TestDaemon;

/// Poll for a path to (dis)appear, tolerating the brief async window between
/// a registry mutation returning and its filesystem effect landing.
async fn wait_until(path: &std::path::Path, want_exists: bool) -> bool {
    for _ in 0..40 {
        if path.exists() == want_exists {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    path.exists() == want_exists
}

#[tokio::test]
async fn reindex_purges_and_rebuilds_then_delete_purge_removes() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    // Register a real corpus from a temp directory with one indexable file.
    let src = tempfile::TempDir::new().unwrap();
    std::fs::write(
        src.path().join("readme.md"),
        "# Title\n\nHello world content for indexing.\n",
    )
    .unwrap();
    let src_path = src.path().to_string_lossy().into_owned();

    let resp = client.register_corpus(&[src_path]).await.unwrap();
    let id = resp.corpus_id.clone();

    // Corpora live under <data_dir>/corpora/<corpus_id>.
    let data_dir = daemon.data_dir().join("corpora").join(&id);
    assert!(
        wait_until(&data_dir, true).await,
        "corpus data dir should exist after register"
    );

    // A stale artifact in the index dir must NOT survive a reindex (rebuild).
    let sentinel = data_dir.join("STALE_SENTINEL");
    std::fs::write(&sentinel, "stale").unwrap();

    // Reindex: returns the same id, the corpus stays registered, and the
    // on-disk index is purged + rebuilt (sentinel gone, dir back).
    let re = client.reindex_corpus(&id).await.unwrap();
    assert_eq!(re.corpus_id, id, "reindex preserves the corpus id");
    assert!(
        wait_until(&data_dir, true).await,
        "data dir is rebuilt after reindex"
    );
    assert!(
        !sentinel.exists(),
        "reindex must purge the stale on-disk index"
    );
    assert!(
        client
            .list_corpora()
            .await
            .unwrap()
            .iter()
            .any(|c| c.id == id),
        "corpus is still registered after reindex"
    );

    // delete-with-purge removes both the registration AND the data dir.
    client.unregister_corpus_purge(&id).await.unwrap();
    assert!(
        wait_until(&data_dir, false).await,
        "purge-delete must remove the data dir"
    );
    let corpora = client.list_corpora().await.unwrap();
    assert!(
        !corpora.iter().any(|c| c.id == id),
        "corpus is unregistered after purge-delete"
    );
    // The pre-existing fixture corpus is untouched by our corpus's lifecycle.
    assert!(
        corpora.iter().any(|c| c.id == daemon.corpus_id),
        "the fixture corpus survives an unrelated reindex/purge"
    );
}

#[tokio::test]
async fn plain_delete_leaves_the_data_dir() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let src = tempfile::TempDir::new().unwrap();
    std::fs::write(src.path().join("readme.md"), "# Doc\n\nbody\n").unwrap();
    let resp = client
        .register_corpus(&[src.path().to_string_lossy().into_owned()])
        .await
        .unwrap();
    let id = resp.corpus_id.clone();

    let data_dir = daemon.data_dir().join("corpora").join(&id);
    assert!(wait_until(&data_dir, true).await);

    // Default DELETE (no purge) unregisters but leaves the index on disk —
    // preserving the cloud/test behavior.
    client.unregister_corpus(&id).await.unwrap();
    assert!(
        data_dir.exists(),
        "plain unregister must not delete the data dir"
    );
}
