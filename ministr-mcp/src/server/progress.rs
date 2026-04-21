//! Background notification tasks for ingestion progress and resource subscriptions.
//!
//! These long-running async tasks push MCP notifications to connected clients:
//! - [`run_ingestion_progress_notifier`] polls `IngestionProgress` and sends
//!   `notifications/progress` updates during corpus indexing.
//! - [`run_subscription_notifier`] listens for coherence change events and sends
//!   `notifications/resources/updated` for subscribed resource URIs.

use std::collections::HashSet;
use std::sync::Arc;

use rmcp::RoleServer;
use rmcp::model::{
    NumberOrString, ProgressNotificationParam, ProgressToken, ResourceUpdatedNotificationParam,
};
use rmcp::service::Peer;
use tokio::sync::Mutex;

use ministr_core::ingestion::IngestionProgress;

use super::helpers::INGESTION_PROGRESS_TOKEN;

/// Poll `IngestionProgress` and push MCP `notifications/progress` to the client.
///
/// Runs until ingestion completes or the peer channel closes. Polls every 2
/// seconds to avoid flooding the client with messages.
pub(crate) async fn run_ingestion_progress_notifier(
    progress: Arc<IngestionProgress>,
    peer_lock: Arc<Mutex<Option<Peer<RoleServer>>>>,
) {
    // Wait briefly for ingestion to start (it may not have begun yet).
    let mut wait_count = 0;
    while progress.status() == 0 {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        wait_count += 1;
        // Give up after 30 seconds if ingestion never starts.
        if wait_count > 60 {
            tracing::debug!("ingestion never started, progress notifier exiting");
            return;
        }
    }

    let token = ProgressToken(NumberOrString::String(INGESTION_PROGRESS_TOKEN.into()));
    let mut last_done = 0;

    loop {
        if !progress.is_running() {
            // Send one final notification with done == total.
            let total = progress.files_total();
            let peer = peer_lock.lock().await;
            if let Some(ref p) = *peer {
                #[allow(clippy::cast_precision_loss)]
                let _ = p
                    .notify_progress(ProgressNotificationParam {
                        progress_token: token.clone(),
                        progress: total as f64,
                        total: Some(total as f64),
                        message: Some("Corpus ready".to_string()),
                    })
                    .await;
            }
            tracing::info!("ingestion complete, progress notifier exiting");
            break;
        }

        let done = progress.files_done();
        let total = progress.files_total();
        let phase = progress.phase();

        // Only send if progress actually changed.
        if done != last_done {
            last_done = done;
            let phase_str = phase.as_str();
            let msg = match phase {
                ministr_core::ingestion::IngestionPhase::Discovering => {
                    "Discovering files…".to_string()
                }
                ministr_core::ingestion::IngestionPhase::Embedding => {
                    let ed = progress.embeddings_done();
                    let et = progress.embeddings_total();
                    format!("Embedding ({ed}/{et}) · {done}/{total} files parsed")
                }
                ministr_core::ingestion::IngestionPhase::Finalizing => {
                    format!("Finalizing · {done}/{total} files parsed")
                }
                _ => format!("{phase_str}: {done}/{total} files"),
            };
            let peer = peer_lock.lock().await;
            if let Some(ref p) = *peer {
                #[allow(clippy::cast_precision_loss)]
                if p.notify_progress(ProgressNotificationParam {
                    progress_token: token.clone(),
                    progress: done as f64,
                    total: Some(total as f64),
                    message: Some(msg.clone()),
                })
                .await
                .is_err()
                {
                    tracing::debug!("peer channel closed, progress notifier exiting");
                    break;
                }
            } else {
                tracing::debug!("no peer available, progress notifier exiting");
                break;
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}

/// Listen for coherence change events and push `notifications/resources/updated`
/// to the MCP client for any subscribed resource URIs.
///
/// Currently only `ministr://status` supports subscriptions — any coherence event
/// (file change → section invalidation) triggers an update notification for it.
/// Runs until the coherence sender is dropped or the peer channel closes.
pub(crate) async fn run_subscription_notifier(
    mut coherence_rx: tokio::sync::mpsc::UnboundedReceiver<Vec<String>>,
    peer_lock: Arc<Mutex<Option<Peer<RoleServer>>>>,
    subscriptions: Arc<Mutex<HashSet<String>>>,
) {
    tracing::info!("resource subscription notifier started");

    while let Some(affected_sections) = coherence_rx.recv().await {
        let subs = subscriptions.lock().await;
        if subs.is_empty() {
            continue;
        }

        // Any coherence event affects ministr://status (it includes session state).
        if subs.contains("ministr://status") {
            let peer = peer_lock.lock().await;
            if let Some(ref p) = *peer {
                tracing::debug!(
                    affected_sections = affected_sections.len(),
                    "pushing resource update notification for ministr://status"
                );
                if p.notify_resource_updated(ResourceUpdatedNotificationParam {
                    uri: "ministr://status".to_string(),
                })
                .await
                .is_err()
                {
                    tracing::debug!("peer channel closed, subscription notifier exiting");
                    break;
                }
            } else {
                tracing::debug!("no peer available, subscription notifier exiting");
                break;
            }
        }
    }

    tracing::info!("resource subscription notifier stopped");
}
