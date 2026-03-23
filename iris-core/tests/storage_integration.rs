//! Integration tests for the `SQLite` storage layer.
//!
//! These tests run against real `SQLite` databases (not mocks) to verify
//! CRUD operations, concurrent access, WAL behavior, and migrations.

use iris_core::session::{EvictionPolicy, Session, SessionId};
use iris_core::storage::{SqliteStorage, Storage, SymbolFilter, SymbolRecord, SymbolRefRecord};
use iris_core::types::{
    Claim, ClaimId, ClaimRelationship, ContentId, DocumentTree, RefKind, RelationType, Resolution,
    Section, SectionId, SymbolId,
};

/// Build a sample document tree for testing.
fn sample_document() -> DocumentTree {
    DocumentTree {
        id: ContentId("doc-1".into()),
        title: "API Reference".into(),
        source_path: "docs/api.md".into(),
        sections: vec![
            Section {
                id: SectionId("s1".into()),
                heading_path: vec!["API Reference".into(), "Authentication".into()],
                depth: 2,
                text: "Authentication uses JWT tokens with RS256 signing.".into(),
                structural_nodes: vec![],
                children: vec![],
                claims: vec![
                    Claim {
                        id: ClaimId("c1".into()),
                        text: "JWT tokens use RS256 signing.".into(),
                        section_id: SectionId("s1".into()),
                    },
                    Claim {
                        id: ClaimId("c2".into()),
                        text: "Tokens expire after 1 hour.".into(),
                        section_id: SectionId("s1".into()),
                    },
                ],
                summary: Some("Auth overview.".into()),
            },
            Section {
                id: SectionId("s2".into()),
                heading_path: vec!["API Reference".into(), "Rate Limits".into()],
                depth: 2,
                text: "Rate limits are 100 requests per minute per API key.".into(),
                structural_nodes: vec![],
                children: vec![],
                claims: vec![Claim {
                    id: ClaimId("c3".into()),
                    text: "Rate limit: 100/min per key.".into(),
                    section_id: SectionId("s2".into()),
                }],
                summary: None,
            },
        ],
        summary: Some("Full API reference documentation.".into()),
    }
}

#[tokio::test]
async fn insert_and_get_document() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let doc = sample_document();

    storage.insert_document(&doc).await.unwrap();

    let retrieved = storage.get_document(&doc.id).await.unwrap();
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.id, doc.id);
    assert_eq!(retrieved.title, "API Reference");
    assert_eq!(retrieved.source_path, "docs/api.md");
    assert_eq!(
        retrieved.summary.as_deref(),
        Some("Full API reference documentation.")
    );
}

#[tokio::test]
async fn list_documents() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    let first = sample_document();
    let second = DocumentTree {
        id: ContentId("doc-2".into()),
        title: "User Guide".into(),
        source_path: "docs/guide.md".into(),
        sections: vec![],
        summary: None,
    };

    storage.insert_document(&first).await.unwrap();
    storage.insert_document(&second).await.unwrap();

    let all = storage.list_documents().await.unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn delete_document_cascades() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let doc = sample_document();

    storage.insert_document(&doc).await.unwrap();

    // Verify sections and claims exist
    let sections = storage.list_sections(&doc.id).await.unwrap();
    assert_eq!(sections.len(), 2);
    let claims = storage.list_claims(&SectionId("s1".into())).await.unwrap();
    assert_eq!(claims.len(), 2);

    // Delete the document
    let deleted = storage.delete_document(&doc.id).await.unwrap();
    assert!(deleted);

    // Verify cascade: sections and claims should be gone
    let sections = storage.list_sections(&doc.id).await.unwrap();
    assert!(sections.is_empty());
    let claims = storage.list_claims(&SectionId("s1".into())).await.unwrap();
    assert!(claims.is_empty());
}

#[tokio::test]
async fn delete_nonexistent_document_returns_false() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let deleted = storage
        .delete_document(&ContentId("nonexistent".into()))
        .await
        .unwrap();
    assert!(!deleted);
}

#[tokio::test]
async fn get_and_list_sections() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let doc = sample_document();
    storage.insert_document(&doc).await.unwrap();

    let section = storage.get_section(&SectionId("s1".into())).await.unwrap();
    assert!(section.is_some());
    let section = section.unwrap();
    assert_eq!(section.id, SectionId("s1".into()));
    assert_eq!(section.document_id, doc.id);
    assert_eq!(
        section.heading_path,
        vec!["API Reference", "Authentication"]
    );
    assert_eq!(section.depth, 2);
    assert_eq!(section.summary.as_deref(), Some("Auth overview."));

    let sections = storage.list_sections(&doc.id).await.unwrap();
    assert_eq!(sections.len(), 2);
    // Should be ordered by position
    assert_eq!(sections[0].id, SectionId("s1".into()));
    assert_eq!(sections[1].id, SectionId("s2".into()));
}

#[tokio::test]
async fn get_and_list_claims() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let doc = sample_document();
    storage.insert_document(&doc).await.unwrap();

    let claim = storage.get_claim(&ClaimId("c1".into())).await.unwrap();
    assert!(claim.is_some());
    let claim = claim.unwrap();
    assert_eq!(claim.text, "JWT tokens use RS256 signing.");
    assert_eq!(claim.section_id, SectionId("s1".into()));

    let claims = storage.list_claims(&SectionId("s1".into())).await.unwrap();
    assert_eq!(claims.len(), 2);
    assert_eq!(claims[0].id, ClaimId("c1".into()));
    assert_eq!(claims[1].id, ClaimId("c2".into()));

    let claims_s2 = storage.list_claims(&SectionId("s2".into())).await.unwrap();
    assert_eq!(claims_s2.len(), 1);
}

