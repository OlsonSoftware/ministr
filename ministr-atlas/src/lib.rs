//! ministr-atlas — proprietary curated repo network (F2.6 + F4).
//!
//! Atlas is the moat made code: a small, curated set of high-frequency
//! OSS repos that the cloud pre-indexes on a weekly cron and serves at
//! `GET /atlas/{slug}/{survey|symbols|references|...}` to every paid
//! tier. The local stack does not depend on this crate — Atlas is
//! cloud-only.
//!
//! # Phases
//!
//! - **F2.6 v0** — 50-repo pilot. Seed list lives in
//!   [`repos::ATLAS_SEED_REPOS`]; the read routes are stubbed (return
//!   503 with a "not yet indexed" payload) because the weekly cron
//!   that produces the blobs ships in F4.2. The public manifest
//!   reflects the seed list verbatim.
//! - **F4.1** — 5K-repo curated expansion. Requires G.1 legal opinion;
//!   adds the copyleft handling branch and the public opt-out
//!   registry.
//! - **F4.2** — production weekly cron writing HNSW blobs to Azure.
//! - **F4.3** — Team annotation overlays on Atlas claims.
//! - **F4.4** — In-VPC Atlas mirror for Enterprise.
//!
//! # SOLID layering
//!
//! Each module owns ONE concern; cross-cutting traits keep the
//! collaborators substitutable:
//!
//! ```text
//!   repos     — SeedRepo data model + ATLAS_SEED_REPOS curated list
//!   license   — LicenseFilter trait + SpdxFilter impl (DIP for the
//!               copyleft branch that lands in F4.1)
//!   optout    — OptOutRegistry trait + InMemoryRegistry impl
//!   manifest  — Manifest struct (the wire shape of
//!               /atlas/manifest.json)
//!   routes    — axum Router for the cloud surface
//!   indexer   — Worker entrypoint the cron Job invokes;
//!               re-index pipeline composed of `Cloner` +
//!               `Indexer` + `BlobWriter` traits so each step is
//!               independently testable
//! ```

#![deny(unsafe_code)]

pub mod indexer;
pub mod license;
pub mod manifest;
pub mod optout;
pub mod repos;
pub mod routes;

pub use indexer::{reindex_once, BlobWriter, Cloner, IndexerStep, ReindexError, ReindexOutcome};
pub use license::{LicenseFilter, SpdxFilter};
pub use manifest::{ManifestEntry, ManifestSnapshot};
pub use optout::{InMemoryRegistry, OptOutRegistry};
pub use repos::{SeedRepo, ATLAS_SEED_REPOS};
pub use routes::{atlas_routes, AtlasState};
