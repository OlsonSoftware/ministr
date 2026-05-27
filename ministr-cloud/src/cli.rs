//! F31.2b — `ClassicCloudMounter` implements the
//! [`ministr_api::CloudRouterMounter`] MIT seam.
//!
//! Owns the cloud-mode side of `cmd_serve_http`: validating the
//! Enterprise license, opening the Postgres pool, running migrations
//! and audit-partition seeding, building the blob backend, mounting
//! every cloud axum router, and wiring `Arc<dyn AdapterTrait>` cloud
//! sinks into the returned `CloudMountOutput` for the MIT serve to
//! splice into its daemon / OAuth / server state.
//!
//! Constructed by the `ministr-cloud-tools` proprietary binary and
//! passed to `ministr_cli::commands::cmd_serve_http` as
//! `Some(&mounter)`. The public `ministr` binary passes `None` and
//! never depends on this crate at compile time.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;

use ministr_api::{
    ApiError, CloudAdminAdapters, CloudDaemonAdapters, CloudMountInput, CloudMountOutput,
    CloudOAuthAdapters, CloudRouterMounter, CloudServerAdapters, RevocationHandle,
};

use crate::revocation_fetch::RevocationShutdownHandle;

/// The classic (today-default) cloud overlay used by the
/// `ministr-cloud-tools serve` subcommand.
///
/// Encapsulates the entire boot-time cloud-mode wiring previously
/// inlined in `cmd_serve_http`. See [`mount_cloud_routes`] for the
/// step-by-step body and [`CloudRouterMounter`] for the trait contract.
#[derive(Debug, Default)]
pub struct ClassicCloudMounter {
    _private: (),
}

impl ClassicCloudMounter {
    /// Build a fresh mounter. The mounter owns no state up front; every
    /// cloud resource is opened lazily inside [`setup`].
    #[must_use]
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl CloudRouterMounter for ClassicCloudMounter {
    fn setup<'a>(
        &'a self,
        input: &'a CloudMountInput,
    ) -> Pin<Box<dyn Future<Output = Result<CloudMountOutput, ApiError>> + Send + 'a>> {
        Box::pin(mount_cloud_routes(input))
    }
}

/// Implements the classic cloud overlay. Mirrors the cloud branch
/// previously inlined in `ministr_cli::commands::cmd_serve_http`.
///
/// # Errors
///
/// Returns an [`ApiError`] when license validation refuses boot, the
/// Postgres pool fails to open, migrations fail to apply, or any other
/// cloud resource refuses to come up.
#[allow(clippy::unused_async)] // stub: real impl in F31.2b-ii becomes async
pub async fn mount_cloud_routes(
    _input: &CloudMountInput,
) -> Result<CloudMountOutput, ApiError> {
    // F31.2b — initial stub. The real cloud-overlay body lands as the
    // matching refactor in ministr-cli::cmd_serve_http rips the inline
    // cloud branch and lifts it into here. Until then this stub returns
    // an empty CloudMountOutput which is functionally identical to
    // "no cloud overlay" — useful as the trait contract test fixture.
    Ok(CloudMountOutput {
        router: Router::new(),
        daemon_adapters: CloudDaemonAdapters::default(),
        server_adapters: CloudServerAdapters::default(),
        oauth_adapters: CloudOAuthAdapters::default(),
        admin_adapters: CloudAdminAdapters::default(),
        shutdown: None,
    })
}

impl RevocationHandle for RevocationShutdownHandle {
    fn shutdown_future(&self) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.shutdown.notified().await;
        })
    }

    fn is_revoked(&self) -> bool {
        self.revoked.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl ClassicCloudMounter {
    /// Helper for callers that want the revocation handle as the
    /// MIT-seam trait object (`Arc<dyn RevocationHandle>`).
    #[must_use]
    pub fn revocation_handle_dyn(handle: RevocationShutdownHandle) -> Arc<dyn RevocationHandle> {
        Arc::new(handle)
    }
}
