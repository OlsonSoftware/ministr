//! Reproduction probe for the degenerate-survey bug (f-ministr-corpus-survey-degenerate).
//!
//! Uses the REAL `CandleEmbedder` (all-MiniLM-L6-v2) on representative content
//! to isolate whether the embedder itself produces degenerate vectors, vs the
//! index/search layer. Run with:
//!
//! ```text
//! cargo test -p ministr-core --test embedder_repro --features candle --release -- --ignored --nocapture
//! ```

#![allow(clippy::cast_precision_loss)]

#[cfg(all(feature = "candle", target_os = "macos"))]
mod repro {
    use std::path::Path;

    use ministr_core::embedding::{CandleEmbedder, Embedder};

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }
    fn l2(a: &[f32]) -> f32 {
        a.iter().map(|x| x * x).sum::<f32>().sqrt()
    }

    const CHANGELOG_MD: &str = r"### Changed

- Onboarding rewritten as a 3-step wizard (Pick -> Index -> Connect) with a step indicator.
- SettingsSurface drops the M1 placeholder grid in favor of the real panel and threads
  setActiveCorpusId through (Explore + Query Playground both need the setter).
- The 10s tooltip refresh stays — corpora count, session breakdown, RSS remain visible on hover.
- refactor(tray): simplify menu to {Open, Add project, Quit}.
";

    const README_MD: &str = r"# ministr

