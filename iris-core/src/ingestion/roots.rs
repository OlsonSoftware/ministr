//! Corpus root management: path helpers, language statistics, and content hashing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use tracing::warn;

use crate::error::IngestionError;
use crate::storage::Storage;
use crate::types::{CorpusRoot, RootKind};

// ── Path helpers ─────────────────────────────────────────────────────────────

/// Compute a relative path — just strips leading `./`.
pub(super) fn compute_relative_path(file: &Path, _sources: &[PathBuf]) -> String {
    let s = file.to_string_lossy();
    s.strip_prefix("./").unwrap_or(&s).to_string()
}

/// Derive a module path from a relative file path.
///
/// ```ignore
/// // "session/mod.rs" → ["session"]
/// // "session/budget.rs" → ["session", "budget"]
/// // "iris-core/src/config.rs" → ["config"]
/// ```
pub(super) fn module_path_from_file(relative_path: &str) -> Vec<String> {
    let path = Path::new(relative_path);

    let components: Vec<&str> = path
        .components()
        .filter_map(|c| {
            if let std::path::Component::Normal(s) = c {
                s.to_str()
            } else {
                None
            }
        })
        .collect();

    let start = components
        .iter()
        .rposition(|&c| c == "src")
        .map_or(0, |i| i + 1);

    let dir_and_file = &components[start..];
    let mut parts: Vec<String> = if dir_and_file.len() > 1 {
        dir_and_file[..dir_and_file.len() - 1]
            .iter()
            .map(|s| (*s).to_string())
            .collect()
    } else {
        Vec::new()
    };

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");
    if !matches!(stem, "lib" | "main" | "mod" | "index") {
        parts.push(stem.to_string());
    }

    parts
}

/// Compute a stable root ID from a path by hashing it.
///
/// # Examples
///
/// ```no_run
/// use iris_core::ingestion::compute_root_id;
/// use std::path::Path;
///
/// let id = compute_root_id(Path::new("/some/path"));
/// assert!(id.starts_with("root-"));
/// ```
#[must_use]
pub fn compute_root_id(path: &Path) -> String {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let hash = hasher.finalize();
    format!(
        "root-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        hash[0], hash[1], hash[2], hash[3], hash[4], hash[5], hash[6], hash[7]
    )
}

/// Prefix a relative path with a root ID to create a namespaced storage path.
///
/// # Examples
///
/// ```
/// use iris_core::ingestion::namespace_path;
///
/// assert_eq!(
///     namespace_path("root-0011223344556677", "src/lib.rs"),
///     "root-0011223344556677/src/lib.rs"
/// );
/// ```
#[must_use]
pub fn namespace_path(root_id: &str, relative_path: &str) -> String {
    format!("{root_id}/{relative_path}")
}

/// Strip a root ID prefix from a namespaced path.
///
/// # Examples
///
/// ```
/// use iris_core::ingestion::strip_root_prefix;
///
/// assert_eq!(
///     strip_root_prefix("root-0011223344556677/src/lib.rs"),
///     Some("src/lib.rs")
/// );
/// assert_eq!(strip_root_prefix("src/lib.rs"), None);
/// ```
#[must_use]
pub fn strip_root_prefix(path: &str) -> Option<&str> {
    if path.len() > 22 && path.starts_with("root-") {
        let slash_pos = path.find('/')?;
        let prefix = &path[..slash_pos];
        if prefix.len() == 21 && prefix[5..].chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(&path[slash_pos + 1..]);
        }
    }
    None
}

/// Determine which source root a file belongs to (longest prefix match).
pub(super) fn find_root_for_file<'a>(
    file: &Path,
    roots: &'a [(PathBuf, String)],
) -> Option<&'a str> {
    let canonical = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    let mut best: Option<(&str, usize)> = None;
    for (root_path, root_id) in roots {
        let root_canonical = root_path
            .canonicalize()
            .unwrap_or_else(|_| root_path.clone());
        if canonical.starts_with(&root_canonical) {
            let depth = root_canonical.as_os_str().len();
            if best.is_none() || depth > best.unwrap().1 {
                best = Some((root_id.as_str(), depth));
            }
        }
    }
    best.map(|(id, _)| id)
}

