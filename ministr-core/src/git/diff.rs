//! Branch/range diff awareness — which files and line ranges a `base..head`
//! revision range touched, mapped to absolute paths that match the symbol
//! index.
//!
//! This is the read-only substrate behind FL7 ("git/branch-diff awareness"):
//! the MCP `ministr_impact` op resolves the head-side changed lines to the
//! enclosing indexed symbols and unions their blast radius. ministr indexes a
//! *tree* and has no notion of "what changed on this branch", so the range is
//! resolved against git on demand, language-agnostically (line spans, not
//! syntax).

use std::path::{Path, PathBuf};
use std::process::Command;

/// Error resolving a diff range against a git work tree.
#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    /// `git` is not installed or could not be launched.
    #[error("git is not installed or could not be launched")]
    GitNotAvailable,
    /// The directory is not inside a git work tree.
    #[error("not inside a git work tree: {0}")]
    NotARepo(String),
    /// The range spec is syntactically unsafe (e.g. starts with `-`, which git
    /// would treat as a flag).
    #[error("invalid revision range: {0}")]
    InvalidRange(String),
    /// `git diff` ran but exited non-zero (e.g. an unknown revision).
    #[error("git diff failed: {0}")]
    DiffFailed(String),
}

/// A contiguous run of changed lines on the *head* side of a diff
/// (1-based, inclusive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedRange {
    /// First changed line (1-based).
    pub start: u32,
    /// Last changed line (1-based, inclusive).
    pub end: u32,
}

impl ChangedRange {
    /// Whether this range overlaps the inclusive symbol span `[lo, hi]`.
    #[must_use]
    pub fn overlaps(&self, lo: u32, hi: u32) -> bool {
        self.start <= hi && lo <= self.end
    }
}

/// One file touched by a revision range, with the head-side line ranges
/// changed. `path` is absolute (joined with the repo top-level) so it matches
/// the `file_path` stored on indexed symbols.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// Absolute path to the file in the work tree.
    pub path: String,
    /// Head-side line ranges changed (added or modified). Empty for a pure
    /// deletion that left no head-side anchor.
    pub ranges: Vec<ChangedRange>,
}

/// Resolve the absolute top-level directory of the git work tree containing
/// `dir`. Returns `None` when `dir` is not inside a work tree (or git is
/// unavailable).
#[must_use]
pub fn toplevel(dir: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8(output.stdout).ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

/// Compute the files and head-side line ranges changed by a revision `range`
/// (e.g. `"main..HEAD"`, `"HEAD~3"`, `"abc123..def456"`).
///
/// Runs `git diff --unified=0 --no-color <range>` and parses the unified-diff
/// hunk headers (`@@ -a,b +c,d @@`), keeping the `+c,d` (head-side) spans. File
/// paths are made absolute against the repo top-level so they match indexed
/// symbol paths. Deleted files (`+++ /dev/null`) are skipped.
///
/// # Errors
///
/// Returns [`DiffError`] when git is unavailable, `dir` is not a repo, the
/// range is unsafe, or `git diff` exits non-zero.
pub fn changed_lines(dir: &Path, range: &str) -> Result<Vec<ChangedFile>, DiffError> {
    let range = range.trim();
    if range.is_empty() || range.starts_with('-') {
        return Err(DiffError::InvalidRange(range.to_string()));
    }

    let root = toplevel(dir).ok_or_else(|| DiffError::NotARepo(dir.display().to_string()))?;

    let output = Command::new("git")
        // quotepath=false keeps non-ASCII paths literal instead of octal-escaped.
        .args([
            "-c",
            "core.quotepath=false",
            "diff",
            "--unified=0",
            "--no-color",
        ])
        .arg(range)
        .current_dir(&root)
        .output()
        .map_err(|_| DiffError::GitNotAvailable)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(DiffError::DiffFailed(stderr));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_unified_diff(&text, &root))
}

