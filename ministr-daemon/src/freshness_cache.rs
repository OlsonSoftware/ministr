//! Short-TTL freshness result cache (gui-rw-freshness-summary).
//!
//! The freshness sweep re-hashes the whole working tree (deliberately —
//! the tree-never-lies invariant forbids stat/mtime shortcuts), so it's
//! CPU-heavy. The desktop app polls it from two screens (Home every 5s
//! per corpus, Mirror every 4s), which would multiply full sweeps. This
//! cache DEDUPES those overlapping polls: within the TTL window every
//! caller gets the same hash-verified result, computed once. It never
//! weakens the invariant — a cached entry IS a full hash sweep, merely
//! a couple of seconds old, which a 4-5s polling UI already tolerates.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ministr_api::corpus::FreshnessResponse;

/// Cache TTL — below the fastest poll cadence (4s) so each screen still
/// observes fresh-enough data, above typical request overlap.
pub const FRESHNESS_TTL: Duration = Duration::from_millis(2_500);

#[derive(Default)]
pub struct FreshnessCache {
    entries: Mutex<HashMap<String, (Instant, FreshnessResponse)>>,
}

impl FreshnessCache {
    /// A cached sweep result for `corpus_id` if one exists within `ttl`.
    pub fn get(&self, corpus_id: &str, ttl: Duration) -> Option<FreshnessResponse> {
        let entries = self.entries.lock().ok()?;
        entries
            .get(corpus_id)
            .filter(|(at, _)| at.elapsed() < ttl)
            .map(|(_, resp)| resp.clone())
    }

    /// Store a freshly computed sweep result.
    pub fn put(&self, corpus_id: &str, resp: FreshnessResponse) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(corpus_id.to_owned(), (Instant::now(), resp));
        }
    }

    /// Drop a corpus's entry (e.g. after a reindex is triggered, so the
    /// next poll reflects the new state without waiting out the TTL).
    pub fn invalidate(&self, corpus_id: &str) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.remove(corpus_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resp(indexing: bool) -> FreshnessResponse {
        FreshnessResponse {
            files: Vec::new(),
            indexing,
        }
    }

    #[test]
    fn hit_within_ttl_miss_after() {
        let cache = FreshnessCache::default();
        cache.put("c1", resp(false));
        assert!(cache.get("c1", Duration::from_secs(60)).is_some());
        assert!(cache.get("c1", Duration::ZERO).is_none());
    }

    #[test]
    fn corpora_are_isolated_and_invalidate_works() {
        let cache = FreshnessCache::default();
        cache.put("c1", resp(true));
        assert!(cache.get("other", Duration::from_secs(60)).is_none());
        cache.invalidate("c1");
        assert!(cache.get("c1", Duration::from_secs(60)).is_none());
    }
}
