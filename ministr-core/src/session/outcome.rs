//! Session-outcome telemetry (gui-rw-session-outcome) — the pure join
//! behind the GUI's trust-evidence receipts.
//!
//! Joins two records the daemon already keeps: a session's delivered
//! content (turn-ordered [`DeliveredItem`]s, whose content ids embed
//! source paths) and the coherence ring's observed file edits. The
//! emitted claim is deliberately the JOIN FACT — "session S had read X
//! (its R-th distinct file) and X changed at T" — never a strict
//! temporal ordering, because deliveries carry turns, not wall clocks
//! (blueprint v4: receipts restate, never embellish).
//!
//! Counts only: no time-saved synthesis lives here or downstream.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use super::types::DeliveredItem;

/// One distinct file a session read, in first-read order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRecord {
    /// Absolute source path extracted from the content id.
    pub path: String,
    /// 1-based rank among the session's distinct files (1 = first file
    /// the session ever read).
    pub rank: usize,
    /// The turn of the FIRST delivery from this file.
    pub first_turn: u32,
}

/// A read→edit join: a file the session had read was observed changing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeEvent {
    pub session_id: String,
    /// Absolute path of the edited file.
    pub path: String,
    /// 1-based rank of this file among the session's distinct reads.
    pub read_rank: usize,
    /// True when this was the FIRST distinct file the session read —
    /// the "first-touch" trust signal.
    pub first_touch: bool,
    /// Distinct files the session read BEFORE this one (the wander).
    pub reads_before: usize,
    /// When the watcher observed the edit (unix ms).
    pub edited_at_ms: u64,
}

/// Per-session aggregates over a window of outcome events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionOutcomeStats {
    pub session_id: String,
    /// Distinct files the session has read in total.
    pub distinct_reads: usize,
    /// Read→edit joins observed (distinct edited paths).
    pub joins: usize,
    /// Joins where the edited file was the session's FIRST read.
    pub first_touch_hits: usize,
}

/// Extract the source-file path from a content id.
///
/// Handles both id families the index emits:
/// `/abs/path/file.rs#section:claim` (sections/claims) and
/// `sym-/abs/path/file.rs::module::Name` (symbols).
#[must_use]
pub fn content_path(content_id: &str) -> Option<&str> {
    if let Some(rest) = content_id.strip_prefix("sym-") {
        return rest.split("::").next().filter(|p| !p.is_empty());
    }
    let path = content_id.split('#').next().unwrap_or(content_id);
    if path.is_empty() { None } else { Some(path) }
}

/// Collapse a session's deliveries into distinct files in first-read
/// order (rank 1 = the session's first file).
pub fn distinct_reads<'a>(items: impl Iterator<Item = &'a DeliveredItem>) -> Vec<ReadRecord> {
    let mut by_turn: Vec<(&str, u32)> = items
        .filter_map(|i| content_path(&i.content_id.0).map(|p| (p, i.turn_delivered)))
        .collect();
    by_turn.sort_by_key(|(_, turn)| *turn);

    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    for (path, turn) in by_turn {
        if seen.insert(path) {
            out.push(ReadRecord {
                path: path.to_owned(),
                rank: out.len() + 1,
                first_turn: turn,
            });
        }
    }
    out
}

/// Join one session's reads against observed edits.
///
/// `edits` are `(path, edited_at_ms)` pairs (the coherence ring's view,
/// any order). One event per distinct edited path — repeat edits of the
/// same file collapse to the LATEST timestamp so stats never
/// double-count a path.
#[must_use]
pub fn join_outcomes(
    session_id: &str,
    reads: &[ReadRecord],
    edits: &[(String, u64)],
) -> Vec<OutcomeEvent> {
    let mut latest_edit: HashMap<&str, u64> = HashMap::new();
    for (path, ts) in edits {
        let e = latest_edit.entry(path.as_str()).or_insert(0);
        if *ts > *e {
            *e = *ts;
        }
    }

    let mut out: Vec<OutcomeEvent> = reads
        .iter()
        .filter_map(|r| {
            latest_edit.get(r.path.as_str()).map(|&ts| OutcomeEvent {
                session_id: session_id.to_owned(),
                path: r.path.clone(),
                read_rank: r.rank,
                first_touch: r.rank == 1,
                reads_before: r.rank - 1,
                edited_at_ms: ts,
            })
        })
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.edited_at_ms));
    out
}

