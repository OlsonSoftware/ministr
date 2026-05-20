//! License filtering for Atlas inclusion.
//!
//! [`LicenseFilter`] is the DIP seam — the F2.6 pilot ships
//! [`SpdxFilter`] which accepts only the permissive whitelist; F4.1
//! adds a `CopyleftAwareFilter` once G.1 closes that admits AGPL /
//! GPL / LGPL under counsel's opinion. The pilot indexer asks the
//! filter for every repo before cloning, so a repo whose license
//! changed between weekly cron runs is dropped automatically.

/// The set of permissive SPDX identifiers Atlas v0 accepts.
///
/// Sourced from the OSI permissive list intersected with what's
/// realistic on a curated 50-repo pilot. Anything else routes through
/// G.1 counsel before joining.
pub const PERMISSIVE_SPDX: &[&str] = &[
    "MIT",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "MPL-2.0",
    "ISC",
    "Unlicense",
    "0BSD",
    "PostgreSQL", // OSI-approved permissive (BSD-style)
    "blessing",   // SQLite's public-domain dedication
];

/// Decide whether a repo's SPDX identifier qualifies for Atlas
/// inclusion. Implementations MUST be deterministic — the same input
/// yields the same outcome on every call, so the weekly cron is
/// reproducible.
pub trait LicenseFilter: Send + Sync + std::fmt::Debug {
    /// Returns `true` when the repo with `spdx` SPDX identifier should
    /// be indexed and served from Atlas.
    fn admits(&self, spdx: &str) -> bool;
}

/// Permissive-only filter (F2.6 v0). Accepts the whitelist verbatim;
/// rejects anything else. Used to flag the Grafana row (AGPL) at
/// indexer time so G.1 counsel can clear it later.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpdxFilter;

impl SpdxFilter {
    /// Construct the filter. `const fn` so the cloud router can hold
    /// one in a `const` slot.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl LicenseFilter for SpdxFilter {
    fn admits(&self, spdx: &str) -> bool {
        // Case-insensitive compare so a `apache-2.0` typo doesn't
        // silently drop the entry. Pre-strip leading/trailing
        // whitespace because the manifest data is hand-curated.
        let needle = spdx.trim();
        PERMISSIVE_SPDX.iter().any(|allowed| allowed.eq_ignore_ascii_case(needle))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissive_whitelist_admits_mit_apache_bsd() {
        let f = SpdxFilter::new();
        assert!(f.admits("MIT"));
        assert!(f.admits("Apache-2.0"));
        assert!(f.admits("BSD-3-Clause"));
        assert!(f.admits("BSD-2-Clause"));
        assert!(f.admits("MPL-2.0"));
        assert!(f.admits("ISC"));
        assert!(f.admits("Unlicense"));
        assert!(f.admits("0BSD"));
    }

    #[test]
    fn permissive_whitelist_rejects_copyleft() {
        let f = SpdxFilter::new();
        assert!(!f.admits("GPL-3.0-only"));
        assert!(!f.admits("AGPL-3.0-only"));
        assert!(!f.admits("LGPL-2.1-only"));
    }

    #[test]
    fn permissive_whitelist_is_case_insensitive() {
        let f = SpdxFilter::new();
        assert!(f.admits("mit"));
        assert!(f.admits("apache-2.0"));
        assert!(f.admits("BSD-3-clause"));
    }

    #[test]
    fn empty_and_unknown_identifiers_are_rejected() {
        let f = SpdxFilter::new();
        assert!(!f.admits(""));
        assert!(!f.admits("SomeWeirdLicense"));
    }

    #[test]
    fn trait_is_dyn_compatible() {
        let f: std::sync::Arc<dyn LicenseFilter> = std::sync::Arc::new(SpdxFilter);
        assert!(f.admits("MIT"));
    }
}
