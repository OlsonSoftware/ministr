//! Token-bucket rate limiting for outbound API calls.
//!
//! A token bucket refills at a fixed rate and allows a burst up to its
//! capacity. This module is part of the code-heavy evaluation corpus
//! (eval/corpus-code) used to benchmark embedders on text-to-code retrieval.

use std::time::{Duration, Instant};

/// A token bucket that throttles how often an operation may proceed.
///
/// The bucket holds up to `capacity` tokens and refills `refill_per_sec`
/// tokens every second. Each admitted request consumes one token; when the
/// bucket is empty the request is rejected until enough time has passed.
pub struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_per_sec: f64,
    last_refill: Instant,
}

impl TokenBucket {
    /// Create a full bucket with the given burst `capacity` and steady-state
    /// `refill_per_sec` refill rate.
    pub fn new(capacity: f64, refill_per_sec: f64) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_per_sec,
            last_refill: Instant::now(),
        }
    }

    /// Add tokens accrued since the last call, saturating at `capacity`.
    ///
    /// Refilling lazily on demand avoids a background timer: elapsed wall-clock
    /// time is converted into newly available tokens.
    fn refill(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        self.last_refill = now;
    }

    /// Try to admit one request. Returns true and consumes a token when one is
    /// available; returns false (rate limited) when the bucket is empty.
    pub fn try_admit(&mut self) -> bool {
        self.refill(Instant::now());
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// How long the caller must wait before a token becomes available.
    ///
    /// Returns zero when a request could be admitted immediately, otherwise the
    /// time until the bucket accrues one whole token.
    pub fn time_until_token(&self) -> Duration {
        if self.tokens >= 1.0 || self.refill_per_sec <= 0.0 {
            return Duration::ZERO;
        }
        let deficit = 1.0 - self.tokens;
        Duration::from_secs_f64(deficit / self.refill_per_sec)
    }
}
