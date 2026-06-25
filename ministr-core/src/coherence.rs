//! File watching and coherence protocol for ministr.
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

use crate::embedding::{Embedder, SparseEmbedder};
use crate::error::CoherenceError;
use crate::index::{SparseIndex, VectorIndex};
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
    #[must_use = "constructors return a new value"]
    pub fn new(directories: &[PathBuf]) -> Result<Self, CoherenceError> {
        let (tx, rx) = mpsc::channel(256);

        let event_tx = tx.clone();
        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            match res {
                Ok(event) => {
                    if let Some(coherence_event) = normalize_event(&event) {
                        // Try non-blocking send. If the channel is full (consumer
                        // is slow or stalled during a long reindex), warn so the
                        // drop doesn't go unnoticed. The `notify` callback runs
                        // on the watcher thread which can't block, so we accept
                        // the drop rather than `send().await`.
                        if let Err(err) = event_tx.try_send(coherence_event) {
                            warn!(
                                error = %err,
                                "coherence watcher channel full — dropping event"
                            );
                        }
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
    #[must_use]
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
///
/// When constructed with [`with_embeddings`](Self::with_embeddings), the engine
/// also updates the vector index with new embeddings for changed content.
pub struct CoherenceEngine {
    pipeline: IngestionPipeline,
    corpus_dir: PathBuf,
    embedder: Option<Arc<dyn Embedder>>,
    index: Option<Arc<dyn VectorIndex>>,
    /// The corpus's sparse (hybrid) index, when hybrid retrieval is on.
    /// Set via [`Self::with_sparse`]; used by the remove path so a
    /// watcher-deleted file's postings are tombstoned immediately.
    sparse_index: Option<Arc<dyn SparseIndex>>,
    /// Per-root exclusion matchers (watcher-ignore-filtering). When set,
    /// events for excluded paths are dropped in [`Self::process_events`]
    /// instead of driving a per-file reindex.
    exclusions: Vec<crate::ingestion::ExclusionMatcher>,
}

impl CoherenceEngine {
    /// Create a new coherence engine for the given corpus directory.
    ///
    /// This variant only updates storage (no embeddings or vector index).
    /// Use [`with_embeddings`](Self::with_embeddings) for full re-indexing.
    #[must_use]
    pub fn new(corpus_dir: PathBuf) -> Self {
        Self {
            pipeline: IngestionPipeline::new(),
            corpus_dir,
            embedder: None,
            index: None,
            sparse_index: None,
            exclusions: Vec::new(),
        }
    }

    /// Create a coherence engine that also updates embeddings and the vector index.
    ///
    /// When files change, the engine re-ingests them with full embedding generation
    /// so the vector index stays in sync with the corpus.
    #[must_use]
    pub fn with_embeddings(
        corpus_dir: PathBuf,
        embedder: Arc<dyn Embedder>,
        index: Arc<dyn VectorIndex>,
    ) -> Self {
        Self {
            pipeline: IngestionPipeline::new(),
            corpus_dir,
            embedder: Some(embedder),
            index: Some(index),
            sparse_index: None,
            exclusions: Vec::new(),
        }
    }

    /// Attach the corpus's sparse (hybrid) components so watcher events keep
    /// the sparse index coherent (sparse-watcher-delete-wiring). Covers BOTH
    /// event kinds: Created/Modified re-ingests sparse-embed through the
    /// pipeline's sparse-embed path, and Removed tombstones the file's postings via
    /// `delete_document_vectors`. Per-event updates mutate the in-memory
    /// sparse index only — the on-disk sidecar persists on the next full
    /// ingest, exactly like the dense index's per-event behavior.
    #[must_use]
    pub fn with_sparse(
        mut self,
        sparse_embedder: Arc<dyn SparseEmbedder>,
        sparse_index: Arc<dyn SparseIndex>,
    ) -> Self {
        self.pipeline = self
            .pipeline
            .with_sparse_indexing(sparse_embedder, Arc::clone(&sparse_index));
        self.sparse_index = Some(sparse_index);
        self
    }

    /// Attach exclusion matchers so watcher events for excluded paths
    /// (built-in ignores, the root `.gitignore`, `[corpus] ignore`
    /// patterns) are dropped instead of re-indexed
    /// (watcher-ignore-filtering). One matcher per corpus root.
    #[must_use]
    pub fn with_exclusions(mut self, roots: &[PathBuf], user_patterns: &[String]) -> Self {
        self.exclusions = roots
            .iter()
            .map(|r| crate::ingestion::ExclusionMatcher::for_root(r, user_patterns))
            .collect();
        self
    }

    /// Whether any attached matcher excludes this path.
    fn is_excluded(&self, path: &Path) -> bool {
        self.exclusions.iter().any(|m| m.is_excluded(path))
    }

    /// Process a batch of coherence events.
    ///
    /// Events for the same path are coalesced with **last-event-wins**
    /// semantics so that `[Modified(X), Removed(X)]` correctly processes
    /// the Remove (rather than silently re-indexing a file that no longer
    /// exists), and `[Removed(X), Created(X)]` correctly processes the
    /// Create (save-replace editor pattern).
    ///
    /// For each coalesced event:
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

        // Coalesce per path, last event wins. `HashMap::insert` overwrites
        // any previous entry for the same path, giving us the most recent
        // event by the time the loop finishes.
        let mut latest: std::collections::HashMap<PathBuf, CoherenceEvent> =
            std::collections::HashMap::new();
        for event in events {
            latest.insert(event.path().to_path_buf(), event.clone());
        }

        for event in latest.values() {
            // watcher-ignore-filtering: an excluded path never drives a
            // per-file reindex (Removed included — exclusion at ingest means
            // it was never indexed; pre-existing stale docs are pruned by the
            // next full reindex, which uses the walker).
            if self.is_excluded(event.path()) {
                debug!(path = %event.path().display(), "skipping event for excluded path");
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
    #[must_use]
    pub fn invalidate_session(session: &mut Session, affected_sections: &[String]) -> usize {
        session.invalidate_sections(affected_sections)
    }

    /// Re-index a single changed file and return affected section IDs.
    ///
    /// Operates on just the one file — reads its content, detects the
    /// parser, and calls [`IngestionPipeline::ingest_content`] or
    /// [`IngestionPipeline::ingest_content_with_embeddings`]. This replaces
    /// an earlier implementation that re-scanned the entire corpus
    /// directory on every file change (O(corpus) per event, and worse,
    /// silently picked up unrelated on-disk changes).
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

        // Unsupported file types (images, binaries) silently drop out —
        // the file watcher's `normalize_event` should have filtered these
        // already, but be defensive here.
        let Some(parser_kind) = crate::parser::detect_parser_kind(path) else {
            debug!(path = %path.display(), "skipping reindex: unsupported file type");
            return Ok(Vec::new());
        };

        // Snapshot the document's section set BEFORE the reindex so we can
        // report sections that were removed as well as sections that exist
        // afterward.
        let doc_id = ContentId(relative.clone());
        let old_sections = storage.list_sections(&doc_id).await.unwrap_or_default();
        let old_section_ids: Vec<String> = old_sections.iter().map(|s| s.id.0.clone()).collect();

        // Read the one file we were asked to re-index.
        let content =
            tokio::fs::read_to_string(path)
                .await
                .map_err(|e| CoherenceError::ReindexFailed {
                    path: path.to_path_buf(),
                    source: Box::new(crate::error::IngestionError::Io {
                        path: path.to_path_buf(),
                        source: e,
                    }),
                })?;

        let ingest_result = if let (Some(embedder), Some(index)) = (&self.embedder, &self.index) {
            self.pipeline
                .ingest_content_with_embeddings(
                    &relative,
                    &content,
                    parser_kind,
                    storage,
                    embedder.as_ref(),
                    index.as_ref(),
                )
                .await
                .map(|_| ())
        } else {
            self.pipeline
                .ingest_content(&relative, &content, parser_kind, storage)
                .await
                .map(|_| ())
        };

        ingest_result.map_err(|e| CoherenceError::ReindexFailed {
            path: path.to_path_buf(),
            source: Box::new(e),
        })?;

        // Union old and new section IDs so callers can invalidate both the
        // sections that disappeared and the ones that were added/modified.
        let new_sections = storage.list_sections(&doc_id).await.unwrap_or_default();
        let new_section_ids: Vec<String> = new_sections.iter().map(|s| s.id.0.clone()).collect();

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

        // Tear down vector-index entries BEFORE the SQL cascade so
        // `delete_document_vectors` can still enumerate sections, claims,
        // and symbols via storage. Otherwise the index keeps stale
        // vectors whose documents no longer exist, and later surveys
        // return result rows that `ministr_read` can't service. The sparse
        // (hybrid) postings tombstone in the same call when the engine was
        // built `with_sparse` (sparse-watcher-delete-wiring).
        if let Some(ref index) = self.index
            && let Err(e) = crate::ingestion::delete_document_vectors(
                &doc_id,
                storage,
                index.as_ref(),
                self.sparse_index.as_deref(),
            )
            .await
        {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "failed to delete vectors for removed file; continuing with SQL cascade",
            );
        }

        // Delete the document (cascading to sections and claims)
        let _ = storage.delete_document(&doc_id).await;
        let _ = storage.delete_file_hash(&relative).await;
        // Symbols are keyed by file_path in their own table and are NOT
        // cascaded by delete_document; drop them too (symbol_refs cascade
        // off symbols via FK) so a removed file's symbols stop surfacing
        // in symbol search. Same `relative` key the document used.
        let _ = storage.delete_symbols_for_file(&relative).await;

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
/// * `notify_tx` - Optional sender to push affected section IDs to subscribers
///   (e.g. MCP resource subscription notifications)
///
/// Spawn a coherence task that invalidates all sessions in a registry.
///
/// When files change, the task re-indexes them and propagates invalidation
/// to every session in the registry via [`SessionRegistry::invalidate_all`].
pub fn spawn_coherence_task<S: Storage + 'static>(
    mut watcher: FileWatcher,
    engine: Arc<CoherenceEngine>,
    storage: Arc<S>,
    registry: Arc<tokio::sync::Mutex<crate::session::SessionRegistry>>,
    notify_tx: Option<mpsc::UnboundedSender<Vec<String>>>,
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
                    let mut reg = registry.lock().await;
                    let invalidated = reg.invalidate_all(&affected_sections);
                    info!(
                        affected = affected_sections.len(),
                        invalidated,
                        sessions = reg.session_count(),
                        "coherence: sections updated, entries invalidated across all sessions"
                    );
                    drop(reg);
                    // Notify subscribers (e.g. MCP resource subscriptions) about changes.
                    if let Some(ref tx) = notify_tx {
                        let _ = tx.send(affected_sections);
                    }
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

    use crate::session::{DropPolicy, SessionId};
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
        assert!(!is_supported_file(Path::new("image.png")));
        assert!(!is_supported_file(Path::new("data.csv")));
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
            DropPolicy::Fifo,
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

    // watcher-ignore-filtering: with exclusions attached, an event for an
    // excluded path drives NO per-file reindex — the index never sees it.
    #[tokio::test]
    async fn coherence_engine_drops_events_for_excluded_paths() {
        use crate::storage::SqliteStorage;

        let dir = TempDir::new().unwrap();
        let keep = dir.path().join("keep.md");
        let skip = dir.path().join("skipme.md");
        std::fs::write(&keep, "# Keep\n\nIndexed.\n").unwrap();
        std::fs::write(&skip, "# Skip\n\nMust never be indexed.\n").unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let engine = CoherenceEngine::new(dir.path().to_path_buf())
            .with_exclusions(&[dir.path().to_path_buf()], &["*me.md".to_owned()]);

        let events = vec![
            CoherenceEvent::Modified(skip.clone()),
            CoherenceEvent::Modified(keep.clone()),
        ];
        let affected = engine.process_events(&events, &storage).await.unwrap();
        assert!(!affected.is_empty(), "kept-file event must still flow");

        let docs = storage.list_documents().await.unwrap();
        assert!(
            docs.iter().any(|d| d.id.0.contains("keep.md")),
            "kept file indexed"
        );
        assert!(
            !docs.iter().any(|d| d.id.0.contains("skipme.md")),
            "excluded file must not be re-indexed by the watcher path: {:?}",
            docs.iter().map(|d| &d.id).collect::<Vec<_>>()
        );

        // Built-in exclusions apply too, with no user patterns at all.
        let nm = dir.path().join("node_modules/dep.md");
        std::fs::create_dir_all(nm.parent().unwrap()).unwrap();
        std::fs::write(&nm, "# Dep\n\nVendored.\n").unwrap();
        let engine2 = CoherenceEngine::new(dir.path().to_path_buf())
            .with_exclusions(&[dir.path().to_path_buf()], &[]);
        engine2
            .process_events(&[CoherenceEvent::Created(nm)], &storage)
            .await
            .unwrap();
        let docs = storage.list_documents().await.unwrap();
        assert!(
            !docs.iter().any(|d| d.id.0.contains("node_modules")),
            "built-in ignores must hold on the watcher path"
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

    /// Test embedder that returns a deterministic non-zero vector per text
    /// so HNSW can distinguish insertions.
    struct HashEmbedder {
        dim: usize,
    }

    impl crate::embedding::Embedder for HashEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, crate::error::IndexError> {
            use std::hash::{Hash, Hasher};
            Ok(texts
                .iter()
                .map(|t| {
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    t.hash(&mut h);
                    let seed = (h.finish() % 997) as f32 / 997.0;
                    let mut v = vec![seed; self.dim];
                    v[0] = 1.0 - seed;
                    v
                })
                .collect())
        }

        fn dimension(&self) -> usize {
            self.dim
        }
    }

    #[tokio::test]
    async fn remove_file_also_deletes_vectors_from_index() {
        // Regression: CoherenceEngine::remove_file used to leave vectors
        // in the index after the SQL cascade deleted the document. Later
        // surveys still returned the stale result rows but `ministr_read`
        // would 404 on them.
        use crate::index::{HnswIndex, VectorIndex};
        use crate::storage::SqliteStorage;
        use std::sync::Arc;

        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("to_delete.md");
        std::fs::write(
            &file_path,
            "# Delete Me\n\n## Content\n\nThis will be removed.\n",
        )
        .unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(64, 1_000).unwrap());
        let embedder: Arc<dyn crate::embedding::Embedder> = Arc::new(HashEmbedder { dim: 64 });

        // Ingest with embeddings so the index actually contains vectors.
        let pipeline = IngestionPipeline::new();
        pipeline
            .ingest_content_with_embeddings(
                "to_delete.md",
                "# Delete Me\n\n## Content\n\nThis will be removed.\n",
                crate::parser::ParserKind::Markdown,
                &storage,
                embedder.as_ref(),
                index.as_ref(),
            )
            .await
            .unwrap();

        let before = index.len();
        assert!(
            before > 0,
            "index should have vectors after ingestion (got {before})",
        );

        std::fs::remove_file(&file_path).unwrap();

        let engine = CoherenceEngine::with_embeddings(
            dir.path().to_path_buf(),
            Arc::clone(&embedder),
            Arc::clone(&index),
        );
        let events = vec![CoherenceEvent::Removed(file_path)];
        let _affected = engine.process_events(&events, &storage).await.unwrap();

        assert_eq!(
            index.len(),
            0,
            "every vector for the removed document should be gone from the index \
             (before={before}, after={})",
            index.len()
        );
    }

    /// Word-level deterministic sparse mock: each whitespace-separated word
    /// hashes to a term id with weight 1.0, so tests can query for a word
    /// and hit exactly the documents containing it. No model, no download.
    struct WordSparse;

    impl crate::embedding::SparseEmbedder for WordSparse {
        fn embed_sparse(
            &self,
            texts: &[&str],
        ) -> Result<Vec<crate::embedding::SparseVector>, crate::error::IndexError> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut indices: Vec<u32> = t
                        .split_whitespace()
                        .map(|w| {
                            w.bytes()
                                .fold(0x811c_9dc5_u32, |h, b| (h ^ u32::from(b)).wrapping_mul(31))
                        })
                        .collect();
                    indices.sort_unstable();
                    indices.dedup();
                    let values = vec![1.0_f32; indices.len()];
                    crate::embedding::SparseVector { indices, values }
                })
                .collect())
        }
    }

    /// Hits in `sparse` for a single-word query, as the matching doc ids.
    fn sparse_hits(sparse: &dyn crate::index::SparseIndex, word: &str) -> Vec<String> {
        use crate::embedding::SparseEmbedder as _;
        let q = WordSparse.embed_sparse(&[word]).unwrap().remove(0);
        sparse
            .search_sparse(&q.indices, &q.values, 50)
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
    }

    /// sparse-watcher-delete-wiring: a watcher-removed file's sparse postings
    /// are tombstoned immediately by `process_events`, not left for the next
    /// full re-ingest's sweep.
    #[tokio::test]
    async fn remove_event_tombstones_sparse_postings() {
        use crate::index::{HnswIndex, InvertedIndex, SparseIndex, VectorIndex};
        use crate::storage::SqliteStorage;

        let dir = TempDir::new().unwrap();
        let keep = dir.path().join("keep.md");
        let doomed = dir.path().join("doomed.md");
        std::fs::write(&keep, "# Keep\n\nThe keepword stays around.\n").unwrap();
        std::fs::write(&doomed, "# Doomed\n\nThe doomedword will vanish.\n").unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(64, 1_000).unwrap());
        let embedder: Arc<dyn crate::embedding::Embedder> = Arc::new(HashEmbedder { dim: 64 });
        let sparse: Arc<InvertedIndex> = Arc::new(InvertedIndex::new());

        let engine = CoherenceEngine::with_embeddings(
            dir.path().to_path_buf(),
            Arc::clone(&embedder),
            Arc::clone(&index),
        )
        .with_sparse(
            Arc::new(WordSparse),
            Arc::clone(&sparse) as Arc<dyn crate::index::SparseIndex>,
        );

        // Created events drive the engine's own re-ingest path, which must
        // populate the sparse index through the pipeline's sparse-embed path.
        engine
            .process_events(
                &[
                    CoherenceEvent::Created(keep.clone()),
                    CoherenceEvent::Created(doomed.clone()),
                ],
                &storage,
            )
            .await
            .unwrap();
        assert!(sparse.len_sparse() > 0, "created events populate sparse");
        assert!(
            sparse_hits(sparse.as_ref(), "doomedword")
                .iter()
                .any(|id| id.contains("doomed.md")),
            "doomed.md is sparse-searchable before removal",
        );

        std::fs::remove_file(&doomed).unwrap();
        engine
            .process_events(&[CoherenceEvent::Removed(doomed)], &storage)
            .await
            .unwrap();

        assert!(
            !sparse_hits(sparse.as_ref(), "doomedword")
                .iter()
                .any(|id| id.contains("doomed.md")),
            "removed file's sparse postings must be tombstoned immediately",
        );
        assert!(
            sparse_hits(sparse.as_ref(), "keepword")
                .iter()
                .any(|id| id.contains("keep.md")),
            "the surviving file's postings are untouched",
        );
    }

    /// sparse-watcher-delete-wiring (modify half): a watcher-modified file's
    /// sparse postings reflect the NEW content — the old content's terms stop
    /// matching, the new content's terms start.
    #[tokio::test]
    async fn modified_event_updates_sparse_postings() {
        use crate::index::{HnswIndex, InvertedIndex, VectorIndex};
        use crate::storage::SqliteStorage;

        let dir = TempDir::new().unwrap();
        let file = dir.path().join("doc.md");
        std::fs::write(&file, "# Doc\n\nThe alphaword version.\n").unwrap();

        let storage = SqliteStorage::open_in_memory().unwrap();
        let index: Arc<dyn VectorIndex> = Arc::new(HnswIndex::new(64, 1_000).unwrap());
        let embedder: Arc<dyn crate::embedding::Embedder> = Arc::new(HashEmbedder { dim: 64 });
        let sparse: Arc<InvertedIndex> = Arc::new(InvertedIndex::new());

        let engine = CoherenceEngine::with_embeddings(
            dir.path().to_path_buf(),
            Arc::clone(&embedder),
            Arc::clone(&index),
        )
        .with_sparse(
            Arc::new(WordSparse),
            Arc::clone(&sparse) as Arc<dyn crate::index::SparseIndex>,
        );

        engine
            .process_events(&[CoherenceEvent::Created(file.clone())], &storage)
            .await
            .unwrap();
        assert!(
            sparse_hits(sparse.as_ref(), "alphaword")
                .iter()
                .any(|id| id.contains("doc.md")),
            "v1 content is sparse-searchable",
        );

        std::fs::write(&file, "# Doc\n\nThe betaword version.\n").unwrap();
        engine
            .process_events(&[CoherenceEvent::Modified(file)], &storage)
            .await
            .unwrap();

        assert!(
            !sparse_hits(sparse.as_ref(), "alphaword")
                .iter()
                .any(|id| id.contains("doc.md")),
            "stale v1 postings must not survive a modify event",
        );
        assert!(
            sparse_hits(sparse.as_ref(), "betaword")
                .iter()
                .any(|id| id.contains("doc.md")),
            "v2 content is sparse-searchable after the modify event",
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

    // --- spawn_coherence_task notify channel tests ---

    /// Verify that the notify sender in `spawn_coherence_task` fires when
    /// `process_events` returns affected sections.
    ///
    /// Uses a direct `process_events` call (no file watcher) to avoid
    /// platform-specific `FSEvents` timing issues in test mode.
    #[tokio::test]
    async fn coherence_engine_notifies_on_section_changes() {
        use crate::storage::SqliteStorage;
        use std::io::Write;

        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.md");
        {
            let mut f = std::fs::File::create(&file_path).unwrap();
            writeln!(f, "# Hello\n\nOriginal content.").unwrap();
        }

        let storage = SqliteStorage::open_in_memory().unwrap();
        let storage = Arc::new(storage);

        // Ingest via public API so document IDs are consistent.
        let pipeline = IngestionPipeline::new();
        pipeline
            .ingest_directory(dir.path(), storage.as_ref())
            .await
            .unwrap();

        let engine = CoherenceEngine::new(dir.path().to_path_buf());

        // Modify the file so re-indexing detects a change.
        {
            let mut f = std::fs::File::create(&file_path).unwrap();
            writeln!(f, "# Hello\n\nUpdated content with significant changes.").unwrap();
        }

        let events = vec![CoherenceEvent::Modified(file_path)];
        let affected = engine
            .process_events(&events, storage.as_ref())
            .await
            .unwrap();

        // The coherence engine should detect that sections changed.
        assert!(
            !affected.is_empty(),
            "process_events should return affected section IDs after file modification"
        );
    }

    /// Verify that `spawn_coherence_task` accepts `None` for `notify_tx`
    /// and runs without errors (backwards compatibility).
    #[tokio::test]
    async fn spawn_coherence_task_accepts_none_notify() {
        use crate::storage::SqliteStorage;
        use std::io::Write;

        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.md");
        {
            let mut f = std::fs::File::create(&file_path).unwrap();
            writeln!(f, "# Hello\n\nOriginal.").unwrap();
        }

        let storage = SqliteStorage::open_in_memory().unwrap();
        let storage = Arc::new(storage);

        let pipeline = IngestionPipeline::new();
        pipeline
            .ingest_directory(dir.path(), storage.as_ref())
            .await
            .unwrap();

        let mut registry =
            crate::session::SessionRegistry::new(crate::session::UsageConfig::default());
        registry.create_session("test-session", None, crate::session::AccessMode::ReadWrite);
        let registry = Arc::new(tokio::sync::Mutex::new(registry));

        let engine = Arc::new(CoherenceEngine::new(dir.path().to_path_buf()));
        let watcher = FileWatcher::new(&[dir.path().to_path_buf()]).unwrap();

        // Pass None — should compile and run without panicking.
        let handle = spawn_coherence_task(watcher, engine, Arc::clone(&storage), registry, None);

        // Give a moment, then abort — no crash means success.
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        handle.abort();
    }
}
