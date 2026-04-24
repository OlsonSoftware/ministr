//! Regression guards for bugs found in the coherence engine trace.

use ministr_core::coherence::{CoherenceEngine, CoherenceEvent};
use ministr_core::ingestion::IngestionPipeline;
use ministr_core::storage::{SqliteStorage, Storage};
use ministr_core::types::ContentId;

/// CO1 regression — `reindex_file` must only re-index the file it was
/// given, not the whole corpus directory. Previously it called
/// `ingest_directory` / `ingest_directory_with_embeddings`, which did
/// a full tree walk on every event — that rescan silently picked up
/// unrelated changes on disk (new files the caller didn't know about).
#[tokio::test]
async fn co1_reindex_file_does_not_rescan_the_whole_corpus() {
    let dir = tempfile::TempDir::new().unwrap();
    let corpus = dir.path().to_path_buf();

    // Only A exists when we first ingest.
    let file_a = corpus.join("a.md");
    std::fs::write(
        &file_a,
        "# Doc A\n\n## Section\n\nOriginal content for A.\n",
    )
    .unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let pipeline = IngestionPipeline::new();
    pipeline.ingest_directory(&corpus, &storage).await.unwrap();

    // Drop B on disk WITHOUT telling the coherence engine about it.
    let file_b = corpus.join("b.md");
    std::fs::write(
        &file_b,
        "# Doc B\n\n## Section\n\nB content — must NOT be auto-ingested.\n",
    )
    .unwrap();

    // Modify A so reindex has something real to do.
    std::fs::write(&file_a, "# Doc A\n\n## Section\n\nUpdated content for A.\n").unwrap();

    let engine = CoherenceEngine::new(corpus.clone());
    let events = vec![CoherenceEvent::Modified(file_a.clone())];
    let _affected = engine.process_events(&events, &storage).await.unwrap();

    let docs_after: Vec<String> = storage
        .list_documents()
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.id.0)
        .collect();

    assert!(
        !docs_after.iter().any(|id| id.contains("b.md")),
        "reindex_file(A) must not ingest B; got docs: {docs_after:?}"
    );
    assert!(
        docs_after.iter().any(|id| id.contains("a.md")),
        "A must still be present after its own reindex"
    );
}

/// CO2 regression — with last-event-wins coalescing, a
/// `[Modified(X), Removed(X)]` sequence processes the Remove, so
/// storage correctly drops X even though there was a stale Modified
/// in the same batch. Previously first-event-wins dedup kept the
/// Modified and silently discarded the Remove.
#[tokio::test]
async fn co2_modify_then_remove_in_same_batch_deletes_the_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let corpus = dir.path().to_path_buf();
    let file_x = corpus.join("x.md");
    std::fs::write(&file_x, "# X\n\n## S\n\nInitial.\n").unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let pipeline = IngestionPipeline::new();
    pipeline.ingest_directory(&corpus, &storage).await.unwrap();
    assert!(!storage.list_documents().await.unwrap().is_empty());

    // File deleted on disk.
    std::fs::remove_file(&file_x).unwrap();

    // Events arrive in this order (editor saves, then user deletes).
    let events = vec![
        CoherenceEvent::Modified(file_x.clone()),
        CoherenceEvent::Removed(file_x.clone()),
    ];

    let engine = CoherenceEngine::new(corpus.clone());
    let _ = engine.process_events(&events, &storage).await.unwrap();

    let x_id = ContentId("x.md".into());
    let sections = storage.list_sections(&x_id).await.unwrap_or_default();
    assert!(
        sections.is_empty(),
        "[Modified, Removed] must delete X from storage; still have {} sections",
        sections.len()
    );
}

/// CO2 regression — the reverse order `[Removed(Y), Modified(Y)]` wins
/// with Modified (save-replace editor pattern: file gets replaced on
/// disk after being briefly removed). The previous first-event-wins
/// dedup handled this direction correctly already; last-event-wins
/// preserves that correctness.
#[tokio::test]
async fn co2_remove_then_modify_reingests_the_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let corpus = dir.path().to_path_buf();
    let file_y = corpus.join("y.md");
    std::fs::write(&file_y, "# Y\n\n## S\n\nOriginal.\n").unwrap();

    let storage = SqliteStorage::open_in_memory().unwrap();
    let pipeline = IngestionPipeline::new();
    pipeline.ingest_directory(&corpus, &storage).await.unwrap();

    // Replace on disk — typical save-replace: delete-then-create.
    std::fs::write(&file_y, "# Y\n\n## S\n\nReplaced content.\n").unwrap();

    let events = vec![
        CoherenceEvent::Removed(file_y.clone()),
        CoherenceEvent::Modified(file_y.clone()),
    ];

    let engine = CoherenceEngine::new(corpus.clone());
    let _ = engine.process_events(&events, &storage).await.unwrap();

    // Y must still be present — the Modified event wins and re-ingests.
    let y_id = ContentId("y.md".into());
    let sections = storage.list_sections(&y_id).await.unwrap_or_default();
    assert!(
        !sections.is_empty(),
        "[Removed, Modified] with Modified last must re-ingest Y"
    );
}
