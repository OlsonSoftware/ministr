//! Wire shape of `ministr.ai/atlas/manifest.json`.
//!
//! F2.6 v0 ships the manifest as a static mirror of [`ATLAS_SEED_REPOS`].
//! F4.1 swaps in the live-blob-set view (each entry carries the
//! `last_indexed_commit` + `last_indexed_at` actually written by the
//! cron). The struct shape stays the same — only the source of truth
//! changes.
//!
//! Stability matters: third parties scrape this manifest to know
//! what's indexable. Adding a field is fine; renaming or removing is
//! a breaking change. Treat the JSON as a public API.

use serde::{Deserialize, Serialize};

use crate::repos::{SeedRepo, ATLAS_SEED_REPOS};

/// One row of the public manifest. Built from a [`SeedRepo`] plus the
/// cron's runtime fields once F4.2 wires the live view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// URL-safe slug. Matches [`SeedRepo::slug`].
    pub slug: String,
    /// HTTPS clone URL.
    pub clone_url: String,
    /// SPDX license identifier.
    pub spdx: String,
    /// One-line summary.
    pub description: String,
    /// Last commit hash indexed. `None` for the F2.6 v0 mirror;
    /// populated once F4.2 ships the cron.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_indexed_commit: Option<String>,
    /// Last successful index timestamp (RFC 3339). `None` for the
    /// F2.6 v0 mirror.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_indexed_at: Option<String>,
    /// `true` when the repo currently appears on the
    /// `ministr.ai/atlas/opt-out` registry. The cron skips opted-out
    /// repos; the manifest carries the flag so scrapers can confirm.
    #[serde(default)]
    pub opted_out: bool,
}

impl From<&SeedRepo> for ManifestEntry {
    fn from(s: &SeedRepo) -> Self {
        Self {
            slug: s.slug.to_owned(),
            clone_url: s.clone_url.to_owned(),
            spdx: s.spdx.to_owned(),
            description: s.description.to_owned(),
            last_indexed_commit: None,
            last_indexed_at: None,
            opted_out: false,
        }
    }
}

/// The whole manifest. Wraps the entry list so the top-level JSON
/// shape leaves room for future metadata (cron version, blob count,
/// etc.) without breaking scrapers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestSnapshot {
    /// Schema version — bump when adding required fields, never when
    /// adding optional ones.
    pub schema_version: u32,
    /// Total entries in this snapshot. Mirrors `entries.len()` so
    /// scrapers can sanity-check.
    pub count: usize,
    /// One entry per Atlas repo.
    pub entries: Vec<ManifestEntry>,
}

impl ManifestSnapshot {
    /// Current schema version. Bumped whenever a field becomes
    /// required.
    pub const SCHEMA_VERSION: u32 = 1;

    /// Build the F2.6 v0 manifest mirror from [`ATLAS_SEED_REPOS`].
    /// No runtime fields populated; the cron rewrites the same shape
    /// with `last_indexed_*` once F4.2 ships.
    #[must_use]
    pub fn from_seed_list() -> Self {
        let entries: Vec<ManifestEntry> =
            ATLAS_SEED_REPOS.iter().map(ManifestEntry::from).collect();
        Self {
            schema_version: Self::SCHEMA_VERSION,
            count: entries.len(),
            entries,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_list_manifest_count_matches_entries() {
        let m = ManifestSnapshot::from_seed_list();
        assert_eq!(m.entries.len(), m.count);
        assert_eq!(m.entries.len(), 50);
    }

    #[test]
    fn manifest_serialises_omitting_none_fields() {
        let m = ManifestSnapshot::from_seed_list();
        let json = serde_json::to_string(&m).unwrap();
        // Optional fields with None values must NOT appear in JSON —
        // scraper tests rely on the compact shape.
        assert!(!json.contains("last_indexed_commit"));
        assert!(!json.contains("last_indexed_at"));
        // Required fields ARE present.
        assert!(json.contains("\"schema_version\":1"));
        assert!(json.contains("\"slug\":\"react\""));
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let m = ManifestSnapshot::from_seed_list();
        let json = serde_json::to_string(&m).unwrap();
        let back: ManifestSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }
}
