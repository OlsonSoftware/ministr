//! Authorship (`git blame`) for a line range — the read-only metadata behind
//! FL7's "blame on `ministr_definition`". Given an indexed symbol's file and
//! line span, it answers "who wrote this, and when was it last touched" without
//! the agent shelling out to git itself.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

/// Error producing blame for a line range.
#[derive(Debug, thiserror::Error)]
pub enum BlameError {
    /// `git` is not installed or could not be launched.
    #[error("git is not installed or could not be launched")]
    GitNotAvailable,
    /// `git blame` exited non-zero (e.g. the path is untracked or not a repo).
    #[error("git blame failed: {0}")]
    BlameFailed(String),
}

/// One contributor's share of a blamed range.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub struct BlameAuthor {
    /// Author name as recorded by git.
    pub name: String,
    /// Number of lines in the range last touched by this author.
    pub lines: u32,
}

/// Authorship summary for a file line range.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
pub struct BlameSummary {
    /// Contributors, sorted by line count descending (ties broken by name).
    pub authors: Vec<BlameAuthor>,
    /// Total lines blamed.
    pub total_lines: u32,
    /// Author of the most-recently-committed line in the range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_author: Option<String>,
    /// Commit time (Unix epoch seconds) of the most-recent line in the range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_commit_epoch: Option<i64>,
}

/// Blame the inclusive 1-based line range `[start, end]` of `file`.
///
/// Runs `git blame --porcelain -L start,end -- <file>` from the file's parent
/// directory and aggregates per-author line counts plus the most-recent commit
/// time in the range. Returns `Ok(None)` when the range is empty.
///
/// # Errors
///
/// Returns [`BlameError`] when git is unavailable or `git blame` exits non-zero
/// (untracked file, not a repo, …) — callers treat this as "no blame available"
/// rather than a hard failure.
pub fn blame_range(file: &Path, start: u32, end: u32) -> Result<Option<BlameSummary>, BlameError> {
    if start == 0 || end < start {
        return Ok(None);
    }
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    let loc = format!("{start},{end}");
    let output = Command::new("git")
        .args(["blame", "--porcelain", "-L", &loc, "--"])
        .arg(file)
        .current_dir(dir)
        .output()
        .map_err(|_| BlameError::GitNotAvailable)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(BlameError::BlameFailed(stderr));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    Ok(Some(parse_porcelain(&text)))
}

/// Aggregate `git blame --porcelain` output into a [`BlameSummary`].
///
/// Porcelain emits a `<sha> <orig> <final> [<count>]` header per line, the
/// author/time metadata once per commit, and the source line prefixed with a
/// tab. Each tab-prefixed line is one blamed line attributed to the current sha.
fn parse_porcelain(text: &str) -> BlameSummary {
    // Per-sha author + author-time, populated as git emits each commit's block.
    let mut sha_author: HashMap<String, String> = HashMap::new();
    let mut sha_time: HashMap<String, i64> = HashMap::new();
    let mut counts: HashMap<String, u32> = HashMap::new();
    let mut total = 0u32;
    let mut last_author: Option<String> = None;
    let mut last_time: Option<i64> = None;

    let mut current_sha: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('\t') {
            // A blamed source line (its content; we only count it).
            let _ = rest;
            if let Some(sha) = &current_sha {
                let author = sha_author.get(sha).cloned().unwrap_or_default();
                *counts.entry(author.clone()).or_insert(0) += 1;
                total += 1;
                if let Some(&t) = sha_time.get(sha)
                    && last_time.is_none_or(|prev| t > prev)
                {
                    last_time = Some(t);
                    last_author = Some(author);
                }
            }
        } else if let Some(author) = line.strip_prefix("author ") {
            if let Some(sha) = &current_sha {
                sha_author.insert(sha.clone(), author.to_string());
            }
        } else if let Some(ts) = line.strip_prefix("author-time ") {
            if let (Some(sha), Ok(t)) = (&current_sha, ts.trim().parse::<i64>()) {
                sha_time.insert(sha.clone(), t);
            }
        } else if is_blame_header(line) {
            // `<40-hex-sha> <orig> <final> [<count>]` — start of a line group.
            current_sha = line.split(' ').next().map(str::to_string);
        }
    }

    let mut authors: Vec<BlameAuthor> = counts
        .into_iter()
        .map(|(name, lines)| BlameAuthor { name, lines })
        .collect();
    authors.sort_by(|a, b| b.lines.cmp(&a.lines).then_with(|| a.name.cmp(&b.name)));

    BlameSummary {
        authors,
        total_lines: total,
        last_author,
        last_commit_epoch: last_time,
    }
}

/// Whether a porcelain line is a `<40-hex-sha> <orig> <final> [<count>]` header.
fn is_blame_header(line: &str) -> bool {
    let mut parts = line.split(' ');
    let sha = parts.next().unwrap_or("");
    sha.len() >= 40
        && sha.bytes().all(|b| b.is_ascii_hexdigit())
        && parts
            .next()
            .is_some_and(|t| t.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_range_is_none() {
        assert!(blame_range(Path::new("x"), 0, 0).unwrap().is_none());
        assert!(blame_range(Path::new("x"), 5, 4).unwrap().is_none());
    }

    #[test]
    fn blame_real_repo_file() {
        // Blame a committed range of this crate's own source (the header doc of
        // git/mod.rs, lines 1–3) — exercises the real `git blame` subprocess.
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/git/mod.rs");
        let summary = blame_range(&file, 1, 3).expect("git blame succeeds in the repo");
        let s = summary.expect("non-empty range yields a summary");
        assert_eq!(s.total_lines, 3, "blamed exactly the 3 requested lines");
        assert!(!s.authors.is_empty(), "at least one author");
        assert_eq!(
            s.authors.iter().map(|a| a.lines).sum::<u32>(),
            3,
            "author line counts sum to the range size",
        );
    }

    #[test]
    fn parse_aggregates_authors_and_latest() {
        // Two commits: Alice (older, 2 lines) and Bob (newer, 1 line).
        let porcelain = "\
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 1 1 2
author Alice
author-time 1000
summary first
\tline one
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 2 2
\tline two
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb 3 3 1
author Bob
author-time 2000
summary second
\tline three
";
        let s = parse_porcelain(porcelain);
        assert_eq!(s.total_lines, 3);
        assert_eq!(s.authors.len(), 2);
        // Alice has the most lines → first.
        assert_eq!(s.authors[0].name, "Alice");
        assert_eq!(s.authors[0].lines, 2);
        assert_eq!(s.authors[1].name, "Bob");
        assert_eq!(s.authors[1].lines, 1);
        // Bob committed most recently (author-time 2000).
        assert_eq!(s.last_author.as_deref(), Some("Bob"));
        assert_eq!(s.last_commit_epoch, Some(2000));
    }
}
