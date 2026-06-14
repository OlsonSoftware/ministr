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
/// // "ministr-core/src/config.rs" → ["config"]
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
/// use ministr_core::ingestion::compute_root_id;
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
/// use ministr_core::ingestion::namespace_path;
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
/// use ministr_core::ingestion::strip_root_prefix;
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

/// Determine which source-root entry a file belongs to (longest prefix match).
///
/// Returns the matched `(root_path, root_id)` tuple; callers that only need
/// one of the two can use [`find_root_for_file`] for the id-only form.
pub(super) fn find_root_entry_for_file<'a>(
    file: &Path,
    roots: &'a [(PathBuf, String)],
) -> Option<&'a (PathBuf, String)> {
    let canonical = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    let mut best: Option<(&(PathBuf, String), usize)> = None;
    for entry in roots {
        let (root_path, _) = entry;
        let root_canonical = root_path
            .canonicalize()
            .unwrap_or_else(|_| root_path.clone());
        if canonical.starts_with(&root_canonical) {
            let depth = root_canonical.as_os_str().len();
            if best.is_none_or(|(_, d)| depth > d) {
                best = Some((entry, depth));
            }
        }
    }
    best.map(|(entry, _)| entry)
}

/// Determine which source root a file belongs to (longest prefix match).
pub(super) fn find_root_for_file<'a>(
    file: &Path,
    roots: &'a [(PathBuf, String)],
) -> Option<&'a str> {
    find_root_entry_for_file(file, roots).map(|(_, id)| id.as_str())
}

/// The storage key for a discovered file: its path relative to the owning
/// source root, namespaced by the root id when the corpus spans more than one
/// registered source (so identically-named files under different roots — e.g.
/// `src/lib.rs` in many crates — don't collide on one key).
///
/// This is the index KEY, deliberately NOT the on-disk locator: read sites
/// reconstruct the absolute path from `corpus_roots.path` + this key (see
/// `QueryService::resolve_source_path`). It matches what
/// [`crate::freshness::compute_freshness`] reconstructs — namespaced
/// (`rid/rel`) for multi-source corpora, bare-relative for single-source —
/// and the GUI display key. (The legacy `compute_relative_path` stub returned
/// the file's ABSOLUTE path here, which embedded the indexing machine's paths
/// and never matched the freshness sweep — see freshness-abs-key-match /
/// ingest-key-locator-decouple.)
///
/// `roots` are the corpus's *directory* sources paired with their
/// [`compute_root_id`]; `source_count` is the FULL registered path count
/// (directories AND files), matching `compute_freshness`'s `multi_root` test.
pub(super) fn relative_storage_key(
    file: &Path,
    roots: &[(PathBuf, String)],
    source_count: usize,
) -> String {
    let Some((root_path, root_id)) = find_root_entry_for_file(file, roots) else {
        // No owning directory root (e.g. a bare file passed directly as a
        // source): fall back to the path with a leading `./` stripped.
        return compute_relative_path(file, &[]);
    };
    // Discovery preserves the literal root prefix, so a literal strip is the
    // common path and matches `compute_freshness` exactly; fall back to a
    // canonical strip only when the root matched via canonicalization (a
    // symlinked source), and to the bare path if even that fails.
    let rel = match file
        .strip_prefix(root_path)
        .map(Path::to_path_buf)
        .or_else(|_| {
            let cf = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
            let cr = root_path
                .canonicalize()
                .unwrap_or_else(|_| root_path.clone());
            cf.strip_prefix(&cr).map(Path::to_path_buf)
        }) {
        Ok(r) => r.to_string_lossy().replace('\\', "/"),
        Err(_) => return compute_relative_path(file, &[]),
    };
    if source_count > 1 {
        namespace_path(root_id, &rel)
    } else {
        rel
    }
}

