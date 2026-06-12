//! File discovery: walking directories, applying ignore rules, and resolving glob patterns.

use std::path::{Path, PathBuf};

use crate::error::IngestionError;
use crate::parser::detect_parser_kind;

/// Directory names that are ~never an authored source root and are safe to
/// prune globally (no `.ministr.toml` needed). Anything ambiguous — `bin`,
/// `obj`, `Library`, `Debug`, `Release` — is deliberately NOT here; those are
/// gated behind project-type detection in `init::default_ignore_patterns`
/// instead, because they collide with legitimate authored directory names.
const ALWAYS_IGNORE_DIRS: &[&str] = &[
    // VCS / editor / tooling
    ".git",
    ".svn",
    ".hg",
    ".jj",
    ".idea",
    ".vs",
    ".vscode",
    ".zed",
    ".fleet",
    ".cache",
    ".ccls-cache",
    ".clangd",
    ".fastembed_cache",
    ".onnx_cache",
    ".ministr",
    // Rust / generic
    "target",
    "out",
    "dist",
    "coverage",
    // Vendored third-party trees (committed deps — the big one).
    "vendor",
    "3rdparty",
    "third_party",
    "third-party",
    "thirdparty",
    "extern",
    "external",
    "externals",
    "deps",
    "_deps",
    // Node / web
    "node_modules",
    "bower_components",
    "jspm_packages",
    "web_modules",
    ".next",
    ".nuxt",
    ".output",
    ".svelte-kit",
    ".turbo",
    ".parcel-cache",
    ".vite",
    ".angular",
    ".docusaurus",
    ".serverless",
    ".nyc_output",
    ".pnpm-store",
    // Python
    "__pycache__",
    ".venv",
    "venv",
    "env",
    ".tox",
    ".nox",
    ".mypy_cache",
    ".pytest_cache",
    ".ruff_cache",
    ".hypothesis",
    ".ipynb_checkpoints",
    ".eggs",
    "develop-eggs",
    "__pypackages__",
    "site-packages",
    "htmlcov",
    // JVM / Gradle / Maven
    ".gradle",
    ".mvn",
    // Go
    ".go",
    // Elixir / Erlang / OCaml-dune
    ".elixir_ls",
    "_build",
    // Haskell
    ".stack-work",
    ".cargo",
    // Dart / Flutter
    ".dart_tool",
    // C / C++ / CMake
    "CMakeFiles",
    "CMakeScripts",
    // Apple / Swift
    "Pods",
    "Carthage",
    "DerivedData",
    "xcuserdata",
    ".swiftpm",
    // IaC
    ".terraform",
];

/// Directory-name *glob* patterns (the exact-match list above can't express
/// these). Supports a single leading or trailing `*`. Covers Bazel symlink
/// trees, CMake/CLion out-of-source build dirs, and macOS bundle dirs that
/// are really just build output (`*.xcodeproj`, `*.framework`, …).
const ALWAYS_IGNORE_DIR_GLOBS: &[&str] = &[
    "bazel-*",
    "cmake-build-*",
    "*.egg-info",
    "*.xcodeproj",
    "*.xcworkspace",
    "*.framework",
    "*.xcassets",
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
    // Unreal Engine: UnrealHeaderTool emits these from the
    // `UCLASS()`/`USTRUCT()`/`GENERATED_BODY()` declarations in the
    // adjacent non-generated header. The output is ~zero authored
    // content (just reflection-macro spew), parser-hostile to
    // tree-sitter-cpp, and on a UE5 source tree it's ~30K files /
    // ~500 MB of pure busywork. Skip globally — they aren't useful in
    // any other codebase either.
    "*.generated.h",
    "*.gen.cpp",
    // Generated serialization / RPC bindings — machine-emitted, ~zero
    // authored content, and they pollute symbol search with thousands
    // of stub identifiers (the WebWowViewerCpp class of problem).
    "*.pb.go",
    "*_pb2.py",
    "*_pb2.pyi",
    "*_pb2_grpc.py",
    "*.pb.cc",
    "*.pb.h",
    "*_pb.rb",
    "*_pb.dart",
    "*.pb.swift",
    "*.pbobjc.h",
    "*.pbobjc.m",
    "*_grpc.pb.cc",
    "*_grpc.pb.h",
    "*.g.dart",
    "*.freezed.dart",
    "*.g.cs",
    "*.Designer.cs",
    "*.generated.cs",
    // Qt / parser-generator output.
    "moc_*.cpp",
    "qrc_*.cpp",
    "ui_*.h",
    "*.tab.c",
    "*.yy.c",
    // Build metadata that is not source.
    "*.tsbuildinfo",
    "compile_commands.json",
    "CMakeCache.txt",
];

