//! Integration tests for the `SQLite` storage layer.
//!
//! These tests run against real `SQLite` databases (not mocks) to verify
//! CRUD operations, concurrent access, WAL behavior, and migrations.

use iris_core::storage::{SqliteStorage, Storage};
use iris_core::types::{Claim, ClaimId, ContentId, DocumentTree, Section, SectionId};

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
    assert_eq!(CURRENT_SCHEMA_VERSION, 1);
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
            children: vec![Section {
                id: SectionId("child".into()),
                heading_path: vec!["Parent".into(), "Child".into()],
                depth: 2,
                text: "Child section.".into(),
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
