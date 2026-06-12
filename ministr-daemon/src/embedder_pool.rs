//! Resident per-model embedder + embedding-service pool
//! (parity-seam-registry-routing).
//!
//! The daemon honors a corpus's per-corpus embedding model (`.ministr.toml`
//! `[corpus] model`). Each distinct resolved model needs its own [`Embedder`]
//! and its own [`EmbeddingService`] — the dedicated, GPU-owning batch queue.
//!
//! ## ADR 0001 D1, refined
//!
//! ADR 0001 D1 said the daemon serves every corpus through ONE shared
//! `EmbeddingService`. That holds exactly when every corpus uses the default
//! model. To honor a per-corpus model the daemon now keeps one
//! `EmbeddingService` **per distinct resolved model**, built and cached on
//! first use, bounded by the handful of models a deployment actually configures
//! (usually one). Embedding models are small — `MiniLM` ~90 MB, `jina-code`
//! ~320 MB — so a *resident* pool is cheaper and simpler than GPU hot-swapping.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use ministr_core::embedding::Embedder;

/// Builds a raw [`Embedder`] for a model name — returning it together with its
/// backend-qualified embedding-cache key (e.g. `"all-MiniLM-L6-v2:onnx"`) — or
/// returns an error message. Injectable so the pool is unit-testable without
/// loading a real model.
pub(crate) type EmbedderBuilder =
    Arc<dyn Fn(&str) -> Result<(Arc<dyn Embedder>, String), String> + Send + Sync>;

/// A model's embedder plus its backend-qualified embedding-cache key. Cheap to
/// clone (`Arc` + `String`).
///
/// Note (ingest-embed-cache-wiring): the pool used to also carry a shared
/// per-model [`EmbeddingService`]. Ingest now spawns a fresh per-corpus
/// service around the per-corpus `CachedEmbedder` (the cache connection is
/// per-corpus), and nothing else consumed the pooled one, so it was removed —
/// no idle embed thread per model.
#[derive(Clone)]
pub(crate) struct PooledEmbedder {
    pub embedder: Arc<dyn Embedder>,
    /// Backend-qualified key for the per-corpus embedding cache
    /// (`{model}{backend_suffix}`, same scheme as the CLI's
    /// `cache_model_key`). The suffix matters: candle and onnx produce
    /// different vector spaces, and the key keeps them apart in one cache.
    pub cache_model_key: String,
}

/// Resident cache of `model name -> (embedder, service)`, built on first use.
pub(crate) struct EmbedderPool {
    entries: Mutex<HashMap<String, PooledEmbedder>>,
    build: EmbedderBuilder,
}

impl EmbedderPool {
    /// A pool backed by `build`. Empty until [`seed`](Self::seed) or
    /// [`get`](Self::get) populate it.
    pub(crate) fn new(build: EmbedderBuilder) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            build,
        }
    }

    /// The production pool: builds embedders via
    /// [`ministr_core::embedding::create_embedder`] under `data_dir`.
    pub(crate) fn with_data_dir(data_dir: PathBuf) -> Self {
        Self::new(Arc::new(move |model: &str| {
            ministr_core::embedding::create_embedder(model, &data_dir)
                .map(|(embedder, info)| {
                    let key = format!("{model}{}", info.cache_key_suffix());
                    (embedder, key)
                })
                .map_err(|e| e.to_string())
        }))
    }

    /// Insert an already-built embedder under `model`. Used to seed the
    /// daemon's default model with the Arc constructed at boot, so the
    /// default path never rebuilds. `cache_model_key` is the backend-qualified
    /// embedding-cache key for that boot-built embedder.
    pub(crate) fn seed(&self, model: &str, embedder: Arc<dyn Embedder>, cache_model_key: String) {
        self.entries
            .lock()
            .expect("embedder pool mutex poisoned")
            .insert(
                model.to_string(),
                PooledEmbedder {
                    embedder,
                    cache_model_key,
                },
            );
    }

    /// The cached entry for `model`, building (and caching) it on first use.
    ///
    /// # Errors
    ///
    /// Returns the builder's error message when the model can't be built (e.g.
    /// an unknown or uninstalled model).
    pub(crate) fn get(&self, model: &str) -> Result<PooledEmbedder, String> {
        if let Some(pooled) = self
            .entries
            .lock()
            .expect("embedder pool mutex poisoned")
            .get(model)
        {
            return Ok(pooled.clone());
        }
        // Build OUTSIDE the lock — model construction loads weights (slow) and
        // must not block cached lookups of other models. A rare concurrent
        // double-build of the same fresh model just discards one result; both
        // are functionally identical weights, so correctness is unaffected.
        let (embedder, cache_model_key) = (self.build)(model)?;
        let pooled = PooledEmbedder {
            embedder,
            cache_model_key,
        };
        self.entries
            .lock()
            .expect("embedder pool mutex poisoned")
            .insert(model.to_string(), pooled.clone());
        Ok(pooled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use ministr_core::error::IndexError;

    /// A trivial embedder so tests never load a real model.
    struct StubEmbedder {
        dim: usize,
    }

    impl Embedder for StubEmbedder {
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, IndexError> {
            Ok(texts.iter().map(|_| vec![0.0; self.dim]).collect())
        }
        fn dimension(&self) -> usize {
            self.dim
        }
    }

    fn counting_pool(counter: &Arc<AtomicUsize>) -> EmbedderPool {
        let c = Arc::clone(counter);
        EmbedderPool::new(Arc::new(move |model: &str| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok((
                Arc::new(StubEmbedder { dim: 384 }) as Arc<dyn Embedder>,
                format!("{model}:stub"),
            ))
        }))
    }

    #[test]
    fn get_builds_once_per_model_and_caches() {
        let builds = Arc::new(AtomicUsize::new(0));
        let pool = counting_pool(&builds);

        let a = pool.get("model-a").expect("build a");
        let b = pool.get("model-a").expect("cached a");
        // Second get for the same model is a cache hit — no rebuild, same Arc.
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&a.embedder, &b.embedder));
        // The builder's backend-qualified cache key rides along.
        assert_eq!(a.cache_model_key, "model-a:stub");

        // A distinct model builds a second entry.
        let _c = pool.get("model-b").expect("build b");
        assert_eq!(builds.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn seeded_model_is_returned_without_building() {
        let pool = EmbedderPool::new(Arc::new(|_| {
            panic!("seeded model must not trigger a build");
        }));
        let embedder: Arc<dyn Embedder> = Arc::new(StubEmbedder { dim: 384 });
        pool.seed("default", Arc::clone(&embedder), "default:stub".to_string());

        let got = pool.get("default").expect("seeded entry");
        assert!(Arc::ptr_eq(&got.embedder, &embedder));
        assert_eq!(got.cache_model_key, "default:stub");
    }
}
