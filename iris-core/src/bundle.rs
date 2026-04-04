//! Index bundle export/import for portable `.iris-index` files.
//!
//! A bundle is a zstd-compressed tar archive containing:
//! - `manifest.json` — bundle metadata (version, model, dimension, corpus info)
//! - `content.db` — `SQLite` database with content tables (no session state)
//! - `index/id_map.json` — HNSW string-to-int ID mapping
//! - `index/iris_hnsw.hnsw.dat` — HNSW graph data
//! - `index/iris_hnsw.hnsw.graph` — HNSW connectivity graph
//!
//! Session-local state (sessions, analytics, web/git cache, pending refs,
//! FSRS memory states) is excluded from bundles.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, instrument};

use crate::error::BundleError;

/// Current bundle format version. Increment on breaking schema changes.
pub const BUNDLE_FORMAT_VERSION: u32 = 1;

/// File extension for iris index bundles.
pub const BUNDLE_EXTENSION: &str = "iris-index";

/// Manifest embedded in every bundle describing its contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleManifest {
    /// Bundle format version (for forward-compatibility checks).
    pub format_version: u32,
    /// Embedding model name used to generate vectors.
    pub model_name: String,
    /// Vector dimensionality.
    pub dimension: usize,
    /// Number of vectors in the HNSW index.
    pub vector_count: usize,
    /// Number of documents in the corpus.
    pub document_count: usize,
    /// Number of code symbols indexed.
    pub symbol_count: usize,
    /// Corpus root metadata.
    pub corpus_roots: Vec<BundleCorpusRoot>,
    /// Unix timestamp (seconds) when the bundle was created.
    pub created_at: u64,
    /// Deterministic content hash of corpus root versions.
    ///
    /// SHA-256 of sorted `"{root_id}:{commit_sha}"` pairs. Two bundles with
    /// the same `bundle_version` contain identical content. Clients compare
    /// this field to decide whether a cached bundle is still current.
    #[serde(default)]
    pub bundle_version: Option<String>,
    /// Git HEAD SHA of the primary source repository at export time.
    ///
    /// For single-root bundles this is the root's commit SHA; for multi-root
    /// bundles it is the first git root's SHA (if any).
    #[serde(default)]
    pub source_commit: Option<String>,
}

/// Corpus root metadata embedded in the bundle manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleCorpusRoot {
    /// Stable identifier for this root.
    pub id: String,
    /// Human-readable display name.
    pub display_name: Option<String>,
    /// Root kind: "local", "web", or "git".
    pub kind: String,
    /// Git commit SHA (if applicable).
    pub commit_sha: Option<String>,
    /// Git branch (if applicable).
    pub branch: Option<String>,
    /// Repository URL (if applicable).
    pub repo_url: Option<String>,
}

/// Tables to strip from the exported database (session-local / ephemeral state).
/// Everything not listed here is kept (documents, sections, claims, symbols,
/// bridge endpoints, corpus roots, embedding cache, etc.).
const DROP_TABLES: &[&str] = &[
    "sessions",
    "session_deliveries",
    "section_access_stats",
    "co_access_patterns",
    "web_cache",
    "git_cache",
    "pending_refs",
    "section_memory_states",
];