/// Match a directory name against the simple glob vocabulary used by
/// [`ALWAYS_IGNORE_DIR_GLOBS`]: a single leading `*` (suffix match) or a
/// single trailing `*` (prefix match). Exact strings match exactly.
fn dir_glob_match(name: &str, pattern: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix('*') {
        name.ends_with(suffix)
    } else if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else {
        name == pattern
    }
}

/// Whether a single path component (directory name) should always be pruned.
fn dir_name_is_ignored(name: &str) -> bool {
    ALWAYS_IGNORE_DIRS.contains(&name)
        || ALWAYS_IGNORE_DIR_GLOBS
            .iter()
            .any(|p| dir_glob_match(name, p))
}

/// Build the shared ignore-aware directory walker.
///
/// Used by both [`discover_files`] and [`compute_corpus_stat_merkle`] so the
/// indexed file set and the change-detection fingerprint can never drift
/// apart. Applies, in order: `.gitignore`/global/exclude rules, the
/// always-ignore file-glob overrides, the always-ignore directory-glob
/// overrides, and an exact+glob directory-name `filter_entry` prune.
fn ignored_walk(dir: &Path, extra_ignores: &[String]) -> Result<ignore::Walk, IngestionError> {
    use ignore::WalkBuilder;
    use ignore::overrides::OverrideBuilder;

    let mut overrides = OverrideBuilder::new(dir);
    for pattern in ALWAYS_IGNORE_PATTERNS {
        let _ = overrides.add(&format!("!{pattern}"));
    }
    for pattern in ALWAYS_IGNORE_DIR_GLOBS {
        let _ = overrides.add(&format!("!**/{pattern}"));
    }
    // User patterns from `.ministr.toml` `[corpus] ignore` — gitignore-style
    // globs relative to the walked root (a pattern without `/` matches at any
    // depth; a trailing `/` matches directories only). An invalid pattern is
    // skipped with a warning rather than failing the whole walk.
    for pattern in extra_ignores {
        if overrides.add(&format!("!{pattern}")).is_err() {
            tracing::warn!(pattern, "invalid [corpus] ignore pattern — skipped");
        }
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
            !(entry.file_type().is_some_and(|ft| ft.is_dir())
                && entry.file_name().to_str().is_some_and(dir_name_is_ignored))
        });
    Ok(walker.build())
}

/// Hard guard: reject files whose path includes an always-ignored
/// directory component *under* the given corpus `root`.
///
/// Only path components below `root` are inspected, so a corpus whose
/// own root path contains an always-ignored name as an ancestor (e.g.
/// `~/.ministr/remote/<repo>/` — `.ministr` is in
/// [`ALWAYS_IGNORE_DIRS`], but here it's the *parent* of the root, not
/// a directory inside the corpus) is not falsely rejected.
///
/// If `root` is `None` or `file_path` is not a descendant of `root`,
/// returns `false`: the discovery walker (`ignored_walk`) is the
/// primary gate at walk time, and this function only acts as a
/// defense-in-depth re-check from callers that already know the file
/// belongs to a particular corpus root (so without that information,
/// the safe default is to defer to the walker).
pub(crate) fn is_in_ignored_dir(root: Option<&Path>, file_path: &Path) -> bool {
    let Some(root) = root else {
        return false;
    };
    let Ok(relative) = file_path.strip_prefix(root) else {
        return false;
    };
    for component in relative.components() {
        if let std::path::Component::Normal(name) = component
            && let Some(name_str) = name.to_str()
            && dir_name_is_ignored(name_str)
        {
            return true;
        }
    }
    false
}

