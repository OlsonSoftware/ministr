//! Tauri commands for the ministr Cloud connection (mcp.ministr.ai).
//!
//! Scope (v1): manage the user's saved cloud endpoint + bearer token,
//! ping the remote `/healthz` to confirm liveness, and surface metrics
//! the Settings panel can render. The OAuth deep-link flow and the SSE
//! indexer-events bridge are follow-up iterations; this file is the
//! seam they slot into.
//!
//! SRP: lives in its own module so the existing `commands.rs` (local
//! daemon control) stays focused.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::{CommandError, ErrorKind};

const CLOUD_CONFIG_FILENAME: &str = "cloud.json";
const HEALTH_PROBE_TIMEOUT_SECS: u64 = 5;

/// Persisted state for the cloud connection. Lives on disk as JSON at
/// `<data-dir>/cloud.json`. Token storage in v1 is plain file (mode
/// 600 on Unix); migrating to the OS keychain is a v2 hardening step.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CloudConfig {
    /// Base URL, e.g. `https://mcp.ministr.ai`. Empty when not configured.
    #[serde(default)]
    pub endpoint: String,
    /// Bearer token issued by the remote OAuth flow. Empty when not
    /// authenticated. Treat as a secret.
    #[serde(default)]
    pub bearer_token: String,
}

impl CloudConfig {
    fn is_configured(&self) -> bool {
        !self.endpoint.trim().is_empty()
    }

    fn is_authenticated(&self) -> bool {
        !self.bearer_token.trim().is_empty()
    }
}

/// Snapshot returned to the UI on every `cloud_status` call.
#[derive(Debug, Clone, Serialize)]
pub struct CloudStatus {
    pub configured: bool,
    pub authenticated: bool,
    pub endpoint: String,
    pub last_health_ok: Option<bool>,
    pub last_health_latency_ms: Option<u64>,
    pub last_health_message: Option<String>,
}

/// `/healthz` response shape mirrored from `ministr-mcp/src/admin/handlers.rs`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CloudHealth {
    pub status: String,
    #[serde(default)]
    pub corpus_count: u64,
    #[serde(default)]
    pub version: String,
    /// Filled in by the command, not the server.
    #[serde(default)]
    pub latency_ms: u64,
}

// ── Disk helpers ───────────────────────────────────────────────────────────

fn cloud_config_path() -> PathBuf {
    ministr_api::daemon_data_dir().join(CLOUD_CONFIG_FILENAME)
}

fn load_config() -> CloudConfig {
    let path = cloud_config_path();
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            warn!(error = %e, path = %path.display(), "cloud.json malformed; starting fresh");
            CloudConfig::default()
        }),
        Err(_) => CloudConfig::default(),
    }
}

fn save_config(cfg: &CloudConfig) -> Result<(), CommandError> {
    let path = cloud_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            CommandError::new(
                ErrorKind::Io,
                format!("create cloud config dir {}: {e}", parent.display()),
            )
        })?;
    }
    let json = serde_json::to_string_pretty(cfg).map_err(|e| {
        CommandError::new(ErrorKind::Internal, format!("serialise cloud config: {e}"))
    })?;
    std::fs::write(&path, json).map_err(|e| {
        CommandError::new(
            ErrorKind::Io,
            format!("write cloud config {}: {e}", path.display()),
        )
    })?;
    set_owner_read_write(&path);
    debug!(path = %path.display(), "saved cloud config");
    Ok(())
}