It gives AI coding agents AST-level understanding of your codebase — semantic search across
code and docs, symbol-level navigation, real reference graphs, and cross-language bridge
detection across 40+ languages.
";

    const CODE_RS: &str = r"pub fn validate_token(token: &str) -> Result<Claims, AuthError> {
    let key = DecodingKey::from_secret(SECRET);
    decode::<Claims>(token, &key, &Validation::default()).map_err(AuthError::from)
}";

    #[test]
    #[ignore = "loads the MiniLM model; run explicitly with --ignored"]
    fn candle_embedder_is_not_degenerate() {
        let home = std::env::var("HOME").unwrap();
        let data_dir = Path::new(&home).join(".ministr");

        let embedder = CandleEmbedder::with_data_dir("all-MiniLM-L6-v2", &data_dir)
            .expect("load MiniLM model");

        let query = "how does authentication token validation work";

        // Embed everything in ONE batch (mixed lengths → exercises padding).
        let texts = vec![
            query,        // 0
            query,        // 1 (stability)
            CHANGELOG_MD, // 2
            README_MD,    // 3
            CODE_RS,      // 4
            "",           // 5 empty
            "   \n  ",    // 6 whitespace
        ];
        let vecs = embedder.embed(&texts).expect("embed batch");

        let labels = [
            "query", "query#2", "CHANGELOG", "README", "code", "empty", "whitespace",
        ];
        eprintln!("=== norms + cosine(query, x) ===");
        for (i, v) in vecs.iter().enumerate() {
            eprintln!(
                "  {:<10} |v|={:.4}  cos(query,·)={:+.4}",
                labels[i],
                l2(v),
                cosine(&vecs[0], v),
            );
        }

        let q_stab = cosine(&vecs[0], &vecs[1]);
        eprintln!("query stability cos = {q_stab:.6}");

        let cos_changelog = cosine(&vecs[0], &vecs[2]);
        let cos_code = cosine(&vecs[0], &vecs[4]);
        eprintln!("cos(query,CHANGELOG)={cos_changelog:.4} cos(query,code)={cos_code:.4}");

        let cos_cl_readme = cosine(&vecs[2], &vecs[3]);
        eprintln!("cos(CHANGELOG,README)={cos_cl_readme:.4}");

        // Embed the query ALONE (mimics search-time, not batched with docs).
        let solo = embedder.embed(&[query]).expect("embed solo");
        let cos_solo_vs_batched = cosine(&vecs[0], &solo[0]);
        eprintln!("cos(query batched, query solo) = {cos_solo_vs_batched:.6}");

        assert!(
            l2(&vecs[0]) > 0.5,
            "query vector has near-zero norm — degenerate"
        );
        assert!(q_stab > 0.999, "query embedding is unstable across calls");
        assert!(
            cos_changelog < 0.95,
            "query is ~identical to unrelated CHANGELOG text (cos={cos_changelog:.4}) — \
             the degenerate distance-0 behavior"
        );
        assert!(
            cos_solo_vs_batched > 0.999,
            "batched vs solo query embedding differ (cos={cos_solo_vs_batched:.4}) — \
             a batching/padding bug corrupts vectors"
        );
    }

    /// Full in-process pipeline: ingest mixed markdown+code with the REAL
    /// embedder + real HNSW, then survey. Prints `raw_distance` per hit. If
    /// distance≈0 dominates here, the bug is in index/search; if results are
    /// varied + relevant, the bug is daemon-specific (on-disk index / routing).
    #[tokio::test]
    #[ignore = "loads the MiniLM model; run explicitly with --ignored"]
    async fn pipeline_survey_is_not_degenerate() {
        use ministr_core::index::HnswIndex;
        use ministr_core::ingestion::IngestionPipeline;
        use ministr_core::search::{MultiResolutionSearch, SearchConfig};
        use ministr_core::storage::SqliteStorage;

        let home = std::env::var("HOME").unwrap();
        let data_dir = Path::new(&home).join(".ministr");
        let embedder = CandleEmbedder::with_data_dir("all-MiniLM-L6-v2", &data_dir)
            .expect("load MiniLM model");

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("corpus");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("CHANGELOG.md"), CHANGELOG_MD).unwrap();
        std::fs::write(dir.join("README.md"), README_MD).unwrap();
        std::fs::write(
            dir.join("auth.rs"),
            format!("//! Authentication token validation.\n{CODE_RS}\n"),
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let index = HnswIndex::new(384, 10_000).unwrap();
        let pipeline = IngestionPipeline::new();
        let stats = pipeline
            .ingest_directory_with_embeddings(&dir, &storage, &embedder, &index)
            .await
            .expect("ingest");
        eprintln!(
            "ingested files={} sections={} embeddings={}",
            stats.files_indexed, stats.total_sections, stats.total_embeddings
        );

        let searcher = MultiResolutionSearch::new(&embedder, &index);
        let cfg = SearchConfig {
            raw_k: 30,
            top_k: 8,
            sparse_weight: 0.0,
            rerank_top_k: None,
        };

        for q in [
            "how does authentication token validation work",
            "onboarding wizard step indicator tray menu",
        ] {
            eprintln!("\n=== query: {q:?} ===");
            let results = searcher.search(q, cfg).expect("search");
            for r in &results {
                eprintln!(
                    "  dist={:.4} score={:.4} {:?} {}",
                    r.raw_distance,
                    r.score,
                    r.resolution,
                    r.vector_id.content_id()
                );
            }
            let all_zero = results.iter().all(|r| r.raw_distance.abs() < 1e-6);
            assert!(
                !all_zero,
                "every hit has raw_distance≈0 — degenerate search reproduced in-process"
            );
        }
    }

    /// Load the daemon's ACTUAL on-disk `ministr` HNSW index and search it
    /// directly with a real query vector. If distances here are ≈0/constant,
    /// the on-disk index the daemon serves is degenerate (not the code).
    #[test]
    #[ignore = "reads the live daemon's on-disk index; run explicitly"]
    fn ondisk_ministr_index_distances() {
        use ministr_core::index::{HnswIndex, VectorIndex, VectorIndexLoad};

        let home = std::env::var("HOME").unwrap();
        let data_dir = Path::new(&home).join(".ministr");
        let index_dir = data_dir.join("corpora/multi-d6edc116/index"); // ministr corpus

        if !index_dir.exists() {
            eprintln!("SKIP: {} does not exist", index_dir.display());
            return;
        }

        let embedder = CandleEmbedder::with_data_dir("all-MiniLM-L6-v2", &data_dir)
            .expect("load MiniLM model");
        let index = HnswIndex::load(&index_dir).expect("load on-disk ministr index");

        eprintln!("ministr index len = {}", index.len());

        let probe = |label: &str, idx: &HnswIndex, qvec: &[f32]| {
            let hits = idx.search_knn(qvec, 6).expect("search_knn");
            let min = hits.iter().map(|h| h.distance).fold(f32::MAX, f32::min);
            let max = hits.iter().map(|h| h.distance).fold(f32::MIN, f32::max);
            eprintln!("[{label}] min={min:.9} max={max:.9}");
            for h in hits.iter().take(3) {
                eprintln!("    dist={:.9} {}", h.distance, h.id);
            }
        };

        let q1 = embedder.embed(&["deferred cross-file reference resolution"]).unwrap();
        let q2 = embedder.embed(&["niagara fluid simulation gpu emitter"]).unwrap();
        let zero = vec![0.0f32; 384];

        eprintln!("\n--- ministr (multi-d6edc116) ---");
        probe("ministr q1", &index, &q1[0]);
        probe("ministr q2", &index, &q2[0]);
        probe("ministr zero-query", &index, &zero);

        // Same code, the WORKING corpus — does it rank with varied distances?
        let pdir = data_dir.join("corpora/multi-a02ba540/index"); // ministr-private
        if pdir.exists() {
            let pindex = HnswIndex::load(&pdir).expect("load ministr-private");
            eprintln!("\n--- ministr-private (multi-a02ba540) len={} ---", pindex.len());
            probe("private q1", &pindex, &q1[0]);
            probe("private q2", &pindex, &q2[0]);
        }
    }

    /// Isolate persist→load: build a known-good in-memory index, search it,
    /// then persist → load → search again. If distances collapse to 0 after the
    /// round-trip, the serialization is the bug.
    #[test]
    #[ignore = "loads the MiniLM model; run explicitly"]
    fn persist_load_roundtrip_preserves_distances() {
        use ministr_core::index::{HnswIndex, VectorIndex, VectorIndexLoad};

        let home = std::env::var("HOME").unwrap();
        let data_dir = Path::new(&home).join(".ministr");
        let embedder = CandleEmbedder::with_data_dir("all-MiniLM-L6-v2", &data_dir)
            .expect("load MiniLM model");

        let index = HnswIndex::new(384, 10_000).unwrap();
        // Insert a handful of distinct vectors.
        let docs = [CHANGELOG_MD, README_MD, CODE_RS, "token validation auth"];
        let vecs = embedder.embed(&docs).expect("embed");
        for (i, v) in vecs.iter().enumerate() {
            index.insert(&format!("doc-{i}"), v).expect("insert");
        }

        let qv = embedder.embed(&["authentication token validation"]).expect("q");
        let pre = index.search_knn(&qv[0], 4).expect("pre search");
        eprintln!("=== PRE-persist ===");
        for h in &pre {
            eprintln!("  dist={:.5} {}", h.distance, h.id);
        }

        let tmp = tempfile::tempdir().unwrap();
        index.persist(tmp.path()).expect("persist");
        let loaded = HnswIndex::load(tmp.path()).expect("load");
        let post = loaded.search_knn(&qv[0], 4).expect("post search");
        eprintln!("=== POST-load ===");
        for h in &post {
            eprintln!("  dist={:.5} {}", h.distance, h.id);
        }

        let post_all_zero = post.iter().all(|h| h.distance.abs() < 1e-5);
        assert!(
            !post_all_zero,
            "distances collapsed to 0 after persist→load — serialization corrupts vectors"
        );
        // Distances should match pre/post (same vectors).
        for (a, b) in pre.iter().zip(&post) {
            assert!(
                (a.distance - b.distance).abs() < 1e-4,
                "distance changed across round-trip: {} vs {}",
                a.distance,
                b.distance
            );
        }
    }
}