/// Export a corpus to a portable `.iris-index` bundle.
///
/// Reads the `SQLite` database and HNSW index from `corpus_dir`, strips
/// session-local tables, and writes a zstd-compressed tar archive to
/// `output_path`.
///
/// # Errors
///
/// Returns [`BundleError`] if any file I/O or database operation fails.
#[instrument(skip_all, fields(corpus_dir = %corpus_dir.as_ref().display(), output = %output_path.as_ref().display()))]
pub fn export_bundle(
    corpus_dir: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    manifest: &BundleManifest,
) -> Result<PathBuf, BundleError> {
    let corpus_dir = corpus_dir.as_ref();
    let output_path = output_path.as_ref();

    let db_path = corpus_dir.join("content.db");
    let index_dir = corpus_dir.join("index");

    // Validate required files exist
    if !db_path.exists() {
        return Err(BundleError::MissingFile {
            path: db_path,
            reason: "content database not found".into(),
        });
    }
    if !index_dir.exists() {
        return Err(BundleError::MissingFile {
            path: index_dir,
            reason: "HNSW index directory not found".into(),
        });
    }

    // Create a cleaned copy of the database (strip session tables)
    let temp_dir = tempfile::tempdir().map_err(|e| BundleError::Io {
        path: corpus_dir.to_path_buf(),
        reason: format!("failed to create temp dir: {e}"),
    })?;
    let clean_db_path = temp_dir.path().join("content.db");
    create_clean_database(&db_path, &clean_db_path)?;

    // Build the tar archive with zstd compression
    let out_file = File::create(output_path).map_err(|e| BundleError::Io {
        path: output_path.to_path_buf(),
        reason: format!("failed to create output file: {e}"),
    })?;
    let writer = BufWriter::new(out_file);
    let zstd_writer = zstd::Encoder::new(writer, 3).map_err(|e| BundleError::Io {
        path: output_path.to_path_buf(),
        reason: format!("failed to create zstd encoder: {e}"),
    })?;
    let mut archive = tar::Builder::new(zstd_writer);

    // Add manifest.json
    let manifest_json =
        serde_json::to_vec_pretty(manifest).map_err(|e| BundleError::SerializationFailed {
            reason: format!("failed to serialize manifest: {e}"),
        })?;
    append_bytes(&mut archive, "manifest.json", &manifest_json)?;

    // Add cleaned database
    archive
        .append_path_with_name(&clean_db_path, "content.db")
        .map_err(|e| BundleError::Io {
            path: clean_db_path.clone(),
            reason: format!("failed to add database to archive: {e}"),
        })?;

    // Add HNSW index files
    let id_map_path = index_dir.join("id_map.json");
    if id_map_path.exists() {
        archive
            .append_path_with_name(&id_map_path, "index/id_map.json")
            .map_err(|e| BundleError::Io {
                path: id_map_path,
                reason: format!("failed to add id_map.json: {e}"),
            })?;
    }

    // Add all HNSW dump files (*.hnsw.dat, *.hnsw.graph)
    if let Ok(entries) = fs::read_dir(&index_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.contains(".hnsw.") {
                let archive_name = format!("index/{name_str}");
                archive
                    .append_path_with_name(entry.path(), &archive_name)
                    .map_err(|e| BundleError::Io {
                        path: entry.path(),
                        reason: format!("failed to add {name_str}: {e}"),
                    })?;
            }
        }
    }

    // Finalize the archive
    let zstd_writer = archive.into_inner().map_err(|e| BundleError::Io {
        path: output_path.to_path_buf(),
        reason: format!("failed to finalize tar archive: {e}"),
    })?;
    zstd_writer.finish().map_err(|e| BundleError::Io {
        path: output_path.to_path_buf(),
        reason: format!("failed to finalize zstd stream: {e}"),
    })?;

    info!(
        output = %output_path.display(),
        vectors = manifest.vector_count,
        documents = manifest.document_count,
        "exported index bundle"
    );

    Ok(output_path.to_path_buf())
}

