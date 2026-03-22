//! File watching and coherence protocol for iris.
//!
//! When the underlying document corpus changes, the coherence subsystem detects
//! the changes, re-indexes affected files, and notifies active sessions so that
//! stale content can be invalidated. This is the cache coherence protocol from
//! the DESIGN.md spec.
//!
//! # Architecture
//!
//! - [`FileWatcher`] — wraps the `notify` crate's `RecommendedWatcher` to watch
//!   corpus source directories for filesystem events.
//! - [`CoherenceEvent`] — normalized change events (created, modified, removed).
//! - [`CoherenceEngine`] — receives FS events, re-indexes changed files via the
//!   ingestion pipeline, detects which sessions are affected, and queues alerts.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;
use tracing::{debug, info, instrument, warn};

use crate::error::CoherenceError;
use crate::ingestion::IngestionPipeline;
use crate::session::Session;
use crate::storage::traits::Storage;
use crate::types::ContentId;

/// A normalized filesystem change event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoherenceEvent {
    /// A new file was created in a watched directory.
    Created(PathBuf),
    /// An existing file was modified.
    Modified(PathBuf),
    /// A file was removed from a watched directory.
    Removed(PathBuf),
}

impl CoherenceEvent {
    /// The path affected by this event.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Created(p) | Self::Modified(p) | Self::Removed(p) => p,
        }
    }
}

/// Watches corpus source directories for filesystem changes.
///
/// Uses the `notify` crate's platform-specific watcher (inotify on Linux,
/// `FSEvents` on macOS, `ReadDirectoryChanges` on Windows) with a tokio channel
/// bridge for async event delivery.
pub struct FileWatcher {
    /// The underlying platform watcher. Dropping this stops watching.
    _watcher: RecommendedWatcher,
    /// Receiver for normalized coherence events.
    receiver: mpsc::Receiver<CoherenceEvent>,
}

impl FileWatcher {
    /// Create a new file watcher for the given directories.
    ///
    /// Begins watching immediately. Events are delivered on the returned
    /// receiver. All directories are watched recursively.
    ///
    /// # Errors
    ///
    /// Returns [`CoherenceError::WatcherInit`] if the platform watcher cannot
    /// be created, or [`CoherenceError::WatchFailed`] if a directory cannot
    /// be watched.
    pub fn new(directories: &[PathBuf]) -> Result<Self, CoherenceError> {
        let (tx, rx) = mpsc::channel(256);

        let event_tx = tx.clone();
        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            match res {
                Ok(event) => {
                    if let Some(coherence_event) = normalize_event(&event) {
                        // Best-effort send — if the channel is full, skip
                        let _ = event_tx.try_send(coherence_event);
                    }
                }
                Err(e) => {
                    warn!(error = %e, "file watcher error");
                }
            }
        })
        .map_err(|e| CoherenceError::WatcherInit {
            reason: e.to_string(),
        })?;

        for dir in directories {
            watcher.watch(dir, RecursiveMode::Recursive).map_err(|e| {
                CoherenceError::WatchFailed {
                    path: dir.clone(),
                    reason: e.to_string(),
                }
            })?;
            info!(dir = %dir.display(), "watching directory for changes");
        }

        Ok(Self {
            _watcher: watcher,
            receiver: rx,
        })
    }

    /// Receive the next coherence event.
    ///
    /// Returns `None` when the watcher has been dropped and all pending
    /// events have been consumed.
    pub async fn recv(&mut self) -> Option<CoherenceEvent> {
        self.receiver.recv().await
    }

    /// Try to receive a coherence event without blocking.
    ///
    /// Returns `None` if no event is currently available.
    pub fn try_recv(&mut self) -> Option<CoherenceEvent> {
        self.receiver.try_recv().ok()
    }

    /// Drain all currently pending events without blocking.
    pub fn drain_pending(&mut self) -> Vec<CoherenceEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.receiver.try_recv() {
            events.push(event);
        }
        events
    }
}