/// Single-path exclusion check for watcher events (watcher-ignore-filtering).
///
/// The walker ([`ignored_walk`]) is the exclusion truth at discovery time,
/// but file-watcher events arrive one path at a time, after discovery. This
/// matcher answers "would the walker have excluded this path?" cheaply per
/// path, from the same three layers:
///
/// 1. the built-in always-ignore directory names/globs (component check) and
///    file patterns,
/// 2. the corpus root's `.gitignore` (and the standard git global/exclude
///    files via the `ignore` crate's gitignore semantics),
/// 3. the user's `.ministr.toml` `[corpus] ignore` patterns.
///
/// Known, deliberate divergence: `.gitignore` files NESTED below the root
/// are not consulted here (the walker honors them). A file excluded only by
/// a nested gitignore may still pass this filter; the full-reindex path
/// (which uses the walker) remains the cleanup backstop.
#[derive(Debug)]
pub struct ExclusionMatcher {
    root: PathBuf,
    gitignore: ignore::gitignore::Gitignore,
}

impl ExclusionMatcher {
    /// Build a matcher for one corpus root with the given user patterns.
    ///
    /// Invalid user patterns are skipped with a warning, matching
    /// [`ignored_walk`]'s tolerance.
    #[must_use]
    pub fn for_root(root: &Path, user_patterns: &[String]) -> Self {
        let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
        // Root .gitignore, if present (nested ones deliberately not
        // consulted) — and ONLY inside a git repository: the walker's
        // `ignore` crate applies gitignore rules only when the tree is
        // git-controlled (`require_git`), so the matcher must too or the
        // two disagree on non-git corpora (caught by
        // `exclusion_matcher_agrees_with_the_walker`).
        let in_git_repo = root.ancestors().any(|a| a.join(".git").exists());
        let gitignore_path = root.join(".gitignore");
        if in_git_repo && gitignore_path.is_file() {
            let _ = builder.add(&gitignore_path);
        }
        // Built-in file patterns + dir globs (dir names are checked separately
        // via the component check, which is cheaper and depth-independent).
        for pattern in ALWAYS_IGNORE_PATTERNS {
            let _ = builder.add_line(None, pattern);
        }
        for pattern in ALWAYS_IGNORE_DIR_GLOBS {
            let _ = builder.add_line(None, &format!("**/{pattern}/"));
        }
        // User `[corpus] ignore` patterns — same vocabulary the walker takes.
        for pattern in user_patterns {
            if builder.add_line(None, pattern).is_err() {
                tracing::warn!(pattern, "invalid [corpus] ignore pattern — skipped");
            }
        }
        let gitignore = builder
            .build()
            .unwrap_or_else(|_| ignore::gitignore::Gitignore::empty());
        Self {
            root: root.to_path_buf(),
            gitignore,
        }
    }

    /// Whether the walker would have excluded this path.
    ///
    /// Returns `false` for paths outside this matcher's root (a different
    /// root's matcher is responsible for them).
    #[must_use]
    pub fn is_excluded(&self, path: &Path) -> bool {
        if path.strip_prefix(&self.root).is_err() {
            return false;
        }
        if is_in_ignored_dir(Some(&self.root), path) {
            return true;
        }
        self.gitignore
            .matched_path_or_any_parents(path, path.is_dir())
            .is_ignore()
    }
}

/// Discover all supported files in a directory recursively.
///
/// Respects `.gitignore` rules and skips well-known junk directories and file patterns.
///
/// # Errors
///
/// Returns [`IngestionError`] when the directory walk fails.
pub fn discover_files(dir: &Path) -> Result<Vec<PathBuf>, IngestionError> {
    discover_files_with_ignores(dir, &[])
}

