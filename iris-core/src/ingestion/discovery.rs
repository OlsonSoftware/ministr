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
    ".iris",
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
        if let std::path::Component::Normal(name) = component {
            if let Some(name_str) = name.to_str() {
                if ALWAYS_IGNORE_DIRS.contains(&name_str) {
                    return true;
                }
            }
        }
    }
    false
}

/// Discover all supported files in a directory recursively.
///
/// Respects `.gitignore` rules and skips well-known junk directories and file patterns.
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
            if entry.file_type().is_some_and(|ft| ft.is_dir()) {
                if let Some(name) = entry.file_name().to_str() {
                    if ALWAYS_IGNORE_DIRS.contains(&name) {
                        return false;
                    }
                }
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
/// use iris_core::ingestion::discover_paths;
/// use std::path::PathBuf;
///
/// let paths = vec![
///     PathBuf::from("docs/"),
///     PathBuf::from("DESIGN.md"),
///     PathBuf::from("*.md"),
/// ];
/// let files = discover_paths(&paths).unwrap();
/// ```
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
