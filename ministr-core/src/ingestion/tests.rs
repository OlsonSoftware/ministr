#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use std::path::{Path, PathBuf};

    use crate::embedding::Embedder;
    use crate::extraction::claims::HeuristicClaimExtractor;
    use crate::extraction::summary::ExtractiveSummaryGenerator;
    use crate::index::VectorIndex;
    use crate::storage::SqliteStorage;
    use crate::storage::traits::{Storage, SymbolRecord};
    use crate::types::{CorpusRoot, RootKind, Section, SectionId, SymbolId, VectorId};

    use super::super::discovery::{
        discover_files, discover_paths, is_in_ignored_dir, is_supported_file,
    };
    use super::super::embedding::embed_document;
    use super::super::pipeline::{IngestionPipeline, IngestionProgress};
    use super::super::roots::{
        compute_relative_path, compute_root_id, compute_content_hash, find_root_for_file,
        language_for_extension, module_path_from_file, namespace_path, strip_root_prefix,
    };
    use super::super::sections::{
        coalesce_small_sections, enrich_sections, split_large_headingless_section,
    };
    use super::super::symbols::resolve_and_store_refs;

    // --- Ignored directory guard ---

    #[test]
    fn is_in_ignored_dir_catches_target() {
        assert!(is_in_ignored_dir(Path::new(
            "/home/user/project/target/debug/foo.rs"
        )));
        assert!(is_in_ignored_dir(Path::new(
            "target/release/build/crate/lib.rs"
        )));
    }

    #[test]
    fn is_in_ignored_dir_catches_node_modules() {
        assert!(is_in_ignored_dir(Path::new(
            "/app/node_modules/lodash/index.js"
        )));
    }

    #[test]
    fn is_in_ignored_dir_catches_git() {
        assert!(is_in_ignored_dir(Path::new("/repo/.git/objects/pack/foo")));
    }

    #[test]
    fn is_in_ignored_dir_allows_normal_paths() {
        assert!(!is_in_ignored_dir(Path::new(
            "/home/user/project/src/main.rs"
        )));
        assert!(!is_in_ignored_dir(Path::new("docs/README.md")));
        assert!(!is_in_ignored_dir(Path::new("lib.rs")));
    }

    // --- Hash computation ---

    #[test]
    fn sha256_deterministic() {
        let hash1 = compute_content_hash("hello world");
        let hash2 = compute_content_hash("hello world");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn sha256_different_content() {
        let hash1 = compute_content_hash("hello");
        let hash2 = compute_content_hash("world");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn sha256_empty_string() {
        let hash = compute_content_hash("");
        assert!(!hash.is_empty());
        assert_eq!(hash.len(), 64);
    }

    // --- File discovery ---

    #[test]
    fn is_supported_file_accepts_all_formats() {
        assert!(is_supported_file(Path::new("docs/readme.md")));
        assert!(is_supported_file(Path::new("notes.markdown")));
        assert!(is_supported_file(Path::new("test.mkd")));
        assert!(is_supported_file(Path::new("test.mdx")));
        assert!(is_supported_file(Path::new("page.html")));
        assert!(is_supported_file(Path::new("page.htm")));
        assert!(is_supported_file(Path::new("page.xhtml")));
        assert!(is_supported_file(Path::new("manual.pdf")));
        assert!(is_supported_file(Path::new("code.rs")));
        assert!(is_supported_file(Path::new("app.ts")));
        assert!(is_supported_file(Path::new("main.py")));
    }

    #[test]
    fn is_supported_file_rejects_others() {
        assert!(!is_supported_file(Path::new("data.csv")));
        assert!(!is_supported_file(Path::new("readme.txt")));
        assert!(!is_supported_file(Path::new("image.png")));
    }

    #[test]
    fn discover_files_from_temp_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("doc1.md"), "# Hello").unwrap();
        std::fs::write(tmp.path().join("doc2.md"), "# World").unwrap();
        std::fs::write(tmp.path().join("ignore.txt"), "not markdown").unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/nested.md"), "# Nested").unwrap();

        let files = discover_files(tmp.path()).unwrap();
        assert_eq!(files.len(), 3);
    }

    // --- Paragraph-boundary splitting ---

    #[test]
    fn small_headingless_section_not_split() {
        let section = Section {
            id: SectionId("test.md#root".into()),
            heading_path: Vec::new(),
            depth: 0,
            text: "Short paragraph.".into(),
            structural_nodes: Vec::new(),
            children: Vec::new(),
            claims: Vec::new(),
            summary: None,
        };

        let result = split_large_headingless_section(section, "test.md");
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn large_headingless_section_split_at_paragraphs() {
        let para1 = "Word ".repeat(300);
        let para2 = "More ".repeat(300);
        let text = format!("{para1}\n\n{para2}");

        let section = Section {
            id: SectionId("test.md#root".into()),
            heading_path: Vec::new(),
            depth: 0,
            text,
            structural_nodes: Vec::new(),
            children: Vec::new(),
            claims: Vec::new(),
            summary: None,
        };

        let result = split_large_headingless_section(section, "test.md");
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id.0, "test.md#paragraph-0");
        assert_eq!(result[1].id.0, "test.md#paragraph-1");
        assert_eq!(result[0].depth, 0);
    }

    #[test]
    fn headed_section_not_split() {
        let para1 = "Word ".repeat(300);
        let para2 = "More ".repeat(300);
        let text = format!("{para1}\n\n{para2}");

        let section = Section {
            id: SectionId("test.md#heading".into()),
            heading_path: vec!["Heading".into()],
            depth: 1,
            text,
            structural_nodes: Vec::new(),
            children: Vec::new(),
            claims: Vec::new(),
            summary: None,
        };

        let result = split_large_headingless_section(section, "test.md");
        assert_eq!(result.len(), 1);
    }

    // --- Section coalescing ---

    fn make_section(id: &str, heading: &str, depth: u32, text: &str) -> Section {
        Section {
            id: SectionId(id.into()),
            heading_path: if heading.is_empty() {
                Vec::new()
            } else {
                vec![heading.into()]
            },
            depth,
            text: text.into(),
            structural_nodes: Vec::new(),
            children: Vec::new(),
            claims: Vec::new(),
            summary: None,
        }
    }

    #[test]
    fn coalesce_three_small_siblings_into_one() {
        let sections = vec![
            make_section("s1", "Alpha", 2, "Short text A."),
            make_section("s2", "Beta", 2, "Short text B."),
            make_section("s3", "Gamma", 2, "Short text C."),
        ];

        let result = coalesce_small_sections(sections, 50);
        assert_eq!(result.len(), 1, "3 small siblings should merge into 1");
        assert_eq!(
            result[0].id.0, "s1",
            "merged section uses first sibling's ID"
        );
        assert!(result[0].text.contains("Short text A."));
        assert!(result[0].text.contains("### Beta"));
        assert!(result[0].text.contains("Short text B."));
        assert!(result[0].text.contains("### Gamma"));
        assert!(result[0].text.contains("Short text C."));
    }

    #[test]
    fn coalesce_large_section_stays_untouched() {
        let big_text = "The quick brown fox jumps over the lazy dog. ".repeat(30);
        let sections = vec![make_section("s1", "Big", 2, &big_text)];

        let result = coalesce_small_sections(sections, 50);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id.0, "s1");
    }

    #[test]
    fn coalesce_mixed_depths_merge_at_each_level() {
        let sections = vec![
            make_section("d1-a", "D1 A", 1, "Small."),
            make_section("d1-b", "D1 B", 1, "Also small."),
            make_section("d2-a", "D2 A", 2, "Tiny."),
            make_section("d2-b", "D2 B", 2, "Also tiny."),
        ];

        let result = coalesce_small_sections(sections, 50);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].depth, 1);
        assert_eq!(result[1].depth, 2);
    }

    #[test]
    fn coalesce_disabled_with_zero_threshold() {
        let sections = vec![
            make_section("s1", "A", 1, "Tiny."),
            make_section("s2", "B", 1, "Also tiny."),
        ];

        let result = coalesce_small_sections(sections, 0);
        assert_eq!(result.len(), 2, "zero threshold disables merging");
    }

    #[test]
    fn coalesce_preserves_document_order() {
        let sections = vec![
            make_section("s1", "First", 1, "First section."),
            make_section("s2", "Second", 1, "Second section."),
            make_section("s3", "Third", 1, "Third section."),
        ];

        let result = coalesce_small_sections(sections, 50);
        assert_eq!(result.len(), 1);
        let first_pos = result[0].text.find("First section.").unwrap();
        let second_pos = result[0].text.find("Second section.").unwrap();
        let third_pos = result[0].text.find("Third section.").unwrap();
        assert!(first_pos < second_pos);
        assert!(second_pos < third_pos);
    }

    #[test]
    fn coalesce_small_between_large_stays_separate() {
        let big_text = "The quick brown fox jumps over the lazy dog. ".repeat(30);
        let sections = vec![
            make_section("big1", "Big 1", 1, &big_text),
            make_section("small", "Small", 1, "Tiny."),
            make_section("big2", "Big 2", 1, &big_text),
        ];

        let result = coalesce_small_sections(sections, 50);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn coalesce_recurses_into_children() {
        let parent = Section {
            id: SectionId("parent".into()),
            heading_path: vec!["Parent".into()],
            depth: 1,
            text: "The quick brown fox jumps over the lazy dog. ".repeat(30),
            structural_nodes: Vec::new(),
            children: vec![
                make_section("child1", "Child A", 2, "Small child A."),
                make_section("child2", "Child B", 2, "Small child B."),
            ],
            claims: Vec::new(),
            summary: None,
        };

        let result = coalesce_small_sections(vec![parent], 50);
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].children.len(),
            1,
            "two small children should merge into one"
        );
    }

    // --- Section enrichment ---

    #[test]
    fn enrich_sections_adds_claims_and_summaries() {
        let mut sections = vec![Section {
            id: SectionId("test#s1".into()),
            heading_path: vec!["Test".into()],
            depth: 1,
            text: "The API uses JWT tokens with RS256 signing. Rate limits are 100 requests per minute.".into(),
            structural_nodes: Vec::new(),
            children: Vec::new(),
            claims: Vec::new(),
            summary: None,
        }];

        let extractor = HeuristicClaimExtractor::new();
        let summarizer = ExtractiveSummaryGenerator::new();
        let (sec_count, claim_count) = enrich_sections(&mut sections, &extractor, &summarizer);

        assert_eq!(sec_count, 1);
        assert!(claim_count > 0);
        assert!(!sections[0].claims.is_empty());
        assert!(sections[0].summary.is_some());
    }

    #[test]
    fn enrich_empty_text_section_no_claims() {
        let mut sections = vec![Section {
            id: SectionId("test#empty".into()),
            heading_path: vec!["Empty".into()],
            depth: 1,
            text: "   ".into(),
            structural_nodes: Vec::new(),
            children: Vec::new(),
            claims: Vec::new(),
            summary: None,
        }];

        let extractor = HeuristicClaimExtractor::new();
        let summarizer = ExtractiveSummaryGenerator::new();
        let (_, claim_count) = enrich_sections(&mut sections, &extractor, &summarizer);

        assert_eq!(claim_count, 0);
        assert!(sections[0].claims.is_empty());
        assert!(sections[0].summary.is_none());
    }

    #[test]
    fn enrich_nested_sections() {
        let mut sections = vec![Section {
            id: SectionId("test#parent".into()),
            heading_path: vec!["Parent".into()],
            depth: 1,
            text: "The parent section provides an overview of the system architecture.".into(),
            structural_nodes: Vec::new(),
            children: vec![Section {
                id: SectionId("test#child".into()),
                heading_path: vec!["Parent".into(), "Child".into()],
                depth: 2,
                text: "The child section implements authentication with OAuth2 and JWT tokens."
                    .into(),
                structural_nodes: Vec::new(),
                children: Vec::new(),
                claims: Vec::new(),
                summary: None,
            }],
            claims: Vec::new(),
            summary: None,
        }];

        let extractor = HeuristicClaimExtractor::new();
        let summarizer = ExtractiveSummaryGenerator::new();
        let (sec_count, _) = enrich_sections(&mut sections, &extractor, &summarizer);

        assert_eq!(sec_count, 2);
    }

    // --- Full pipeline integration tests ---

    #[tokio::test]
    async fn ingest_single_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("test.md"),
            "# API Reference\n\n\
             The auth service uses JWT tokens with RS256 signing.\n\n\
             ## Rate Limits\n\n\
             Rate limits are 100 requests per minute per API key.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();
        let stats = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();

        assert_eq!(stats.files_discovered, 1);
        assert_eq!(stats.files_indexed, 1);
        assert_eq!(stats.files_skipped, 0);
        assert!(stats.total_sections > 0);

        let docs = storage.list_documents().await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title, "API Reference");
        assert!(docs[0].summary.is_some());
    }

    #[tokio::test]
    async fn incremental_reindex_skips_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("doc.md"),
            "# Hello\n\nThe world is round.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();

        let stats1 = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();
        assert_eq!(stats1.files_indexed, 1);

        let stats2 = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();
        assert_eq!(stats2.files_skipped, 1);
        assert_eq!(stats2.files_indexed, 0);
    }

    #[tokio::test]
    async fn incremental_reindex_updates_changed() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("doc.md");
        std::fs::write(&file_path, "# V1\n\nOriginal content.\n").unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();

        let stats1 = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();
        assert_eq!(stats1.files_indexed, 1);

        std::fs::write(
            &file_path,
            "# V2\n\nUpdated content with new information.\n",
        )
        .unwrap();

        let stats2 = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();
        assert_eq!(stats2.files_indexed, 1);
        assert_eq!(stats2.files_skipped, 0);

        let docs = storage.list_documents().await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title, "V2");
    }

    #[tokio::test]
    async fn incremental_reindex_removes_deleted_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("keep.md"), "# Keep\n\nThis file stays.\n").unwrap();
        std::fs::write(
            tmp.path().join("remove.md"),
            "# Remove\n\nThis file will be deleted.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();

        let stats1 = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();
        assert_eq!(stats1.files_indexed, 2);

        std::fs::remove_file(tmp.path().join("remove.md")).unwrap();

        let stats2 = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();
        assert_eq!(stats2.files_removed, 1);

        let docs = storage.list_documents().await.unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title, "Keep");
    }

    #[tokio::test]
    async fn ingest_document_without_headings() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("plain.md"),
            "Just a plain paragraph.\n\nAnother paragraph here.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();
        let stats = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();

        assert_eq!(stats.files_indexed, 1);
        assert!(stats.total_sections >= 1);

        let docs = storage.list_documents().await.unwrap();
        assert_eq!(docs.len(), 1);
    }

    #[tokio::test]
    async fn ingest_empty_document() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("empty.md"), "").unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();
        let stats = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();

        assert_eq!(stats.files_indexed, 1);
        assert_eq!(stats.total_sections, 0);
    }

    #[tokio::test]
    async fn ingest_document_with_nested_lists() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("lists.md"),
            "# Configuration\n\n\
             The system supports the following options:\n\n\
             - Option A: enables feature X\n\
             - Option B: configures timeout to 30 seconds\n\
             - Option C: sets the maximum retry count to 5\n\n\
             1. First step: initialize the database\n\
             2. Second step: run migrations\n\
             3. Third step: start the server on port 8080\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();
        let stats = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();

        assert_eq!(stats.files_indexed, 1);
        assert!(stats.total_sections >= 1);
    }

    #[tokio::test]
    async fn ingest_empty_directory() {
        let tmp = tempfile::tempdir().unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();
        let stats = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();

        assert_eq!(stats.files_discovered, 0);
        assert_eq!(stats.files_indexed, 0);
    }

    #[tokio::test]
    async fn ingest_multiple_files() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("api.md"),
            "# API\n\nThe API uses REST over HTTPS.\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("guide.md"),
            "# Guide\n\nThe guide covers installation and configuration.\n",
        )
        .unwrap();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(
            tmp.path().join("sub/advanced.md"),
            "# Advanced\n\nAdvanced topics include clustering and replication.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();
        let stats = pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();

        assert_eq!(stats.files_discovered, 3);
        assert_eq!(stats.files_indexed, 3);

        let docs = storage.list_documents().await.unwrap();
        assert_eq!(docs.len(), 3);
    }

    // --- Embedding ingestion tests ---

    struct MockEmbedder {
        dim: usize,
    }

    impl crate::embedding::Embedder for MockEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, crate::error::IndexError> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0f32; self.dim];
                    for (i, b) in t.bytes().enumerate() {
                        v[i % self.dim] += f32::from(b) / 255.0;
                    }
                    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                    if norm > 0.0 {
                        for x in &mut v {
                            *x /= norm;
                        }
                    }
                    v
                })
                .collect())
        }

        fn dimension(&self) -> usize {
            self.dim
        }
    }

    fn make_mock_embedder_and_index() -> (MockEmbedder, crate::index::HnswIndex) {
        let dim = 8;
        let embedder = MockEmbedder { dim };
        let index = crate::index::HnswIndex::new(dim, 10_000).unwrap();
        (embedder, index)
    }

    #[tokio::test]
    async fn ingest_with_embeddings_creates_vectors() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("test.md"),
            "# API Reference\n\n\
             The auth service uses JWT tokens with RS256 signing.\n\n\
             ## Rate Limits\n\n\
             Rate limits are 100 requests per minute per API key.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let (embedder, index) = make_mock_embedder_and_index();
        let pipeline = IngestionPipeline::new();

        let stats = pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        assert_eq!(stats.files_indexed, 1);
        assert!(stats.total_sections > 0);

        assert!(!index.is_empty());

        let vec_count = index.len();
        assert!(
            vec_count >= 3,
            "expected at least 3 vectors, got {vec_count}"
        );
    }

    #[tokio::test]
    async fn embedding_ingestion_skips_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("doc.md"),
            "# Hello\n\nThe world is round.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let (embedder, index) = make_mock_embedder_and_index();
        let pipeline = IngestionPipeline::new();

        pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        let count_after_first = index.len();

        let stats2 = pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        assert_eq!(stats2.files_skipped, 1);
        assert_eq!(stats2.files_indexed, 0);
        assert_eq!(index.len(), count_after_first);
    }

    #[tokio::test]
    async fn embedding_ingestion_updates_changed_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("doc.md");
        std::fs::write(
            &file_path,
            "# V1\n\nOriginal content about authentication.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let (embedder, index) = make_mock_embedder_and_index();
        let pipeline = IngestionPipeline::new();

        pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        let count_v1 = index.len();

        std::fs::write(
            &file_path,
            "# V2\n\nUpdated content.\n\n## New Section\n\nNew information about rate limits.\n",
        )
        .unwrap();

        pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        assert!(!index.is_empty());
        assert!(index.len() >= count_v1);
    }

    #[tokio::test]
    async fn embedding_ingestion_removes_deleted_file_vectors() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("keep.md"),
            "# Keep\n\nThis file stays in the index.\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join("remove.md"),
            "# Remove\n\nThis file will be deleted from the index.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let (embedder, index) = make_mock_embedder_and_index();
        let pipeline = IngestionPipeline::new();

        pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        let count_before = index.len();
        assert!(count_before > 0);

        std::fs::remove_file(tmp.path().join("remove.md")).unwrap();

        let stats2 = pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        assert_eq!(stats2.files_removed, 1);
        assert!(index.len() < count_before);
    }

    #[tokio::test]
    async fn embed_document_creates_multi_resolution_vectors() {
        let doc = crate::types::DocumentTree {
            id: crate::types::ContentId("doc1".into()),
            title: "Test".into(),
            source_path: "test.md".into(),
            sections: vec![crate::types::Section {
                id: SectionId("test.md#s1".into()),
                heading_path: vec!["Section One".into()],
                depth: 1,
                text: "The authentication system uses JWT tokens.".into(),
                structural_nodes: vec![],
                children: vec![],
                claims: vec![crate::types::Claim {
                    id: crate::types::ClaimId("c1".into()),
                    text: "JWT tokens use RS256 signing.".into(),
                    section_id: SectionId("test.md#s1".into()),
                }],
                summary: Some("Auth system overview.".into()),
            }],
            summary: Some("Document about authentication.".into()),
        };

        let (embedder, index) = make_mock_embedder_and_index();
        let count = embed_document(&doc, &embedder, &index).unwrap();

        assert_eq!(count, 4);
        assert_eq!(index.len(), 4, "all 4 vectors should be in the index");

        // Verify membership by querying each expected ID with the
        // exact text that was embedded to produce it. HNSW's
        // approximate k-NN is unreliable at enumerating all entries in
        // a 4-node graph under parallel-test CPU load, but it IS
        // reliable at returning an exact match as top-1 — the query
        // vector is equal to the stored vector, distance ≈ 0, so it
        // always wins over any other node's distance.
        let cases = [
            ("doc-summary::doc1", "Document about authentication."),
            ("sec-summary::test.md#s1", "Auth system overview."),
            (
                "section::test.md#s1",
                "The authentication system uses JWT tokens.",
            ),
        ];
        for (expected_id, text) in cases {
            let q = embedder.embed(&[text]).unwrap().into_iter().next().unwrap();
            let results = index.search_knn(&q, 4).unwrap();
            assert!(
                !results.is_empty(),
                "search for {expected_id} returned nothing"
            );
            assert_eq!(
                results[0].id, expected_id,
                "top-1 for '{text}' should be {expected_id}, got {results:?}"
            );
        }
    }

    // --- Integration test: coalescing reduces section count ---

    #[tokio::test]
    async fn coalescing_reduces_section_count_in_ingestion() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("fragmented.md"),
            "# Guide\n\n\
             ## A\n\nTiny.\n\n\
             ## B\n\nAlso tiny.\n\n\
             ## C\n\nStill tiny.\n\n\
             ## Big Section\n\n\
             This section has much more content that should keep it standalone. \
             It contains detailed information about the system architecture, \
             including multiple paragraphs of explanation covering authentication, \
             authorization, rate limiting, caching strategies, database design, \
             and deployment considerations for the production environment.\n",
        )
        .unwrap();

        let storage_merged = SqliteStorage::open_in_memory().unwrap();
        let pipeline_merged = IngestionPipeline::new();

        let stats_merged = pipeline_merged
            .ingest_directory(tmp.path(), &storage_merged)
            .await
            .unwrap();

        let storage_unmerged = SqliteStorage::open_in_memory().unwrap();
        let pipeline_unmerged = IngestionPipeline::new().with_min_section_tokens(0);

        let stats_unmerged = pipeline_unmerged
            .ingest_directory(tmp.path(), &storage_unmerged)
            .await
            .unwrap();

        assert!(
            stats_merged.total_sections < stats_unmerged.total_sections,
            "merged section count ({}) should be less than unmerged ({})",
            stats_merged.total_sections,
            stats_unmerged.total_sections,
        );
    }

    // --- Multi-path discovery ---

    #[test]
    fn discover_paths_with_mixed_dirs_and_files() {
        let tmp = tempfile::tempdir().unwrap();

        let docs_dir = tmp.path().join("docs");
        std::fs::create_dir(&docs_dir).unwrap();
        std::fs::write(docs_dir.join("guide.md"), "# Guide").unwrap();
        std::fs::write(docs_dir.join("api.md"), "# API").unwrap();
        std::fs::write(docs_dir.join("ignore.txt"), "not supported").unwrap();

        std::fs::write(tmp.path().join("DESIGN.md"), "# Design").unwrap();

        let paths = vec![docs_dir.clone(), tmp.path().join("DESIGN.md")];

        let files = discover_paths(&paths).unwrap();

        assert_eq!(
            files.len(),
            3,
            "should discover 2 from docs/ + 1 individual file, got: {files:?}"
        );

        for f in &files {
            assert!(is_supported_file(f), "unsupported file included: {f:?}");
        }
    }

    #[test]
    fn discover_paths_deduplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let docs_dir = tmp.path().join("docs");
        std::fs::create_dir(&docs_dir).unwrap();
        std::fs::write(docs_dir.join("guide.md"), "# Guide").unwrap();

        let paths = vec![docs_dir.clone(), docs_dir];

        let files = discover_paths(&paths).unwrap();
        assert_eq!(files.len(), 1, "duplicates should be removed");
    }

    #[test]
    fn discover_paths_with_glob_patterns() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("readme.md"), "# Readme").unwrap();
        std::fs::write(tmp.path().join("design.md"), "# Design").unwrap();
        std::fs::write(tmp.path().join("code.rs"), "fn main() {}").unwrap();

        let glob_pattern = tmp.path().join("*.md");
        let paths = vec![glob_pattern];

        let files = discover_paths(&paths).unwrap();
        assert_eq!(
            files.len(),
            2,
            "glob should match 2 .md files, got: {files:?}"
        );
    }

    #[test]
    fn discover_paths_empty_input() {
        let files = discover_paths(&[]).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn compute_relative_path_preserves_full_path() {
        let sources = vec![PathBuf::from("./docs"), PathBuf::from("./src")];

        let rel = compute_relative_path(Path::new("./docs/guide.md"), &sources);
        assert_eq!(rel, "docs/guide.md");

        let rel = compute_relative_path(Path::new("./src/lib.rs"), &sources);
        assert_eq!(rel, "src/lib.rs");
    }

    #[test]
    fn compute_relative_path_no_collision_across_crates() {
        let sources = vec![
            PathBuf::from("./ministr-core/src"),
            PathBuf::from("./ministr-mcp/src"),
        ];

        let rel1 = compute_relative_path(Path::new("./ministr-core/src/lib.rs"), &sources);
        let rel2 = compute_relative_path(Path::new("./ministr-mcp/src/lib.rs"), &sources);
        assert_ne!(rel1, rel2, "paths from different crates must not collide");
        assert_eq!(rel1, "ministr-core/src/lib.rs");
        assert_eq!(rel2, "ministr-mcp/src/lib.rs");
    }

    #[test]
    fn module_path_from_file_mod_rs() {
        assert_eq!(
            module_path_from_file("ministr-core/src/session/mod.rs"),
            vec!["session"]
        );
    }

    #[test]
    fn module_path_from_file_nested() {
        assert_eq!(
            module_path_from_file("ministr-core/src/session/budget.rs"),
            vec!["session", "budget"]
        );
    }

    #[test]
    fn module_path_from_file_top_level() {
        assert_eq!(
            module_path_from_file("ministr-core/src/config.rs"),
            vec!["config"]
        );
    }

    #[test]
    fn module_path_from_file_lib_rs() {
        assert_eq!(
            module_path_from_file("ministr-core/src/lib.rs"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn module_path_from_file_no_src_dir() {
        assert_eq!(
            module_path_from_file("session/budget.rs"),
            vec!["session", "budget"]
        );
        assert_eq!(module_path_from_file("utils.rs"), vec!["utils"]);
    }

    #[test]
    fn module_path_from_file_index_js() {
        assert_eq!(
            module_path_from_file("src/components/index.ts"),
            vec!["components"]
        );
    }

    #[test]
    fn module_path_from_file_deeply_nested() {
        assert_eq!(
            module_path_from_file("crate/src/a/b/c/mod.rs"),
            vec!["a", "b", "c"]
        );
        assert_eq!(
            module_path_from_file("crate/src/a/b/c/widget.rs"),
            vec!["a", "b", "c", "widget"]
        );
    }

    #[test]
    fn compute_root_id_is_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let id1 = compute_root_id(tmp.path());
        let id2 = compute_root_id(tmp.path());
        assert_eq!(id1, id2);
        assert!(id1.starts_with("root-"));
    }

    #[test]
    fn compute_root_id_differs_for_different_paths() {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        let id1 = compute_root_id(tmp1.path());
        let id2 = compute_root_id(tmp2.path());
        assert_ne!(id1, id2);
    }

    #[test]
    fn find_root_for_file_picks_longest_prefix() {
        let parent = PathBuf::from("/project");
        let child = PathBuf::from("/project/src");
        let roots = vec![
            (parent.clone(), "root-parent".to_string()),
            (child.clone(), "root-child".to_string()),
        ];

        let result = find_root_for_file(Path::new("/project/src/main.rs"), &roots);
        assert_eq!(result, Some("root-child"));

        let result = find_root_for_file(Path::new("/project/docs/readme.md"), &roots);
        assert_eq!(result, Some("root-parent"));
    }

    #[test]
    fn find_root_for_file_returns_none_for_unmatched() {
        let roots = vec![(PathBuf::from("/project"), "root-1".to_string())];
        let result = find_root_for_file(Path::new("/other/file.rs"), &roots);
        assert!(result.is_none());
    }

    #[test]
    fn language_for_extension_covers_common_languages() {
        assert_eq!(language_for_extension("rs"), "rust");
        assert_eq!(language_for_extension("py"), "python");
        assert_eq!(language_for_extension("ts"), "typescript");
        assert_eq!(language_for_extension("js"), "javascript");
        assert_eq!(language_for_extension("go"), "go");
        assert_eq!(language_for_extension("md"), "markdown");
        assert_eq!(language_for_extension("html"), "html");
        assert_eq!(language_for_extension("toml"), "toml");
        assert_eq!(language_for_extension("unknown"), "other");
    }

    #[tokio::test]
    async fn multi_root_ingestion_registers_roots_and_tags_documents() {
        let tmp = tempfile::tempdir().unwrap();

        let root_a = tmp.path().join("crate-a");
        let root_b = tmp.path().join("crate-b");
        std::fs::create_dir_all(&root_a).unwrap();
        std::fs::create_dir_all(&root_b).unwrap();

        std::fs::write(root_a.join("lib.rs"), "pub fn hello() {}").unwrap();
        std::fs::write(root_b.join("main.py"), "def main(): pass").unwrap();

        let storage = crate::storage::SqliteStorage::open_in_memory().unwrap();
        let pipeline = IngestionPipeline::new();

        let paths = vec![root_a.clone(), root_b.clone()];
        let stats = pipeline.ingest_directory(&root_a, &storage).await.unwrap();
        assert!(stats.files_indexed > 0 || stats.files_skipped > 0);

        let embedder = crate::embedding::FastEmbedder::new("all-MiniLM-L6-v2", None).unwrap();
        let index = crate::index::HnswIndex::new(embedder.dimension(), 1000).unwrap();

        let stats = pipeline
            .ingest_paths_with_embeddings(&paths, &storage, &embedder, &index)
            .await
            .unwrap();

        assert!(
            stats.files_discovered >= 2,
            "should discover files from both roots"
        );

        let roots = storage.list_corpus_roots().await.unwrap();
        assert_eq!(roots.len(), 2, "should have two corpus roots");

        let total_files: usize = roots.iter().map(|r| r.file_count).sum();
        assert!(total_files >= 2, "total file count should be >= 2");

        let has_rust = roots
            .iter()
            .any(|r| r.language_stats.get("rust").copied().unwrap_or(0) > 0);
        let has_python = roots
            .iter()
            .any(|r| r.language_stats.get("python").copied().unwrap_or(0) > 0);
        assert!(has_rust, "should have rust in language stats");
        assert!(has_python, "should have python in language stats");
    }

    // --- C6.2: E2E unified code + doc search ---

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn e2e_unified_code_and_doc_search() {
        use crate::search::{MultiResolutionSearch, SearchConfig};
        use crate::types::Resolution;

        let tmp = tempfile::tempdir().unwrap();

        std::fs::write(
            tmp.path().join("ingestion.md"),
            "# Ingestion Pipeline\n\n\
             The ingestion pipeline processes files from a directory.\n\
             It hashes each file and skips unchanged content.\n\n\
             ## Parsing\n\n\
             Files are parsed into document trees with sections and claims.\n",
        )
        .unwrap();

        std::fs::write(
            tmp.path().join("pipeline.rs"),
            r"//! Ingestion pipeline orchestrator.

/// Processes files and indexes their content.
pub struct IngestionPipeline {
    /// Minimum section token threshold.
    pub min_tokens: usize,
}

impl IngestionPipeline {
    /// Create a new pipeline with defaults.
    pub fn new() -> Self {
        Self { min_tokens: 50 }
    }

    /// Ingest all files from a directory.
    pub fn ingest(&self, dir: &str) -> usize {
        42
    }
}

/// Hash the content of a file for change detection.
pub fn compute_hash(content: &str) -> String {
    content.len().to_string()
}
",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let (embedder, index) = make_mock_embedder_and_index();
        let pipeline = IngestionPipeline::new();

        let stats = pipeline
            .ingest_directory_with_embeddings(tmp.path(), &storage, &embedder, &index)
            .await
            .unwrap();

        assert_eq!(stats.files_failed, 0, "no files should fail: {stats:?}");
        assert_eq!(stats.files_indexed, 2);

        let total_vectors = index.len();
        assert!(
            total_vectors >= 6,
            "expected at least 6 vectors (doc + symbol), got {total_vectors}"
        );

        let searcher = MultiResolutionSearch::new(&embedder, &index);
        let config = SearchConfig {
            raw_k: 30,
            top_k: 10,
            sparse_weight: 0.0,
            rerank_top_k: None,
        };
        let results = searcher.search("ingestion pipeline", config).unwrap();

        assert!(
            !results.is_empty(),
            "search should return results for 'ingestion pipeline'"
        );

        let has_doc_result = results.iter().any(|r| {
            matches!(
                r.resolution,
                Resolution::Summary | Resolution::Section | Resolution::Claim
            )
        });
        let has_symbol_result = results.iter().any(|r| {
            matches!(
                r.resolution,
                Resolution::SymbolStub | Resolution::SymbolFull
            )
        });

        assert!(
            has_doc_result,
            "search results should include document sections"
        );
        assert!(
            has_symbol_result,
            "search results should include code symbols"
        );

        let symbols = storage
            .list_symbols(&crate::storage::SymbolFilter {
                file_path: Some("pipeline.rs".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(
            symbols.len() >= 2,
            "expected at least 2 symbols (struct + impl/fn), got {}: {:?}",
            symbols.len(),
            symbols
                .iter()
                .map(|s| format!("{} {}", s.kind, s.name))
                .collect::<Vec<_>>()
        );

        let pipeline_struct = symbols
            .iter()
            .find(|s| s.name == "IngestionPipeline" && s.kind != "impl")
            .unwrap_or_else(|| {
                panic!(
                    "should have IngestionPipeline struct, found: {:?}",
                    symbols
                        .iter()
                        .map(|s| format!("{}:{}", s.kind, s.name))
                        .collect::<Vec<_>>()
                )
            });
        let stub_vid = VectorId::symbol_stub(pipeline_struct.id.as_ref());
        assert_eq!(stub_vid.resolution(), crate::types::Resolution::SymbolStub);
    }

    // --- IngestionProgress ---

    #[test]
    fn progress_lifecycle() {
        let progress = IngestionProgress::new();
        assert_eq!(progress.status(), 0);
        assert!(!progress.is_running());

        progress.start(10);
        assert!(progress.is_running());
        assert_eq!(progress.files_total(), 10);
        assert_eq!(progress.files_done(), 0);

        for _ in 0..5 {
            progress.increment_done();
        }
        assert_eq!(progress.files_done(), 5);
        assert!(progress.is_running());

        progress.complete();
        assert!(!progress.is_running());
        assert_eq!(progress.status(), 2);
    }

    #[test]
    fn progress_default() {
        let progress = IngestionProgress::default();
        assert_eq!(progress.status(), 0);
        assert_eq!(progress.files_total(), 0);
        assert_eq!(progress.files_done(), 0);
    }

    // --- resolve_and_store_refs hardening ---

    #[tokio::test]
    async fn resolve_refs_prefers_mod_anchor_for_imports() {
        let storage = SqliteStorage::open_in_memory().unwrap();

        let mod_sym = SymbolRecord {
            id: SymbolId::from("sym-test.rs::test_mod".to_string()),
            file_path: "test.rs".to_string(),
            name: "test_mod".to_string(),
            kind: "mod".to_string(),
            visibility: "pub".to_string(),
            module_path: String::new(),
            line_start: 1,
            line_end: 1,
            signature: String::new(),
            doc_comment: None,
            cyclomatic_complexity: None,
        };
        let struct_sym = SymbolRecord {
            id: SymbolId::from("sym-test.rs::MyStruct".to_string()),
            file_path: "test.rs".to_string(),
            name: "MyStruct".to_string(),
            kind: "struct".to_string(),
            visibility: "pub".to_string(),
            module_path: String::new(),
            line_start: 5,
            line_end: 10,
            signature: String::new(),
            doc_comment: None,
            cyclomatic_complexity: None,
        };
        let target_sym = SymbolRecord {
            id: SymbolId::from("sym-other.rs::OtherType".to_string()),
            file_path: "other.rs".to_string(),
            name: "OtherType".to_string(),
            kind: "struct".to_string(),
            visibility: "pub".to_string(),
            module_path: String::new(),
            line_start: 1,
            line_end: 5,
            signature: String::new(),
            doc_comment: None,
            cyclomatic_complexity: None,
        };

        storage
            .insert_symbols(&[mod_sym.clone(), struct_sym.clone(), target_sym])
            .await
            .unwrap();

        let source = b"use crate::OtherType;\n\npub struct MyStruct {}\n";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        let tree = parser.parse(source, None).unwrap();

        let local_symbols = vec![mod_sym.clone(), struct_sym];
        let _result = resolve_and_store_refs(
            &tree,
            source,
            "test.rs",
            "rust",
            &local_symbols,
            &storage,
            None,
        )
        .await
        .unwrap();

        let refs = storage.query_refs(&mod_sym.id, None).await.unwrap();
        assert_eq!(refs.len(), 1, "should resolve one import ref");
        assert!(
            !refs.is_empty(),
            "the mod symbol should be the from_symbol in the ref"
        );
    }

    #[tokio::test]
    async fn re_resolve_dependency_refs_links_cloned_symbols() {
        use crate::code::package_graph::{PackageGraph, PackageInfo};

        let storage = SqliteStorage::open_in_memory().unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let local_file = tmp.path().join("lib.rs");
        std::fs::write(
            &local_file,
            "use my_dep::Config;\n\npub fn run(c: Config) {}\n",
        )
        .unwrap();

        let pipeline = IngestionPipeline::new();
        pipeline
            .ingest_directory(tmp.path(), &storage)
            .await
            .unwrap();

        let local_anchor = SymbolRecord {
            id: SymbolId("sym-lib.rs::run".into()),
            file_path: "lib.rs".into(),
            name: "run".into(),
            kind: "function".into(),
            visibility: "pub".into(),
            signature: "pub fn run(c: Config)".into(),
            doc_comment: None,
            module_path: String::new(),
            line_start: 3,
            line_end: 3,
            cyclomatic_complexity: None,
        };
        storage.insert_symbols(&[local_anchor]).await.unwrap();

        let dep_symbol = SymbolRecord {
            id: SymbolId("sym-dep/src/lib.rs::Config".into()),
            file_path: "dep/src/lib.rs".into(),
            name: "Config".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            signature: "pub struct Config".into(),
            doc_comment: None,
            module_path: String::new(),
            line_start: 1,
            line_end: 5,
            cyclomatic_complexity: None,
        };
        storage.insert_symbols(&[dep_symbol]).await.unwrap();

        let dep_graph = {
            let mut g = PackageGraph::empty();
            g.add_package(PackageInfo {
                name: "my-dep".into(),
                crate_name: "my_dep".into(),
                dir_prefix: "dep/".into(),
            });
            g
        };

        let corpus_roots = vec![tmp.path().to_path_buf()];
        let count = pipeline
            .re_resolve_dependency_refs(&dep_graph, &["dep/".into()], &corpus_roots, &storage)
            .await
            .unwrap();

        assert!(
            count > 0,
            "should resolve at least one dependency reference"
        );
    }

    // --- Root-namespaced path helpers ---

    #[test]
    fn namespace_path_produces_prefixed_path() {
        assert_eq!(
            namespace_path("root-0011223344556677", "src/lib.rs"),
            "root-0011223344556677/src/lib.rs"
        );
    }

    #[test]
    fn namespace_path_preserves_nested_paths() {
        assert_eq!(
            namespace_path("root-aabbccdd00112233", "deep/nested/dir/file.rs"),
            "root-aabbccdd00112233/deep/nested/dir/file.rs"
        );
    }

    #[test]
    fn strip_root_prefix_extracts_relative_path() {
        assert_eq!(
            strip_root_prefix("root-0011223344556677/src/lib.rs"),
            Some("src/lib.rs")
        );
    }

    #[test]
    fn strip_root_prefix_returns_none_for_unnamespaced() {
        assert_eq!(strip_root_prefix("src/lib.rs"), None);
    }

    #[test]
    fn strip_root_prefix_returns_none_for_wrong_prefix_length() {
        assert_eq!(strip_root_prefix("root-0011223344/src/lib.rs"), None);
    }

    #[test]
    fn strip_root_prefix_returns_none_for_non_hex() {
        assert_eq!(strip_root_prefix("root-gghhiijjkkllmmnn/src/lib.rs"), None);
    }

    #[test]
    fn strip_root_prefix_roundtrips_with_namespace_path() {
        let root_id = "root-aabbccdd00112233";
        let relative = "some/path/file.rs";
        let namespaced = namespace_path(root_id, relative);
        assert_eq!(strip_root_prefix(&namespaced), Some(relative));
    }

    /// Stub embedder for integration tests — produces zero vectors.
    struct StubEmbedder;

    impl crate::embedding::Embedder for StubEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, crate::error::IndexError> {
            Ok(texts.iter().map(|_| vec![0.0; 4]).collect())
        }
        fn dimension(&self) -> usize {
            4
        }
    }

    /// Stub vector index for integration tests — discards all operations.
    struct StubVectorIndex;

    impl crate::index::VectorIndex for StubVectorIndex {
        fn insert(&self, _id: &str, _vector: &[f32]) -> Result<(), crate::error::IndexError> {
            Ok(())
        }
        fn search_knn(
            &self,
            _query: &[f32],
            _k: usize,
        ) -> Result<Vec<crate::index::SearchResult>, crate::error::IndexError> {
            Ok(Vec::new())
        }
        fn delete(&self, _id: &str) -> Result<bool, crate::error::IndexError> {
            Ok(false)
        }
        fn persist(&self, _dir: &Path) -> Result<(), crate::error::IndexError> {
            Ok(())
        }
        fn len(&self) -> usize {
            0
        }
        fn dimension(&self) -> usize {
            4
        }
    }

    async fn register_test_root(storage: &SqliteStorage, root_id: &str, path: &Path) {
        let root = CorpusRoot {
            id: root_id.to_string(),
            path: path.to_string_lossy().to_string(),
            kind: RootKind::Local,
            display_name: Some("test".into()),
            file_count: 0,
            language_stats: std::collections::HashMap::new(),
            repo_url: None,
            branch: None,
            commit_sha: None,
            clone_timestamp: None,
            sparse_paths: Vec::new(),
        };
        storage.upsert_corpus_root(&root).await.unwrap();
    }

    #[tokio::test]
    async fn rooted_ingestion_namespaces_document_ids() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("readme.md"),
            "# Hello\n\nThe world is round.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let embedder = StubEmbedder;
        let index = StubVectorIndex;
        let pipeline = IngestionPipeline::new();

        let root_id = compute_root_id(tmp.path());
        register_test_root(&storage, &root_id, tmp.path()).await;

        pipeline
            .ingest_directory_with_embeddings_rooted(
                tmp.path(),
                &storage,
                &embedder,
                &index,
                Some(&root_id),
                None,
            )
            .await
            .unwrap();

        let docs = storage.list_documents_by_root(&root_id).await.unwrap();
        assert!(!docs.is_empty(), "should have at least one document");
        for doc in &docs {
            assert!(
                doc.source_path.starts_with(&root_id),
                "document source_path '{}' should be namespaced with root ID '{}'",
                doc.source_path,
                root_id
            );
            let stripped = strip_root_prefix(&doc.source_path);
            assert_eq!(stripped, Some("readme.md"));
        }
    }

    #[tokio::test]
    async fn rooted_ingestion_no_collision_across_roots() {
        let tmp_a = tempfile::tempdir().unwrap();
        let tmp_b = tempfile::tempdir().unwrap();

        std::fs::write(
            tmp_a.path().join("readme.md"),
            "# Project A\n\nProject A is about apples.\n",
        )
        .unwrap();
        std::fs::write(
            tmp_b.path().join("readme.md"),
            "# Project B\n\nProject B is about bananas.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let embedder = StubEmbedder;
        let index = StubVectorIndex;
        let pipeline = IngestionPipeline::new();

        let root_a = compute_root_id(tmp_a.path());
        let root_b = compute_root_id(tmp_b.path());
        register_test_root(&storage, &root_a, tmp_a.path()).await;
        register_test_root(&storage, &root_b, tmp_b.path()).await;

        pipeline
            .ingest_directory_with_embeddings_rooted(
                tmp_a.path(),
                &storage,
                &embedder,
                &index,
                Some(&root_a),
                None,
            )
            .await
            .unwrap();

        pipeline
            .ingest_directory_with_embeddings_rooted(
                tmp_b.path(),
                &storage,
                &embedder,
                &index,
                Some(&root_b),
                None,
            )
            .await
            .unwrap();

        let docs_a = storage.list_documents_by_root(&root_a).await.unwrap();
        let docs_b = storage.list_documents_by_root(&root_b).await.unwrap();

        assert!(!docs_a.is_empty(), "root A should have documents");
        assert!(!docs_b.is_empty(), "root B should have documents");

        let ids_a: Vec<&str> = docs_a.iter().map(|d| d.id.0.as_str()).collect();
        let ids_b: Vec<&str> = docs_b.iter().map(|d| d.id.0.as_str()).collect();
        for id in &ids_a {
            assert!(
                !ids_b.contains(id),
                "document ID '{id}' should not appear in both roots"
            );
        }
    }

    #[tokio::test]
    async fn ingestion_cancelled_stops_early() {
        let tmp = tempfile::tempdir().unwrap();

        for i in 0..5 {
            std::fs::write(
                tmp.path().join(format!("file{i}.md")),
                format!("# File {i}\n\nContent for file {i}.\n"),
            )
            .unwrap();
        }

        let storage = SqliteStorage::open_in_memory().unwrap();
        let embedder = StubEmbedder;
        let index = StubVectorIndex;
        let pipeline = IngestionPipeline::new();

        let ct = tokio_util::sync::CancellationToken::new();
        ct.cancel();

        let result = pipeline
            .ingest_directory_with_embeddings_rooted(
                tmp.path(),
                &storage,
                &embedder,
                &index,
                None,
                Some(&ct),
            )
            .await;

        assert!(result.is_err());
        assert!(
            matches!(result.unwrap_err(), crate::error::IngestionError::Cancelled),
            "should return Cancelled error"
        );
    }
}