/// Parse `git diff --unified=0` output into per-file head-side changed ranges.
/// `root` is the absolute repo top-level used to resolve the `+++ b/<path>`
/// entries to absolute paths.
fn parse_unified_diff(text: &str, root: &Path) -> Vec<ChangedFile> {
    let mut files: Vec<ChangedFile> = Vec::new();
    let mut current: Option<ChangedFile> = None;

    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            // Flush the previous file before starting a new one.
            if let Some(f) = current.take() {
                files.push(f);
            }
            // `+++ b/path`, `+++ /dev/null` (deletion), or `+++ "b/quoted"`.
            let raw = rest.split('\t').next().unwrap_or(rest).trim();
            if raw == "/dev/null" {
                current = None;
                continue;
            }
            let rel = raw
                .strip_prefix("b/")
                .or_else(|| raw.strip_prefix("a/"))
                .unwrap_or(raw);
            current = Some(ChangedFile {
                path: root.join(rel).to_string_lossy().into_owned(),
                ranges: Vec::new(),
            });
        } else if line.starts_with("@@")
            && let Some(f) = current.as_mut()
            && let Some(range) = parse_hunk_head_side(line)
        {
            f.ranges.push(range);
        }
    }
    if let Some(f) = current.take() {
        files.push(f);
    }
    // Drop files where the only change was a deletion that left no head anchor.
    files.retain(|f| !f.ranges.is_empty());
    files
}

/// Extract the head-side range from a hunk header `@@ -a,b +c,d @@ ...`.
/// `+c` with no count means a single line; `+c,0` (pure deletion) anchors at
/// line `c` (clamped to ≥1).
fn parse_hunk_head_side(line: &str) -> Option<ChangedRange> {
    // Between the two `@@` markers: ` -a,b +c,d `.
    let inner = line.strip_prefix("@@")?;
    let inner = inner.split("@@").next()?;
    let plus = inner.split_whitespace().find(|t| t.starts_with('+'))?;
    let spec = plus.strip_prefix('+')?;
    let mut parts = spec.split(',');
    let start: u32 = parts.next()?.parse().ok()?;
    let count: u32 = match parts.next() {
        Some(c) => c.parse().ok()?,
        None => 1,
    };
    if count == 0 {
        let anchor = start.max(1);
        Some(ChangedRange {
            start: anchor,
            end: anchor,
        })
    } else {
        Some(ChangedRange {
            start,
            end: start + count - 1,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunk_with_count() {
        let r = parse_hunk_head_side("@@ -10,2 +11,3 @@ fn foo()").unwrap();
        assert_eq!(r, ChangedRange { start: 11, end: 13 });
    }

    #[test]
    fn hunk_without_count() {
        let r = parse_hunk_head_side("@@ -5 +6 @@").unwrap();
        assert_eq!(r, ChangedRange { start: 6, end: 6 });
    }

    #[test]
    fn hunk_pure_deletion_anchors() {
        // `+c,0` — lines removed; head side anchors at line c.
        let r = parse_hunk_head_side("@@ -10,3 +9,0 @@").unwrap();
        assert_eq!(r, ChangedRange { start: 9, end: 9 });
    }

    #[test]
    fn range_overlap() {
        let r = ChangedRange { start: 10, end: 20 };
        assert!(r.overlaps(5, 12)); // straddles start
        assert!(r.overlaps(15, 16)); // inside
        assert!(r.overlaps(18, 40)); // straddles end
        assert!(!r.overlaps(1, 9)); // before
        assert!(!r.overlaps(21, 30)); // after
    }

    #[test]
    fn parse_multi_file_diff() {
        let root = Path::new("/repo");
        let diff = "\
diff --git a/src/foo.rs b/src/foo.rs
index 111..222 100644
--- a/src/foo.rs
+++ b/src/foo.rs
@@ -10,0 +11,2 @@ fn a()
+    let x = 1;
+    let y = 2;
@@ -30,1 +33,1 @@ fn b()
-    old
+    new
diff --git a/gone.rs b/gone.rs
deleted file mode 100644
--- a/gone.rs
+++ /dev/null
@@ -1,3 +0,0 @@
-a
-b
-c
diff --git a/src/new.py b/src/new.py
new file mode 100644
--- /dev/null
+++ b/src/new.py
@@ -0,0 +1,4 @@
+def f():
+    pass
";
        let files = parse_unified_diff(diff, root);
        // gone.rs (deleted, head=/dev/null) is dropped; foo.rs + new.py remain.
        assert_eq!(files.len(), 2);
        let foo = &files[0];
        assert_eq!(foo.path, "/repo/src/foo.rs");
        assert_eq!(
            foo.ranges,
            vec![
                ChangedRange { start: 11, end: 12 },
                ChangedRange { start: 33, end: 33 },
            ]
        );
        let new = &files[1];
        assert_eq!(new.path, "/repo/src/new.py");
        assert_eq!(new.ranges, vec![ChangedRange { start: 1, end: 4 }]);
    }

    #[test]
    fn invalid_range_rejected() {
        // A range starting with `-` would be a git flag.
        let err = changed_lines(Path::new("."), "--exec=rm").unwrap_err();
        assert!(matches!(err, DiffError::InvalidRange(_)));
    }
}
