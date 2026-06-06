//! run_digest — token-lean rendering of exec run logs (exec-mcp-tools).
//!
//! Pure functions that turn a captured run log into what an agent
//! actually needs: the exit code, every error/warning line, a small
//! head+tail window, and exact totals — instead of the raw dump that
//! 2026 agents demonstrably choke on. The digest is the default
//! `ministr_run` response; `ministr_run_logs` pages the full log on
//! demand with a never-resend cursor.

use serde::Serialize;

/// Cap on preserved diagnostic (error/warning) lines in a digest.
const MAX_DIAGNOSTIC_LINES: usize = 200;
/// Head lines preserved in the digest window.
const WINDOW_HEAD_LINES: usize = 15;
/// Tail lines preserved in the digest window.
const WINDOW_TAIL_LINES: usize = 35;

/// A token-lean summary of one run's output.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
pub struct RunDigest {
    /// Every line that matched the diagnostic patterns (error / warning /
    /// panic / failure), consecutive duplicates collapsed to `N× line`.
    /// Capped at 200 lines; `diagnostics_truncated` reports the overflow.
    pub diagnostics: Vec<String>,
    /// True when more diagnostic lines matched than were kept.
    pub diagnostics_truncated: bool,
    /// Head + tail window of the log (consecutive duplicates collapsed);
    /// the middle is elided when the log is longer than the window.
    pub window: String,
    /// Total lines in the captured log.
    pub lines_total: usize,
}

/// Is this line a diagnostic an agent must never lose?
///
/// Tuned for compiler / test-runner output (cargo, tsc, pytest, jest):
/// error and warning headers, panics, assertion failures, fatal aborts.
fn is_diagnostic(line: &str) -> bool {
    let l = line.trim_start();
    let lower = l.to_lowercase();
    lower.starts_with("error")
        || lower.starts_with("warning")
        || lower.starts_with("fatal")
        || lower.starts_with("panicked")
        || lower.starts_with("thread '")
        || lower.starts_with("assertion")
        || lower.contains("error:")
        || lower.contains("error[")
        || lower.contains(" panicked at ")
        || lower.contains("traceback (most recent call last)")
        || lower.contains("exception")
        || lower.contains("failed")
        || lower.contains("failure")
}

/// Collapse consecutive duplicate lines to `N× line`.
fn collapse_repeats<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut last: Option<(&str, usize)> = None;
    for line in lines {
        match &mut last {
            Some((prev, count)) if *prev == line => *count += 1,
            _ => {
                if let Some((prev, count)) = last.take() {
                    out.push(render_repeat(prev, count));
                }
                last = Some((line, 1));
            }
        }
    }
    if let Some((prev, count)) = last {
        out.push(render_repeat(prev, count));
    }
    out
}

fn render_repeat(line: &str, count: usize) -> String {
    if count > 1 {
        format!("{count}× {line}")
    } else {
        line.to_string()
    }
}

/// Build the digest for a captured log.
#[must_use]
pub fn digest(log: &str) -> RunDigest {
    let lines: Vec<&str> = log.lines().collect();
    let lines_total = lines.len();

    let diag_lines: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| is_diagnostic(l))
        .collect();
    let collapsed_diags = collapse_repeats(diag_lines.into_iter());
    let diagnostics_truncated = collapsed_diags.len() > MAX_DIAGNOSTIC_LINES;
    let mut diagnostics = collapsed_diags;
    diagnostics.truncate(MAX_DIAGNOSTIC_LINES);

    let window = if lines_total <= WINDOW_HEAD_LINES + WINDOW_TAIL_LINES {
        collapse_repeats(lines.iter().copied()).join("\n")
    } else {
        let head = collapse_repeats(lines[..WINDOW_HEAD_LINES].iter().copied());
        let tail =
            collapse_repeats(lines[lines_total - WINDOW_TAIL_LINES..].iter().copied());
        let elided = lines_total - WINDOW_HEAD_LINES - WINDOW_TAIL_LINES;
        format!(
            "{}\n…[{elided} lines elided — ministr_run_logs for the rest]…\n{}",
            head.join("\n"),
            tail.join("\n")
        )
    };

    RunDigest {
        diagnostics,
        diagnostics_truncated,
        window,
        lines_total,
    }
}

