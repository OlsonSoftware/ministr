//! Line-based delta computation for content change detection.
//!
//! When a previously-delivered section has changed, the delta module computes
//! a minimal textual diff so the agent receives only what changed rather than
//! the full section text again.

use serde::Serialize;

/// A line-level change in a delta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DeltaLine {
    /// A line present in both old and new versions (context).
    Context { line: String },
    /// A line added in the new version.
    Added { line: String },
    /// A line removed from the old version.
    Removed { line: String },
}

/// A computed delta between two versions of content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContentDelta {
    /// The individual line changes.
    pub lines: Vec<DeltaLine>,
    /// Number of lines added.
    pub additions: usize,
    /// Number of lines removed.
    pub removals: usize,
}

/// Compute a line-based delta between old and new content.
///
/// Uses a simple longest-common-subsequence (LCS) algorithm to produce
/// a minimal diff. For typical section sizes (<200 lines), this is fast
/// enough without optimization.
///
/// # Examples
///
/// ```
/// use iris_core::session::delta::compute_delta;
///
/// let old = "line1\nline2\nline3";
/// let new = "line1\nmodified\nline3";
/// let delta = compute_delta(old, new);
///
/// assert_eq!(delta.additions, 1);
/// assert_eq!(delta.removals, 1);
/// ```
#[must_use]
pub fn compute_delta(old: &str, new: &str) -> ContentDelta {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let lcs = lcs_table(&old_lines, &new_lines);
    let lines = build_diff(&old_lines, &new_lines, &lcs);

    let additions = lines
        .iter()
        .filter(|l| matches!(l, DeltaLine::Added { .. }))
        .count();
    let removals = lines
        .iter()
        .filter(|l| matches!(l, DeltaLine::Removed { .. }))
        .count();

    ContentDelta {
        lines,
        additions,
        removals,
    }
}

/// Build the LCS length table for two slices of lines.
fn lcs_table(old: &[&str], new: &[&str]) -> Vec<Vec<usize>> {
    let m = old.len();
    let n = new.len();
    let mut table = vec![vec![0usize; n + 1]; m + 1];

    for i in 1..=m {
        for j in 1..=n {
            if old[i - 1] == new[j - 1] {
                table[i][j] = table[i - 1][j - 1] + 1;
            } else {
                table[i][j] = table[i - 1][j].max(table[i][j - 1]);
            }
        }
    }

    table
}

/// Backtrack through the LCS table to produce diff lines.
fn build_diff(old: &[&str], new: &[&str], table: &[Vec<usize>]) -> Vec<DeltaLine> {
    let mut result = Vec::new();
    let mut i = old.len();
    let mut j = new.len();

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old[i - 1] == new[j - 1] {
            result.push(DeltaLine::Context {
                line: old[i - 1].to_string(),
            });
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || table[i][j - 1] >= table[i - 1][j]) {
            result.push(DeltaLine::Added {
                line: new[j - 1].to_string(),
            });
            j -= 1;
        } else {
            result.push(DeltaLine::Removed {
                line: old[i - 1].to_string(),
            });
            i -= 1;
        }
    }

    result.reverse();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_content_produces_no_changes() {
        let text = "line1\nline2\nline3";
        let delta = compute_delta(text, text);

        assert_eq!(delta.additions, 0);
        assert_eq!(delta.removals, 0);
        assert_eq!(delta.lines.len(), 3);
        assert!(
            delta
                .lines
                .iter()
                .all(|l| matches!(l, DeltaLine::Context { .. }))
        );
    }

    #[test]
    fn single_line_modification() {
        let old = "line1\nline2\nline3";
        let new = "line1\nmodified\nline3";
        let delta = compute_delta(old, new);

        assert_eq!(delta.additions, 1);
        assert_eq!(delta.removals, 1);
    }

    #[test]
    fn line_added_at_end() {
        let old = "line1\nline2";
        let new = "line1\nline2\nline3";
        let delta = compute_delta(old, new);

        assert_eq!(delta.additions, 1);
        assert_eq!(delta.removals, 0);
        assert!(
            delta.lines.last().unwrap()
                == &DeltaLine::Added {
                    line: "line3".to_string(),
                }
        );
    }

    #[test]
    fn line_removed_from_middle() {
        let old = "line1\nline2\nline3";
        let new = "line1\nline3";
        let delta = compute_delta(old, new);

        assert_eq!(delta.additions, 0);
        assert_eq!(delta.removals, 1);
        assert!(delta.lines.contains(&DeltaLine::Removed {
            line: "line2".to_string(),
        }));
    }

    #[test]
    fn completely_different_content() {
        let old = "aaa\nbbb";
        let new = "ccc\nddd";
        let delta = compute_delta(old, new);

        assert_eq!(delta.additions, 2);
        assert_eq!(delta.removals, 2);
    }

    #[test]
    fn empty_old_content() {
        let delta = compute_delta("", "line1\nline2");

        assert_eq!(delta.additions, 2);
        assert_eq!(delta.removals, 0);
    }

    #[test]
    fn empty_new_content() {
        let delta = compute_delta("line1\nline2", "");

        assert_eq!(delta.additions, 0);
        assert_eq!(delta.removals, 2);
    }

    #[test]
    fn both_empty() {
        let delta = compute_delta("", "");

        assert_eq!(delta.additions, 0);
        assert_eq!(delta.removals, 0);
        assert!(delta.lines.is_empty());
    }

    #[test]
    fn delta_serializes_to_json() {
        let delta = compute_delta("old line", "new line");
        let json = serde_json::to_string(&delta).unwrap();
        assert!(json.contains("\"type\":\"added\""));
        assert!(json.contains("\"type\":\"removed\""));
    }

    #[test]
    fn context_lines_preserved_in_order() {
        let old = "a\nb\nc\nd\ne";
        let new = "a\nb\nX\nd\ne";
        let delta = compute_delta(old, new);

        // Context lines should maintain document order
        let context_lines: Vec<&str> = delta
            .lines
            .iter()
            .filter_map(|l| match l {
                DeltaLine::Context { line } => Some(line.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(context_lines, vec!["a", "b", "d", "e"]);
    }
}