#[tokio::test]
async fn get_nonexistent_returns_none() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    assert!(
        storage
            .get_document(&ContentId("nope".into()))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .get_section(&SectionId("nope".into()))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .get_claim(&ClaimId("nope".into()))
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn file_hash_crud() {
    use iris_core::storage::traits::FileHashRecord;

    let storage = SqliteStorage::open_in_memory().unwrap();

    // Insert
    let record = FileHashRecord {
        path: "docs/api.md".into(),
        content_hash: "abc123".into(),
    };
    storage.upsert_file_hash(&record).await.unwrap();

    // Get
    let retrieved = storage.get_file_hash("docs/api.md").await.unwrap();
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.content_hash, "abc123");

    // Update (upsert)
    let updated = FileHashRecord {
        path: "docs/api.md".into(),
        content_hash: "def456".into(),
    };
    storage.upsert_file_hash(&updated).await.unwrap();
    let retrieved = storage.get_file_hash("docs/api.md").await.unwrap().unwrap();
    assert_eq!(retrieved.content_hash, "def456");

    // Delete
    let deleted = storage.delete_file_hash("docs/api.md").await.unwrap();
    assert!(deleted);
    assert!(
        storage
            .get_file_hash("docs/api.md")
            .await
            .unwrap()
            .is_none()
    );

    // Delete nonexistent
    let deleted = storage.delete_file_hash("nope.md").await.unwrap();
    assert!(!deleted);
}

#[tokio::test]
async fn concurrent_reads() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let doc = sample_document();
    storage.insert_document(&doc).await.unwrap();

    // Spawn multiple concurrent read tasks
    let mut handles = vec![];
    for _ in 0..10 {
        let s = storage.clone();
        let id = doc.id.clone();
        handles.push(tokio::spawn(async move {
            let result = s.get_document(&id).await.unwrap();
            assert!(result.is_some());
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }
}

#[tokio::test]
async fn on_disk_database_persists() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_path_buf();

    // Write
    {
        let storage = SqliteStorage::open(&db_path).unwrap();
        let doc = sample_document();
        storage.insert_document(&doc).await.unwrap();
    }

    // Re-open and verify
    {
        let storage = SqliteStorage::open(&db_path).unwrap();
        let doc = storage
            .get_document(&ContentId("doc-1".into()))
            .await
            .unwrap();
        assert!(doc.is_some());
        assert_eq!(doc.unwrap().title, "API Reference");
    }
}

#[tokio::test]
async fn migration_rollforward() {
    use iris_core::storage::CURRENT_SCHEMA_VERSION;

    let tmp = tempfile::NamedTempFile::new().unwrap();

    // Open creates the schema at the current version
    let storage = SqliteStorage::open(tmp.path()).unwrap();
    let doc = sample_document();
    storage.insert_document(&doc).await.unwrap();

    // Re-opening should succeed (migrations are idempotent)
    let storage2 = SqliteStorage::open(tmp.path()).unwrap();
    let docs = storage2.list_documents().await.unwrap();
    assert_eq!(docs.len(), 1);

    // Current version should match
    assert_eq!(CURRENT_SCHEMA_VERSION, 8);
}

#[tokio::test]
async fn nested_sections_are_stored() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    let doc = DocumentTree {
        id: ContentId("doc-nested".into()),
        title: "Nested".into(),
        source_path: "nested.md".into(),
        sections: vec![Section {
            id: SectionId("parent".into()),
            heading_path: vec!["Parent".into()],
            depth: 1,
            text: "Parent section.".into(),
            structural_nodes: vec![],
            children: vec![Section {
                id: SectionId("child".into()),
                heading_path: vec!["Parent".into(), "Child".into()],
                depth: 2,
                text: "Child section.".into(),
                structural_nodes: vec![],
                children: vec![],
                claims: vec![],
                summary: None,
            }],
            claims: vec![],
            summary: None,
        }],
        summary: None,
    };

    storage.insert_document(&doc).await.unwrap();

    let sections = storage.list_sections(&doc.id).await.unwrap();
    assert_eq!(sections.len(), 2);
    assert_eq!(sections[0].id, SectionId("parent".into()));
    assert_eq!(sections[1].id, SectionId("child".into()));
}

// --- Session persistence tests ---

fn make_test_session() -> Session {
    let mut session = Session::new(
        SessionId::from("test-session".to_string()),
        100_000,
        EvictionPolicy::Fifo,
    );

    session.record_delivery(
        &ContentId::from("s1".to_string()),
        Resolution::Section,
        200,
        1,
        "hash1".to_string(),
    );
    session.record_delivery(
        &ContentId::from("s2".to_string()),
        Resolution::Summary,
        100,
        1,
        "hash2".to_string(),
    );
    session.record_delivery(
        &ContentId::from("c1".to_string()),
        Resolution::Claim,
        30,
        2,
        "hash3".to_string(),
    );

    session
}

#[tokio::test]
async fn save_and_load_session_roundtrip() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let session = make_test_session();

    storage.save_session(&session).await.unwrap();

    let loaded = storage
        .load_session(&SessionId::from("test-session".to_string()))
        .await
        .unwrap();
    assert!(loaded.is_some());

    let loaded = loaded.unwrap();
    assert_eq!(loaded.id.0, "test-session");
    assert_eq!(loaded.agent_context_budget, 100_000);
    assert_eq!(loaded.current_turn(), 2);
    assert_eq!(loaded.delivered_count(), 3);
    assert_eq!(loaded.total_delivered_tokens(), 330);

    // Verify individual items
    assert!(loaded.is_delivered(&ContentId::from("s1".to_string())));
    assert!(loaded.is_delivered(&ContentId::from("s2".to_string())));
    assert!(loaded.is_delivered(&ContentId::from("c1".to_string())));

    let item = loaded
        .get_delivered(&ContentId::from("s1".to_string()))
        .unwrap();
    assert_eq!(item.resolution, Resolution::Section);
    assert_eq!(item.token_count, 200);
    assert_eq!(item.turn_delivered, 1);
    assert_eq!(item.content_hash, "hash1");
}