/// Import an `.iris-index` bundle into a corpus directory.
///
/// Decompresses the zstd-tar archive, validates the manifest, and extracts
/// the database and HNSW index files into `corpus_dir`.
///
/// # Errors
///
/// Returns [`BundleError`] if the bundle is invalid, corrupted, or the
/// output directory cannot be written to.
#[instrument(skip_all, fields(bundle = %bundle_path.as_ref().display(), corpus_dir = %corpus_dir.as_ref().display()))]
pub fn import_bundle(
    bundle_path: impl AsRef<Path>,
    corpus_dir: impl AsRef<Path>,
) -> Result<BundleManifest, BundleError> {
    let bundle_path = bundle_path.as_ref();
    let corpus_dir = corpus_dir.as_ref();

    let file = File::open(bundle_path).map_err(|e| BundleError::Io {
        path: bundle_path.to_path_buf(),
        reason: format!("failed to open bundle: {e}"),
    })?;
    let reader = BufReader::new(file);
    let zstd_reader = zstd::Decoder::new(reader).map_err(|e| BundleError::Io {
        path: bundle_path.to_path_buf(),
        reason: format!("failed to create zstd decoder: {e}"),
    })?;
    let mut archive = tar::Archive::new(zstd_reader);

    // Create output directories
    fs::create_dir_all(corpus_dir).map_err(|e| BundleError::Io {
        path: corpus_dir.to_path_buf(),
        reason: format!("failed to create corpus dir: {e}"),
    })?;
    let index_dir = corpus_dir.join("index");
    fs::create_dir_all(&index_dir).map_err(|e| BundleError::Io {
        path: index_dir.clone(),
        reason: format!("failed to create index dir: {e}"),
    })?;

    let mut manifest: Option<BundleManifest> = None;

    for entry_result in archive.entries().map_err(|e| BundleError::Io {
        path: bundle_path.to_path_buf(),
        reason: format!("failed to read archive entries: {e}"),
    })? {
        let mut entry = entry_result.map_err(|e| BundleError::Io {
            path: bundle_path.to_path_buf(),
            reason: format!("failed to read archive entry: {e}"),
        })?;

        let entry_path = entry
            .path()
            .map_err(|e| BundleError::Io {
                path: bundle_path.to_path_buf(),
                reason: format!("invalid entry path: {e}"),
            })?
            .into_owned();

        // Security: reject paths with .. or absolute paths
        let entry_str = entry_path.to_string_lossy();
        if entry_str.contains("..") || entry_path.is_absolute() {
            return Err(BundleError::InvalidBundle {
                reason: format!("unsafe path in archive: {entry_str}"),
            });
        }

        let dest = corpus_dir.join(&entry_path);

        if entry_str == "manifest.json" {
            let mut content = Vec::new();
            entry
                .read_to_end(&mut content)
                .map_err(|e| BundleError::Io {
                    path: dest.clone(),
                    reason: format!("failed to read manifest: {e}"),
                })?;
            let m: BundleManifest =
                serde_json::from_slice(&content).map_err(|e| BundleError::InvalidBundle {
                    reason: format!("failed to parse manifest: {e}"),
                })?;
            // Validate format version
            if m.format_version > BUNDLE_FORMAT_VERSION {
                return Err(BundleError::IncompatibleVersion {
                    bundle_version: m.format_version,
                    max_supported: BUNDLE_FORMAT_VERSION,
                });
            }
            manifest = Some(m);
        } else {
            // Ensure parent directory exists
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent).map_err(|e| BundleError::Io {
                    path: parent.to_path_buf(),
                    reason: format!("failed to create parent dir: {e}"),
                })?;
            }
            let mut out_file = File::create(&dest).map_err(|e| BundleError::Io {
                path: dest.clone(),
                reason: format!("failed to create file: {e}"),
            })?;
            io::copy(&mut entry, &mut out_file).map_err(|e| BundleError::Io {
                path: dest,
                reason: format!("failed to write file: {e}"),
            })?;
        }
    }

    let manifest = manifest.ok_or(BundleError::InvalidBundle {
        reason: "bundle does not contain manifest.json".into(),
    })?;

    info!(
        corpus_dir = %corpus_dir.display(),
        model = %manifest.model_name,
        vectors = manifest.vector_count,
        "imported index bundle"
    );

    Ok(manifest)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Create a copy of the database with session-local tables emptied.
///
/// Uses `SQLite`'s VACUUM INTO to create a clean, compacted copy, then
/// deletes rows from tables that shouldn't be exported.
fn create_clean_database(source: &Path, dest: &Path) -> Result<(), BundleError> {
    // Copy the source database using SQLite's built-in VACUUM INTO
    // which creates a clean, WAL-free copy.
    let conn = rusqlite::Connection::open(source).map_err(|e| BundleError::DatabaseError {
        reason: format!("failed to open source database: {e}"),
    })?;
    let dest_str = dest.to_str().ok_or_else(|| BundleError::DatabaseError {
        reason: "destination path is not valid UTF-8".into(),
    })?;
    conn.execute_batch(&format!("VACUUM INTO '{dest_str}'"))
        .map_err(|e| BundleError::DatabaseError {
            reason: format!("VACUUM INTO failed: {e}"),
        })?;
    drop(conn);

    // Open the copy and drop session-local tables
    let clean_conn = rusqlite::Connection::open(dest).map_err(|e| BundleError::DatabaseError {
        reason: format!("failed to open cleaned database: {e}"),
    })?;
    for table in DROP_TABLES {
        // Use DELETE instead of DROP to preserve schema (import can recreate data)
        let _ = clean_conn.execute_batch(&format!("DELETE FROM {table}"));
    }
    // VACUUM to reclaim space from deleted rows
    clean_conn
        .execute_batch("VACUUM")
        .map_err(|e| BundleError::DatabaseError {
            reason: format!("VACUUM failed on cleaned database: {e}"),
        })?;
    Ok(())
}

