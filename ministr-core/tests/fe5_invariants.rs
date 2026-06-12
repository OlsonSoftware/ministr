//! FE5 — index invariants: the corpus stat-merkle short-circuit.
//!
//! Scope note (honest coverage map): FE5's other two acceptance areas are
//! already covered elsewhere and are NOT duplicated here —
//!
//! - **Occurrence index** (extract byte/col spans + resolve to `symbol_id`):
//!   `code::occurrence` unit tests (`extracts_rust_identifier_occurrences_with_spans`,
//!   `unsupported_language_yields_no_occurrences`), `ingestion::occurrences`
//!   resolve unit tests, and `storage_integration::occurrences_round_trip`.
//! - **`EXTRACTOR_VERSION` / `RESOLVER_VERSION` round-trips** (a bump forces
//!   re-extract / re-resolve): `storage_integration::file_hash_extractor_version_round_trips`,
//!   `…::file_hash_resolver_version_round_trips`, and the
//!   `re_resolve_stale_files_*` heal tests.
//!
//! What was NOT tested — and is added here — is the **behavior** of the
//! corpus-root stat-merkle short-circuit in `IngestionPipeline`
//! (`pipeline.rs`): when a rooted corpus is re-ingested unchanged it must
//! skip without re-indexing, and when the stored `extractor_version` differs
//! from the current one it must fall through and re-extract (auto-heal). Only
//! the rooted ingest path (`ingest_directory_with_embeddings_rooted`) takes
//! this branch — the storage round-trip test alone never exercises the
//! decision.

mod langtest;

use std::path::Path;

use langtest::MockEmbedder;
use ministr_core::embedding::Embedder;
use ministr_core::index::HnswIndex;
use ministr_core::ingestion::{EXTRACTOR_VERSION, IngestionPipeline, IngestionStats};
use ministr_core::storage::SqliteStorage;
use ministr_core::storage::traits::{CorpusMerkleRecord, Storage};

/// Ingest `root` through the **rooted** pipeline path (so the corpus stat-merkle
/// short-circuit is in play) against a persistent `storage`. A fresh embedder +
/// index per call is fine — the merkle fingerprint lives in `storage`.
async fn ingest_rooted(root: &Path, storage: &SqliteStorage, root_id: &str) -> IngestionStats {
    let embedder = MockEmbedder::default();
    let index = HnswIndex::new(embedder.dimension(), 10_000).expect("hnsw index");
    let pipeline = IngestionPipeline::new();
    pipeline
        .ingest_directory_with_embeddings_rooted(
            root,
            storage,
            &embedder,
            &index,
            Some(root_id),
            None,
        )
        .await
        .expect("rooted ingest")
}

fn write_project() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("proj");
    std::fs::create_dir_all(&root).expect("create proj dir");
    std::fs::write(root.join("lib.rs"), "pub fn answer() -> i32 {\n    42\n}\n")
        .expect("write lib.rs");
    tmp
}

/// An unchanged rooted corpus re-ingests as a stat-merkle short-circuit: the
/// second run skips every file and indexes none.
#[tokio::test]
async fn stat_merkle_short_circuits_unchanged_corpus() {
    let tmp = write_project();
    let root = tmp.path().join("proj");
    let storage = SqliteStorage::open_in_memory().expect("storage");

    let first = ingest_rooted(&root, &storage, "corpus-a").await;
    assert!(
        first.files_indexed >= 1,
        "first ingest should index the source file, got {first:?}",
    );

    let second = ingest_rooted(&root, &storage, "corpus-a").await;
    assert_eq!(
        second.files_indexed, 0,
        "unchanged re-ingest must short-circuit and re-index nothing, got {second:?}",
    );
    assert!(second.files_discovered >= 1);
    assert_eq!(
        second.files_skipped, second.files_discovered,
        "short-circuit must mark all discovered files skipped, got {second:?}",
    );
}