/// The next undelivered span of a log, for cursor-based delta delivery.
///
/// Returns `(span, next_cursor)`: the slice of `log` starting at byte
/// `cursor` (snapped back to a char boundary if mid-codepoint), capped at
/// `max_bytes` but extended to the end of its final line so a line is
/// never split. Calling again with the returned cursor yields only new
/// content — the never-resend contract.
#[must_use]
pub fn next_span(log: &str, cursor: usize, max_bytes: usize) -> (&str, usize) {
    let len = log.len();
    let mut start = cursor.min(len);
    while start < len && !log.is_char_boundary(start) {
        start -= 1;
    }
    if start >= len {
        return ("", len);
    }
    let rest = &log[start..];
    if rest.len() <= max_bytes {
        return (rest, len);
    }
    let mut end = max_bytes;
    while end < rest.len() && !rest.is_char_boundary(end) {
        end += 1;
    }
    // Extend to the end of the current line so lines are never split.
    let end = match rest[end..].find('\n') {
        Some(nl) => end + nl + 1,
        None => rest.len(),
    };
    (&rest[..end], start + end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthesize a noisy cargo-build-shaped log: heavy compile chatter
    /// with a handful of buried error lines.
    fn noisy_build_log() -> (String, Vec<String>) {
        let mut log = String::new();
        let mut errors = Vec::new();
        for i in 0..3000 {
            log.push_str(&format!(
                "   Compiling crate-{i} v0.1.{} (/work/deps/crate-{i})\n",
                i % 10
            ));
            if i % 700 == 350 {
                let err = format!("error[E0308]: mismatched types in crate-{i}");
                log.push_str(&err);
                log.push('\n');
                errors.push(err);
            }
        }
        log.push_str("error: could not compile `app` (bin \"app\") due to 4 previous errors\n");
        errors.push(
            "error: could not compile `app` (bin \"app\") due to 4 previous errors".to_string(),
        );
        (log, errors)
    }

    #[test]
    fn digest_is_at_most_ten_percent_of_raw_and_keeps_every_error_line() {
        let (log, errors) = noisy_build_log();
        let d = digest(&log);

        // Every error line survives, verbatim.
        for err in &errors {
            assert!(
                d.diagnostics.iter().any(|l| l.contains(err)),
                "digest lost error line: {err}"
            );
        }

        // The whole serialized digest is <=10% of the raw log bytes
        // (bytes as the token proxy — both sides tokenize comparably).
        let rendered = serde_json::to_string(&d).expect("serialize");
        let ratio = rendered.len() as f64 / log.len() as f64;
        assert!(
            ratio <= 0.10,
            "digest must be <=10% of raw ({} / {} = {ratio:.3})",
            rendered.len(),
            log.len()
        );
        assert_eq!(d.lines_total, log.lines().count());
    }

    #[test]
    fn digest_collapses_repeated_lines_with_counts() {
        let log = "warning: unused import\n".repeat(50) + "done\n";
        let d = digest(&log);
        assert!(
            d.diagnostics
                .iter()
                .any(|l| l.starts_with("50× warning: unused import")),
            "repeats must collapse with a count: {:?}",
            d.diagnostics
        );
    }

    #[test]
    fn digest_window_keeps_head_and_tail_and_elides_middle() {
        let log: String = (0..200).map(|i| format!("line-{i}\n")).collect();
        let d = digest(&log);
        assert!(d.window.contains("line-0"), "head preserved");
        assert!(d.window.contains("line-199"), "tail preserved");
        assert!(d.window.contains("lines elided"), "middle elided");
        assert!(!d.window.contains("line-100"), "middle actually dropped");
    }

    #[test]
    fn next_span_never_resends_and_never_splits_lines() {
        let log: String = (0..100).map(|i| format!("entry-{i:03}\n")).collect();
        let (first, cursor1) = next_span(&log, 0, 64);
        assert!(first.ends_with('\n'), "span ends at a line boundary");
        let (second, cursor2) = next_span(&log, cursor1, 64);
        assert!(
            !second.contains(first.lines().last().expect("non-empty")),
            "second span must not resend the first span's content"
        );
        // Drain to the end; the union must reconstruct the log exactly.
        let mut all = String::new();
        all.push_str(first);
        all.push_str(second);
        let mut cursor = cursor2;
        loop {
            let (span, next) = next_span(&log, cursor, 64);
            if span.is_empty() {
                break;
            }
            all.push_str(span);
            cursor = next;
        }
        assert_eq!(all, log, "delta spans reconstruct the log losslessly");

        // A cursor at the end yields nothing — repeat calls stay empty.
        let (empty, end) = next_span(&log, log.len(), 64);
        assert!(empty.is_empty());
        assert_eq!(end, log.len());
    }
}