// ── Language statistics ──────────────────────────────────────────────────────

pub(super) fn language_for_extension(ext: &str) -> &'static str {
    match ext {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "jsx" => "jsx",
        "tsx" => "tsx",
        "go" => "go",
        "rb" => "ruby",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "c" | "h" => "c",
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" => "cpp",
        "cs" => "csharp",
        "swift" => "swift",
        "lua" => "lua",
        "sh" | "bash" | "zsh" => "shell",
        "php" => "php",
        "scala" => "scala",
        "r" => "r",
        "ex" | "exs" => "elixir",
        "zig" => "zig",
        "md" | "markdown" | "mdx" => "markdown",
        "html" | "htm" | "xhtml" => "html",
        "css" | "scss" | "sass" | "less" => "css",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "xml" | "svg" => "xml",
        "sql" => "sql",
        "pdf" => "pdf",
        "txt" | "text" => "text",
        _ => "other",
    }
}

pub(super) fn accumulate_language_stats(
    files: &[PathBuf],
    roots: &[(PathBuf, String)],
    root_lang_stats: &mut HashMap<String, HashMap<String, usize>>,
    root_file_counts: &mut HashMap<String, usize>,
) {
    for file_path in files {
        if let Some(root_id) = find_root_for_file(file_path, roots) {
            let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let lang = language_for_extension(ext);
            *root_lang_stats
                .entry(root_id.to_string())
                .or_default()
                .entry(lang.to_string())
                .or_insert(0) += 1;
            *root_file_counts.entry(root_id.to_string()).or_insert(0) += 1;
        }
    }
}

pub(super) async fn update_root_stats<S: Storage + ?Sized>(
    storage: &S,
    roots: &[(PathBuf, String)],
    root_lang_stats: &HashMap<String, HashMap<String, usize>>,
    root_file_counts: &HashMap<String, usize>,
) {
    for (root_path, root_id) in roots {
        let file_count = root_file_counts.get(root_id).copied().unwrap_or(0);
        let lang_stats = root_lang_stats.get(root_id).cloned().unwrap_or_default();
        let display_name = root_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string());
        let root = CorpusRoot {
            id: root_id.clone(),
            path: root_path.to_string_lossy().to_string(),
            kind: RootKind::Local,
            display_name,
            file_count,
            language_stats: lang_stats,
            repo_url: None,
            branch: None,
            commit_sha: None,
            clone_timestamp: None,
            sparse_paths: Vec::new(),
        };
        if let Err(e) = storage.upsert_corpus_root(&root).await {
            warn!(root_id = %root_id, error = %e, "failed to update corpus root stats");
        }
    }
}

// ── Content hashing & mtime ──────────────────────────────────────────────────

/// Compute the SHA-256 hex digest of a string.
#[must_use]
pub(super) fn compute_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Get file mtime as nanoseconds since Unix epoch (async).
pub(super) async fn file_mtime_nanos(path: &Path) -> Option<i64> {
    let meta = tokio::fs::metadata(path).await.ok()?;
    let mtime = meta.modified().ok()?;
    let duration = mtime.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(duration.as_nanos()).ok()
}

/// Check if all discovered files have unchanged mtimes vs stored records.
pub(super) async fn all_files_unchanged_by_mtime<S: Storage + ?Sized>(
    files: &[PathBuf],
    paths: &[PathBuf],
    storage: &S,
) -> Result<bool, IngestionError> {
    let stored = storage
        .list_file_hashes()
        .await
        .map_err(IngestionError::from)?;

    let stored_map: std::collections::HashMap<&str, Option<i64>> = stored
        .iter()
        .map(|r| (r.path.as_str(), r.mtime_ns))
        .collect();

    if stored_map.len() != files.len() {
        return Ok(false);
    }

    for file_path in files {
        let relative = compute_relative_path(file_path, paths);
        let Some(stored_mtime) = stored_map.get(relative.as_str()) else {
            return Ok(false);
        };
        let Some(stored_ns) = stored_mtime else {
            return Ok(false);
        };
        let Some(current_ns) = file_mtime_nanos(file_path).await else {
            return Ok(false);
        };
        if *stored_ns != current_ns {
            return Ok(false);
        }
    }

    Ok(true)
}