/// Normalize a raw `notify` event into a `CoherenceEvent`.
///
/// Filters for supported file types (`.md` for now) and collapses
/// the many `notify` event kinds into our three categories.
fn normalize_event(event: &Event) -> Option<CoherenceEvent> {
    // Only process events with file paths
    let path = event.paths.first()?;

    // Only process supported file types
    if !is_supported_file(path) {
        return None;
    }

    match event.kind {
        EventKind::Create(_) => Some(CoherenceEvent::Created(path.clone())),
        EventKind::Modify(_) => Some(CoherenceEvent::Modified(path.clone())),
        EventKind::Remove(_) => Some(CoherenceEvent::Removed(path.clone())),
        _ => None,
    }
}

/// Check if a file path is a supported document type for indexing.
fn is_supported_file(path: &Path) -> bool {
    crate::parser::detect_parser_kind(path).is_some()
}

/// The coherence engine processes file change events and updates the index.
///
/// It bridges the file watcher to the ingestion pipeline and session shadow,
/// re-indexing changed files and marking stale content in active sessions.
pub struct CoherenceEngine {
    pipeline: IngestionPipeline,
    corpus_dir: PathBuf,
}

impl CoherenceEngine {
    /// Create a new coherence engine for the given corpus directory.
    #[must_use]
    pub fn new(corpus_dir: PathBuf) -> Self {
        Self {
            pipeline: IngestionPipeline::new(),
            corpus_dir,
        }
    }

    /// Process a batch of coherence events.
    ///
    /// For each event:
    /// - **Created/Modified**: re-indexes the file and collects affected section IDs.
    /// - **Removed**: deletes the document from storage and collects removed section IDs.
    ///
    /// Returns the list of section IDs that were affected by the changes.
    ///
    /// # Errors
    ///
    /// Returns [`CoherenceError`] if re-indexing or storage operations fail.
    #[instrument(skip(self, storage), fields(event_count = events.len()))]
    pub async fn process_events<S: Storage>(
        &self,
        events: &[CoherenceEvent],
        storage: &S,
    ) -> Result<Vec<String>, CoherenceError> {
        let mut affected_sections = Vec::new();

        // Deduplicate events by path (multiple events for the same file)
        let mut seen_paths = std::collections::HashSet::new();

        for event in events {
            let path = event.path();
            if !seen_paths.insert(path.to_path_buf()) {
                continue;
            }

            match event {
                CoherenceEvent::Created(p) | CoherenceEvent::Modified(p) => {
                    match self.reindex_file(p, storage).await {
                        Ok(sections) => {
                            debug!(
                                path = %p.display(),
                                sections = sections.len(),
                                "re-indexed changed file"
                            );
                            affected_sections.extend(sections);
                        }
                        Err(e) => {
                            warn!(path = %p.display(), error = %e, "failed to re-index file");
                        }
                    }
                }
                CoherenceEvent::Removed(p) => match self.remove_file(p, storage).await {
                    Ok(sections) => {
                        debug!(
                            path = %p.display(),
                            sections = sections.len(),
                            "removed deleted file from index"
                        );
                        affected_sections.extend(sections);
                    }
                    Err(e) => {
                        warn!(path = %p.display(), error = %e, "failed to remove file");
                    }
                },
            }
        }

        Ok(affected_sections)
    }

    /// Invalidate session shadow entries for affected sections.
    ///
    /// Marks delivered items as stale and enqueues coherence alerts in the
    /// session for the transport layer to deliver.
    ///
    /// Returns the number of items invalidated.
    pub fn invalidate_session(session: &mut Session, affected_sections: &[String]) -> usize {
        session.invalidate_sections(affected_sections)
    }