#[tokio::test]
async fn save_session_overwrites_on_re_save() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let mut session = make_test_session();

    storage.save_session(&session).await.unwrap();

    // Add another delivery and re-save
    session.record_delivery(
        &ContentId::from("s3".to_string()),
        Resolution::Section,
        150,
        3,
        "hash4".to_string(),
    );
    storage.save_session(&session).await.unwrap();

    let loaded = storage
        .load_session(&SessionId::from("test-session".to_string()))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(loaded.delivered_count(), 4);
    assert_eq!(loaded.current_turn(), 3);
    assert!(loaded.is_delivered(&ContentId::from("s3".to_string())));
}

#[tokio::test]
async fn load_nonexistent_session_returns_none() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    let loaded = storage
        .load_session(&SessionId::from("nonexistent".to_string()))
        .await
        .unwrap();
    assert!(loaded.is_none());
}

#[tokio::test]
async fn delete_session_removes_all_data() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let session = make_test_session();

    storage.save_session(&session).await.unwrap();

    let deleted = storage
        .delete_session(&SessionId::from("test-session".to_string()))
        .await
        .unwrap();
    assert!(deleted);

    let loaded = storage
        .load_session(&SessionId::from("test-session".to_string()))
        .await
        .unwrap();
    assert!(loaded.is_none());
}

#[tokio::test]
async fn delete_nonexistent_session_returns_false() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    let deleted = storage
        .delete_session(&SessionId::from("nonexistent".to_string()))
        .await
        .unwrap();
    assert!(!deleted);
}

#[tokio::test]
async fn session_trajectory_ordering_preserved() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let mut session = Session::new(
        SessionId::from("traj-test".to_string()),
        50_000,
        EvictionPolicy::Fifo,
    );

    // Deliver in specific order
    session.record_delivery(
        &ContentId::from("alpha".to_string()),
        Resolution::Section,
        100,
        1,
        "h1".to_string(),
    );
    session.record_delivery(
        &ContentId::from("beta".to_string()),
        Resolution::Section,
        100,
        1,
        "h2".to_string(),
    );
    session.record_delivery(
        &ContentId::from("gamma".to_string()),
        Resolution::Section,
        100,
        2,
        "h3".to_string(),
    );

    storage.save_session(&session).await.unwrap();

    let loaded = storage
        .load_session(&SessionId::from("traj-test".to_string()))
        .await
        .unwrap()
        .unwrap();

    let trajectory = loaded.trajectory();
    assert_eq!(trajectory.len(), 3);
    assert_eq!(trajectory[0], ContentId::from("alpha".to_string()));
    assert_eq!(trajectory[1], ContentId::from("beta".to_string()));
    assert_eq!(trajectory[2], ContentId::from("gamma".to_string()));
}

#[tokio::test]
async fn session_persists_across_db_reopens() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let db_path = tmp.path().to_path_buf();

    // Save session
    {
        let storage = SqliteStorage::open(&db_path).unwrap();
        let session = make_test_session();
        storage.save_session(&session).await.unwrap();
    }

    // Reopen and verify
    {
        let storage = SqliteStorage::open(&db_path).unwrap();
        let loaded = storage
            .load_session(&SessionId::from("test-session".to_string()))
            .await
            .unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.delivered_count(), 3);
        assert_eq!(loaded.agent_context_budget, 100_000);
    }
}

#[tokio::test]
async fn session_has_changed_works_after_restore() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let session = make_test_session();
    storage.save_session(&session).await.unwrap();

    let loaded = storage
        .load_session(&SessionId::from("test-session".to_string()))
        .await
        .unwrap()
        .unwrap();

    // Content hash should be preserved
    assert!(!loaded.has_changed(&ContentId::from("s1".to_string()), "hash1"));
    assert!(loaded.has_changed(&ContentId::from("s1".to_string()), "different"));
}

// --- get_next_section tests ---

#[tokio::test]
async fn get_next_section_returns_next_by_position() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let doc = sample_document();
    storage.insert_document(&doc).await.unwrap();

    // s1 is position 0, s2 is position 1
    let next = storage
        .get_next_section(&SectionId("s1".into()))
        .await
        .unwrap();
    assert!(next.is_some());
    let next = next.unwrap();
    assert_eq!(next.id, SectionId("s2".into()));
    assert_eq!(
        next.text,
        "Rate limits are 100 requests per minute per API key."
    );
}

#[tokio::test]
async fn get_next_section_returns_none_for_last_section() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let doc = sample_document();
    storage.insert_document(&doc).await.unwrap();

    // s2 is the last section — no next
    let next = storage
        .get_next_section(&SectionId("s2".into()))
        .await
        .unwrap();
    assert!(next.is_none());
}

#[tokio::test]
async fn get_next_section_returns_none_for_nonexistent() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    let next = storage
        .get_next_section(&SectionId("nonexistent".into()))
        .await
        .unwrap();
    assert!(next.is_none());
}

#[tokio::test]
async fn get_next_section_scoped_to_document() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    // Insert two documents with sections
    let doc1 = sample_document();
    let doc2 = DocumentTree {
        id: ContentId("doc-2".into()),
        title: "Other Doc".into(),
        source_path: "other.md".into(),
        sections: vec![Section {
            id: SectionId("other-s1".into()),
            heading_path: vec!["Other".into()],
            depth: 1,
            text: "Other document section.".into(),
            structural_nodes: vec![],
            children: vec![],
            claims: vec![],
            summary: None,
        }],
        summary: None,
    };

    storage.insert_document(&doc1).await.unwrap();
    storage.insert_document(&doc2).await.unwrap();

    // s2 is the last section in doc-1, should not return other-s1 from doc-2
    let next = storage
        .get_next_section(&SectionId("s2".into()))
        .await
        .unwrap();
    assert!(next.is_none());
}

// --- get_document_for_section tests ---

#[tokio::test]
async fn get_document_for_section_returns_parent_doc() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let doc = sample_document();
    storage.insert_document(&doc).await.unwrap();

    let parent = storage
        .get_document_for_section(&SectionId("s1".into()))
        .await
        .unwrap();
    assert!(parent.is_some());
    let parent = parent.unwrap();
    assert_eq!(parent.id, ContentId("doc-1".into()));
    assert_eq!(parent.title, "API Reference");
    assert_eq!(
        parent.summary.as_deref(),
        Some("Full API reference documentation.")
    );
}

