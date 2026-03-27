//! Lightweight RSS memory profiler for diagnosing memory leaks.
//!
//! Logs resident set size (RSS) at key checkpoints during ingestion.
//! Uses `proc_pidinfo` on macOS or `/proc/self/statm` on Linux.

use std::sync::atomic::{AtomicU64, Ordering};

use tracing::warn;

/// Tracks the high-water mark and previous checkpoint for delta reporting.
static HIGH_WATER_BYTES: AtomicU64 = AtomicU64::new(0);
static LAST_CHECKPOINT_BYTES: AtomicU64 = AtomicU64::new(0);

/// Get current RSS in bytes by reading `/proc/self/statm` (Linux)
/// or shelling out to `ps` (macOS).
#[cfg(target_os = "macos")]
fn rss_bytes() -> Option<u64> {
    let pid = std::process::id();
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .ok()?;
    let kb: u64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;
    Some(kb * 1024)
}

#[cfg(target_os = "linux")]
fn rss_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let rss_pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    let page_size = 4096u64; // typical
    Some(rss_pages * page_size)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn rss_bytes() -> Option<u64> {
    None
}

/// Format bytes as a human-readable string.
fn fmt_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

/// Log a memory checkpoint with a label. Reports current RSS, delta from
/// last checkpoint, and high-water mark.
pub fn checkpoint(label: &str) {
    let Some(rss) = rss_bytes() else { return };

    let prev = LAST_CHECKPOINT_BYTES.swap(rss, Ordering::Relaxed);
    let delta = rss as i64 - prev as i64;
    HIGH_WATER_BYTES.fetch_max(rss, Ordering::Relaxed);
    let hwm = HIGH_WATER_BYTES.load(Ordering::Relaxed);

    let sign = if delta >= 0 { "+" } else { "" };
    warn!(
        rss = %fmt_mb(rss),
        delta = %format!("{sign}{}", fmt_mb(delta.unsigned_abs())),
        high_water = %fmt_mb(hwm),
        "[mem] {label}"
    );
}

/// Log a memory checkpoint every N calls (to avoid log spam during per-file loops).
/// Returns the current RSS in bytes for callers that want to act on it.
pub fn checkpoint_every(n: usize, counter: usize, label: &str) -> Option<u64> {
    if counter % n != 0 {
        return rss_bytes();
    }
    let rss = rss_bytes()?;
    let prev = LAST_CHECKPOINT_BYTES.swap(rss, Ordering::Relaxed);
    let delta = rss as i64 - prev as i64;
    HIGH_WATER_BYTES.fetch_max(rss, Ordering::Relaxed);
    let hwm = HIGH_WATER_BYTES.load(Ordering::Relaxed);

    let sign = if delta >= 0 { "+" } else { "" };
    warn!(
        rss = %fmt_mb(rss),
        delta = %format!("{sign}{}", fmt_mb(delta.unsigned_abs())),
        high_water = %fmt_mb(hwm),
        file_num = counter,
        "[mem] {label}"
    );
    Some(rss)
}