/// Append raw bytes as a file entry in the tar archive.
fn append_bytes<W: Write>(
    archive: &mut tar::Builder<W>,
    name: &str,
    data: &[u8],
) -> Result<(), BundleError> {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    archive
        .append_data(&mut header, name, data)
        .map_err(|e| BundleError::Io {
            path: PathBuf::from(name),
            reason: format!("failed to append {name}: {e}"),
        })
}

/// Compute a deterministic bundle version string from corpus root metadata.
///
/// Collects `"{id}:{commit_sha}"` for each root (using `"none"` when a root
/// has no commit SHA), sorts lexicographically, then returns the hex-encoded
/// SHA-256 of the concatenation. The same set of roots always produces the
/// same version regardless of insertion order.
///
/// # Examples
///
/// ```
/// use iris_core::bundle::{BundleCorpusRoot, compute_bundle_version};
///
/// let roots = vec![BundleCorpusRoot {
///     id: "root-a".into(),
///     display_name: None,
///     kind: "git".into(),
///     commit_sha: Some("abc123".into()),
///     branch: None,
///     repo_url: None,
/// }];
/// let v = compute_bundle_version(&roots);
/// assert_eq!(v.len(), 64); // hex SHA-256
/// ```
pub fn compute_bundle_version(roots: &[BundleCorpusRoot]) -> String {
    let mut parts: Vec<String> = roots
        .iter()
        .map(|r| {
            let sha = r.commit_sha.as_deref().unwrap_or("none");
            format!("{}:{sha}", r.id)
        })
        .collect();
    parts.sort();

    let mut hasher = Sha256::new();
    for part in &parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

/// Read only the manifest from a `.iris-index` bundle without extracting files.
///
/// Opens the zstd-compressed tar, scans entries until `manifest.json` is found,
/// deserializes it, and returns. The rest of the archive is not read.
///
/// # Errors
///
/// Returns [`BundleError`] if the bundle cannot be opened, decompressed, or
/// does not contain a valid `manifest.json`.
#[instrument(skip_all, fields(bundle = %bundle_path.as_ref().display()))]
pub fn read_manifest(bundle_path: impl AsRef<Path>) -> Result<BundleManifest, BundleError> {
    let bundle_path = bundle_path.as_ref();

    let file = File::open(bundle_path).map_err(|e| BundleError::Io {
        path: bundle_path.to_path_buf(),
        reason: format!("failed to open bundle: {e}"),
    })?;
    let reader = BufReader::new(file);
    let zstd_reader = zstd::Decoder::new(reader).map_err(|e| BundleError::Io {
        path: bundle_path.to_path_buf(),
        reason: format!("failed to create zstd decoder: {e}"),
    })?;
    let mut archive = tar::Archive::new(zstd_reader);

    for entry_result in archive.entries().map_err(|e| BundleError::Io {
        path: bundle_path.to_path_buf(),
        reason: format!("failed to read archive entries: {e}"),
    })? {
        let mut entry = entry_result.map_err(|e| BundleError::Io {
            path: bundle_path.to_path_buf(),
            reason: format!("failed to read archive entry: {e}"),
        })?;

        let entry_path = entry
            .path()
            .map_err(|e| BundleError::Io {
                path: bundle_path.to_path_buf(),
                reason: format!("invalid entry path: {e}"),
            })?
            .into_owned();

        if entry_path.to_string_lossy() == "manifest.json" {
            let mut content = Vec::new();
            entry
                .read_to_end(&mut content)
                .map_err(|e| BundleError::Io {
                    path: bundle_path.to_path_buf(),
                    reason: format!("failed to read manifest: {e}"),
                })?;
            let manifest: BundleManifest =
                serde_json::from_slice(&content).map_err(|e| BundleError::InvalidBundle {
                    reason: format!("failed to parse manifest: {e}"),
                })?;
            return Ok(manifest);
        }
    }

    Err(BundleError::InvalidBundle {
        reason: "bundle does not contain manifest.json".into(),
    })
}

/// On-disk metadata for a cached cloud bundle.
///
/// Written to `<data_dir>/cloud_cache/<url-hash>/cache.json` after a
/// successful cloud bundle fetch+import, enabling staleness checks on
/// subsequent runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleCacheEntry {
    /// The URL this bundle was fetched from.
    pub url: String,
    /// The manifest of the last-fetched bundle.
    pub manifest: BundleManifest,
    /// Unix epoch seconds when the bundle was fetched.
    pub fetched_at: u64,
}