#[tokio::test]
async fn get_document_for_section_returns_none_for_nonexistent() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    let parent = storage
        .get_document_for_section(&SectionId("nonexistent".into()))
        .await
        .unwrap();
    assert!(parent.is_none());
}

// --- Claim relationship tests ---

#[tokio::test]
async fn insert_and_query_claim_relationships() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let doc = sample_document();
    storage.insert_document(&doc).await.unwrap();

    let relationships = vec![ClaimRelationship {
        source_claim_id: ClaimId("c1".into()),
        target_claim_id: ClaimId("c3".into()),
        relation_type: RelationType::References,
        confidence: 0.85,
    }];

    storage
        .insert_claim_relationships(&relationships)
        .await
        .unwrap();

    // Query from source side
    let related = storage
        .get_related_claims(&ClaimId("c1".into()), None)
        .await
        .unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].claim_id, ClaimId("c3".into()));
    assert_eq!(related[0].relation_type, RelationType::References);
    assert!((related[0].confidence - 0.85).abs() < f32::EPSILON);

    // Query from target side (bidirectional)
    let related = storage
        .get_related_claims(&ClaimId("c3".into()), None)
        .await
        .unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].claim_id, ClaimId("c1".into()));
}

#[tokio::test]
async fn query_relationships_with_type_filter() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let doc = sample_document();
    storage.insert_document(&doc).await.unwrap();

    let relationships = vec![
        ClaimRelationship {
            source_claim_id: ClaimId("c1".into()),
            target_claim_id: ClaimId("c3".into()),
            relation_type: RelationType::References,
            confidence: 0.8,
        },
        ClaimRelationship {
            source_claim_id: ClaimId("c1".into()),
            target_claim_id: ClaimId("c2".into()),
            relation_type: RelationType::Updates,
            confidence: 0.7,
        },
    ];

    storage
        .insert_claim_relationships(&relationships)
        .await
        .unwrap();

    // Filter to only References
    let related = storage
        .get_related_claims(&ClaimId("c1".into()), Some(&[RelationType::References]))
        .await
        .unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].relation_type, RelationType::References);

    // Filter to only Updates
    let related = storage
        .get_related_claims(&ClaimId("c1".into()), Some(&[RelationType::Updates]))
        .await
        .unwrap();
    assert_eq!(related.len(), 1);
    assert_eq!(related[0].relation_type, RelationType::Updates);
}

#[tokio::test]
async fn delete_relationships_for_section_cleans_up() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let doc = sample_document();
    storage.insert_document(&doc).await.unwrap();

    let relationships = vec![ClaimRelationship {
        source_claim_id: ClaimId("c1".into()),
        target_claim_id: ClaimId("c3".into()),
        relation_type: RelationType::References,
        confidence: 0.9,
    }];

    storage
        .insert_claim_relationships(&relationships)
        .await
        .unwrap();

    // Delete relationships for section s1 (contains c1 and c2)
    storage
        .delete_relationships_for_section(&SectionId("s1".into()))
        .await
        .unwrap();

    // Relationship should be gone
    let related = storage
        .get_related_claims(&ClaimId("c1".into()), None)
        .await
        .unwrap();
    assert!(related.is_empty());

    let related = storage
        .get_related_claims(&ClaimId("c3".into()), None)
        .await
        .unwrap();
    assert!(related.is_empty());
}

#[tokio::test]
async fn no_relationships_returns_empty() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let doc = sample_document();
    storage.insert_document(&doc).await.unwrap();

    let related = storage
        .get_related_claims(&ClaimId("c1".into()), None)
        .await
        .unwrap();
    assert!(related.is_empty());
}

// --- Web cache tests ---

#[tokio::test]
async fn web_cache_crud() {
    use iris_core::storage::WebCacheRecord;

    let storage = SqliteStorage::open_in_memory().unwrap();

    let record = WebCacheRecord {
        source_url: "https://example.com/docs/".into(),
        fetch_timestamp: "2026-03-21T12:00:00Z".into(),
        etag: Some("\"abc123\"".into()),
        last_modified: Some("Fri, 20 Mar 2026 10:00:00 GMT".into()),
        content_hash: "deadbeef".into(),
        content_type: Some("text/html".into()),
    };

    // Insert
    storage.upsert_web_cache(&record).await.unwrap();

    // Get
    let retrieved = storage
        .get_web_cache("https://example.com/docs/")
        .await
        .unwrap();
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.source_url, "https://example.com/docs/");
    assert_eq!(retrieved.etag.as_deref(), Some("\"abc123\""));
    assert_eq!(
        retrieved.last_modified.as_deref(),
        Some("Fri, 20 Mar 2026 10:00:00 GMT")
    );
    assert_eq!(retrieved.content_hash, "deadbeef");

    // Update (upsert with new ETag)
    let updated = WebCacheRecord {
        source_url: "https://example.com/docs/".into(),
        fetch_timestamp: "2026-03-21T13:00:00Z".into(),
        etag: Some("\"def456\"".into()),
        last_modified: Some("Fri, 21 Mar 2026 13:00:00 GMT".into()),
        content_hash: "newcafe".into(),
        content_type: Some("text/html".into()),
    };
    storage.upsert_web_cache(&updated).await.unwrap();
    let retrieved = storage
        .get_web_cache("https://example.com/docs/")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retrieved.etag.as_deref(), Some("\"def456\""));
    assert_eq!(retrieved.content_hash, "newcafe");

    // List
    let all = storage.list_web_cache().await.unwrap();
    assert_eq!(all.len(), 1);

    // Delete
    let deleted = storage
        .delete_web_cache("https://example.com/docs/")
        .await
        .unwrap();
    assert!(deleted);
    assert!(
        storage
            .get_web_cache("https://example.com/docs/")
            .await
            .unwrap()
            .is_none()
    );

    // Delete nonexistent
    let deleted = storage
        .delete_web_cache("https://example.com/missing/")
        .await
        .unwrap();
    assert!(!deleted);
}

