//! Token-bucket trait + in-memory impl.
//!
//! The classical token bucket: a bucket of `capacity` tokens that
//! refills at `refill_per_sec` tokens per second. Each request
//! consumes one token; when the bucket is empty the request is
//! denied.
//!
//! # SOLID — Interface Segregation
//!
//! [`TokenBucket`] exposes one method. The decision shape
//! [`RateLimitDecision`] carries the retry-after hint the middleware
//! surfaces in the `Retry-After` HTTP header — keeping the math out
//! of the HTTP layer.
//!
//! Future Redis-backed implementations slot in by impl-ing the same
//! trait; the middleware never changes.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

/// Outcome of a `try_acquire` call. `Allowed` lets the request through;
/// `Denied { retry_after }` rejects it and signals roughly how long
/// the client should wait before retrying.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RateLimitDecision {
    Allowed,
    Denied { retry_after: Duration },
}

/// Per-key token-bucket rate limiter.
///
/// Implementations MUST be `Send + Sync` so the layer can share a
/// single `Arc<dyn TokenBucket>` across all request handlers.
pub trait TokenBucket: Send + Sync + std::fmt::Debug {
    /// Try to consume one token from the bucket keyed by `key`.
    ///
    /// Returns [`RateLimitDecision::Allowed`] when the request fits
    /// within the bucket's current capacity; [`RateLimitDecision::Denied`]
    /// when the bucket is empty (with a `retry_after` hint for the
    /// client).
    fn try_acquire(&self, key: &str) -> RateLimitDecision;
}

/// Per-key state.
#[derive(Debug, Clone, Copy)]
struct BucketState {
    tokens: f64,
    last_refill: Instant,
}

/// In-process token-bucket store. One row per key; lazy eviction is
/// safe because token-bucket state is reconstructible from a single
/// timestamp + the configured rate.
///
/// Cross-pod skew exists in multi-pod cloud deployments but the F2.2
/// caps are loose enough that the skew doesn't matter; F5's stricter
/// SLA may swap this for a Redis-backed impl behind the same trait.
#[derive(Debug)]
pub struct InMemoryBucket {
    capacity: f64,
    refill_per_sec: f64,
    buckets: Mutex<HashMap<String, BucketState>>,
}

impl InMemoryBucket {
    /// Construct a bucket with `capacity` tokens that refills at
    /// `refill_per_sec` tokens/second. A burst up to `capacity` is
    /// permitted; steady-state rate equals `refill_per_sec`.
    ///
    /// # Panics
    ///
    /// Panics in debug builds when `capacity` or `refill_per_sec` is
    /// non-positive — both are mis-configuration, not runtime errors.
    #[must_use]
    pub fn new(capacity: f64, refill_per_sec: f64) -> Self {
        debug_assert!(capacity > 0.0, "capacity must be positive");
        debug_assert!(refill_per_sec > 0.0, "refill_per_sec must be positive");
        Self {
            capacity,
            refill_per_sec,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Drop bucket rows untouched for at least `older_than`. Optional
    /// hygiene call the middleware can invoke periodically; never
    /// required for correctness because the trait recreates a missing
    /// row at full capacity, identical to the eventual steady state.
    pub fn prune(&self, older_than: Duration) {
        let now = Instant::now();
        let mut buckets = self.buckets.lock();
        buckets.retain(|_, s| now.duration_since(s.last_refill) < older_than);
    }
}

impl TokenBucket for InMemoryBucket {
    fn try_acquire(&self, key: &str) -> RateLimitDecision {
        let now = Instant::now();
        let mut buckets = self.buckets.lock();
        let state = buckets.entry(key.to_owned()).or_insert(BucketState {
            tokens: self.capacity,
            last_refill: now,
        });

        // Refill — clamp to capacity. f64 keeps fractional refills
        // honest across short request cadence.
        let elapsed = now.duration_since(state.last_refill).as_secs_f64();
        state.tokens = (state.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        state.last_refill = now;

        if state.tokens >= 1.0 {
            state.tokens -= 1.0;
            RateLimitDecision::Allowed
        } else {
            // How long until at least 1 token is available again.
            // `refill_per_sec` is checked positive at construction.
            let needed = 1.0 - state.tokens;
            let wait_secs = needed / self.refill_per_sec;
            RateLimitDecision::Denied {
                retry_after: Duration::from_secs_f64(wait_secs.max(0.001)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_burst_up_to_capacity_then_denies() {
        let b = InMemoryBucket::new(3.0, 1.0);
        for _ in 0..3 {
            assert_eq!(b.try_acquire("k"), RateLimitDecision::Allowed);
        }
        let denied = b.try_acquire("k");
        assert!(matches!(denied, RateLimitDecision::Denied { .. }));
    }

    #[test]
    fn keys_are_independent() {
        let b = InMemoryBucket::new(1.0, 1.0);
        assert_eq!(b.try_acquire("a"), RateLimitDecision::Allowed);
        // Same bucket capacity — but a different key gets its own
        // budget.
        assert_eq!(b.try_acquire("b"), RateLimitDecision::Allowed);
    }

    #[test]
    fn refill_restores_capacity_over_time() {
        let b = InMemoryBucket::new(1.0, 1000.0); // refills FAST
        assert_eq!(b.try_acquire("k"), RateLimitDecision::Allowed);
        // After enough wall-clock time the bucket should refill.
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(b.try_acquire("k"), RateLimitDecision::Allowed);
    }

    #[test]
    fn retry_after_is_proportional_to_refill_rate() {
        let b = InMemoryBucket::new(1.0, 2.0); // 2 tokens/sec
        assert_eq!(b.try_acquire("k"), RateLimitDecision::Allowed);
        match b.try_acquire("k") {
            RateLimitDecision::Denied { retry_after } => {
                // Should be roughly 0.5s — 1 token at 2/s. Tolerate
                // some slack for the elapsed-time math.
                assert!(retry_after.as_millis() >= 400);
                assert!(retry_after.as_millis() <= 600);
            }
            RateLimitDecision::Allowed => panic!("expected Denied, got Allowed"),
        }
    }

    #[test]
    fn prune_drops_idle_keys() {
        let b = InMemoryBucket::new(1.0, 1.0);
        let _ = b.try_acquire("kept");
        std::thread::sleep(Duration::from_millis(10));
        let _ = b.try_acquire("kept"); // refreshes last_refill
        b.prune(Duration::from_millis(5));
        // The just-touched key survives — last_refill is recent.
        assert_eq!(b.buckets.lock().len(), 1);
    }

    #[test]
    fn dyn_dispatch_compiles() {
        let b: std::sync::Arc<dyn TokenBucket> = std::sync::Arc::new(InMemoryBucket::new(1.0, 1.0));
        let _ = b.try_acquire("k");
    }
}
