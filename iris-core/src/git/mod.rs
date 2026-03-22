//! Git repository cloning with sparse checkout support.
//!
//! Provides [`GitFetcher`] for cloning remote repositories into a local cache
//! directory (`~/.iris/remote/<repo-hash>/`) using shallow, filtered clones
//! with optional sparse checkout. Clone metadata is tracked in TOML files
//! to enable cache reuse and staleness detection.

pub mod fetcher;

pub use fetcher::{CloneMetadata, CloneResult, GitFetcher, GitFetcherConfig, GitStalenessResult};