#[tokio::test]
async fn web_cache_list_multiple() {
    use iris_core::storage::WebCacheRecord;

    let storage = SqliteStorage::open_in_memory().unwrap();

    for i in 0..3 {
        let record = WebCacheRecord {
            source_url: format!("https://example.com/page-{i}/"),
            fetch_timestamp: format!("2026-03-21T1{i}:00:00Z"),
            etag: None,
            last_modified: None,
            content_hash: format!("hash{i}"),
            content_type: None,
        };
        storage.upsert_web_cache(&record).await.unwrap();
    }

    let all = storage.list_web_cache().await.unwrap();
    assert_eq!(all.len(), 3);
}

#[tokio::test]
async fn web_cache_optional_fields_are_nullable() {
    use iris_core::storage::WebCacheRecord;

    let storage = SqliteStorage::open_in_memory().unwrap();

    let record = WebCacheRecord {
        source_url: "https://example.com/".into(),
        fetch_timestamp: "2026-03-21T12:00:00Z".into(),
        etag: None,
        last_modified: None,
        content_hash: "abcdef".into(),
        content_type: None,
    };
    storage.upsert_web_cache(&record).await.unwrap();

    let retrieved = storage
        .get_web_cache("https://example.com/")
        .await
        .unwrap()
        .unwrap();
    assert!(retrieved.etag.is_none());
    assert!(retrieved.last_modified.is_none());
    assert!(retrieved.content_type.is_none());
}

// --- Git cache CRUD tests ---

#[tokio::test]
async fn git_cache_upsert_and_get() {
    use iris_core::storage::GitCacheRecord;

    let storage = SqliteStorage::open_in_memory().unwrap();

    let record = GitCacheRecord {
        repo_url: "https://github.com/user/repo.git".into(),
        branch: Some("main".into()),
        commit_sha: "abc123def456".into(),
        clone_timestamp: "1711036800".into(),
        clone_dir: "/tmp/iris/remote/abcdef1234567890".into(),
        checked_out_paths: vec!["docs".into(), "src".into()],
    };
    storage.upsert_git_cache(&record).await.unwrap();

    let retrieved = storage
        .get_git_cache("https://github.com/user/repo.git")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retrieved.repo_url, record.repo_url);
    assert_eq!(retrieved.branch, record.branch);
    assert_eq!(retrieved.commit_sha, record.commit_sha);
    assert_eq!(retrieved.clone_dir, record.clone_dir);
    assert_eq!(retrieved.checked_out_paths, record.checked_out_paths);
}

#[tokio::test]
async fn git_cache_upsert_updates_on_conflict() {
    use iris_core::storage::GitCacheRecord;

    let storage = SqliteStorage::open_in_memory().unwrap();
    let repo_url = "https://github.com/user/repo.git";

    let record_v1 = GitCacheRecord {
        repo_url: repo_url.into(),
        branch: Some("main".into()),
        commit_sha: "sha_v1".into(),
        clone_timestamp: "1000".into(),
        clone_dir: "/tmp/clone-v1".into(),
        checked_out_paths: vec![],
    };
    storage.upsert_git_cache(&record_v1).await.unwrap();

    let record_v2 = GitCacheRecord {
        repo_url: repo_url.into(),
        branch: Some("main".into()),
        commit_sha: "sha_v2".into(),
        clone_timestamp: "2000".into(),
        clone_dir: "/tmp/clone-v2".into(),
        checked_out_paths: vec!["docs".into()],
    };
    storage.upsert_git_cache(&record_v2).await.unwrap();

    let retrieved = storage.get_git_cache(repo_url).await.unwrap().unwrap();
    assert_eq!(retrieved.commit_sha, "sha_v2");
    assert_eq!(retrieved.clone_dir, "/tmp/clone-v2");
    assert_eq!(retrieved.checked_out_paths, vec!["docs".to_string()]);
}

#[tokio::test]
async fn git_cache_list_returns_all() {
    use iris_core::storage::GitCacheRecord;

    let storage = SqliteStorage::open_in_memory().unwrap();

    for i in 0..3 {
        let record = GitCacheRecord {
            repo_url: format!("https://github.com/user/repo-{i}.git"),
            branch: None,
            commit_sha: format!("sha_{i}"),
            clone_timestamp: format!("{}", 1000 + i),
            clone_dir: format!("/tmp/clone-{i}"),
            checked_out_paths: vec![],
        };
        storage.upsert_git_cache(&record).await.unwrap();
    }

    let all = storage.list_git_cache().await.unwrap();
    assert_eq!(all.len(), 3);
}

#[tokio::test]
async fn git_cache_delete() {
    use iris_core::storage::GitCacheRecord;

    let storage = SqliteStorage::open_in_memory().unwrap();
    let repo_url = "https://github.com/user/repo.git";

    let record = GitCacheRecord {
        repo_url: repo_url.into(),
        branch: None,
        commit_sha: "sha".into(),
        clone_timestamp: "1000".into(),
        clone_dir: "/tmp/clone".into(),
        checked_out_paths: vec![],
    };
    storage.upsert_git_cache(&record).await.unwrap();

    let deleted = storage.delete_git_cache(repo_url).await.unwrap();
    assert!(deleted);

    let retrieved = storage.get_git_cache(repo_url).await.unwrap();
    assert!(retrieved.is_none());

    let deleted_again = storage.delete_git_cache(repo_url).await.unwrap();
    assert!(!deleted_again);
}

#[tokio::test]
async fn git_cache_get_nonexistent_returns_none() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let result = storage
        .get_git_cache("https://github.com/user/nope.git")
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn git_cache_no_branch_is_nullable() {
    use iris_core::storage::GitCacheRecord;

    let storage = SqliteStorage::open_in_memory().unwrap();

    let record = GitCacheRecord {
        repo_url: "https://github.com/user/repo.git".into(),
        branch: None,
        commit_sha: "sha".into(),
        clone_timestamp: "1000".into(),
        clone_dir: "/tmp/clone".into(),
        checked_out_paths: vec![],
    };
    storage.upsert_git_cache(&record).await.unwrap();

    let retrieved = storage
        .get_git_cache("https://github.com/user/repo.git")
        .await
        .unwrap()
        .unwrap();
    assert!(retrieved.branch.is_none());
    assert!(retrieved.checked_out_paths.is_empty());
}