#[cfg(unix)]
fn set_owner_read_write(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    if let Ok(meta) = std::fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn set_owner_read_write(_path: &std::path::Path) {}

// ── Commands ───────────────────────────────────────────────────────────────

/// Read the current cloud connection state. Pure file read — never
/// touches the network. The UI uses this on mount to render the initial
/// panel state; call `cloud_health_check` to actually probe the endpoint.
#[tauri::command]
pub async fn cloud_status() -> Result<CloudStatus, CommandError> {
    let cfg = load_config();
    Ok(CloudStatus {
        configured: cfg.is_configured(),
        authenticated: cfg.is_authenticated(),
        endpoint: cfg.endpoint,
        last_health_ok: None,
        last_health_latency_ms: None,
        last_health_message: None,
    })
}

/// Save the cloud endpoint URL. Empty string clears it. Trailing slashes
/// are normalised away so subsequent URL joins stay clean.
#[tauri::command]
pub async fn cloud_set_endpoint(endpoint: String) -> Result<(), CommandError> {
    let mut cfg = load_config();
    cfg.endpoint = endpoint.trim().trim_end_matches('/').to_string();
    save_config(&cfg)
}

/// Save a Bearer token issued by the remote OAuth flow. Empty clears it.
/// In v2 this moves to the OS keychain.
#[tauri::command]
pub async fn cloud_set_bearer_token(token: String) -> Result<(), CommandError> {
    let mut cfg = load_config();
    cfg.bearer_token = token.trim().to_string();
    save_config(&cfg)
}

/// Clear endpoint + token. Used by the "Disconnect" button.
#[tauri::command]
pub async fn cloud_disconnect() -> Result<(), CommandError> {
    save_config(&CloudConfig::default())
}

/// Probe `<endpoint>/healthz`. Records latency, returns the parsed body.
#[tauri::command]
pub async fn cloud_health_check() -> Result<CloudHealth, CommandError> {
    let cfg = load_config();
    if !cfg.is_configured() {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "no cloud endpoint configured",
        ));
    }
    let url = format!("{}/healthz", cfg.endpoint);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(HEALTH_PROBE_TIMEOUT_SECS))
        .build()
        .map_err(|e| CommandError::new(ErrorKind::Internal, format!("http client: {e}")))?;
    let started = std::time::Instant::now();
    let resp = client.get(&url).send().await.map_err(|e| {
        CommandError::new(ErrorKind::Io, format!("health probe to {url}: {e}"))
    })?;
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    if !resp.status().is_success() {
        return Err(CommandError::new(
            ErrorKind::Io,
            format!("health probe returned HTTP {}", resp.status()),
        ));
    }
    let mut health: CloudHealth = resp
        .json()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("parse /healthz: {e}")))?;
    health.latency_ms = latency_ms;
    Ok(health)
}

/// Build an `authed_client()` — returns the configured endpoint base URL,
/// bearer token, and a reqwest client. Rejects with structured errors when
/// the local cloud config is missing endpoint or token.
fn authed_client(timeout_secs: u64) -> Result<(reqwest::Client, String, String), CommandError> {
    let cfg = load_config();
    if !cfg.is_configured() {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "no cloud endpoint configured",
        ));
    }
    if !cfg.is_authenticated() {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "cloud connection has no bearer token (sign in first)",
        ));
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| CommandError::new(ErrorKind::Internal, format!("http client: {e}")))?;
    Ok((client, cfg.endpoint, cfg.bearer_token))
}

/// GET `/api/v1/corpora` — list all corpora the remote server has registered.
#[tauri::command]
pub async fn cloud_list_corpora() -> Result<serde_json::Value, CommandError> {
    let (client, endpoint, token) = authed_client(10)?;
    let url = format!("{endpoint}/api/v1/corpora");
    let resp = client
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("get {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(CommandError::new(
            ErrorKind::Io,
            format!("list corpora returned HTTP {}", resp.status()),
        ));
    }
    resp.json()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("parse list: {e}")))
}

/// POST `/api/v1/corpora` — register a corpus by paths on the remote server.
/// The remote server resolves the paths inside its own filesystem (e.g.
/// container `/data/...`), not the local desktop's.
#[tauri::command]
pub async fn cloud_register_corpus(
    paths: Vec<String>,
) -> Result<serde_json::Value, CommandError> {
    let (client, endpoint, token) = authed_client(15)?;
    let url = format!("{endpoint}/api/v1/corpora");
    let body = serde_json::json!({ "paths": paths });
    let resp = client
        .post(&url)
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("post {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(CommandError::new(
            ErrorKind::Io,
            format!("register returned HTTP {}", resp.status()),
        ));
    }
    resp.json()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("parse register: {e}")))
}