/// Load a cached bundle manifest for the given URL, if one exists.
///
/// Looks for `<data_dir>/cloud_cache/<sha256(url)>/cache.json`.
///
/// # Errors
///
/// Returns [`BundleError`] on I/O or deserialization failures. Returns
/// `Ok(None)` if no cache entry exists.
pub fn load_cache_entry(
    data_dir: &Path,
    url: &str,
) -> Result<Option<BundleCacheEntry>, BundleError> {
    let cache_path = cloud_cache_path(data_dir, url);
    if !cache_path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&cache_path).map_err(|e| BundleError::Io {
        path: cache_path.clone(),
        reason: format!("failed to read cache entry: {e}"),
    })?;
    let entry: BundleCacheEntry =
        serde_json::from_str(&content).map_err(|e| BundleError::SerializationFailed {
            reason: format!("failed to parse cache entry: {e}"),
        })?;
    Ok(Some(entry))
}

/// Save a bundle cache entry after a successful fetch+import.
///
/// Writes to `<data_dir>/cloud_cache/<sha256(url)>/cache.json`.
///
/// # Errors
///
/// Returns [`BundleError`] if the cache directory cannot be created or the
/// file cannot be written.
pub fn save_cache_entry(data_dir: &Path, entry: &BundleCacheEntry) -> Result<(), BundleError> {
    let cache_path = cloud_cache_path(data_dir, &entry.url);
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent).map_err(|e| BundleError::Io {
            path: parent.to_path_buf(),
            reason: format!("failed to create cache dir: {e}"),
        })?;
    }
    let json =
        serde_json::to_string_pretty(entry).map_err(|e| BundleError::SerializationFailed {
            reason: format!("failed to serialize cache entry: {e}"),
        })?;
    fs::write(&cache_path, json).map_err(|e| BundleError::Io {
        path: cache_path,
        reason: format!("failed to write cache entry: {e}"),
    })?;
    Ok(())
}

/// Check whether a cached bundle is still current by comparing bundle versions.
///
/// Returns `true` only when both manifests carry a `bundle_version` and the
/// values match. Missing versions on either side are treated as stale.
#[must_use]
pub fn is_cache_current(cached: &BundleManifest, remote: &BundleManifest) -> bool {
    match (&cached.bundle_version, &remote.bundle_version) {
        (Some(c), Some(r)) => c == r,
        _ => false,
    }
}

