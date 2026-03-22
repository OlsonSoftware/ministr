//! The [`Storage`] trait defines the async persistence interface for iris-core.
//!
//! All operations are async to allow implementations to use `spawn_blocking`
//! or other async-safe wrappers around synchronous backends like `SQLite`.

use std::future::Future;

use crate::error::StorageError;
use crate::session::{Session, SessionId};
use crate::types::{ClaimId, ClaimRelationship, ContentId, DocumentTree, RelationType, SectionId};

/// Stored document metadata (without the full section tree).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentRecord {
    /// Unique document ID.
    pub id: ContentId,
    /// Document title.
    pub title: String,
    /// Source file path relative to corpus root.
    pub source_path: String,
    /// Document-level summary.
    pub summary: Option<String>,
}

/// Stored section with its metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SectionRecord {
    /// Unique section ID.
    pub id: SectionId,
    /// Parent document ID.
    pub document_id: ContentId,
    /// Heading hierarchy path.
    pub heading_path: Vec<String>,
    /// Heading depth.
    pub depth: u32,
    /// Full text content.
    pub text: String,
    /// Section summary.
    pub summary: Option<String>,
    /// Ordering position within the document.
    pub position: i64,
}

/// Stored claim with its metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimRecord {
    /// Unique claim ID.
    pub id: ClaimId,
    /// Parent section ID.
    pub section_id: SectionId,
    /// Claim text.
    pub text: String,
    /// Ordering position within the section.
    pub position: i64,
}

/// A related claim record returned by relationship queries.
#[derive(Debug, Clone, PartialEq)]
pub struct RelatedClaimRecord {
    /// The related claim's ID.
    pub claim_id: ClaimId,
    /// The related claim's text.
    pub text: String,
    /// The type of relationship.
    pub relation_type: RelationType,
    /// The section containing the related claim.
    pub section_id: SectionId,
    /// Confidence score (0.0–1.0).
    pub confidence: f32,
}

/// A file hash record for incremental re-indexing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHashRecord {
    /// File path relative to corpus root.
    pub path: String,
    /// Content hash (e.g. SHA-256 hex).
    pub content_hash: String,
}

/// Async storage interface for the iris content database.
///
/// Implementations must be `Send + Sync` to work across async tasks.
pub trait Storage: Send + Sync {
    // -- Documents --

    /// Insert a full document tree (document + sections + claims).
    fn insert_document(
        &self,
        doc: &DocumentTree,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Get a document by ID (metadata only, no sections).
    fn get_document(
        &self,
        id: &ContentId,
    ) -> impl Future<Output = Result<Option<DocumentRecord>, StorageError>> + Send;

    /// List all documents in the corpus.
    fn list_documents(
        &self,
    ) -> impl Future<Output = Result<Vec<DocumentRecord>, StorageError>> + Send;

    /// Delete a document and all its sections/claims (cascading).
    fn delete_document(
        &self,
        id: &ContentId,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    // -- Sections --

    /// Get a section by ID.
    fn get_section(
        &self,
        id: &SectionId,
    ) -> impl Future<Output = Result<Option<SectionRecord>, StorageError>> + Send;

    /// List all sections for a document.
    fn list_sections(
        &self,
        document_id: &ContentId,
    ) -> impl Future<Output = Result<Vec<SectionRecord>, StorageError>> + Send;

    /// Get the next section after the given section within the same document.
    ///
    /// Returns the section with the next higher position value, or `None`
    /// if this is the last section in the document.
    fn get_next_section(
        &self,
        section_id: &SectionId,
    ) -> impl Future<Output = Result<Option<SectionRecord>, StorageError>> + Send;

    /// Get the parent document for a given section.
    ///
    /// Returns the document record that contains the given section, or `None`
    /// if the section does not exist.
    fn get_document_for_section(
        &self,
        section_id: &SectionId,
    ) -> impl Future<Output = Result<Option<DocumentRecord>, StorageError>> + Send;

    // -- Claims --

    /// Get a claim by ID.
    fn get_claim(
        &self,
        id: &ClaimId,
    ) -> impl Future<Output = Result<Option<ClaimRecord>, StorageError>> + Send;

    /// List all claims for a section.
    fn list_claims(
        &self,
        section_id: &SectionId,
    ) -> impl Future<Output = Result<Vec<ClaimRecord>, StorageError>> + Send;

    // -- Claim relationships --

    /// Insert a batch of claim relationships.
    fn insert_claim_relationships(
        &self,
        relationships: &[ClaimRelationship],
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Get claims related to the given claim, optionally filtered by relation type.
    fn get_related_claims(
        &self,
        claim_id: &ClaimId,
        relation_types: Option<&[RelationType]>,
    ) -> impl Future<Output = Result<Vec<RelatedClaimRecord>, StorageError>> + Send;

    /// Delete all relationships involving claims in the given section.
    ///
    /// Used during re-indexing to clean up stale relationships.
    fn delete_relationships_for_section(
        &self,
        section_id: &SectionId,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    // -- File hashes --

    /// Upsert a file hash record (insert or update on conflict).
    fn upsert_file_hash(
        &self,
        record: &FileHashRecord,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Get the stored hash for a file path.
    fn get_file_hash(
        &self,
        path: &str,
    ) -> impl Future<Output = Result<Option<FileHashRecord>, StorageError>> + Send;

    /// Delete a file hash record.
    fn delete_file_hash(
        &self,
        path: &str,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;

    // -- Sessions --

    /// Save a session to persistent storage for crash recovery.
    ///
    /// Uses upsert semantics — creates the session if it doesn't exist,
    /// or replaces all delivered items if it does.
    fn save_session(
        &self,
        session: &Session,
    ) -> impl Future<Output = Result<(), StorageError>> + Send;

    /// Load a previously persisted session by ID.
    ///
    /// Returns `None` if no session with the given ID exists.
    fn load_session(
        &self,
        id: &SessionId,
    ) -> impl Future<Output = Result<Option<Session>, StorageError>> + Send;

    /// Delete a persisted session and all its delivery records.
    fn delete_session(
        &self,
        id: &SessionId,
    ) -> impl Future<Output = Result<bool, StorageError>> + Send;
}