/// Candidate stored index keys for an ABSOLUTE on-disk path, most-specific
/// first: namespaced (`rid/rel`) and bare-relative against the owning corpus
/// root (post-decouple corpora), then the raw absolute path (pre-decouple /
/// legacy corpora). Diff-impact tries each until one resolves to symbols, so
/// it works across both key schemes without a reindex. `roots` are the
/// corpus's directory roots paired with their [`compute_root_id`].
#[must_use]
pub fn symbol_key_candidates(abs_path: &str, roots: &[(PathBuf, String)]) -> Vec<String> {
    let p = Path::new(abs_path);
    let mut keys = Vec::new();
    if let Some((root_path, root_id)) = roots
        .iter()
        .filter(|(rp, _)| p.starts_with(rp))
        .max_by_key(|(rp, _)| rp.as_os_str().len())
        && let Ok(rel) = p.strip_prefix(root_path)
    {
        let rel = rel.to_string_lossy().replace('\\', "/");
        keys.push(namespace_path(root_id, &rel));
        keys.push(rel);
    }
    keys.push(abs_path.to_string());
    keys
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
        "asm" | "s" | "S" | "inc" => "assembly",
        // Shaders — coarse "shader" label so the language-stats
        // breakdown picks them up as one bucket. Per-language splits
        // (HLSL vs GLSL vs MSL vs WGSL) can come back when symbol-level
        // extraction lands and starts caring about the distinction.
        "hlsl" | "usf" | "ush" | "fx" | "fxh" | "shader" | "glsl" | "vert" | "frag" | "geom"
        | "comp" | "tesc" | "tese" | "mesh" | "task" | "rgen" | "rmiss" | "rchit" | "rahit"
        | "rint" | "rcall" | "metal" | "wgsl" => "shader",
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

/// Compute a 64-char hex content-change fingerprint for a file's text.
///
/// Uses BLAKE3 (~3× faster than SHA-256 per core, with multithreaded
/// hashing available in the underlying crate). Output is the same
/// 32-byte digest length as SHA-256, formatted as 64 lowercase hex
/// chars, so the value drops in wherever the previous SHA-256 hash
/// was stored.
///
/// On upgrade from a SHA-256-era index, every stored hash will
/// mismatch and the existing per-file change-detection logic will
/// trigger one re-extraction pass — bumping `EXTRACTOR_VERSION`
/// already produces this same effect, so the swap is free.
#[must_use]
pub fn compute_content_hash(content: &str) -> String {
    blake3::hash(content.as_bytes()).to_hex().to_string()
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

    // Key each file EXACTLY as `ingest_paths_with_embeddings` wrote it
    // (relative_storage_key over the same dir-roots + source count), so the
    // fast-skip's lookups hit the stored rows instead of always missing.
    let dir_roots: Vec<(PathBuf, String)> = paths
        .iter()
        .filter(|p| p.is_dir())
        .map(|p| (p.clone(), compute_root_id(p)))
        .collect();

    for file_path in files {
        let relative = relative_storage_key(file_path, &dir_roots, paths.len());
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

    // Consistency guard: `file_hashes` says everything is unchanged, but
    // if the `documents` table is empty the two tables have drifted apart
    // (most often from a previous run that wrote hashes before failing
    // to commit documents — e.g. an HNSW persist error or a half-deleted
    // cleanup loop). Without this check the corpus stays stuck reporting
    // "0 files indexed, ready" forever — both the manifest-level mtime
    // fast-skip (early return below) AND the per-file mtime fast-skip in
    // `parse_and_store_file` would suppress every subsequent re-ingest.
    //
    // Wiping `file_hashes` is the only intervention that actually works:
    // the per-file path keys on a hash row existing for that path, so
    // returning false here without clearing the rows still hits the same
    // wall one level down. After clearing, the next ingestion run sees
    // `existing_hash = None` per file and parses fresh.
    let doc_count = storage
        .document_count()
        .await
        .map_err(IngestionError::from)?;
    if doc_count == 0 && !files.is_empty() {
        let cleared = storage
            .clear_file_hashes()
            .await
            .map_err(IngestionError::from)?;
        tracing::warn!(
            stored_hashes = cleared,
            files = files.len(),
            "mtime fast-skip would fire but documents table is empty — \
             cleared stale file_hashes and forcing full re-ingest to \
             repair drifted state"
        );
        return Ok(false);
    }

    Ok(true)
}

#[cfg(test)]
mod relative_key_tests {
    use super::*;

    #[test]
    fn single_source_key_is_bare_relative() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let file = root.join("src/lib.rs");
        std::fs::write(&file, "fn x() {}").unwrap();

        let roots = vec![(root.clone(), compute_root_id(&root))];
        let key = relative_storage_key(&file, &roots, 1);
        assert_eq!(key, "src/lib.rs");
        assert!(!key.starts_with('/'), "key must never be absolute: {key}");
    }

    #[test]
    fn multi_source_key_is_namespaced_and_collision_free() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("crate_a");
        let b = dir.path().join("crate_b");
        std::fs::create_dir_all(a.join("src")).unwrap();
        std::fs::create_dir_all(b.join("src")).unwrap();
        let fa = a.join("src/lib.rs");
        let fb = b.join("src/lib.rs");
        std::fs::write(&fa, "fn a() {}").unwrap();
        std::fs::write(&fb, "fn b() {}").unwrap();

        let roots = vec![
            (a.clone(), compute_root_id(&a)),
            (b.clone(), compute_root_id(&b)),
        ];
        let ka = relative_storage_key(&fa, &roots, 2);
        let kb = relative_storage_key(&fb, &roots, 2);
        assert_ne!(ka, kb, "same rel under different roots must not collide");
        assert!(
            ka.ends_with("/src/lib.rs") && ka.starts_with("root-"),
            "{ka}"
        );
        assert!(
            kb.ends_with("/src/lib.rs") && kb.starts_with("root-"),
            "{kb}"
        );
        assert!(!ka.starts_with('/') && !kb.starts_with('/'));
    }

    #[test]
    fn absolute_discovered_path_keys_relative_not_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::write(root.join("a.rs"), "fn a() {}").unwrap();
        let abs_file = root.join("a.rs");
        assert!(abs_file.is_absolute());

        let roots = vec![(root.clone(), compute_root_id(&root))];
        let key = relative_storage_key(&abs_file, &roots, 1);
        assert_eq!(key, "a.rs");
        assert!(!key.starts_with('/'), "regressed to absolute key: {key}");
    }
}