/// [`discover_files`] plus user ignore patterns from `.ministr.toml`
/// `[corpus] ignore`, enforced through the same walker the change-detection
/// merkle uses so the file set and the fingerprint can never drift.
///
/// # Errors
///
/// Returns [`IngestionError`] when the directory walk fails.
pub fn discover_files_with_ignores(
    dir: &Path,
    extra_ignores: &[String],
) -> Result<Vec<PathBuf>, IngestionError> {
    let mut files = Vec::new();
    for result in ignored_walk(dir, extra_ignores)? {
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
    discover_paths_with_ignores(paths, &[])
}

/// [`discover_paths`] plus user ignore patterns from `.ministr.toml`
/// `[corpus] ignore`. Patterns apply when walking directories; a file
/// listed explicitly (or matched by an explicit glob) is kept — naming a
/// file directly is a stronger signal than an ignore pattern.
///
/// # Errors
///
/// Returns [`IngestionError`] when a walk or glob expansion fails.
pub fn discover_paths_with_ignores(
    paths: &[PathBuf],
    extra_ignores: &[String],
) -> Result<Vec<PathBuf>, IngestionError> {
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
                collect_path_entry(&entry_path, extra_ignores, &mut all_files, &mut seen)?;
            }
        } else {
            collect_path_entry(path, extra_ignores, &mut all_files, &mut seen)?;
        }
    }

    all_files.sort();
    Ok(all_files)
}

