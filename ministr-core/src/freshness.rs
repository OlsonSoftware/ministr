//! Per-file freshness truth — the "tree never lies" backend
//! (gui-rw-backend-freshness, UX-BLUEPRINT v4 invariant 1).
//!
//! Computes, for every file an agent could see, whether the index holds
//! the CURRENT content — by re-hashing the working tree with the exact
//! same walker ([`crate::ingestion::discover_files`]) and hash
//! ([`crate::ingestion::compute_content_hash`]) the indexer uses, and
//! comparing against the stored [`FileHashRecord`]s.
//!
//! Deliberately NO mtime fast-path: the indexer may trust mtimes for
//! throughput, but this module's whole purpose is hash-verified truth
//! for the GUI's trust display. blake3 makes the sweep cheap enough.
//!
//! Excluded files are NOT enumerated here — the walker never visits
//! them (enumerating `node_modules` would be explosive); the GUI shows
//! exclusion from the ignore RULES instead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::IngestionError;
use crate::ingestion::{compute_content_hash, compute_root_id, discover_files, namespace_path};
use crate::storage::FileHashRecord;

/// One file's hash-verified trust state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FreshnessState {
    /// Working-tree content hash equals the indexed hash.
    Current,
    /// The file exists in both but the content differs — the index is
    /// behind the working tree.
    Stale,
    /// On disk (and indexable) but the index has never seen it.
    New,
    /// The index has it but it is gone from the working tree (deleted,
    /// renamed, or newly ignored).
    Missing,
}

/// One file's freshness verdict. `path` uses the same key the index
/// stores (namespaced `root-<id>/rel` for multi-root corpora, plain
/// relative otherwise), so it joins 1:1 with the files API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileFreshness {
    pub path: String,
    pub state: FreshnessState,
}

/// Hash-verify every discoverable file under `roots` against `records`.
///
/// Synchronous and I/O-heavy (reads every file): callers on an async
/// runtime should wrap it in `spawn_blocking`.
///
/// # Errors
///
/// Returns [`IngestionError`] only for walk failures on a root; an
/// individual unreadable or non-UTF-8 file is skipped exactly as the
/// indexer would skip it (it can't be indexed, so it has no trust state).
pub fn compute_freshness(
    roots: &[PathBuf],
    records: &[FileHashRecord],
) -> Result<Vec<FileFreshness>, IngestionError> {
    let multi_root = roots.len() > 1;
    let mut by_path: HashMap<&str, &FileHashRecord> =
        records.iter().map(|r| (r.path.as_str(), r)).collect();

    let mut out = Vec::new();
    for root in roots {
        let root_id = compute_root_id(root);
        for file in discover_files(root)? {
            let Ok(rel) = file.strip_prefix(root) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let namespaced = namespace_path(&root_id, &rel);
            // Tolerant join: storage keys are namespaced for multi-root
            // corpora and bare-relative for single-root; accept either.
            let (key, record) = match by_path.remove_entry(namespaced.as_str()) {
                Some((k, r)) => (k.to_owned(), Some(r)),
                None => match by_path.remove_entry(rel.as_str()) {
                    Some((k, r)) => (k.to_owned(), Some(r)),
                    None => (
                        if multi_root {
                            namespaced.clone()
                        } else {
                            rel.clone()
                        },
                        None,
                    ),
                },
            };
            let state = match record {
                None => FreshnessState::New,
                // Unreadable/non-UTF-8 now but indexed before reads as
                // stale: the index can no longer mirror it.
                Some(rec) => match hash_working_file(&file) {
                    Some(h) if h == rec.content_hash => FreshnessState::Current,
                    _ => FreshnessState::Stale,
                },
            };
            out.push(FileFreshness { path: key, state });
        }
    }

    // Anything left in the record map was never matched by the walk:
    // deleted, renamed, or newly ignored.
    out.extend(by_path.keys().map(|p| FileFreshness {
        path: (*p).to_owned(),
        state: FreshnessState::Missing,
    }));

    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Hash a working-tree file EXACTLY as ingestion does: bytes → UTF-8
/// string (reject, don't replace) → blake3. Any divergence here breaks
/// the "tree never lies" invariant.
fn hash_working_file(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let content = String::from_utf8(bytes).ok()?;
    Some(compute_content_hash(&content))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(path: &str, content: &str) -> FileHashRecord {
        FileHashRecord {
            path: path.to_owned(),
            content_hash: compute_content_hash(content),
            mtime_ns: None,
            extractor_version: 0,
            resolver_version: 0,
        }
    }

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, content).unwrap();
    }

    fn state_of<'a>(report: &'a [FileFreshness], path: &str) -> &'a FreshnessState {
        &report
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("no entry for {path}"))
            .state
    }

    #[test]
    fn current_stale_new_missing_all_detected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/same.rs", "fn same() {}");
        write(root, "src/changed.rs", "fn changed() { /* v2 */ }");
        write(root, "src/brand_new.rs", "fn new_file() {}");

        let records = vec![
            rec("src/same.rs", "fn same() {}"),
            rec("src/changed.rs", "fn changed() {}"),
            rec("src/deleted.rs", "fn gone() {}"),
        ];

        let report = compute_freshness(&[root.to_path_buf()], &records).unwrap();
        assert_eq!(state_of(&report, "src/same.rs"), &FreshnessState::Current);
        assert_eq!(state_of(&report, "src/changed.rs"), &FreshnessState::Stale);
        assert_eq!(state_of(&report, "src/brand_new.rs"), &FreshnessState::New);
        assert_eq!(
            state_of(&report, "src/deleted.rs"),
            &FreshnessState::Missing
        );
    }

    #[test]
    fn mtime_is_never_trusted() {
        // A content change with any mtime story must read Stale — the
        // verdict comes from the hash alone.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "a.rs", "fn v2() {}");
        let mut record = rec("a.rs", "fn v1() {}");
        record.mtime_ns = Some(0); // irrelevant by design
        let report = compute_freshness(&[root.to_path_buf()], &[record]).unwrap();
        assert_eq!(state_of(&report, "a.rs"), &FreshnessState::Stale);
    }

    #[test]
    fn namespaced_records_join_across_roots() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        write(&root, "lib.rs", "pub fn x() {}");
        let root_id = compute_root_id(&root);
        let records = vec![rec(&namespace_path(&root_id, "lib.rs"), "pub fn x() {}")];
        let report = compute_freshness(std::slice::from_ref(&root), &records).unwrap();
        assert_eq!(
            state_of(&report, &namespace_path(&root_id, "lib.rs")),
            &FreshnessState::Current
        );
    }

    #[test]
    fn ignored_files_are_invisible() {
        // Uses the walker's always-ignore directory list (node_modules)
        // rather than .gitignore: gitignore rules only apply inside a
        // git repo, and this temp dir isn't one — faithful to how the
        // indexer itself behaves.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "node_modules/dep/index.js", "hidden");
        write(root, "seen.rs", "fn seen() {}");
        let report = compute_freshness(&[root.to_path_buf()], &[]).unwrap();
        assert!(report.iter().all(|f| !f.path.contains("node_modules")));
        assert_eq!(state_of(&report, "seen.rs"), &FreshnessState::New);
    }
}