/// Aggregate a session's events into stats.
#[must_use]
pub fn session_stats(
    session_id: &str,
    distinct_reads: usize,
    events: &[OutcomeEvent],
) -> SessionOutcomeStats {
    let mine: Vec<&OutcomeEvent> = events
        .iter()
        .filter(|e| e.session_id == session_id)
        .collect();
    SessionOutcomeStats {
        session_id: session_id.to_owned(),
        distinct_reads,
        joins: mine.len(),
        first_touch_hits: mine.iter().filter(|e| e.first_touch).count(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::types::CompressionTier;
    use crate::types::{ContentId, Resolution};

    fn item(content_id: &str, turn: u32) -> DeliveredItem {
        DeliveredItem {
            content_id: ContentId(content_id.to_owned()),
            resolution: Resolution::Section,
            token_count: 10,
            turn_delivered: turn,
            content_hash: "h".to_owned(),
            compression_tier: CompressionTier::Full,
            compressed_summary: None,
        }
    }

    #[test]
    fn content_path_handles_both_id_families() {
        assert_eq!(
            content_path("/repo/src/auth.ts#root:c1"),
            Some("/repo/src/auth.ts")
        );
        assert_eq!(
            content_path("sym-/repo/src/db.rs::db::connect"),
            Some("/repo/src/db.rs")
        );
        assert_eq!(content_path("#weird"), None);
    }

    #[test]
    fn distinct_reads_rank_by_first_turn_and_dedupe() {
        let items = [
            item("/r/b.rs#x", 3),
            item("/r/a.rs#x", 1),
            item("/r/a.rs#y", 2), // re-read: still rank 1
            item("sym-/r/c.rs::m::f", 5),
        ];
        let reads = distinct_reads(items.iter());
        assert_eq!(reads.len(), 3);
        assert_eq!((reads[0].path.as_str(), reads[0].rank), ("/r/a.rs", 1));
        assert_eq!((reads[1].path.as_str(), reads[1].rank), ("/r/b.rs", 2));
        assert_eq!((reads[2].path.as_str(), reads[2].rank), ("/r/c.rs", 3));
    }

    #[test]
    fn scripted_session_first_touch_and_wander() {
        // The agent reads a.rs first, wanders through b.rs and c.rs;
        // the user then edits a.rs (first-touch hit) and c.rs (rank 3).
        let items = [
            item("/r/a.rs#x", 1),
            item("/r/b.rs#x", 2),
            item("/r/c.rs#x", 3),
        ];
        let reads = distinct_reads(items.iter());
        let edits = vec![
            ("/r/a.rs".to_owned(), 100),
            ("/r/c.rs".to_owned(), 200),
            ("/r/never-read.rs".to_owned(), 300),
        ];
        let events = join_outcomes("s1", &reads, &edits);
        assert_eq!(events.len(), 2, "unread files never join");
        // newest-first: c.rs (200) then a.rs (100)
        assert_eq!(events[0].path, "/r/c.rs");
        assert!(!events[0].first_touch);
        assert_eq!(events[0].reads_before, 2);
        assert_eq!(events[1].path, "/r/a.rs");
        assert!(events[1].first_touch);
        assert_eq!(events[1].reads_before, 0);

        let stats = session_stats("s1", reads.len(), &events);
        assert_eq!(stats.joins, 2);
        assert_eq!(stats.first_touch_hits, 1);
        assert_eq!(stats.distinct_reads, 3);
    }

    #[test]
    fn repeat_edits_collapse_to_latest() {
        let reads = distinct_reads([item("/r/a.rs#x", 1)].iter());
        let edits = vec![
            ("/r/a.rs".to_owned(), 100),
            ("/r/a.rs".to_owned(), 500),
            ("/r/a.rs".to_owned(), 300),
        ];
        let events = join_outcomes("s1", &reads, &edits);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].edited_at_ms, 500);
    }
}
