//! Workspace-shared Unix-seconds → ISO-8601 formatter.
//!
//! Before this module landed, the Howard Hinnant `civil_from_days`
//! algorithm was hand-rolled in four places — `ministr-core`'s
//! `session::registry`, `ministr-mcp`'s `admin::handlers`, and
//! twice in `ministr-cli`'s `commands` (under two slightly different
//! names: `format_unix_secs_iso` + `civil_from_unix_secs_cli`).
//! Each copy was identical, but the duplication was a latent
//! correctness hazard: a leap-year fix or formatting change could
//! land in one copy and silently disagree with the others.
//!
//! This module is the single source of truth. Mainline call sites
//! (audit log, /sla endpoint, SLA flush task, license-mint audit log,
//! session snapshots) all delegate here.
//!
//! Output format: `YYYY-MM-DDTHH:MM:SSZ` — RFC 3339 / ISO 8601 with
//! a hard `Z` suffix because every caller wants UTC.
//!
//! Algorithm: Howard Hinnant's `civil_from_days` (the canonical
//! `chrono`-free implementation). `u64` input covers all dates up to
//! ~584 billion years past the epoch — comfortably more than any
//! realistic timestamp source.

use std::fmt::Write as _;

/// Format a Unix-seconds timestamp (UTC) as `YYYY-MM-DDTHH:MM:SSZ`.
///
/// Pure function — no allocations besides the returned `String`.
/// Handles leap years and leap-day boundaries via Howard Hinnant's
/// `civil_from_days` algorithm; tested against epoch zero, a known
/// 2026 anchor, and 2024-02-29.
#[must_use]
#[allow(clippy::many_single_char_names)] // single-letter names mirror Hinnant's published pseudocode (y/m/d/era/yoe/doe/doy/mp)
pub fn format_unix_secs_iso(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let time = secs % 86_400;
    let hour = time / 3_600;
    let minute = (time % 3_600) / 60;
    let second = time % 60;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = u64::try_from(z - era * 146_097).unwrap_or(0); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = i64::try_from(yoe).unwrap_or(0) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    let mut s = String::with_capacity(20);
    let _ = write!(
        s,
        "{year:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z"
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_zero() {
        assert_eq!(format_unix_secs_iso(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn known_2026_anchor() {
        // 2026-05-22T12:00:00Z. Same anchor F5.3-d-iii-b-dispatch and
        // the previous handlers.rs tests use, after the off-by-5-days
        // fix that chunk documented.
        assert_eq!(format_unix_secs_iso(1_779_451_200), "2026-05-22T12:00:00Z");
    }

    #[test]
    fn leap_day_2024() {
        // 2024-02-29T00:00:00Z. Hinnant's algorithm handles leap years
        // without special-casing; this guards against a future
        // "simplification" that breaks Feb 29.
        assert_eq!(format_unix_secs_iso(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn end_of_2026_q4() {
        // 2026-12-31T23:59:59Z — end-of-year edge.
        assert_eq!(format_unix_secs_iso(1_798_761_599), "2026-12-31T23:59:59Z");
    }

    #[test]
    fn output_is_always_20_chars() {
        // "YYYY-MM-DDTHH:MM:SSZ" = 20 chars. Useful for harness +
        // dashboards that fixed-width the column.
        assert_eq!(format_unix_secs_iso(0).len(), 20);
        assert_eq!(format_unix_secs_iso(1_798_761_599).len(), 20);
    }
}