/// POST `/api/v1/corpora/{slug}/clone` — git-clone a remote repo and register
/// it as a corpus. The `slug` is derived from the URL when not supplied.
#[tauri::command]
pub async fn cloud_clone_repo(
    repo: String,
    branch: Option<String>,
    label: Option<String>,
) -> Result<serde_json::Value, CommandError> {
    let (client, endpoint, token) = authed_client(120)?;
    // Derive a slug from the URL when label is missing: last path segment,
    // strip `.git`. The daemon uses this as the corpus id prefix.
    let slug = label.unwrap_or_else(|| derive_slug_from_repo(&repo));
    let url = format!("{endpoint}/api/v1/corpora/{slug}/clone");
    let mut body = serde_json::json!({ "repo": repo });
    if let Some(b) = branch {
        body["branch"] = serde_json::Value::String(b);
    }
    let resp = client
        .post(&url)
        .bearer_auth(&token)
        .json(&body)
        .send()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("post {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(CommandError::new(
            ErrorKind::Io,
            format!("clone returned HTTP {}", resp.status()),
        ));
    }
    resp.json()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("parse clone: {e}")))
}

fn derive_slug_from_repo(repo: &str) -> String {
    repo.rsplit('/')
        .find(|s| !s.is_empty())
        .map_or_else(
            || "corpus".to_string(),
            |s| s.trim_end_matches(".git").to_string(),
        )
}

/// DELETE `/api/v1/corpora/{id}` — unregister a corpus on the remote server.
#[tauri::command]
pub async fn cloud_unregister_corpus(corpus_id: String) -> Result<(), CommandError> {
    let (client, endpoint, token) = authed_client(10)?;
    let url = format!("{endpoint}/api/v1/corpora/{corpus_id}");
    let resp = client
        .delete(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("delete {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(CommandError::new(
            ErrorKind::Io,
            format!("unregister returned HTTP {}", resp.status()),
        ));
    }
    Ok(())
}

/// Subscribe to `GET /api/v1/corpora/{id}/progress` (SSE) and forward each
/// `data:` event as JSON to the given Tauri Channel. Stops when the remote
/// stream closes (terminal status) or when the channel is dropped.
///
/// The function reads `Response::chunk()` and parses SSE frames inline so
/// we don't need to enable reqwest's `stream` feature.
#[tauri::command]
pub async fn cloud_corpus_progress(
    corpus_id: String,
    on_event: tauri::ipc::Channel<serde_json::Value>,
) -> Result<(), CommandError> {
    let (client, endpoint, token) = authed_client(0)?; // no overall timeout — SSE is long-lived
    let url = format!("{endpoint}/api/v1/corpora/{corpus_id}/progress");
    let resp = client
        .get(&url)
        .bearer_auth(&token)
        .header("accept", "text/event-stream")
        .send()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("get {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(CommandError::new(
            ErrorKind::Io,
            format!("progress SSE returned HTTP {}", resp.status()),
        ));
    }

    let mut buf = String::new();
    let mut resp = resp;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("chunk read: {e}")))?
    {
        let Ok(s) = std::str::from_utf8(&chunk) else {
            continue; // skip non-utf8 (shouldn't happen on SSE)
        };
        buf.push_str(s);

        // Each SSE event ends with a blank line (\n\n). Process complete events.
        while let Some(idx) = buf.find("\n\n") {
            let frame = buf[..idx].to_string();
            buf.drain(..idx + 2);
            for line in frame.lines() {
                if let Some(json_str) = line.strip_prefix("data:") {
                    let trimmed = json_str.trim();
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
                        // If the channel is closed (panel navigated away),
                        // the send returns an error — that's our exit signal.
                        if on_event.send(value).is_err() {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// POST `/reindex` on the configured cloud endpoint. Returns the
/// server-assigned `job_id` that can later be subscribed to via SSE.
#[tauri::command]
pub async fn cloud_trigger_reindex(corpus_id: String) -> Result<String, CommandError> {
    let cfg = load_config();
    if !cfg.is_configured() {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "no cloud endpoint configured",
        ));
    }
    if !cfg.is_authenticated() {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "cloud connection has no bearer token (sign in first)",
        ));
    }
    let url = format!("{}/reindex", cfg.endpoint);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| CommandError::new(ErrorKind::Internal, format!("http client: {e}")))?;
    let body = serde_json::json!({ "corpus_id": corpus_id });
    let resp = client
        .post(&url)
        .bearer_auth(&cfg.bearer_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("post {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(CommandError::new(
            ErrorKind::Io,
            format!("reindex returned HTTP {}", resp.status()),
        ));
    }
    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("parse /reindex: {e}")))?;
    v.get("job_id")
        .and_then(|j| j.as_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            CommandError::new(ErrorKind::Io, "reindex response missing job_id".to_string())
        })
}