// --- Symbol storage tests ---

fn sample_symbols() -> Vec<SymbolRecord> {
    vec![
        SymbolRecord {
            id: SymbolId("sym-config::IrisConfig".into()),
            file_path: "src/config.rs".into(),
            name: "IrisConfig".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            signature: "pub struct IrisConfig".into(),
            doc_comment: Some("Configuration for iris.".into()),
            module_path: "config".into(),
            line_start: 10,
            line_end: 25,
            cyclomatic_complexity: None,
        },
        SymbolRecord {
            id: SymbolId("sym-config::PrefetchConfig".into()),
            file_path: "src/config.rs".into(),
            name: "PrefetchConfig".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            signature: "pub struct PrefetchConfig".into(),
            doc_comment: None,
            module_path: "config".into(),
            line_start: 30,
            line_end: 40,
            cyclomatic_complexity: None,
        },
        SymbolRecord {
            id: SymbolId("sym-service::run".into()),
            file_path: "src/service.rs".into(),
            name: "run".into(),
            kind: "function".into(),
            visibility: "pub".into(),
            signature: "pub async fn run(config: &IrisConfig) -> Result<()>".into(),
            doc_comment: Some("Starts the service.".into()),
            module_path: "service".into(),
            line_start: 5,
            line_end: 50,
            cyclomatic_complexity: None,
        },
        SymbolRecord {
            id: SymbolId("sym-service::helper".into()),
            file_path: "src/service.rs".into(),
            name: "helper".into(),
            kind: "function".into(),
            visibility: String::new(),
            signature: "fn helper()".into(),
            doc_comment: None,
            module_path: "service".into(),
            line_start: 55,
            line_end: 60,
            cyclomatic_complexity: None,
        },
        SymbolRecord {
            id: SymbolId("sym-storage::Storage".into()),
            file_path: "src/storage/traits.rs".into(),
            name: "Storage".into(),
            kind: "trait".into(),
            visibility: "pub".into(),
            signature: "pub trait Storage: Send + Sync".into(),
            doc_comment: Some("Async storage interface.".into()),
            module_path: "storage::traits".into(),
            line_start: 1,
            line_end: 100,
            cyclomatic_complexity: None,
        },
    ]
}

#[tokio::test]
async fn insert_and_get_symbol() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let symbols = sample_symbols();
    storage.insert_symbols(&symbols).await.unwrap();

    let sym = storage
        .get_symbol(&SymbolId("sym-config::IrisConfig".into()))
        .await
        .unwrap();
    assert!(sym.is_some());
    let sym = sym.unwrap();
    assert_eq!(sym.name, "IrisConfig");
    assert_eq!(sym.kind, "struct");
    assert_eq!(sym.visibility, "pub");
    assert_eq!(sym.file_path, "src/config.rs");
    assert_eq!(sym.module_path, "config");
    assert_eq!(sym.line_start, 10);
    assert_eq!(sym.line_end, 25);
    assert_eq!(sym.doc_comment.as_deref(), Some("Configuration for iris."));
}

#[tokio::test]
async fn get_nonexistent_symbol_returns_none() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let result = storage
        .get_symbol(&SymbolId("nonexistent".into()))
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn list_symbols_no_filter_returns_all() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    let symbols = sample_symbols();
    storage.insert_symbols(&symbols).await.unwrap();

    let all = storage
        .list_symbols(&SymbolFilter::default())
        .await
        .unwrap();
    assert_eq!(all.len(), 5);
}

