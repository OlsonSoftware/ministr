//! In-memory rolling-window latency histogram.
//!
//! Captures the elapsed time of every request the cloud serves and
//! exposes p50/p95/p99 percentiles via the `/sla` endpoint. The SLA
//! contract (p95 ≤200ms query latency) is meaningless without a number
//! to measure it against; this module ships the data source.
//!
//! # Storage choice
//!
//! Fixed-capacity `VecDeque<u32>` wrapped in an `Arc<Mutex<...>>`.
//! Samples are microseconds (u32 covers ~71 minutes per sample —
//! comfortably more than any in-pocket request budget). When the
//! buffer is full, `record_micros` pops the oldest entry before
//! pushing the new one, giving a true rolling window of the most
//! recent N samples.
//!
//! Mutex is fine at the few-hundred-QPS scale a single cloud pod
//! handles. `HdrHistogram` would buy O(1) percentile lookup at the
//! cost of a new dependency; the P-square algorithm would avoid
//! storage entirely at the cost of bounded error and more code.
//! Neither matters here — `cargo bench` shows sort-on-read of a
//! 1024-element buffer at ~30µs, well under any SLA budget.
//!
//! # What this chunk does NOT do
//!
//! - Per-route or per-method buckets — every request lands in one
//!   pool. `/sla` and `/healthz` are sub-millisecond and don't shift
//!   the p95 of real query workloads at 1024-sample window depth.
//! - Cross-pod aggregation — restart drops the buffer. A separate
//!   DB-backed metrics table covers 30-day rolling windows.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use serde::Serialize;

/// Default rolling-window capacity. ~30µs sort cost at this size.
pub const DEFAULT_CAPACITY: usize = 1024;

/// Cheap-to-clone latency-sample collector.
///
/// `Arc<Mutex<VecDeque<u32>>>` — every clone shares the same backing
/// buffer, mirroring the existing `AdminState::corpus_count` pattern
/// (an `Arc<AtomicUsize>`). Sample units are microseconds.
#[derive(Debug, Clone)]
pub struct LatencyTracker {
    samples: Arc<Mutex<VecDeque<u32>>>,
    capacity: usize,
}

impl LatencyTracker {
    /// Construct a tracker with the default rolling-window capacity
    /// ([`DEFAULT_CAPACITY`] samples).
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// Construct a tracker with an explicit capacity. Capacity is
    /// clamped to at least 1 — a 0-cap buffer can't take samples.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            samples: Arc::new(Mutex::new(VecDeque::with_capacity(cap))),
            capacity: cap,
        }
    }

    /// Record one latency sample. Drops the oldest entry to make room
    /// when the buffer is full so the window stays at exactly the
    /// configured capacity. Poison-recovers the mutex
    /// (`unwrap_or_else` into `into_inner`) so a panicking handler
    /// never wedges the tracker.
    pub fn record_micros(&self, micros: u32) {
        let mut buf = match self.samples.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if buf.len() == self.capacity {
            buf.pop_front();
        }
        buf.push_back(micros);
    }

    /// Compute the current percentiles. Returns `None` when no
    /// samples have arrived yet so the `/sla` handler can render
    /// JSON `null` rather than a misleading `0`.
    #[must_use]
    pub fn snapshot(&self) -> Option<LatencySnapshot> {
        let buf = match self.samples.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if buf.is_empty() {
            return None;
        }
        let mut sorted: Vec<u32> = buf.iter().copied().collect();
        sorted.sort_unstable();
        let count = sorted.len();
        Some(LatencySnapshot {
            count,
            p50_us: percentile(&sorted, 0.50),
            p95_us: percentile(&sorted, 0.95),
            p99_us: percentile(&sorted, 0.99),
        })
    }
}

impl Default for LatencyTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Snapshot of the tracker at a point in time. Renders to JSON via
/// `serde::Serialize` so the `/sla` handler can embed it directly.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct LatencySnapshot {
    /// Number of samples in the rolling window at snapshot time.
    pub count: usize,
    /// 50th percentile (median) in microseconds.
    pub p50_us: u32,
    /// 95th percentile in microseconds. The SLA contract is
    /// expressed against this number (≤200ms = `200_000`µs).
    pub p95_us: u32,
    /// 99th percentile in microseconds.
    pub p99_us: u32,
}