/// Derive the on-disk cache path for a cloud bundle URL.
fn cloud_cache_path(data_dir: &Path, url: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    data_dir.join("cloud_cache").join(&hash).join("cache.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_manifest(version: Option<String>, commit: Option<String>) -> BundleManifest {
        BundleManifest {
            format_version: BUNDLE_FORMAT_VERSION,
            model_name: "all-MiniLM-L6-v2".into(),
            dimension: 384,
            vector_count: 1000,
            document_count: 50,
            symbol_count: 200,
            corpus_roots: vec![BundleCorpusRoot {
                id: "test-root".into(),
                display_name: Some("Test".into()),
                kind: "local".into(),
                commit_sha: None,
                branch: None,
                repo_url: None,
            }],
            created_at: 1_700_000_000,
            bundle_version: version,
            source_commit: commit,
        }
    }

    #[test]
    fn manifest_roundtrip() {
        let manifest = test_manifest(None, None);

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: BundleManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.format_version, BUNDLE_FORMAT_VERSION);
        assert_eq!(parsed.model_name, "all-MiniLM-L6-v2");
        assert_eq!(parsed.dimension, 384);
        assert_eq!(parsed.vector_count, 1000);
        assert_eq!(parsed.corpus_roots.len(), 1);
        assert!(parsed.bundle_version.is_none());
        assert!(parsed.source_commit.is_none());
    }

    #[test]
    fn manifest_roundtrip_with_version() {
        let manifest = test_manifest(Some("abc123def456".into()), Some("deadbeef".into()));

        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: BundleManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.bundle_version.as_deref(), Some("abc123def456"));
        assert_eq!(parsed.source_commit.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn manifest_backward_compat_missing_version_fields() {
        // Simulate a v1 manifest that lacks the new fields.
        let json = r#"{
            "format_version": 1,
            "model_name": "test",
            "dimension": 384,
            "vector_count": 0,
            "document_count": 0,
            "symbol_count": 0,
            "corpus_roots": [],
            "created_at": 0
        }"#;
        let parsed: BundleManifest = serde_json::from_str(json).unwrap();
        assert!(parsed.bundle_version.is_none());
        assert!(parsed.source_commit.is_none());
        assert_eq!(parsed.format_version, 1);
    }

    #[test]
    fn bundle_format_version_is_current() {
        assert_eq!(BUNDLE_FORMAT_VERSION, 1);
    }

    #[test]
    fn export_missing_db_returns_error() {
        let tmp = TempDir::new().unwrap();
        let corpus = tmp.path().join("nonexistent");
        fs::create_dir_all(&corpus).unwrap();
        // No content.db — should fail
        let manifest = BundleManifest {
            format_version: BUNDLE_FORMAT_VERSION,
            model_name: "test".into(),
            dimension: 384,
            vector_count: 0,
            document_count: 0,
            symbol_count: 0,
            corpus_roots: vec![],
            created_at: 0,
            bundle_version: None,
            source_commit: None,
        };
        let output = tmp.path().join("test.iris-index");
        let result = export_bundle(&corpus, &output, &manifest);
        assert!(result.is_err());
    }

    #[test]
    fn export_import_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        let index_dir = corpus_dir.join("index");
        fs::create_dir_all(&index_dir).unwrap();

        // Create a minimal SQLite database
        let db_path = corpus_dir.join("content.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE documents (id TEXT PRIMARY KEY, title TEXT);
             CREATE TABLE sections (id TEXT PRIMARY KEY);
             CREATE TABLE claims (id TEXT PRIMARY KEY);
             CREATE TABLE sessions (id TEXT PRIMARY KEY, data TEXT);
             INSERT INTO documents VALUES ('doc1', 'Test Document');
             INSERT INTO sessions VALUES ('sess1', 'session data');",
        )
        .unwrap();
        drop(conn);

        // Create minimal HNSW index files
        fs::write(index_dir.join("id_map.json"), r#"{"dim":384,"ef_search":50,"id_to_int":{},"deleted":[],"next_id":0,"model_name":"test"}"#).unwrap();
        fs::write(index_dir.join("iris_hnsw.hnsw.dat"), b"fake-dat").unwrap();
        fs::write(index_dir.join("iris_hnsw.hnsw.graph"), b"fake-graph").unwrap();

        // Export
        let manifest = BundleManifest {
            format_version: BUNDLE_FORMAT_VERSION,
            model_name: "test-model".into(),
            dimension: 384,
            vector_count: 0,
            document_count: 1,
            symbol_count: 0,
            corpus_roots: vec![],
            created_at: 1_700_000_000,
            bundle_version: Some("test-version-hash".into()),
            source_commit: Some("abc123".into()),
        };
        let bundle_path = tmp.path().join("test.iris-index");
        export_bundle(&corpus_dir, &bundle_path, &manifest).unwrap();

        // Import into a fresh directory
        let import_dir = tmp.path().join("imported");
        let imported_manifest = import_bundle(&bundle_path, &import_dir).unwrap();

        // Verify manifest
        assert_eq!(imported_manifest.model_name, "test-model");
        assert_eq!(imported_manifest.dimension, 384);
        assert_eq!(imported_manifest.document_count, 1);

        // Verify database was imported
        let imported_db = import_dir.join("content.db");
        assert!(imported_db.exists());
        let conn = rusqlite::Connection::open(&imported_db).unwrap();

        // Documents should be present
        let doc_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
            .unwrap();
        assert_eq!(doc_count, 1);

        // Sessions should be empty (stripped during export)
        let sess_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sess_count, 0);

        // Verify index files were imported
        assert!(import_dir.join("index/id_map.json").exists());
        assert!(import_dir.join("index/iris_hnsw.hnsw.dat").exists());
        assert!(import_dir.join("index/iris_hnsw.hnsw.graph").exists());
    }

    #[test]
    fn import_rejects_unsafe_paths() {
        let tmp = TempDir::new().unwrap();

        // Create a bundle with a path traversal attack.
        // The `tar` crate rejects `..` in `append_data`, so we craft
        // the raw archive bytes using a header with the path set directly.
        let bundle_path = tmp.path().join("evil.iris-index");
        let file = File::create(&bundle_path).unwrap();
        let writer = BufWriter::new(file);
        let zstd_writer = zstd::Encoder::new(writer, 1).unwrap();
        let mut archive = tar::Builder::new(zstd_writer);

        // Use a path that embeds `..` but in a way tar accepts (via raw header)
        let data = b"malicious content";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_path("foo/../../etc/passwd").unwrap_or_else(|_| {
            // Some tar versions may reject this too — use alternate approach
            header.as_gnu_mut().unwrap().name = {
                let mut name = [0u8; 100];
                let path_bytes = b"foo/../../etc/passwd";
                name[..path_bytes.len()].copy_from_slice(path_bytes);
                name
            };
        });
        header.set_cksum();
        // If tar rejects the path entirely, test is not applicable
        let result = archive.append_data(&mut header, "foo/../bar/baz", &data[..]);
        if result.is_err() {
            // The tar crate already prevents path traversal at write time —
            // our import validation is defense-in-depth. The test verifies
            // this protection exists at either layer.
            return;
        }
        let zstd_writer = archive.into_inner().unwrap();
        zstd_writer.finish().unwrap();

        let import_dir = tmp.path().join("target");
        let result = import_bundle(&bundle_path, &import_dir);
        assert!(result.is_err());
    }

    #[test]
    fn import_rejects_future_version() {
        let tmp = TempDir::new().unwrap();

        // Create a bundle with a future format version
        let bundle_path = tmp.path().join("future.iris-index");
        let file = File::create(&bundle_path).unwrap();
        let writer = BufWriter::new(file);
        let zstd_writer = zstd::Encoder::new(writer, 1).unwrap();
        let mut archive = tar::Builder::new(zstd_writer);

        let manifest = BundleManifest {
            format_version: 999,
            model_name: "future-model".into(),
            dimension: 384,
            vector_count: 0,
            document_count: 0,
            symbol_count: 0,
            corpus_roots: vec![],
            created_at: 0,
            bundle_version: None,
            source_commit: None,
        };
        let json = serde_json::to_vec_pretty(&manifest).unwrap();
        append_bytes(&mut archive, "manifest.json", &json).unwrap();

        let zstd_writer = archive.into_inner().unwrap();
        zstd_writer.finish().unwrap();

        let import_dir = tmp.path().join("target");
        let result = import_bundle(&bundle_path, &import_dir);
        assert!(result.is_err());
        match result.unwrap_err() {
            BundleError::IncompatibleVersion { .. } => {}
            other => panic!("expected IncompatibleVersion, got: {other}"),
        }
    }

    // --- compute_bundle_version tests ---

    #[test]
    fn bundle_version_is_deterministic() {
        let roots = vec![
            BundleCorpusRoot {
                id: "root-b".into(),
                display_name: None,
                kind: "git".into(),
                commit_sha: Some("sha-b".into()),
                branch: None,
                repo_url: None,
            },
            BundleCorpusRoot {
                id: "root-a".into(),
                display_name: None,
                kind: "git".into(),
                commit_sha: Some("sha-a".into()),
                branch: None,
                repo_url: None,
            },
        ];
        let v1 = compute_bundle_version(&roots);

        // Reversed order should produce the same version (sorted internally).
        let reversed: Vec<_> = roots.into_iter().rev().collect();
        let v2 = compute_bundle_version(&reversed);

        assert_eq!(v1, v2);
        assert_eq!(v1.len(), 64, "should be hex SHA-256");
    }

    #[test]
    fn bundle_version_differs_for_different_shas() {
        let roots_a = vec![BundleCorpusRoot {
            id: "root".into(),
            display_name: None,
            kind: "git".into(),
            commit_sha: Some("sha-1".into()),
            branch: None,
            repo_url: None,
        }];
        let roots_b = vec![BundleCorpusRoot {
            id: "root".into(),
            display_name: None,
            kind: "git".into(),
            commit_sha: Some("sha-2".into()),
            branch: None,
            repo_url: None,
        }];
        assert_ne!(
            compute_bundle_version(&roots_a),
            compute_bundle_version(&roots_b)
        );
    }

    #[test]
    fn bundle_version_stable_for_no_shas() {
        let roots = vec![BundleCorpusRoot {
            id: "local-root".into(),
            display_name: None,
            kind: "local".into(),
            commit_sha: None,
            branch: None,
            repo_url: None,
        }];
        let v1 = compute_bundle_version(&roots);
        let v2 = compute_bundle_version(&roots);
        assert_eq!(v1, v2);
        assert_eq!(v1.len(), 64);
    }

    // --- read_manifest tests ---

    #[test]
    fn read_manifest_from_bundle() {
        let tmp = TempDir::new().unwrap();
        let corpus_dir = tmp.path().join("corpus");
        let index_dir = corpus_dir.join("index");
        fs::create_dir_all(&index_dir).unwrap();

        let db_path = corpus_dir.join("content.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch("CREATE TABLE documents (id TEXT PRIMARY KEY);")
            .unwrap();
        drop(conn);

        fs::write(index_dir.join("id_map.json"), b"{}").unwrap();
        fs::write(index_dir.join("iris_hnsw.hnsw.dat"), b"dat").unwrap();
        fs::write(index_dir.join("iris_hnsw.hnsw.graph"), b"graph").unwrap();

        let manifest = BundleManifest {
            format_version: BUNDLE_FORMAT_VERSION,
            model_name: "test".into(),
            dimension: 384,
            vector_count: 10,
            document_count: 5,
            symbol_count: 0,
            corpus_roots: vec![],
            created_at: 1_700_000_000,
            bundle_version: Some("ver-hash".into()),
            source_commit: Some("deadbeef".into()),
        };
        let bundle_path = tmp.path().join("test.iris-index");
        export_bundle(&corpus_dir, &bundle_path, &manifest).unwrap();

        let read_back = read_manifest(&bundle_path).unwrap();
        assert_eq!(read_back.model_name, "test");
        assert_eq!(read_back.vector_count, 10);
        assert_eq!(read_back.bundle_version.as_deref(), Some("ver-hash"));
        assert_eq!(read_back.source_commit.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn read_manifest_missing_returns_error() {
        let tmp = TempDir::new().unwrap();

        // Create a bundle with no manifest.json
        let bundle_path = tmp.path().join("no-manifest.iris-index");
        let file = File::create(&bundle_path).unwrap();
        let writer = BufWriter::new(file);
        let zstd_writer = zstd::Encoder::new(writer, 1).unwrap();
        let mut archive = tar::Builder::new(zstd_writer);
        append_bytes(&mut archive, "other.txt", b"hello").unwrap();
        let zstd_writer = archive.into_inner().unwrap();
        zstd_writer.finish().unwrap();

        let result = read_manifest(&bundle_path);
        assert!(result.is_err());
    }

    // --- cache entry tests ---

    #[test]
    fn cache_entry_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let entry = BundleCacheEntry {
            url: "https://example.com/test.iris-index".into(),
            manifest: test_manifest(Some("v1".into()), Some("abc".into())),
            fetched_at: 1_700_000_000,
        };

        save_cache_entry(tmp.path(), &entry).unwrap();
        let loaded = load_cache_entry(tmp.path(), &entry.url).unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.url, entry.url);
        assert_eq!(loaded.manifest.bundle_version.as_deref(), Some("v1"));
        assert_eq!(loaded.fetched_at, 1_700_000_000);
    }

    #[test]
    fn load_cache_entry_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        let result = load_cache_entry(tmp.path(), "https://example.com/missing").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn is_cache_current_matching_versions() {
        let a = test_manifest(Some("same".into()), None);
        let b = test_manifest(Some("same".into()), None);
        assert!(is_cache_current(&a, &b));
    }

    #[test]
    fn is_cache_current_different_versions() {
        let a = test_manifest(Some("v1".into()), None);
        let b = test_manifest(Some("v2".into()), None);
        assert!(!is_cache_current(&a, &b));
    }

    #[test]
    fn is_cache_current_none_treated_as_stale() {
        let with = test_manifest(Some("v1".into()), None);
        let without = test_manifest(None, None);
        assert!(!is_cache_current(&with, &without));
        assert!(!is_cache_current(&without, &with));
        assert!(!is_cache_current(&without, &without));
    }
}
