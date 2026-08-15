//! Orphaned-index surface: enumeration, adoption, GC, and the
//! tombstoned manifest prune (daemon-orphan-index-adoption).
//!
//! An "orphan" is a directory under `{data_dir}/corpora/` that no
//! registered corpus accounts for — index data an older daemon build left
//! unreachable. The daemon exposes list/adopt/remove verbs so a console
//! can surface them; `restore()` must never drop a live manifest entry,
//! and every pruned dead entry must leave a tombstone.

mod common;

use common::TestDaemon;

/// Create a fake orphan dir with a `meta.toml` pointing at `source_dir`
/// and `payload_bytes` of index data. Returns the payload size written.
fn plant_orphan(data_dir: &std::path::Path, name: &str, source_dir: &str, payload_bytes: usize) {
    let dir = data_dir.join("corpora").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let meta = format!("name = \"{name}\"\nsource_dirs = [\"{source_dir}\"]\n");
    std::fs::write(dir.join("meta.toml"), meta).unwrap();
    // A neutral payload (not `content.db` — a zero-filled file is not a
    // valid SQLite database, and adoption re-opens the real one).
    std::fs::write(dir.join("payload.bin"), vec![0u8; payload_bytes]).unwrap();
}

#[tokio::test]
async fn orphans_are_listed_with_sizes_and_registered_dirs_are_not() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    // A registered corpus's dir must never appear as an orphan.
    let registered_dir = daemon.data_dir().join("corpora").join(&daemon.corpus_id);
    std::fs::create_dir_all(&registered_dir).unwrap();
    std::fs::write(registered_dir.join("content.db"), b"live").unwrap();

    // A dead-path orphan (the /tmp test-leftover class).
    plant_orphan(
        daemon.data_dir(),
        "orphan-dead",
        "/nonexistent/ministr-zzz",
        128,
    );

    let resp = client.list_orphan_indexes().await.unwrap();
    let names: Vec<&str> = resp.orphans.iter().map(|o| o.dir_name.as_str()).collect();
    assert!(names.contains(&"orphan-dead"), "orphan missing: {names:?}");
    assert!(
        !names.contains(&daemon.corpus_id.as_str()),
        "registered dir listed as orphan"
    );

    let dead = resp
        .orphans
        .iter()
        .find(|o| o.dir_name == "orphan-dead")
        .unwrap();
    assert!(!dead.adoptable, "dead-path orphan must not be adoptable");
    assert!(dead.size_bytes >= 128, "size must count the payload");
    assert_eq!(dead.paths, vec!["/nonexistent/ministr-zzz".to_string()]);
    assert!(
        resp.total_bytes >= dead.size_bytes,
        "total must cover every orphan"
    );
}

#[tokio::test]
async fn removing_an_orphan_reclaims_bytes_and_deletes_the_dir() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    plant_orphan(
        daemon.data_dir(),
        "orphan-gc",
        "/nonexistent/ministr-gc",
        256,
    );
    let dir = daemon.data_dir().join("corpora").join("orphan-gc");
    assert!(dir.is_dir());

    let resp = client.remove_orphan_index("orphan-gc").await.unwrap();
    assert!(
        resp.bytes_reclaimed >= 256,
        "reclaimed {}",
        resp.bytes_reclaimed
    );
    assert!(!dir.exists(), "orphan dir must be gone after remove");
}

#[tokio::test]
async fn removing_a_registered_corpus_dir_is_refused() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    let registered_dir = daemon.data_dir().join("corpora").join(&daemon.corpus_id);
    std::fs::create_dir_all(&registered_dir).unwrap();
    std::fs::write(registered_dir.join("content.db"), b"live").unwrap();

    let err = client.remove_orphan_index(&daemon.corpus_id).await;
    assert!(err.is_err(), "deleting a registered corpus dir must fail");
    assert!(
        registered_dir.exists(),
        "registered dir must survive the refused delete"
    );
}

#[tokio::test]
async fn path_traversal_in_orphan_name_is_refused() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();
    // An encoded traversal must be rejected by name validation, never
    // resolved against the filesystem.
    let err = client.remove_orphan_index("..%2F..%2Fetc").await;
    assert!(err.is_err());
}

#[tokio::test]
async fn adopting_a_live_orphan_registers_it_under_the_canonical_id() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();

    // A live source tree the orphan's meta.toml points at.
    let source = tempfile::TempDir::new().unwrap();
    std::fs::write(source.path().join("readme.md"), "# adopted\n\nhello").unwrap();
    let source_str = source.path().display().to_string();

    plant_orphan(daemon.data_dir(), "orphan-live", &source_str, 64);

    let resp = client.adopt_orphan_index("orphan-live").await.unwrap();
    assert!(!resp.corpus_id.is_empty());

    // The data moved under the canonical id; the old name is gone.
    let corpora = daemon.data_dir().join("corpora");
    assert!(
        !corpora.join("orphan-live").exists(),
        "old dir must be renamed"
    );
    assert!(
        corpora.join(&resp.corpus_id).is_dir(),
        "canonical dir must exist"
    );

    // And the corpus is now registered (visible to list_corpora).
    let listed = client.list_corpora().await.unwrap();
    assert!(
        listed.iter().any(|c| c.id == resp.corpus_id),
        "adopted corpus must be listed"
    );

    // No longer an orphan.
    let orphans = client.list_orphan_indexes().await.unwrap();
    assert!(
        !orphans.orphans.iter().any(|o| o.dir_name == resp.corpus_id),
        "adopted dir must stop being an orphan"
    );
}

#[tokio::test]
async fn adopting_a_dead_orphan_is_refused() {
    let daemon = TestDaemon::start().await;
    let client = daemon.client();
    plant_orphan(
        daemon.data_dir(),
        "orphan-gone",
        "/nonexistent/ministr-gone",
        32,
    );
    let err = client.adopt_orphan_index("orphan-gone").await;
    assert!(err.is_err(), "dead-path orphan must not adopt");
    // The data is untouched — refusal must not destroy anything.
    assert!(daemon.data_dir().join("corpora/orphan-gone").is_dir());
}