/// Nearest-rank percentile over a pre-sorted slice. `q` is in `[0, 1]`.
/// `sorted` MUST be non-empty (the caller checks).
fn percentile(sorted: &[u32], q: f64) -> u32 {
    debug_assert!(!sorted.is_empty(), "percentile on empty slice");
    // Nearest-rank method per NIST handbook: rank = ceil(q * N).
    let n = sorted.len();
    let q_clamped = q.clamp(0.0, 1.0);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let rank_one_based = (q_clamped * n as f64).ceil() as usize;
    let idx = rank_one_based.saturating_sub(1).min(n - 1);
    sorted[idx]
}

/// Axum middleware that records every request's total elapsed time.
/// Mount via `axum::middleware::from_fn_with_state` on the final
/// composed router so it sees every route. The /sla handler reads from
/// the same tracker via `AdminState`.
///
/// Elapsed time is clamped to `u32::MAX` microseconds (~71min) which
/// is generous — any single request taking longer than that is
/// already broken in ways the SLA dashboard isn't going to fix.
pub async fn record_latency_middleware(
    State(tracker): State<LatencyTracker>,
    req: Request,
    next: Next,
) -> Response {
    let start = std::time::Instant::now();
    let response = next.run(req).await;
    let elapsed = start.elapsed().as_micros();
    let clamped = u32::try_from(elapsed).unwrap_or(u32::MAX);
    tracker.record_micros(clamped);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_returns_none_when_empty() {
        let t = LatencyTracker::new();
        assert!(t.snapshot().is_none());
    }

    #[test]
    fn single_sample_collapses_to_one_value() {
        let t = LatencyTracker::new();
        t.record_micros(150_000);
        let s = t.snapshot().expect("non-empty");
        assert_eq!(s.count, 1);
        assert_eq!(s.p50_us, 150_000);
        assert_eq!(s.p95_us, 150_000);
        assert_eq!(s.p99_us, 150_000);
    }

    #[test]
    fn percentiles_against_known_distribution() {
        // 100 samples: 1µs through 100µs. Nearest-rank percentiles
        // give p50=50, p95=95, p99=99.
        let t = LatencyTracker::new();
        for i in 1u32..=100 {
            t.record_micros(i);
        }
        let s = t.snapshot().expect("non-empty");
        assert_eq!(s.count, 100);
        assert_eq!(s.p50_us, 50);
        assert_eq!(s.p95_us, 95);
        assert_eq!(s.p99_us, 99);
    }

    #[test]
    fn rolling_window_drops_oldest_when_full() {
        let t = LatencyTracker::with_capacity(3);
        t.record_micros(10);
        t.record_micros(20);
        t.record_micros(30);
        t.record_micros(40); // displaces 10
        let s = t.snapshot().expect("non-empty");
        assert_eq!(s.count, 3);
        // sorted = [20, 30, 40]; p50 (rank=ceil(.5*3)=2 → idx 1) = 30.
        assert_eq!(s.p50_us, 30);
        // p95 (rank=ceil(.95*3)=3 → idx 2) = 40.
        assert_eq!(s.p95_us, 40);
    }

    #[test]
    fn zero_capacity_clamps_to_one() {
        // A 0-cap buffer would silently drop every sample. Clamp to 1
        // so the tracker is always usable.
        let t = LatencyTracker::with_capacity(0);
        t.record_micros(42);
        assert_eq!(t.snapshot().expect("non-empty").p50_us, 42);
    }

    #[test]
    fn clone_shares_backing_buffer() {
        // Cheap-to-clone property: two clones see each other's samples
        // through the same Arc.
        let a = LatencyTracker::new();
        let b = a.clone();
        a.record_micros(123);
        let from_b = b.snapshot().expect("clone shares state");
        assert_eq!(from_b.count, 1);
        assert_eq!(from_b.p50_us, 123);
    }
}