/// When the stored `extractor_version` is below the current one (simulating an
/// extractor/grammar bump), an unchanged-on-disk corpus must NOT short-circuit:
/// the gate falls through to the full reindex path, which re-stamps the corpus
/// merkle back to the current `EXTRACTOR_VERSION`. Asserting the re-stamp
/// isolates the merkle gate from the per-file mtime fast-skip.
#[tokio::test]
async fn extractor_version_mismatch_defeats_short_circuit() {
    let tmp = write_project();
    let root = tmp.path().join("proj");
    let storage = SqliteStorage::open_in_memory().expect("storage");

    ingest_rooted(&root, &storage, "corpus-b").await;
    let rec = storage
        .get_corpus_merkle("corpus-b")
        .await
        .expect("query merkle")
        .expect("merkle stored after first ingest");
    assert_eq!(
        rec.extractor_version, EXTRACTOR_VERSION,
        "first ingest stamps the current extractor version",
    );

    // Simulate an extractor-version bump by downgrading the stored stamp while
    // keeping the same root_hash (files untouched on disk).
    storage
        .upsert_corpus_merkle(&CorpusMerkleRecord {
            extractor_version: 0,
            ..rec
        })
        .await
        .expect("downgrade merkle stamp");

    // Re-ingest unchanged: root_hash matches but version differs → must fall
    // through (re-extract) and re-stamp the version to current.
    let _ = ingest_rooted(&root, &storage, "corpus-b").await;

    let after = storage
        .get_corpus_merkle("corpus-b")
        .await
        .expect("query merkle")
        .expect("merkle stored after re-ingest");
    assert_eq!(
        after.extractor_version, EXTRACTOR_VERSION,
        "a version-differ re-ingest must re-extract and re-stamp the merkle to \
         the current version (not short-circuit), got {}",
        after.extractor_version,
    );
}

/// ingest-lazy-embedder-load: the pre-load no-op probe
/// (`paths_ingest_would_noop`) must agree with the multi-path fast-skip gate
/// — false on a fresh corpus, true after an unchanged ingest, false again the
/// moment a file's mtime moves. The probe runs WITHOUT an embedder or index,
/// which is the whole point: the CLI consults it before paying the
/// embedding-model load.
#[tokio::test]
async fn noop_probe_mirrors_the_multi_path_fast_skip() {
    let tmp = write_project();
    let root = tmp.path().join("proj");
    let paths = vec![root.clone()];
    let storage = SqliteStorage::open_in_memory().expect("storage");
    let probe = IngestionPipeline::new();

    assert!(
        !probe
            .paths_ingest_would_noop(&paths, &storage)
            .await
            .expect("probe on fresh storage"),
        "a never-ingested corpus must not probe as a no-op",
    );

    // Real multi-path ingest (the entry the CLI uses).
    let embedder = MockEmbedder::default();
    let index = HnswIndex::new(embedder.dimension(), 10_000).expect("hnsw index");
    let pipeline = IngestionPipeline::new();
    let stats = pipeline
        .ingest_paths_with_embeddings(&paths, &storage, &embedder, &index)
        .await
        .expect("multi-path ingest");
    assert!(
        stats.files_indexed >= 1,
        "first ingest indexes, got {stats:?}"
    );

    assert!(
        probe
            .paths_ingest_would_noop(&paths, &storage)
            .await
            .expect("probe after ingest"),
        "an unchanged corpus must probe as a no-op (this is the gate the CLI \
         consults before loading the embedding model)",
    );

    // Move one file's mtime into the future — content identical, but the
    // mtime gate (deliberately conservative) must fall through.
    let lib = root.join("lib.rs");
    let future = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
    let f = std::fs::OpenOptions::new()
        .append(true)
        .open(&lib)
        .expect("open lib.rs");
    f.set_modified(future).expect("set mtime");
    drop(f);

    assert!(
        !probe
            .paths_ingest_would_noop(&paths, &storage)
            .await
            .expect("probe after touch"),
        "a touched file must defeat the no-op probe exactly like the real gate",
    );
}
