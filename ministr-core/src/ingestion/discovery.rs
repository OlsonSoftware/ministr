//! File discovery: walking directories, applying ignore rules, and resolving glob patterns.

use std::path::{Path, PathBuf};

use crate::error::IngestionError;
use crate::parser::detect_parser_kind;

const ALWAYS_IGNORE_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".next",
    ".nuxt",
    ".output",
    "__pycache__",
    ".venv",
    "venv",
    "env",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".gradle",
    ".mvn",
    ".go",
    "dist",
    "build",
    "out",
    ".git",
    ".svn",
    ".hg",
    ".idea",
    ".vs",
    ".vscode",
    ".zed",
    ".cache",
    ".fastembed_cache",
    ".onnx_cache",
    ".ministr",
    "vendor",
    "coverage",
    ".nyc_output",
    "htmlcov",
    ".terraform",
];

const ALWAYS_IGNORE_PATTERNS: &[&str] = &[
    "*.min.js",
    "*.min.css",
    "*.map",
    "*.chunk.js",
    "*.bundle.js",
    "*.lock",
    "package-lock.json",
    "Cargo.lock",
    "yarn.lock",
    "pnpm-lock.yaml",
    "tokenizer.json",
    "*.onnx",
    "*.bin",
    "*.safetensors",
    "*.snap",
];

/// Hard guard: reject files inside always-ignored directories.
pub(super) fn is_in_ignored_dir(path: &Path) -> bool {
    for component in path.components() {
        if let std::path::Component::Normal(name) = component
            && let Some(name_str) = name.to_str()
            && ALWAYS_IGNORE_DIRS.contains(&name_str)
        {
            return true;
        }
    }
    false
}