#[tokio::test]
async fn list_symbols_filter_by_kind() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.insert_symbols(&sample_symbols()).await.unwrap();

    let structs = storage
        .list_symbols(&SymbolFilter {
            kind: Some("struct".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(structs.len(), 2);

    let functions = storage
        .list_symbols(&SymbolFilter {
            kind: Some("function".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(functions.len(), 2);

    let traits = storage
        .list_symbols(&SymbolFilter {
            kind: Some("trait".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(traits.len(), 1);
    assert_eq!(traits[0].name, "Storage");
}

#[tokio::test]
async fn list_symbols_filter_by_visibility() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.insert_symbols(&sample_symbols()).await.unwrap();

    let public = storage
        .list_symbols(&SymbolFilter {
            visibility: Some("pub".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(public.len(), 4);

    let private = storage
        .list_symbols(&SymbolFilter {
            visibility: Some(String::new()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(private.len(), 1);
    assert_eq!(private[0].name, "helper");
}

#[tokio::test]
async fn list_symbols_filter_by_module() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.insert_symbols(&sample_symbols()).await.unwrap();

    // Exact match
    let config = storage
        .list_symbols(&SymbolFilter {
            module: Some("config".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(config.len(), 2);

    // Prefix match: "storage" should match "storage::traits"
    let storage_mod = storage
        .list_symbols(&SymbolFilter {
            module: Some("storage".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(storage_mod.len(), 1);
    assert_eq!(storage_mod[0].name, "Storage");

    // Exact match for nested module
    let traits = storage
        .list_symbols(&SymbolFilter {
            module: Some("storage::traits".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(traits.len(), 1);
}

#[tokio::test]
async fn list_symbols_filter_by_name() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.insert_symbols(&sample_symbols()).await.unwrap();

    let results = storage
        .list_symbols(&SymbolFilter {
            name: Some("Config".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|s| s.name.contains("Config")));
}

#[tokio::test]
async fn list_symbols_combined_filters() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.insert_symbols(&sample_symbols()).await.unwrap();

    let results = storage
        .list_symbols(&SymbolFilter {
            kind: Some("function".into()),
            visibility: Some("pub".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "run");
}

#[tokio::test]
async fn delete_symbols_for_file() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.insert_symbols(&sample_symbols()).await.unwrap();

    let deleted = storage
        .delete_symbols_for_file("src/config.rs")
        .await
        .unwrap();
    assert_eq!(deleted, 2);

    let remaining = storage
        .list_symbols(&SymbolFilter::default())
        .await
        .unwrap();
    assert_eq!(remaining.len(), 3);
    assert!(remaining.iter().all(|s| s.file_path != "src/config.rs"));
}

#[tokio::test]
async fn insert_symbols_upsert_on_conflict() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    let sym = SymbolRecord {
        id: SymbolId("sym-foo".into()),
        file_path: "src/foo.rs".into(),
        name: "foo".into(),
        kind: "function".into(),
        visibility: "pub".into(),
        signature: "pub fn foo()".into(),
        doc_comment: None,
        module_path: "foo".into(),
        line_start: 1,
        line_end: 5,
        cyclomatic_complexity: None,
    };
    storage.insert_symbols(&[sym]).await.unwrap();

    // Update with new signature
    let updated = SymbolRecord {
        id: SymbolId("sym-foo".into()),
        file_path: "src/foo.rs".into(),
        name: "foo".into(),
        kind: "function".into(),
        visibility: "pub".into(),
        signature: "pub fn foo() -> i32".into(),
        doc_comment: Some("Returns an int.".into()),
        module_path: "foo".into(),
        line_start: 1,
        line_end: 8,
        cyclomatic_complexity: None,
    };
    storage.insert_symbols(&[updated]).await.unwrap();

    let result = storage
        .get_symbol(&SymbolId("sym-foo".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.signature, "pub fn foo() -> i32");
    assert_eq!(result.doc_comment.as_deref(), Some("Returns an int."));
    assert_eq!(result.line_end, 8);
}

// --- Symbol refs tests ---

#[tokio::test]
async fn insert_and_query_symbol_refs() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.insert_symbols(&sample_symbols()).await.unwrap();

    let refs = vec![
        SymbolRefRecord {
            from_symbol_id: SymbolId("sym-service::run".into()),
            to_symbol_id: SymbolId("sym-config::IrisConfig".into()),
            ref_kind: RefKind::Uses,
        },
        SymbolRefRecord {
            from_symbol_id: SymbolId("sym-service::run".into()),
            to_symbol_id: SymbolId("sym-service::helper".into()),
            ref_kind: RefKind::Calls,
        },
    ];
    storage.insert_symbol_refs(&refs).await.unwrap();

    // Query all refs for `run`
    let all_refs = storage
        .query_refs(&SymbolId("sym-service::run".into()), None)
        .await
        .unwrap();
    assert_eq!(all_refs.len(), 2);

    // Query only Calls refs
    let calls = storage
        .query_refs(&SymbolId("sym-service::run".into()), Some(RefKind::Calls))
        .await
        .unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].to_symbol_id,
        SymbolId("sym-service::helper".into())
    );

    // Query from the target side — IrisConfig should appear in Uses refs
    let uses = storage
        .query_refs(
            &SymbolId("sym-config::IrisConfig".into()),
            Some(RefKind::Uses),
        )
        .await
        .unwrap();
    assert_eq!(uses.len(), 1);
    assert_eq!(uses[0].from_symbol_id, SymbolId("sym-service::run".into()));
}

#[tokio::test]
async fn query_refs_empty_for_unreferenced_symbol() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.insert_symbols(&sample_symbols()).await.unwrap();

    let refs = storage
        .query_refs(&SymbolId("sym-storage::Storage".into()), None)
        .await
        .unwrap();
    assert!(refs.is_empty());
}

#[tokio::test]
async fn delete_refs_for_file_cascades() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.insert_symbols(&sample_symbols()).await.unwrap();

    let refs = vec![SymbolRefRecord {
        from_symbol_id: SymbolId("sym-service::run".into()),
        to_symbol_id: SymbolId("sym-config::IrisConfig".into()),
        ref_kind: RefKind::Uses,
    }];
    storage.insert_symbol_refs(&refs).await.unwrap();

    // Delete refs for service.rs
    storage
        .delete_refs_for_file("src/service.rs")
        .await
        .unwrap();

    // Ref should be gone from both sides
    let remaining = storage
        .query_refs(&SymbolId("sym-service::run".into()), None)
        .await
        .unwrap();
    assert!(remaining.is_empty());

    let remaining = storage
        .query_refs(&SymbolId("sym-config::IrisConfig".into()), None)
        .await
        .unwrap();
    assert!(remaining.is_empty());
}

#[tokio::test]
async fn symbol_refs_cascade_on_symbol_delete() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.insert_symbols(&sample_symbols()).await.unwrap();

    let refs = vec![SymbolRefRecord {
        from_symbol_id: SymbolId("sym-service::run".into()),
        to_symbol_id: SymbolId("sym-config::IrisConfig".into()),
        ref_kind: RefKind::Uses,
    }];
    storage.insert_symbol_refs(&refs).await.unwrap();

    // Delete symbols for service.rs — FK cascade should remove refs
    storage
        .delete_symbols_for_file("src/service.rs")
        .await
        .unwrap();

    let remaining = storage
        .query_refs(&SymbolId("sym-config::IrisConfig".into()), None)
        .await
        .unwrap();
    assert!(remaining.is_empty());
}

#[tokio::test]
async fn list_symbols_filter_by_file_path() {
    let storage = SqliteStorage::open_in_memory().unwrap();
    storage.insert_symbols(&sample_symbols()).await.unwrap();

    let results = storage
        .list_symbols(&SymbolFilter {
            file_path: Some("src/config.rs".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|s| s.file_path == "src/config.rs"));
}

// --- Transitive caller count tests ---

#[tokio::test]
async fn transitive_caller_counts_linear_chain() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    // Create a call chain: A → B → C
    let symbols = vec![
        SymbolRecord {
            id: SymbolId("sym-a".into()),
            file_path: "src/a.rs".into(),
            name: "a".into(),
            kind: "function".into(),
            visibility: "pub".into(),
            signature: "pub fn a()".into(),
            doc_comment: None,
            module_path: "a".into(),
            line_start: 1,
            line_end: 5,
            cyclomatic_complexity: Some(1),
        },
        SymbolRecord {
            id: SymbolId("sym-b".into()),
            file_path: "src/b.rs".into(),
            name: "b".into(),
            kind: "function".into(),
            visibility: "pub".into(),
            signature: "pub fn b()".into(),
            doc_comment: None,
            module_path: "b".into(),
            line_start: 1,
            line_end: 5,
            cyclomatic_complexity: Some(2),
        },
        SymbolRecord {
            id: SymbolId("sym-c".into()),
            file_path: "src/c.rs".into(),
            name: "c".into(),
            kind: "function".into(),
            visibility: "pub".into(),
            signature: "pub fn c()".into(),
            doc_comment: None,
            module_path: "c".into(),
            line_start: 1,
            line_end: 5,
            cyclomatic_complexity: Some(3),
        },
    ];
    storage.insert_symbols(&symbols).await.unwrap();

    let refs = vec![
        SymbolRefRecord {
            from_symbol_id: SymbolId("sym-a".into()),
            to_symbol_id: SymbolId("sym-b".into()),
            ref_kind: RefKind::Calls,
        },
        SymbolRefRecord {
            from_symbol_id: SymbolId("sym-b".into()),
            to_symbol_id: SymbolId("sym-c".into()),
            ref_kind: RefKind::Calls,
        },
    ];
    storage.insert_symbol_refs(&refs).await.unwrap();

    let counts = storage
        .transitive_caller_counts(&[
            SymbolId("sym-a".into()),
            SymbolId("sym-b".into()),
            SymbolId("sym-c".into()),
        ])
        .await
        .unwrap();

    // A has no callers
    assert_eq!(counts.get(&SymbolId("sym-a".into())), None);
    // B is called by A (1 caller)
    assert_eq!(counts.get(&SymbolId("sym-b".into())), Some(&1));
    // C is called by B, and transitively by A (2 callers)
    assert_eq!(counts.get(&SymbolId("sym-c".into())), Some(&2));
}

#[tokio::test]
async fn transitive_caller_counts_diamond() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    // Diamond: A → C, B → C, A → D → C
    let symbols: Vec<SymbolRecord> = ["a", "b", "c", "d"]
        .iter()
        .map(|name| SymbolRecord {
            id: SymbolId(format!("sym-{name}")),
            file_path: format!("src/{name}.rs"),
            name: (*name).to_string(),
            kind: "function".into(),
            visibility: "pub".into(),
            signature: format!("pub fn {name}()"),
            doc_comment: None,
            module_path: (*name).to_string(),
            line_start: 1,
            line_end: 5,
            cyclomatic_complexity: Some(1),
        })
        .collect();
    storage.insert_symbols(&symbols).await.unwrap();

    let refs = vec![
        SymbolRefRecord {
            from_symbol_id: SymbolId("sym-a".into()),
            to_symbol_id: SymbolId("sym-c".into()),
            ref_kind: RefKind::Calls,
        },
        SymbolRefRecord {
            from_symbol_id: SymbolId("sym-b".into()),
            to_symbol_id: SymbolId("sym-c".into()),
            ref_kind: RefKind::Calls,
        },
        SymbolRefRecord {
            from_symbol_id: SymbolId("sym-a".into()),
            to_symbol_id: SymbolId("sym-d".into()),
            ref_kind: RefKind::Calls,
        },
        SymbolRefRecord {
            from_symbol_id: SymbolId("sym-d".into()),
            to_symbol_id: SymbolId("sym-c".into()),
            ref_kind: RefKind::Calls,
        },
    ];
    storage.insert_symbol_refs(&refs).await.unwrap();

    let counts = storage
        .transitive_caller_counts(&[SymbolId("sym-c".into())])
        .await
        .unwrap();

    // C is called by A (direct), B (direct), D (direct), and A transitively via D
    // Unique transitive callers: A, B, D = 3
    assert_eq!(counts.get(&SymbolId("sym-c".into())), Some(&3));
}

#[tokio::test]
async fn cyclomatic_complexity_stored_and_retrieved() {
    let storage = SqliteStorage::open_in_memory().unwrap();

    let symbols = vec![
        SymbolRecord {
            id: SymbolId("sym-fn-simple".into()),
            file_path: "src/lib.rs".into(),
            name: "simple".into(),
            kind: "function".into(),
            visibility: "pub".into(),
            signature: "pub fn simple()".into(),
            doc_comment: None,
            module_path: String::new(),
            line_start: 1,
            line_end: 3,
            cyclomatic_complexity: Some(1),
        },
        SymbolRecord {
            id: SymbolId("sym-fn-complex".into()),
            file_path: "src/lib.rs".into(),
            name: "complex".into(),
            kind: "function".into(),
            visibility: "pub".into(),
            signature: "pub fn complex()".into(),
            doc_comment: None,
            module_path: String::new(),
            line_start: 5,
            line_end: 30,
            cyclomatic_complexity: Some(10),
        },
        SymbolRecord {
            id: SymbolId("sym-struct-foo".into()),
            file_path: "src/lib.rs".into(),
            name: "Foo".into(),
            kind: "struct".into(),
            visibility: "pub".into(),
            signature: "pub struct Foo".into(),
            doc_comment: None,
            module_path: String::new(),
            line_start: 32,
            line_end: 35,
            cyclomatic_complexity: None,
        },
    ];
    storage.insert_symbols(&symbols).await.unwrap();

    let simple = storage
        .get_symbol(&SymbolId("sym-fn-simple".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(simple.cyclomatic_complexity, Some(1));

    let complex = storage
        .get_symbol(&SymbolId("sym-fn-complex".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(complex.cyclomatic_complexity, Some(10));

    let struc = storage
        .get_symbol(&SymbolId("sym-struct-foo".into()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(struc.cyclomatic_complexity, None);
}