fn collect_path_entry(
    path: &Path,
    extra_ignores: &[String],
    files: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<PathBuf>,
) -> Result<(), IngestionError> {
    if path.is_dir() {
        let dir_files = discover_files_with_ignores(path, extra_ignores)?;
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

/// Detect whether a corpus root looks like an Unreal Engine project.
///
/// Returns `true` when any `*.uproject` or `*.uplugin` file exists
/// within 2 directories of the root. Covers the standard UE layouts:
/// - `<root>/MyGame.uproject`
/// - `<root>/Plugins/MyPlugin/MyPlugin.uplugin` (UE projects nest
///   plugins one level under `Plugins/`)
///
/// Currently unused — Phase 2 ended up swapping the C++ grammar
/// globally (`tree-sitter-cpp` → `tree-sitter-unreal-cpp`, a strict
/// superset that handles `UCLASS()` / `UFUNCTION()` /
/// `GENERATED_BODY()` correctly while parsing vanilla C++
/// byte-identically), so per-corpus grammar dispatch turned out not
/// to be necessary. The function is kept available because future
/// per-corpus routing decisions (e.g. content-extract dedupe gating
/// on UE-vs-non-UE in Phase 6) would naturally reuse it.
///
/// Cheap probe — at most a handful of `read_dir` calls. Callers can
/// memoize per-corpus if they call this hot.
#[must_use]
#[allow(dead_code)]
pub fn is_unreal_corpus(root: &Path) -> bool {
    fn has_ue_marker(dir: &Path) -> bool {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in rd.flatten() {
            if let Some(name) = entry.file_name().to_str()
                && (name.ends_with(".uproject") || name.ends_with(".uplugin"))
            {
                return true;
            }
        }
        false
    }
    fn search(dir: &Path, depth: u8) -> bool {
        if has_ue_marker(dir) {
            return true;
        }
        if depth == 0 {
            return false;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in rd.flatten() {
            if entry.file_type().is_ok_and(|ft| ft.is_dir()) && search(&entry.path(), depth - 1) {
                return true;
            }
        }
        false
    }
    // 2 levels deep — covers root + child + grandchild, which is
    // enough for UE's `Plugins/<name>/<name>.uplugin` layout.
    search(root, 2)
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
pub fn compute_corpus_stat_merkle(dir: &Path) -> Result<(String, Vec<PathBuf>), IngestionError> {
    compute_corpus_stat_merkle_with_ignores(dir, &[])
}

/// [`compute_corpus_stat_merkle`] plus user ignore patterns — the same
/// patterns MUST be passed here and to [`discover_files_with_ignores`]
/// or the indexed file set and the fingerprint drift apart.
///
/// # Errors
///
/// Returns [`IngestionError`] when the walk fails or file metadata
/// cannot be read.
#[must_use = "returns (root_hash, files)"]
pub fn compute_corpus_stat_merkle_with_ignores(
    dir: &Path,
    extra_ignores: &[String],
) -> Result<(String, Vec<PathBuf>), IngestionError> {
    let walk = ignored_walk(dir, extra_ignores)?;

    // Collect (rel_path, abs_path, mtime_ns, size) tuples; we need both
    // the relative form (for the fingerprint, so absolute paths don't
    // poison the hash on a different machine) and the absolute form
    // (for the returned file list).
    let mut entries: Vec<(String, PathBuf, i64, u64)> = Vec::new();
    for result in walk {
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

    // corpus-ignore-enforcement-gap: `[corpus] ignore` patterns must actually
    // exclude files — for the indexed set AND the merkle fingerprint.
    #[test]
    fn user_ignore_patterns_exclude_files_and_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("keep.rs"), "fn main() {}");
        write(&tmp.path().join("skipme.md"), "# skip");
        write(&tmp.path().join("Binaries/junk.rs"), "fn junk() {}");
        write(&tmp.path().join("nested/alsome.md"), "# skip too");

        // A file glob (matching at any depth, gitignore-style) + a dir pattern.
        let ignores = vec!["*me.md".to_owned(), "Binaries/".to_owned()];
        let files = discover_files_with_ignores(tmp.path(), &ignores).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|f| {
                f.strip_prefix(tmp.path())
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(names, vec!["keep.rs"], "got {names:?}");

        // No patterns → everything supported is discovered (regression guard).
        let all = discover_files(tmp.path()).unwrap();
        assert_eq!(all.len(), 4, "got {all:?}");
    }

    #[test]
    fn user_ignore_patterns_keep_walk_and_merkle_in_lockstep() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("keep.rs"), "fn main() {}");
        write(&tmp.path().join("skip.md"), "# snapshot v1");

        let ignores = vec!["skip.md".to_owned()];
        let (h1, files1) = compute_corpus_stat_merkle_with_ignores(tmp.path(), &ignores).unwrap();
        assert_eq!(
            files1,
            discover_files_with_ignores(tmp.path(), &ignores).unwrap(),
            "merkle and discovery must agree on the file set"
        );

        // Changing an IGNORED file must not change the fingerprint.
        std::thread::sleep(std::time::Duration::from_millis(5));
        write(
            &tmp.path().join("skip.md"),
            "# snapshot v2 — much longer body",
        );
        let (h2, _) = compute_corpus_stat_merkle_with_ignores(tmp.path(), &ignores).unwrap();
        assert_eq!(h1, h2, "an ignored file's change must not dirty the merkle");

        // Changing a KEPT file must still change it.
        write(&tmp.path().join("keep.rs"), "fn main() { println!(); }");
        let (h3, _) = compute_corpus_stat_merkle_with_ignores(tmp.path(), &ignores).unwrap();
        assert_ne!(h1, h3, "a kept file's change must dirty the merkle");
    }

    #[test]
    fn explicitly_listed_file_wins_over_ignore_pattern() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("notes.md"), "# notes");
        let ignores = vec!["*.md".to_owned()];
        let files = discover_paths_with_ignores(&[tmp.path().join("notes.md")], &ignores).unwrap();
        assert_eq!(
            files.len(),
            1,
            "naming a file directly beats an ignore pattern"
        );
    }

    // watcher-ignore-filtering: the single-path matcher answers "would the
    // walker have excluded this?" — prove agreement on a fixture tree.
    #[test]
    fn exclusion_matcher_agrees_with_the_walker() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("keep.rs"), "fn main() {}");
        write(&tmp.path().join("skipme.md"), "# skip");
        write(&tmp.path().join("Binaries/junk.rs"), "fn junk() {}");
        write(&tmp.path().join("node_modules/dep.js"), "x");
        write(&tmp.path().join(".gitignore"), "gitignored.md\n");
        write(&tmp.path().join("gitignored.md"), "# hidden by git");
        write(&tmp.path().join("nested/ok.rs"), "fn ok() {}");

        let ignores = vec!["*me.md".to_owned(), "Binaries/".to_owned()];
        let walked = discover_files_with_ignores(tmp.path(), &ignores).unwrap();
        let matcher = ExclusionMatcher::for_root(tmp.path(), &ignores);

        // Every supported file in the tree: walker-included <=> not matcher-excluded.
        for rel in [
            "keep.rs",
            "skipme.md",
            "Binaries/junk.rs",
            "node_modules/dep.js",
            "gitignored.md",
            "nested/ok.rs",
        ] {
            let abs = tmp.path().join(rel);
            let in_walk = walked.contains(&abs);
            let excluded = matcher.is_excluded(&abs);
            assert_eq!(
                in_walk, !excluded,
                "walker and matcher disagree on {rel}: in_walk={in_walk}, excluded={excluded}"
            );
        }
        // Sanity on the expected split (no .git here, so gitignore is
        // inert — exactly like the walker).
        assert!(matcher.is_excluded(&tmp.path().join("skipme.md")));
        assert!(matcher.is_excluded(&tmp.path().join("Binaries/junk.rs")));
        assert!(matcher.is_excluded(&tmp.path().join("node_modules/dep.js")));
        assert!(!matcher.is_excluded(&tmp.path().join("gitignored.md")));
        assert!(!matcher.is_excluded(&tmp.path().join("keep.rs")));
        assert!(!matcher.is_excluded(&tmp.path().join("nested/ok.rs")));
        // Paths outside the root are not this matcher's business.
        assert!(!matcher.is_excluded(Path::new("/somewhere/else.rs")));

        // Now make it a git repo: gitignore applies in BOTH walker and
        // matcher, and they still agree.
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        let walked = discover_files_with_ignores(tmp.path(), &ignores).unwrap();
        let matcher = ExclusionMatcher::for_root(tmp.path(), &ignores);
        assert!(matcher.is_excluded(&tmp.path().join("gitignored.md")));
        assert!(!walked.contains(&tmp.path().join("gitignored.md")));
        for rel in ["keep.rs", "gitignored.md", "skipme.md", "nested/ok.rs"] {
            let abs = tmp.path().join(rel);
            assert_eq!(
                walked.contains(&abs),
                !matcher.is_excluded(&abs),
                "git-repo case disagreement on {rel}"
            );
        }
    }

    #[test]
    fn invalid_ignore_pattern_is_skipped_not_fatal() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("keep.rs"), "fn main() {}");
        // "**.rs**" style garbage that the override parser rejects.
        let ignores = vec!["[".to_owned()];
        let files = discover_files_with_ignores(tmp.path(), &ignores).unwrap();
        assert_eq!(files.len(), 1, "invalid pattern must not fail the walk");
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

    #[test]
    fn generated_headers_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        write(
            &tmp.path().join("Source/MyClass.h"),
            "class FOO_API UMyClass {};",
        );
        write(
            &tmp.path().join("Source/MyClass.generated.h"),
            "// auto-generated UnrealHeaderTool spew\n",
        );
        write(&tmp.path().join("Source/MyClass.gen.cpp"), "// generated\n");
        let files = discover_files(tmp.path()).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"MyClass.h".to_string()));
        assert!(!names.iter().any(|n| n.ends_with(".generated.h")));
        assert!(!names.iter().any(|n| n.ends_with(".gen.cpp")));
    }

    #[test]
    fn dir_glob_match_vocabulary() {
        assert!(dir_glob_match("bazel-out", "bazel-*"));
        assert!(dir_glob_match("bazel-bin", "bazel-*"));
        assert!(!dir_glob_match("bazelisk", "bazel-*"));
        assert!(dir_glob_match("cmake-build-debug", "cmake-build-*"));
        assert!(dir_glob_match("mylib.egg-info", "*.egg-info"));
        assert!(dir_glob_match("App.xcodeproj", "*.xcodeproj"));
        assert!(!dir_glob_match("src", "*.xcodeproj"));
    }

    #[test]
    fn vendored_and_build_dirs_are_pruned() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("src/main.rs"), "fn main(){}");
        // Vendored / build trees that must NOT be indexed.
        write(&tmp.path().join("3rdparty/glew/glew.c"), "int x;");
        write(&tmp.path().join("third_party/protobuf/x.cc"), "int y;");
        write(&tmp.path().join("extern/dep/d.c"), "int z;");
        write(&tmp.path().join("deps/foo/foo.ex"), "defmodule F do\nend");
        write(&tmp.path().join("bazel-out/gen.go"), "package m");
        write(
            &tmp.path().join("cmake-build-debug/CMakeFiles/x.cpp"),
            "int q;",
        );
        write(&tmp.path().join("mylib.egg-info/top.py"), "x=1");
        write(&tmp.path().join(".dart_tool/pkg.dart"), "void m(){}");

        let files = discover_files(tmp.path()).unwrap();
        let rels: Vec<String> = files
            .iter()
            .map(|p| {
                p.strip_prefix(tmp.path())
                    .unwrap_or(p)
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(rels, vec!["src/main.rs".to_string()], "got {rels:?}");
    }

    #[test]
    fn generated_bindings_are_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("svc.go"), "package svc");
        write(&tmp.path().join("svc.pb.go"), "package svc // generated");
        write(&tmp.path().join("svc_pb2.py"), "# generated");
        write(&tmp.path().join("svc_pb2_grpc.py"), "# generated");
        write(&tmp.path().join("model.g.dart"), "// generated");
        write(&tmp.path().join("View.Designer.cs"), "// generated");
        let files = discover_files(tmp.path()).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["svc.go".to_string()], "got {names:?}");
    }

    #[test]
    fn is_unreal_corpus_detects_root_uproject() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("MyGame.uproject"), "{}").unwrap();
        std::fs::write(tmp.path().join("README.md"), "# game").unwrap();
        assert!(is_unreal_corpus(tmp.path()));
    }

    #[test]
    fn is_unreal_corpus_detects_nested_uplugin() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("Plugins/MyPlugin")).unwrap();
        std::fs::write(tmp.path().join("Plugins/MyPlugin/MyPlugin.uplugin"), "{}").unwrap();
        assert!(is_unreal_corpus(tmp.path()));
    }

    #[test]
    fn is_unreal_corpus_returns_false_for_plain_codebase() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join("src/main.rs"), "fn main(){}");
        write(&tmp.path().join("Cargo.toml"), "[package]");
        assert!(!is_unreal_corpus(tmp.path()));
    }

    /// Regression: a corpus rooted *under* `.ministr/...` (e.g. the daemon's
    /// clone cache at `~/.ministr/remote/<hash>/`) must not have every file
    /// rejected just because `.ministr` appears as an ancestor of the root.
    /// Before the fix, `is_in_ignored_dir` walked the absolute path including
    /// ancestors, so a clone living under `.ministr` was unindexable.
    #[test]
    fn ignored_dir_check_only_inspects_components_under_root() {
        // Cross-platform: build paths with PathBuf::push so the test
        // doesn't bake in `/` or `\`.
        let mut root = PathBuf::from("home");
        root.push("alrik");
        root.push(".ministr");
        root.push("remote");
        root.push("abc123");

        let mut clean_file = root.clone();
        clean_file.push("src");
        clean_file.push("main.rs");

        let mut nested_ignored = root.clone();
        nested_ignored.push("vendor");
        nested_ignored.push("node_modules");
        nested_ignored.push("foo.js");

        // `.ministr` is an ANCESTOR of the root → must not match.
        assert!(
            !is_in_ignored_dir(Some(&root), &clean_file),
            "ancestor .ministr must not poison a corpus rooted under it"
        );
        // An always-ignored dir name appearing UNDER the root → must match.
        assert!(
            is_in_ignored_dir(Some(&root), &nested_ignored),
            "node_modules under the corpus root must be rejected"
        );
    }

    #[test]
    fn ignored_dir_check_returns_false_without_root() {
        // No root context = defer to the discovery walker. Even a path
        // containing `.git` returns false (the walker already filtered).
        let mut p = PathBuf::from("anywhere");
        p.push(".git");
        p.push("HEAD");
        assert!(!is_in_ignored_dir(None, &p));
    }

    #[test]
    fn ignored_dir_check_returns_false_when_file_not_under_root() {
        let root = PathBuf::from("home").join("alrik").join("project");
        let unrelated = PathBuf::from("tmp").join(".git").join("x");
        assert!(!is_in_ignored_dir(Some(&root), &unrelated));
    }
}