/// Discover all supported files in a directory recursively.
///
/// Respects `.gitignore` rules and skips well-known junk directories and file patterns.
#[must_use = "returns discovered files"]
pub fn discover_files(dir: &Path) -> Result<Vec<PathBuf>, IngestionError> {
    use ignore::WalkBuilder;
    use ignore::overrides::OverrideBuilder;

    let mut overrides = OverrideBuilder::new(dir);
    for pattern in ALWAYS_IGNORE_PATTERNS {
        let _ = overrides.add(&format!("!{pattern}"));
    }
    let overrides = overrides.build().map_err(|e| IngestionError::Io {
        path: dir.to_path_buf(),
        source: std::io::Error::other(format!("invalid ignore pattern: {e}")),
    })?;

    let mut walker = WalkBuilder::new(dir);
    walker
        .hidden(false)
        .parents(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .overrides(overrides)
        .filter_entry(|entry| {
            if entry.file_type().is_some_and(|ft| ft.is_dir())
                && let Some(name) = entry.file_name().to_str()
                && ALWAYS_IGNORE_DIRS.contains(&name)
            {
                return false;
            }
            true
        });

    let mut files = Vec::new();
    for result in walker.build() {
        let entry = result.map_err(|e| IngestionError::Io {
            path: dir.to_path_buf(),
            source: std::io::Error::other(format!("walk error: {e}")),
        })?;
        let path = entry.into_path();
        if path.is_file() && is_supported_file(&path) {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

/// Discover supported files from a mix of directories, individual files, and glob patterns.
///
/// # Examples
///
/// ```no_run
/// use ministr_core::ingestion::discover_paths;
/// use std::path::PathBuf;
///
/// let paths = vec![
///     PathBuf::from("docs/"),
///     PathBuf::from("README.md"),
///     PathBuf::from("*.md"),
/// ];
/// let files = discover_paths(&paths).unwrap();
/// ```
#[must_use = "returns discovered files"]
pub fn discover_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>, IngestionError> {
    use std::collections::HashSet;

    let mut all_files = Vec::new();
    let mut seen = HashSet::new();

    for path in paths {
        let path_str = path.to_string_lossy();

        if path_str.contains('*') || path_str.contains('?') || path_str.contains('[') {
            let entries = glob::glob(&path_str).map_err(|e| IngestionError::Io {
                path: path.clone(),
                source: std::io::Error::new(std::io::ErrorKind::InvalidInput, e.to_string()),
            })?;

            for entry in entries {
                let entry_path = entry.map_err(|e| IngestionError::Io {
                    path: path.clone(),
                    source: std::io::Error::other(e.to_string()),
                })?;
                collect_path_entry(&entry_path, &mut all_files, &mut seen)?;
            }
        } else {
            collect_path_entry(path, &mut all_files, &mut seen)?;
        }
    }

    all_files.sort();
    Ok(all_files)
}

fn collect_path_entry(
    path: &Path,
    files: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<PathBuf>,
) -> Result<(), IngestionError> {
    if path.is_dir() {
        let dir_files = discover_files(path)?;
        for f in dir_files {
            let canonical = f.canonicalize().unwrap_or_else(|_| f.clone());
            if seen.insert(canonical) {
                files.push(f);
            }
        }
    } else if path.is_file() && is_supported_file(path) {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if seen.insert(canonical) {
            files.push(path.to_path_buf());
        }
    }
    Ok(())
}

pub(super) fn is_supported_file(path: &Path) -> bool {
    detect_parser_kind(path).is_some()
}

/// Stat-fingerprint of an entire corpus root, plus the discovered file list.
///
/// Walks `dir` exactly once, computing both:
/// - `Vec<PathBuf>` — every supported file (same set [`discover_files`] returns)
/// - `String` — a BLAKE3 hex digest over the sorted
///   `(rel_path, mtime_ns, size)` tuples of those files
///
/// The fingerprint deliberately avoids reading file *contents*: hashing
/// 10 GB of source on every reindex defeats the purpose. mtime+size is
/// the standard fast-fingerprint used by Cursor, CocoIndex, and other
/// 2026-era incremental indexers. When mtime drifts but content
/// actually matches (rare but possible — e.g. `touch -a`), the existing
/// per-file `file_hashes.content_hash` cache catches it inside the
/// regular ingestion path.
///
/// Determinism: the file list is sorted lexicographically before
/// hashing, so the same corpus state always produces the same root
/// hash regardless of walk order. The version prefix `v1\0` lets us
/// tweak the fingerprint encoding later without colliding.
///
/// # Errors
///
/// Returns [`IngestionError::Io`] when the walk fails or any file's
/// metadata cannot be read.
#[must_use = "returns (root_hash, files)"]
pub fn compute_corpus_stat_merkle(
    dir: &Path,
) -> Result<(String, Vec<PathBuf>), IngestionError> {
    use ignore::WalkBuilder;
    use ignore::overrides::OverrideBuilder;

    let mut overrides = OverrideBuilder::new(dir);
    for pattern in ALWAYS_IGNORE_PATTERNS {
        let _ = overrides.add(&format!("!{pattern}"));
    }
    let overrides = overrides.build().map_err(|e| IngestionError::Io {
        path: dir.to_path_buf(),
        source: std::io::Error::other(format!("invalid ignore pattern: {e}")),
    })?;

    let mut walker = WalkBuilder::new(dir);
    walker
        .hidden(false)
        .parents(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .overrides(overrides)
        .filter_entry(|entry| {
            if entry.file_type().is_some_and(|ft| ft.is_dir())
                && let Some(name) = entry.file_name().to_str()
                && ALWAYS_IGNORE_DIRS.contains(&name)
            {
                return false;
            }
            true
        });

    // Collect (rel_path, abs_path, mtime_ns, size) tuples; we need both
    // the relative form (for the fingerprint, so absolute paths don't
    // poison the hash on a different machine) and the absolute form
    // (for the returned file list).
    let mut entries: Vec<(String, PathBuf, i64, u64)> = Vec::new();
    for result in walker.build() {
        let entry = result.map_err(|e| IngestionError::Io {
            path: dir.to_path_buf(),
            source: std::io::Error::other(format!("walk error: {e}")),
        })?;
        let path = entry.into_path();
        if !path.is_file() || !is_supported_file(&path) {
            continue;
        }
        let meta = std::fs::metadata(&path).map_err(|e| IngestionError::Io {
            path: path.clone(),
            source: e,
        })?;
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| i64::try_from(d.as_nanos()).ok())
            .unwrap_or(0);
        let size = meta.len();
        let rel = path
            .strip_prefix(dir)
            .unwrap_or(&path)
            .to_string_lossy()
            // Normalize Windows backslashes so the fingerprint matches
            // across platforms for the same corpus content.
            .replace('\\', "/");
        entries.push((rel, path, mtime_ns, size));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = blake3::Hasher::new();
    // Version tag — bump this constant whenever the fingerprint encoding
    // changes (e.g. if we add ctime, switch hash algorithm, etc.).
    hasher.update(b"v1\0");
    for (rel, _, mtime_ns, size) in &entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\0");
        hasher.update(&mtime_ns.to_le_bytes());
        hasher.update(&size.to_le_bytes());
        hasher.update(b"\n");
    }
    let root_hash = hasher.finalize().to_hex().to_string();

    let mut files: Vec<PathBuf> = entries.into_iter().map(|(_, abs, _, _)| abs).collect();
    files.sort();
    Ok((root_hash, files))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn compute_corpus_stat_merkle_is_deterministic_and_skips_unsupported() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("a.rs"), "fn main(){}");
        write(&tmp.path().join("b.md"), "# hello");
        // Unsupported binary — must not affect the fingerprint.
        write(&tmp.path().join("ignore.bin"), "\x00\x01\x02");

        let (h1, files1) = compute_corpus_stat_merkle(tmp.path()).unwrap();
        let (h2, files2) = compute_corpus_stat_merkle(tmp.path()).unwrap();

        assert_eq!(h1, h2, "stable across calls");
        assert_eq!(files1, files2, "file list stable");
        assert_eq!(files1.len(), 2, "binary excluded");
        assert_eq!(h1.len(), 64, "blake3 hex is 64 chars");
    }

    #[test]
    fn compute_corpus_stat_merkle_changes_when_file_grows() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("a.rs"), "fn main(){}");
        let (h1, _) = compute_corpus_stat_merkle(tmp.path()).unwrap();

        // Append more content — size + mtime both shift.
        write(&tmp.path().join("a.rs"), "fn main(){}\nfn extra(){}");
        let (h2, _) = compute_corpus_stat_merkle(tmp.path()).unwrap();

        assert_ne!(h1, h2, "fingerprint reflects size/mtime change");
    }

    #[test]
    fn compute_corpus_stat_merkle_changes_when_file_added() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("a.rs"), "fn main(){}");
        let (h1, files1) = compute_corpus_stat_merkle(tmp.path()).unwrap();

        write(&tmp.path().join("b.rs"), "fn b(){}");
        let (h2, files2) = compute_corpus_stat_merkle(tmp.path()).unwrap();

        assert_ne!(h1, h2);
        assert_eq!(files1.len() + 1, files2.len());
    }
}
