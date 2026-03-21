//! Integration tests for the `SQLite` storage layer.
//!
//! These tests run against real `SQLite` databases (not mocks) to verify
//! CRUD operations, concurrent access, WAL behavior, and migrations.

use iris_core::session::{EvictionPolicy, Session, SessionId};
use iris_core::storage::{SqliteStorage, Storage};
use iris_core::types::{Claim, ClaimId, ContentId, DocumentTree, Resolution, Section, SectionId};

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
    assert_eq!(CURRENT_SCHEMA_VERSION, 2);
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
