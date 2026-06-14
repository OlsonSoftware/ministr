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
use crate::ingestion::{
    compute_content_hash, compute_root_id, discover_files_with_ignores, namespace_path,
};
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
    ignore_patterns: &[String],
) -> Result<Vec<FileFreshness>, IngestionError> {
    let multi_root = roots.len() > 1;
    let mut by_path: HashMap<&str, &FileHashRecord> =
        records.iter().map(|r| (r.path.as_str(), r)).collect();

    let mut out = Vec::new();
    for root in roots {
        let root_id = compute_root_id(root);
        for file in discover_files_with_ignores(root, ignore_patterns)? {
            let Ok(rel) = file.strip_prefix(root) else {
                continue;
            };
            let rel = rel.to_string_lossy().replace('\\', "/");
            let namespaced = namespace_path(&root_id, &rel);
            // The key the GUI displays: namespaced for multi-root corpora,
            // bare-relative for single-root.
            let display_key = if multi_root {
                namespaced.clone()
            } else {
                rel.clone()
            };
            // Tolerant join: accept whichever key shape ingestion actually
            // wrote for this file — namespaced (`rid/rel`, rooted),
            // bare-relative (single- / NULL-root), OR the absolute path.
            // The rooted writer keys some corpora by their full absolute
            // path (a `strip_prefix` fallback in `build_file_items` /
            // `parse_and_store_file`; tracked by ingest-rooted-abs-path-keys).
            // Without the absolute candidate, every such file is reported
            // New while its record is reported Missing, so the corpus shows
            // permanently "out of date" and no reindex can clear it.
            let abs = file.to_string_lossy().replace('\\', "/");
            let record = by_path
                .remove(namespaced.as_str())
                .or_else(|| by_path.remove(rel.as_str()))
                .or_else(|| by_path.remove(abs.as_str()));
            let state = match record {
                None => FreshnessState::New,
                // Unreadable/non-UTF-8 now but indexed before reads as
                // stale: the index can no longer mirror it.
                Some(rec) => match hash_working_file(&file) {
                    Some(h) if h == rec.content_hash => FreshnessState::Current,
                    _ => FreshnessState::Stale,
                },
            };
            out.push(FileFreshness {
                path: display_key,
                state,
            });
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

        let report = compute_freshness(&[root.to_path_buf()], &records, &[]).unwrap();
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
        let report = compute_freshness(&[root.to_path_buf()], &[record], &[]).unwrap();
        assert_eq!(state_of(&report, "a.rs"), &FreshnessState::Stale);
    }

    #[test]
    fn namespaced_records_join_across_roots() {
        // Genuinely multi-root (2 roots) so the namespaced key is the
        // reported display key — single-root corpora report bare-relative.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let other = dir.path().join("other");
        write(&root, "lib.rs", "pub fn x() {}");
        write(&other, "keep.rs", "pub fn y() {}");
        let root_id = compute_root_id(&root);
        let records = vec![rec(&namespace_path(&root_id, "lib.rs"), "pub fn x() {}")];
        let report = compute_freshness(&[root.clone(), other.clone()], &records, &[]).unwrap();
        assert_eq!(
            state_of(&report, &namespace_path(&root_id, "lib.rs")),
            &FreshnessState::Current
        );
    }

    #[test]
    fn absolute_keyed_records_still_match() {
        // Regression (freshness-abs-key-match): the rooted writer keys some
        // corpora by each file's ABSOLUTE path. The sweep must still match
        // those records — not report a phantom New (file) + Missing (record)
        // for every file, which made the GUI show a corpus permanently "out
        // of date" with no reindex able to clear it. Present files resolve
        // Current/Stale and are reported under the clean relative key.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "src/same.rs", "fn same() {}");
        write(root, "src/changed.rs", "fn changed() { /* v2 */ }");

        let abs = |rel: &str| root.join(rel).to_string_lossy().replace('\\', "/");
        let records = vec![
            rec(&abs("src/same.rs"), "fn same() {}"),
            rec(&abs("src/changed.rs"), "fn changed() {}"),
            rec(&abs("src/deleted.rs"), "fn gone() {}"),
        ];

        let report = compute_freshness(&[root.to_path_buf()], &records, &[]).unwrap();
        // Matched by the absolute candidate, reported under the relative key.
        assert_eq!(state_of(&report, "src/same.rs"), &FreshnessState::Current);
        assert_eq!(state_of(&report, "src/changed.rs"), &FreshnessState::Stale);
        // The genuinely deleted file's absolute record is still Missing.
        assert_eq!(
            state_of(&report, &abs("src/deleted.rs")),
            &FreshnessState::Missing
        );
        // No phantom duplicates: a present file appears once, relative-keyed,
        // and never under its absolute path.
        assert_eq!(report.iter().filter(|f| f.path == "src/same.rs").count(), 1);
        assert!(!report.iter().any(|f| f.path == abs("src/same.rs")));
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
        let report = compute_freshness(&[root.to_path_buf()], &[], &[]).unwrap();
        assert!(report.iter().all(|f| !f.path.contains("node_modules")));
        assert_eq!(state_of(&report, "seen.rs"), &FreshnessState::New);
    }

    // corpus-ignore-enforcement-gap: the freshness sweep honors user ignore
    // patterns — an ignored on-disk file is invisible (no "new" noise), and a
    // file that was indexed and then ignored reports Missing.
    #[test]
    fn ignored_files_are_invisible_to_the_sweep() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        std::fs::write(root.join("keep.rs"), "fn main() {}").unwrap();
        std::fs::write(root.join("skipme.md"), "# snapshot").unwrap();

        let report =
            compute_freshness(std::slice::from_ref(&root), &[], &["*me.md".to_owned()]).unwrap();
        let paths: Vec<&str> = report.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"keep.rs"), "got {paths:?}");
        assert!(
            !paths.iter().any(|p| p.contains("skipme.md")),
            "ignored file must not appear in the sweep: {paths:?}"
        );

        // Indexed before, ignored now → Missing (the index still remembers it).
        let record = FileHashRecord {
            path: "skipme.md".to_owned(),
            content_hash: "deadbeef".to_owned(),
            mtime_ns: Some(0),
            extractor_version: 0,
            resolver_version: 0,
        };
        let report = compute_freshness(&[root], &[record], &["*me.md".to_owned()]).unwrap();
        let snap = report.iter().find(|f| f.path == "skipme.md").unwrap();
        assert_eq!(snap.state, FreshnessState::Missing);
    }
}
