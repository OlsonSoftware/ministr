//! Crate-private time helpers shared by `auth` and `admin`.
//!
//! Lives outside either submodule because both depend on it and neither
//! owns the concept of "current epoch seconds."

use std::time::{SystemTime, UNIX_EPOCH};

/// Current epoch timestamp in seconds. Saturates to 0 if the clock is
/// somehow before the Unix epoch.
pub(crate) fn epoch_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
