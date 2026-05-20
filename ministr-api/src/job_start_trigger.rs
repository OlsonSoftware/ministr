//! PHASE5 chunk 1 — fire-and-forget hook that the serve pod calls after
//! enqueuing an `indexer_jobs` row.
//!
//! Background. PHASE4 chunk 1 wired KEDA's `postgresql` scaler against
//! `indexer_jobs` polling every 5s. That's "event-driven" in the ACA
//! marketing sense but is still a poll, with KEDA-cycle latency between
//! enqueue and worker boot. PHASE5 chunk 1 retires the framing: the
//! serve pod calls Azure ARM directly (`POST .../jobs/{name}/start`)
//! right after the enqueue commits, latency drops from KEDA-cycle to
//! ~1-2s, and KEDA stays on a slow (5-min) poll as the safety net.
//!
//! # The seam
//!
//! Same shape as [`IndexJobSink`](crate::index_job_sink::IndexJobSink):
//! a trait in this MIT crate, a closed-source `AcaJobStartTrigger` impl
//! in `ministr-cloud` that owns the IMDS-token + reqwest plumbing. The
//! serve pod's [`crate::index_job_sink::IndexJobSink::create_pending`]
//! takes an optional trigger by composition and invokes it
//! fire-and-forget AFTER the enqueue commits — a trigger failure must
//! never roll the row back, because KEDA's safety-net poll picks the
//! row up within five minutes.
//!
//! # Why fire-and-forget specifically
//!
//! - The trigger is a best-effort latency optimisation, not the
//!   correctness path. Worker boot still works without it.
//! - The serve pod's HTTP handler should not block on Azure ARM
//!   round-trips. ARM 5xx and timeouts happen; the handler returns
//!   `{job_id}` to the client immediately.
//! - On a successful start, ACA boots the indexer in seconds. On a
//!   failed start, the KEDA safety net (5-min cycle) catches the row.
//!
//! Self-hosted single-user serve leaves the field `None`; no Azure
//! plumbing runs and the open-core stack is unaffected.

use std::future::Future;
use std::pin::Pin;

/// Errors surfaced by [`JobStartTrigger`] implementations.
#[derive(Debug, thiserror::Error)]
pub enum JobStartError {
    /// IMDS or ARM HTTP transport failed (network, DNS, timeout). The
    /// trigger logs and returns this so callers can record the miss.
    #[error("http: {0}")]
    Http(String),
    /// IMDS refused to mint a token — usually a missing MI on the pod
    /// or a configuration mismatch. Production should never hit this
    /// once the role assignment lands; surfaces during local-dev runs
    /// where the pod's MI chain falls back to the developer creds.
    #[error("imds: {0}")]
    Imds(String),
    /// ARM returned a non-2xx response. Body excerpt kept short for log
    /// triage; never echoed to clients.
    #[error("arm: {status} {body}")]
    Arm {
        /// HTTP status code returned by ARM (e.g. 403, 404, 429).
        status: u16,
        /// Truncated body — usually JSON `{ "error": { "code", "message" } }`.
        body: String,
    },
    /// Required configuration (subscription / resource group / job name)
    /// missing or empty. Indicates a Pulumi wiring bug; not retryable.
    #[error("config: {0}")]
    Config(&'static str),
}

/// Boxed future returned by [`JobStartTrigger::start_job_for`]. Lifetime
/// ties the future to the borrow of `&self` and the borrowed corpus id,
/// matching the [`IndexJobSink`](crate::index_job_sink::IndexJobSink)
/// convention so impls can capture references.
pub type JobStartFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), JobStartError>> + Send + 'a>>;

/// Cloud-mode fire-and-forget hook that asks ACA to start a replica of
/// the indexer Job. Called by [`crate::index_job_sink::IndexJobSink`]
/// implementations after the `indexer_jobs` INSERT commits.
///
/// Implementations MUST be safe to call concurrently — the serve pod
/// hits this from every enqueue handler, fire-and-forget under
/// `tokio::spawn`. Implementations SHOULD complete inside a few seconds
/// or surface a [`JobStartError::Http`]; the caller does not block on
/// the future but extended hangs leak tasks.
pub trait JobStartTrigger: Send + Sync + std::fmt::Debug {
    /// Ask the platform to start one replica of the indexer Job. The
    /// `corpus_id` is informational — used for log correlation only;
    /// ACA's `/start` endpoint takes no body argument identifying which
    /// row to claim, the worker's `claim_next` does that.
    ///
    /// Returns `Ok(())` when ARM accepts the start request (typically
    /// 200/202). Returns `Err` for any IMDS, transport, or non-2xx ARM
    /// outcome. The caller never propagates this Err to the user; it's
    /// logged and the KEDA safety net is allowed to pick the row up.
    fn start_job_for<'a>(&'a self, corpus_id: &'a str) -> JobStartFuture<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, Default)]
    struct MockTrigger {
        calls: AtomicUsize,
        last_corpus: Mutex<Option<String>>,
    }

    impl JobStartTrigger for MockTrigger {
        fn start_job_for<'a>(&'a self, corpus_id: &'a str) -> JobStartFuture<'a> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                *self.last_corpus.lock().unwrap() = Some(corpus_id.to_string());
                Ok(())
            })
        }
    }

    #[tokio::test]
    async fn trait_is_dyn_compatible_and_round_trips() {
        let trig: std::sync::Arc<dyn JobStartTrigger> =
            std::sync::Arc::new(MockTrigger::default());
        trig.start_job_for("c1").await.unwrap();
        // Downcast just for assertion clarity in the test; production
        // callers only see the trait object.
        let mock = trig as std::sync::Arc<dyn JobStartTrigger>;
        // We can't downcast through dyn JobStartTrigger; instead build
        // a fresh MockTrigger and exercise it directly to keep the
        // counter assertion meaningful.
        let _ = mock;
        let direct = MockTrigger::default();
        direct.start_job_for("c2").await.unwrap();
        assert_eq!(direct.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            direct.last_corpus.lock().unwrap().as_deref(),
            Some("c2"),
        );
    }

    #[test]
    fn error_renders_arm_status_and_body() {
        let err = JobStartError::Arm {
            status: 403,
            body: "Forbidden".into(),
        };
        assert_eq!(err.to_string(), "arm: 403 Forbidden");
    }

    #[test]
    fn config_error_is_static_str() {
        let err = JobStartError::Config("MINISTR_ACA_SUBSCRIPTION_ID missing");
        assert_eq!(
            err.to_string(),
            "config: MINISTR_ACA_SUBSCRIPTION_ID missing",
        );
    }
}