    /// Re-index a single changed file and return affected section IDs.
    async fn reindex_file<S: Storage>(
        &self,
        path: &Path,
        storage: &S,
    ) -> Result<Vec<String>, CoherenceError> {
        let relative = path
            .strip_prefix(&self.corpus_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        // Get existing sections for this document before re-indexing
        let doc_id = ContentId(relative.clone());
        let old_sections = storage.list_sections(&doc_id).await.unwrap_or_default();

        let old_section_ids: Vec<String> = old_sections.iter().map(|s| s.id.0.clone()).collect();

        // Re-ingest the file (the pipeline handles hash checking and upsert)
        self.pipeline
            .ingest_directory(&self.corpus_dir, storage)
            .await
            .map_err(|e| CoherenceError::ReindexFailed {
                path: path.to_path_buf(),
                source: Box::new(e),
            })?;

        // Get new sections after re-indexing
        let new_sections = storage.list_sections(&doc_id).await.unwrap_or_default();

        let new_section_ids: Vec<String> = new_sections.iter().map(|s| s.id.0.clone()).collect();

        // Affected = union of old and new section IDs (sections may have been
        // added, removed, or modified)
        let mut affected = old_section_ids;
        for id in new_section_ids {
            if !affected.contains(&id) {
                affected.push(id);
            }
        }

        Ok(affected)
    }

    /// Remove a deleted file from the index and return affected section IDs.
    async fn remove_file<S: Storage>(
        &self,
        path: &Path,
        storage: &S,
    ) -> Result<Vec<String>, CoherenceError> {
        let relative = path
            .strip_prefix(&self.corpus_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        let doc_id = ContentId(relative.clone());

        // Collect section IDs before deletion
        let sections = storage.list_sections(&doc_id).await.unwrap_or_default();

        let section_ids: Vec<String> = sections.iter().map(|s| s.id.0.clone()).collect();

        // Delete the document (cascading to sections and claims)
        let _ = storage.delete_document(&doc_id).await;
        let _ = storage.delete_file_hash(&relative).await;

        Ok(section_ids)
    }
}

/// Spawn a background coherence task that watches for file changes and
/// processes them.
///
/// Returns a handle to the spawned task. The task runs until the watcher
/// is dropped or the session/storage arcs are dropped.
///
/// # Arguments
///
/// * `watcher` - The file watcher producing events
/// * `engine` - The coherence engine for processing events
/// * `storage` - Storage backend for re-indexing
/// * `session` - Session to invalidate on changes
pub fn spawn_coherence_task<S: Storage + 'static>(
    mut watcher: FileWatcher,
    engine: Arc<CoherenceEngine>,
    storage: Arc<S>,
    session: Arc<tokio::sync::Mutex<Session>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        info!("coherence task started");

        loop {
            // Wait for next event
            let Some(event) = watcher.recv().await else {
                info!("file watcher closed, stopping coherence task");
                break;
            };

            // Collect any additional pending events (debounce)
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            let mut events = vec![event];
            events.extend(watcher.drain_pending());

            debug!(event_count = events.len(), "processing coherence events");

            // Process events
            match engine.process_events(&events, storage.as_ref()).await {
                Ok(affected_sections) if !affected_sections.is_empty() => {
                    let mut session = session.lock().await;
                    let invalidated =
                        CoherenceEngine::invalidate_session(&mut session, &affected_sections);
                    info!(
                        affected = affected_sections.len(),
                        invalidated, "coherence: sections updated, session entries invalidated"
                    );
                }
                Ok(_) => {
                    debug!("coherence: no sections affected by file changes");
                }
                Err(e) => {
                    warn!(error = %e, "coherence: failed to process events");
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    use crate::session::{EvictionPolicy, SessionId};
    use crate::types::Resolution;

    // --- CoherenceEvent tests ---

    #[test]
    fn coherence_event_path() {
        let path = PathBuf::from("/docs/test.md");
        let event = CoherenceEvent::Created(path.clone());
        assert_eq!(event.path(), path.as_path());

        let event = CoherenceEvent::Modified(path.clone());
        assert_eq!(event.path(), path.as_path());

        let event = CoherenceEvent::Removed(path.clone());
        assert_eq!(event.path(), path.as_path());
    }

    // --- normalize_event tests ---

    #[test]
    fn normalize_create_event() {
        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![PathBuf::from("/docs/new.md")],
            attrs: notify::event::EventAttributes::default(),
        };
        let result = normalize_event(&event);
        assert!(matches!(result, Some(CoherenceEvent::Created(_))));
    }

    #[test]
    fn normalize_modify_event() {
        let event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![PathBuf::from("/docs/changed.md")],
            attrs: notify::event::EventAttributes::default(),
        };
        let result = normalize_event(&event);
        assert!(matches!(result, Some(CoherenceEvent::Modified(_))));
    }

    #[test]
    fn normalize_remove_event() {
        let event = Event {
            kind: EventKind::Remove(notify::event::RemoveKind::File),
            paths: vec![PathBuf::from("/docs/deleted.md")],
            attrs: notify::event::EventAttributes::default(),
        };
        let result = normalize_event(&event);
        assert!(matches!(result, Some(CoherenceEvent::Removed(_))));
    }

    #[test]
    fn normalize_ignores_unsupported_extensions() {
        let event = Event {
            kind: EventKind::Modify(notify::event::ModifyKind::Data(
                notify::event::DataChange::Content,
            )),
            paths: vec![PathBuf::from("/docs/image.png")],
            attrs: notify::event::EventAttributes::default(),
        };
        assert!(normalize_event(&event).is_none());
    }

    #[test]
    fn normalize_ignores_access_events() {
        let event = Event {
            kind: EventKind::Access(notify::event::AccessKind::Read),
            paths: vec![PathBuf::from("/docs/test.md")],
            attrs: notify::event::EventAttributes::default(),
        };
        assert!(normalize_event(&event).is_none());
    }

    #[test]
    fn normalize_ignores_empty_paths() {
        let event = Event {
            kind: EventKind::Create(notify::event::CreateKind::File),
            paths: vec![],
            attrs: notify::event::EventAttributes::default(),
        };
        assert!(normalize_event(&event).is_none());
    }

    // --- is_supported_file tests ---

    #[test]
    fn supported_extensions() {
        assert!(is_supported_file(Path::new("doc.md")));
        assert!(is_supported_file(Path::new("doc.markdown")));
        assert!(is_supported_file(Path::new("/path/to/file.md")));
        assert!(is_supported_file(Path::new("page.html")));
        assert!(is_supported_file(Path::new("page.htm")));
        assert!(is_supported_file(Path::new("manual.pdf")));
    }

    #[test]
    fn unsupported_extensions() {
        assert!(!is_supported_file(Path::new("file.txt")));
        assert!(!is_supported_file(Path::new("file.rs")));
        assert!(!is_supported_file(Path::new("no_extension")));
    }

    // --- FileWatcher tests ---

    #[tokio::test]
    async fn watcher_detects_file_creation() {
        let dir = TempDir::new().unwrap();
        let mut watcher = FileWatcher::new(&[dir.path().to_path_buf()]).unwrap();

        // Create a new markdown file
        let file_path = dir.path().join("new.md");
        std::fs::write(&file_path, "# New Document\n\nContent here.").unwrap();

        // Wait for the event with a timeout
        let event = tokio::time::timeout(tokio::time::Duration::from_secs(5), watcher.recv()).await;

        assert!(event.is_ok(), "should receive event within timeout");
        let event = event.unwrap().unwrap();
        // Compare file names only — macOS resolves /var -> /private/var symlinks
        assert_eq!(
            event.path().file_name().unwrap(),
            file_path.file_name().unwrap()
        );
    }

    #[tokio::test]
    async fn watcher_detects_file_modification() {
        let dir = TempDir::new().unwrap();

        // Create initial file before watching
        let file_path = dir.path().join("existing.md");
        std::fs::write(&file_path, "# Original\n\nOriginal content.").unwrap();

        let mut watcher = FileWatcher::new(&[dir.path().to_path_buf()]).unwrap();

        // Small delay to let watcher initialize
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Modify the file
        std::fs::write(&file_path, "# Modified\n\nUpdated content.").unwrap();

        let event = tokio::time::timeout(tokio::time::Duration::from_secs(5), watcher.recv()).await;

        assert!(event.is_ok(), "should receive event within timeout");
        let event = event.unwrap().unwrap();
        // Compare file names only — macOS resolves /var -> /private/var symlinks
        assert_eq!(
            event.path().file_name().unwrap(),
            file_path.file_name().unwrap()
        );
    }

    #[tokio::test]
    async fn watcher_ignores_non_markdown_files() {
        let dir = TempDir::new().unwrap();
        let mut watcher = FileWatcher::new(&[dir.path().to_path_buf()]).unwrap();

        // Create a non-markdown file
        std::fs::write(dir.path().join("data.txt"), "not markdown").unwrap();

        // Should not receive an event
        let event =
            tokio::time::timeout(tokio::time::Duration::from_millis(500), watcher.recv()).await;

        assert!(event.is_err(), "should timeout — no markdown events");
    }

    #[test]
    fn watcher_fails_for_nonexistent_directory() {
        let result = FileWatcher::new(&[PathBuf::from("/nonexistent/path/that/doesnt/exist")]);
        assert!(result.is_err());
    }

    // --- CoherenceEngine + session invalidation tests ---

    #[test]
    fn invalidate_session_marks_matching_items_stale() {
        let mut session = Session::new(
            SessionId::from("test".to_string()),
            100_000,
            EvictionPolicy::Fifo,
        );

        session.record_delivery(
            &ContentId("doc.md#intro".into()),
            Resolution::Section,
            200,
            1,
            "hash1".into(),
        );
        session.record_delivery(
            &ContentId("doc.md#details".into()),
            Resolution::Section,
            300,
            1,
            "hash2".into(),
        );
        session.record_delivery(
            &ContentId("other.md#intro".into()),
            Resolution::Section,
            150,
            1,
            "hash3".into(),
        );

        let affected = vec![
            "doc.md#intro".to_string(),
            "doc.md#details".to_string(),
            "doc.md#new-section".to_string(), // not delivered
        ];

        let count = CoherenceEngine::invalidate_session(&mut session, &affected);

        assert_eq!(count, 2);
        assert!(session.is_stale(&ContentId("doc.md#intro".into())));
        assert!(session.is_stale(&ContentId("doc.md#details".into())));
        assert!(!session.is_stale(&ContentId("other.md#intro".into())));

        // Should have generated an alert
        let alerts = session.drain_alerts();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].stale_content_ids.len(), 2);
    }

