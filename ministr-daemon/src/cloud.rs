//! Cloud bundle fetch — download and import remote `.ministr-index` bundles.
//!
//! Checks local cache staleness via `bundle_version`, fetches from URL,
//! imports into the corpus directory, and updates the cache entry.

use std::path::Path;

use ministr_core::bundle::{self, BundleCacheEntry, BundleManifest};
use ministr_core::config::CloudInclude;
use tracing::{info, warn};

/// Fetch and import cloud bundles, skipping those already cached and current.
///
/// Returns a list of `(corpus_id, manifest)` pairs for successfully imported bundles.
pub async fn fetch_cloud_bundles(
    cloud_includes: &[CloudInclude],
    data_dir: &Path,
) -> Vec<(String, BundleManifest)> {
    let mut imported = Vec::new();

    for include in cloud_includes {
        match fetch_one(include, data_dir).await {
            Ok(Some((id, manifest))) => {
                info!(url = %include.url, corpus_id = %id, "cloud bundle imported");
                imported.push((id, manifest));
            }
            Ok(None) => {
                info!(url = %include.url, "cloud bundle cache is current, skipped");
            }
            Err(e) => {
                warn!(url = %include.url, error = %e, "cloud bundle fetch failed");
            }
        }
    }

    imported
}

async fn fetch_one(
    include: &CloudInclude,
    data_dir: &Path,
) -> Result<Option<(String, BundleManifest)>, Box<dyn std::error::Error + Send + Sync>> {
    // Check local cache.
    if let Ok(Some(cached)) = bundle::load_cache_entry(data_dir, &include.url) {
        // If pinned and matching, skip.
        if let Some(ref pin) = include.pin_version
            && cached.manifest.bundle_version.as_deref() == Some(pin.as_str())
        {
            return Ok(None);
        }
    }

    // Fetch the bundle.
    let client = reqwest::Client::new();
    let resp = client.get(&include.url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()).into());
    }

    let bytes = resp.bytes().await?;

    // Write to temp file then import.
    let tmp_dir = tempfile::TempDir::new()?;
    let tmp_path = tmp_dir.path().join("bundle.ministr-index");
    tokio::fs::write(&tmp_path, &bytes).await?;

    let manifest = bundle::read_manifest(&tmp_path)?;
    let corpus_id = format!(
        "cloud-{}",
        &bundle::compute_bundle_version(&manifest.corpus_roots)[..8]
    );
    let corpus_dir = data_dir.join("corpora").join(&corpus_id);

    bundle::import_bundle(&tmp_path, &corpus_dir)?;

    // Update cache.
    let cache_entry = BundleCacheEntry {
        url: include.url.clone(),
        manifest: manifest.clone(),
        fetched_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };
    if let Err(e) = bundle::save_cache_entry(data_dir, &cache_entry) {
        warn!(error = %e, "failed to save bundle cache entry");
    }

    Ok(Some((corpus_id, manifest)))
}