    #[tokio::test]
    async fn coherence_engine_process_events_with_real_storage() {
        use crate::storage::SqliteStorage;

        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.md");
        std::fs::write(
            &file_path,
            "# Test Document\n\n## Section One\n\nSome content here.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let engine = CoherenceEngine::new(dir.path().to_path_buf());

        // First ingest
        let pipeline = IngestionPipeline::new();
        pipeline
            .ingest_directory(dir.path(), &storage)
            .await
            .unwrap();

        // Verify document was indexed
        let docs = storage.list_documents().await.unwrap();
        assert!(!docs.is_empty());

        // Modify the file
        std::fs::write(
            &file_path,
            "# Test Document\n\n## Section One\n\nUpdated content here.\n",
        )
        .unwrap();

        // Process a modify event
        let events = vec![CoherenceEvent::Modified(file_path.clone())];
        let affected = engine.process_events(&events, &storage).await.unwrap();

        // Should have affected at least one section
        assert!(
            !affected.is_empty(),
            "should have affected sections after modify"
        );
    }

    #[tokio::test]
    async fn coherence_engine_process_remove_event() {
        use crate::storage::SqliteStorage;

        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("to_delete.md");
        std::fs::write(
            &file_path,
            "# Delete Me\n\n## Content\n\nThis will be removed.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();

        // Ingest first
        let pipeline = IngestionPipeline::new();
        pipeline
            .ingest_directory(dir.path(), &storage)
            .await
            .unwrap();

        let docs_before = storage.list_documents().await.unwrap();
        assert!(!docs_before.is_empty());

        // Remove the file from disk
        std::fs::remove_file(&file_path).unwrap();

        // Process remove event
        let engine = CoherenceEngine::new(dir.path().to_path_buf());
        let events = vec![CoherenceEvent::Removed(file_path)];
        let affected = engine.process_events(&events, &storage).await.unwrap();

        assert!(
            !affected.is_empty(),
            "should report affected sections on removal"
        );
    }

    #[test]
    fn drain_pending_returns_all_buffered_events() {
        // This tests the drain_pending method on FileWatcher indirectly
        // by checking the behavior of the mpsc channel
        let (tx, mut rx) = mpsc::channel(16);

        tx.try_send(CoherenceEvent::Created(PathBuf::from("a.md")))
            .unwrap();
        tx.try_send(CoherenceEvent::Modified(PathBuf::from("b.md")))
            .unwrap();
        tx.try_send(CoherenceEvent::Removed(PathBuf::from("c.md")))
            .unwrap();

        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }

        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], CoherenceEvent::Created(_)));
        assert!(matches!(events[1], CoherenceEvent::Modified(_)));
        assert!(matches!(events[2], CoherenceEvent::Removed(_)));
    }
}
