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
use sha2::{Digest, Sha256};
use tauri::AppHandle;
use tauri_plugin_shell::ShellExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tracing::{debug, info, warn};

use crate::error::{CommandError, ErrorKind};

const CLOUD_CONFIG_FILENAME: &str = "cloud.json";
const HEALTH_PROBE_TIMEOUT_SECS: u64 = 5;

/// OS-keychain coordinates for the cloud bearer token. macOS sees these
/// as the Keychain item's `service` + `account`; secret-service as the
/// item's attributes; Windows as the Credential Manager target. The
/// values are stable so a desktop upgrade picks up the token written
/// by the previous version.
const KEYRING_SERVICE: &str = "ministr-cloud";
const KEYRING_TOKEN_ACCOUNT: &str = "bearer_token";

/// Persisted state for the cloud connection. Lives on disk as JSON at
/// `<data-dir>/cloud.json` for the non-secret endpoint URL only — the
/// bearer token moved to the OS keychain in F1.3 and is never written
/// to this file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CloudConfig {
    /// Base URL, e.g. `https://mcp.ministr.ai`. Empty when not configured.
    #[serde(default)]
    pub endpoint: String,
}

impl CloudConfig {
    fn is_configured(&self) -> bool {
        !self.endpoint.trim().is_empty()
    }
}

/// True when the OS keychain currently holds a non-empty cloud bearer
/// token. Replaces the old `bearer_token`-on-disk check.
fn is_authenticated() -> bool {
    load_bearer_token().is_some_and(|t| !t.trim().is_empty())
}

/// Read the cloud bearer token from the OS keychain. Returns `None`
/// when no entry exists or the keychain is unreachable; callers treat
/// either case as "not signed in".
fn load_bearer_token() -> Option<String> {
    let entry = match keyring::Entry::new(KEYRING_SERVICE, KEYRING_TOKEN_ACCOUNT) {
        Ok(e) => e,
        Err(e) => {
            warn!(error = %e, "keyring entry construction failed");
            return None;
        }
    };
    match entry.get_password() {
        Ok(s) => Some(s),
        Err(keyring::Error::NoEntry) => None,
        Err(e) => {
            warn!(error = %e, "keyring read failed");
            None
        }
    }
}

/// Write the cloud bearer token to the OS keychain.
fn save_bearer_token(token: &str) -> Result<(), CommandError> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_TOKEN_ACCOUNT).map_err(|e| {
        CommandError::new(ErrorKind::Internal, format!("keyring entry: {e}"))
    })?;
    entry.set_password(token).map_err(|e| {
        CommandError::new(ErrorKind::Internal, format!("keyring write: {e}"))
    })?;
    debug!("saved cloud bearer token to OS keychain");
    Ok(())
}

/// Remove the cloud bearer token from the OS keychain. A missing entry
/// is treated as success — the post-condition (no entry) holds either
/// way.
fn delete_bearer_token() -> Result<(), CommandError> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_TOKEN_ACCOUNT).map_err(|e| {
        CommandError::new(ErrorKind::Internal, format!("keyring entry: {e}"))
    })?;
    match entry.delete_credential() {
        Ok(()) => {
            debug!("removed cloud bearer token from OS keychain");
            Ok(())
        }
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(CommandError::new(
            ErrorKind::Internal,
            format!("keyring delete: {e}"),
        )),
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
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return CloudConfig::default();
    };
    let cfg = serde_json::from_str::<CloudConfig>(&raw).unwrap_or_else(|e| {
        warn!(error = %e, path = %path.display(), "cloud.json malformed; starting fresh");
        CloudConfig::default()
    });

    // Pre-F1.3 builds wrote the bearer token alongside the endpoint in
    // cloud.json. Detect that legacy shape and migrate the token into
    // the OS keychain, then rewrite the file without the secret field.
    // The migration is best-effort: if the keychain write fails we
    // leave cloud.json alone so the next launch can retry.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw)
        && let Some(legacy) = value.get("bearer_token").and_then(|t| t.as_str())
        && !legacy.trim().is_empty()
    {
        if load_bearer_token().is_some() {
            // Keychain already holds a token — drop the legacy field
            // silently to avoid replaying a stale value over a freshly
            // refreshed one.
            let _ = save_config(&cfg);
        } else if save_bearer_token(legacy).is_ok() {
            info!("migrated legacy cloud bearer token from cloud.json to OS keychain");
            let _ = save_config(&cfg);
        } else {
            warn!("failed to migrate legacy bearer token to OS keychain; cloud.json left in place");
        }
    }
    cfg
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
        authenticated: is_authenticated(),
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

/// Save a Bearer token issued by the remote OAuth flow. Empty clears
/// the OS-keychain entry. Token lives in the platform credential
/// store (F1.3 OS-keychain carry-over) — never on disk.
#[tauri::command]
pub async fn cloud_set_bearer_token(token: String) -> Result<(), CommandError> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        delete_bearer_token()
    } else {
        save_bearer_token(trimmed)
    }
}

/// Drive the full OAuth 2.1 + PKCE flow against the configured endpoint.
///
/// Native-app pattern (RFC 8252): bind a one-shot loopback listener on
/// `127.0.0.1:0` to receive the redirect, register a public client via
/// RFC 7591 with that loopback URL, open the system browser to
/// `/oauth/authorize`, wait for the redirect, exchange the code at
/// `/oauth/token`, persist the access token. No new deps, no OS URL
/// scheme registration needed.
///
/// Cancellation: a 3-minute deadline aborts the listener if the user
/// never completes the flow in their browser.
#[tauri::command]
#[allow(clippy::too_many_lines)] // OAuth flow has 6 sequential phases; splitting is artificial
pub async fn cloud_authenticate(app: AppHandle) -> Result<(), CommandError> {
    let cfg = load_config();
    let endpoint = if cfg.is_configured() {
        cfg.endpoint.clone()
    } else {
        "https://mcp.ministr.ai".to_string()
    };

    // PKCE materials. The verifier is ephemeral; "good enough" entropy is
    // SHA-256 over nanos + an OS heap pointer (same shape the server uses
    // for `generate_id`). For multi-tenant prod we'd want OS RNG, but this
    // is a single-user desktop session.
    let verifier = random_url_safe_id(64);
    let challenge = pkce_s256(&verifier);
    let state_nonce = random_url_safe_id(32);

    // Bind the callback listener BEFORE registering the redirect_uri so
    // we know the port number we're committing to. Kernel-assigned port.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("bind callback: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("local_addr: {e}")))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/oauth/callback");
    info!(port, "cloud_authenticate: callback listener bound");

    // RFC 7591 dynamic client registration. `none` auth means public
    // client (no client secret) — correct for a desktop without a server-
    // side credential vault.
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| CommandError::new(ErrorKind::Internal, format!("http client: {e}")))?;
    let reg_url = format!("{endpoint}/oauth/register");
    let reg: serde_json::Value = http
        .post(&reg_url)
        .json(&serde_json::json!({
            "redirect_uris": [redirect_uri.clone()],
            "client_name": "ministr-desktop",
            "token_endpoint_auth_method": "none",
        }))
        .send()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("register: {e}")))?
        .error_for_status()
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("register status: {e}")))?
        .json()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("parse register: {e}")))?;
    let client_id = reg
        .get("client_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            CommandError::new(ErrorKind::Io, "register response missing client_id")
        })?
        .to_string();

    // Open the system browser. urlencoding the redirect_uri is mandatory
    // since it contains `:`/`/` characters that would break query parsing.
    let scopes = "ministr:read ministr:write ministr:bundle:read ministr:bundle:write";
    let authorize_url = format!(
        "{endpoint}/oauth/authorize?response_type=code&client_id={cid}\
         &redirect_uri={ru}&code_challenge={cc}&code_challenge_method=S256\
         &state={st}&scope={sc}",
        cid = url_encode(&client_id),
        ru = url_encode(&redirect_uri),
        cc = url_encode(&challenge),
        st = url_encode(&state_nonce),
        sc = url_encode(scopes),
    );
    #[allow(deprecated)]
    // `Shell::open` is deprecated in favour of tauri-plugin-opener; migrating
    // is a separate refactor. For now this is the supported path under
    // tauri-plugin-shell 2.x.
    app.shell()
        .open(authorize_url.clone(), None)
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("open browser: {e}")))?;
    debug!("cloud_authenticate: browser launched to {authorize_url}");

    // Wait for the redirect. Time-box at 3 min so a user who abandons the
    // flow doesn't leave the listener dangling.
    let (code, state_recv) = tokio::time::timeout(
        Duration::from_secs(180),
        await_oauth_callback(listener),
    )
    .await
    .map_err(|_| {
        CommandError::new(
            ErrorKind::Io,
            "OAuth flow timed out — user did not complete sign-in within 3 minutes",
        )
    })??;

    if state_recv != state_nonce {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "OAuth state mismatch (possible CSRF attempt) — please retry",
        ));
    }

    // Exchange code → token. axum's `Form` extractor expects
    // application/x-www-form-urlencoded. Encoded by hand to avoid pulling
    // in reqwest's `form` feature (and the serde_urlencoded transitive).
    let token_body = format!(
        "grant_type=authorization_code&code={c}&redirect_uri={ru}&client_id={cid}\
         &code_verifier={v}",
        c = url_encode(&code),
        ru = url_encode(&redirect_uri),
        cid = url_encode(&client_id),
        v = url_encode(&verifier),
    );
    let token_url = format!("{endpoint}/oauth/token");
    let token_resp: serde_json::Value = http
        .post(&token_url)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(token_body)
        .send()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("token exchange: {e}")))?
        .error_for_status()
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("token status: {e}")))?
        .json()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("parse token: {e}")))?;
    let access_token = token_resp
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            CommandError::new(ErrorKind::Io, "token response missing access_token")
        })?
        .to_string();

    let mut saved = load_config();
    saved.endpoint = endpoint;
    save_config(&saved)?;
    save_bearer_token(&access_token)?;
    info!("cloud_authenticate: token acquired and persisted to OS keychain");
    Ok(())
}

/// Drive the F1.3 GitHub sign-in flow against the configured cloud
/// endpoint. Same RFC 8252 loopback-listener pattern as
/// [`cloud_authenticate`], but instead of running the OAuth code-grant
/// dance directly against `/oauth/*`, this command bounces through the
/// cloud's `/auth/github/start` route — the cloud handles federation,
/// upserts the user, and redirects the bearer token back to our
/// loopback.
///
/// The cloud must be configured with the GitHub OAuth App credentials
/// (`MINISTR_GITHUB_CLIENT_ID` + `MINISTR_GITHUB_CLIENT_SECRET`) and
/// `MINISTR_CLOUD_BASE_URL`, otherwise the route returns 404.
///
/// Cancellation: a 3-minute deadline aborts the listener if the user
/// never completes the GitHub consent screen.
#[tauri::command]
pub async fn cloud_authenticate_github(app: AppHandle) -> Result<(), CommandError> {
    let cfg = load_config();
    let endpoint = if cfg.is_configured() {
        cfg.endpoint.clone()
    } else {
        "https://mcp.ministr.ai".to_string()
    };

    // CSRF nonce for the loopback redirect. The cloud echoes this back
    // verbatim; the listener verifies it matches before saving the token.
    let state_nonce = random_url_safe_id(32);

    // Bind the callback listener BEFORE building the start URL so we
    // know the port we're committing to.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("bind callback: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("local_addr: {e}")))?
        .port();
    let loopback_redirect = format!("http://127.0.0.1:{port}/cb");
    info!(port, "cloud_authenticate_github: callback listener bound");

    let start_url = format!(
        "{endpoint}/auth/github/start?loopback_redirect={lr}&state={st}",
        lr = url_encode(&loopback_redirect),
        st = url_encode(&state_nonce),
    );

    #[allow(deprecated)]
    // Same `Shell::open` migration tracked in F2.7 as `cloud_authenticate`.
    app.shell()
        .open(start_url.clone(), None)
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("open browser: {e}")))?;
    debug!("cloud_authenticate_github: browser launched to {start_url}");

    let outcome = tokio::time::timeout(
        Duration::from_secs(180),
        await_github_signin_callback(listener),
    )
    .await
    .map_err(|_| {
        CommandError::new(
            ErrorKind::Io,
            "GitHub sign-in flow timed out — user did not complete sign-in within 3 minutes",
        )
    })??;

    if outcome.state != state_nonce {
        return Err(CommandError::new(
            ErrorKind::InvalidInput,
            "GitHub sign-in state mismatch (possible CSRF attempt) — please retry",
        ));
    }
    if let Some(err) = outcome.error {
        return Err(CommandError::new(
            ErrorKind::Io,
            format!("GitHub sign-in declined or failed: {err}"),
        ));
    }
    let token = outcome.token.ok_or_else(|| {
        CommandError::new(
            ErrorKind::Io,
            "GitHub sign-in completed without delivering a bearer token",
        )
    })?;

    let mut saved = load_config();
    saved.endpoint = endpoint;
    save_config(&saved)?;
    save_bearer_token(&token)?;
    info!("cloud_authenticate_github: token acquired and persisted to OS keychain");
    Ok(())
}

/// Parsed query-string parameters the cloud delivers to the loopback.
struct GitHubSigninCallback {
    state: String,
    token: Option<String>,
    error: Option<String>,
}

async fn await_github_signin_callback(
    listener: TcpListener,
) -> Result<GitHubSigninCallback, CommandError> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("accept callback: {e}")))?;
    let (read_half, mut write_half) = stream.split();
    let mut reader = BufReader::new(read_half);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("read request: {e}")))?;
    let mut discard = String::new();
    loop {
        discard.clear();
        let n = reader
            .read_line(&mut discard)
            .await
            .map_err(|e| CommandError::new(ErrorKind::Io, format!("drain headers: {e}")))?;
        if n == 0 || discard == "\r\n" || discard == "\n" {
            break;
        }
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        let _ = write_html_response(&mut write_half, "Malformed request").await;
        return Err(CommandError::new(
            ErrorKind::Io,
            format!("malformed callback request line: {request_line:?}"),
        ));
    }
    let path_and_query = parts[1];
    let query = path_and_query.split_once('?').map_or("", |(_, q)| q);
    let mut state: Option<String> = None;
    let mut token: Option<String> = None;
    let mut error: Option<String> = None;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let v = url_decode(v);
            match k {
                "state" => state = Some(v),
                "token" => token = Some(v),
                "error" => error = Some(v),
                _ => {}
            }
        }
    }

    let Some(state) = state else {
        let _ = write_html_response(
            &mut write_half,
            "Missing state on callback. Please retry.",
        )
        .await;
        return Err(CommandError::new(
            ErrorKind::Io,
            "github callback missing state",
        ));
    };

    let message = if error.is_some() {
        "GitHub sign-in failed. You can close this window."
    } else if token.is_some() {
        "Signed in to ministr Cloud via GitHub. You can close this window."
    } else {
        "GitHub sign-in returned no token. Please retry."
    };
    write_html_response(&mut write_half, message)
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("write response: {e}")))?;

    Ok(GitHubSigninCallback {
        state,
        token,
        error,
    })
}

/// Accept one TCP connection on `listener`, read the HTTP request line,
/// extract `code` and `state` from the query string, write a friendly
/// "you can close this window" HTML response, and return `(code, state)`.
async fn await_oauth_callback(listener: TcpListener) -> Result<(String, String), CommandError> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("accept callback: {e}")))?;
    let (read_half, mut write_half) = stream.split();
    let mut reader = BufReader::new(read_half);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("read request: {e}")))?;
    // Drain remaining headers so the browser sees a clean response.
    let mut discard = String::new();
    loop {
        discard.clear();
        let n = reader
            .read_line(&mut discard)
            .await
            .map_err(|e| CommandError::new(ErrorKind::Io, format!("drain headers: {e}")))?;
        if n == 0 || discard == "\r\n" || discard == "\n" {
            break;
        }
    }

    // Request line is "GET /oauth/callback?code=...&state=... HTTP/1.1".
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        let _ = write_html_response(&mut write_half, "Malformed request").await;
        return Err(CommandError::new(
            ErrorKind::Io,
            format!("malformed callback request line: {request_line:?}"),
        ));
    }
    let path_and_query = parts[1];
    let query = path_and_query.split_once('?').map_or("", |(_, q)| q);
    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    let mut err_param: Option<String> = None;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let v = url_decode(v);
            match k {
                "code" => code = Some(v),
                "state" => state = Some(v),
                "error" => err_param = Some(v),
                _ => {}
            }
        }
    }

    if let Some(e) = err_param {
        let _ = write_html_response(
            &mut write_half,
            "Sign-in failed. You can close this window.",
        )
        .await;
        return Err(CommandError::new(
            ErrorKind::Io,
            format!("OAuth provider returned error: {e}"),
        ));
    }
    let (Some(code), Some(state)) = (code, state) else {
        let _ = write_html_response(
            &mut write_half,
            "Missing OAuth response parameters. Please retry.",
        )
        .await;
        return Err(CommandError::new(
            ErrorKind::Io,
            "callback missing code or state",
        ));
    };

    write_html_response(
        &mut write_half,
        "Signed in to ministr Cloud. You can close this window.",
    )
    .await
    .map_err(|e| CommandError::new(ErrorKind::Io, format!("write response: {e}")))?;
    Ok((code, state))
}

async fn write_html_response<W: AsyncWriteExt + Unpin>(
    w: &mut W,
    message: &str,
) -> std::io::Result<()> {
    let body = format!(
        "<!doctype html><html><head><meta charset=utf-8><title>ministr Cloud</title>\
         <style>body{{font-family:system-ui,sans-serif;margin:0;padding:48px;\
         background:#0a0a0a;color:#e0e0e0;text-align:center}}\
         .card{{display:inline-block;border:1px solid #333;border-radius:12px;\
         padding:32px 48px;background:#161616}}h1{{font-size:18px;margin:0 0 8px}}\
         p{{margin:0;color:#888;font-size:14px}}</style></head><body>\
         <div class=card><h1>{message}</h1><p>This window can be closed.</p></div>\
         </body></html>",
    );
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    w.write_all(response.as_bytes()).await?;
    w.flush().await?;
    Ok(())
}

/// PKCE code-challenge = base64url(sha256(verifier)).
fn pkce_s256(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    base64_url_no_pad(&hasher.finalize())
}

/// Generate a URL-safe identifier from `bytes` cryptographically-random
/// bytes. Used for the PKCE code verifier and the OAuth `state` nonce.
///
/// `getrandom::fill` delegates to the platform CSPRNG, which is what
/// RFC 7636 §4.1 requires for the verifier (the previous SHA-256-over-
/// nanos shim leaked too much structure to be safe against a network
/// adversary who can observe the challenge).
///
/// # Panics
///
/// Panics if the OS RNG is unreachable. On all supported platforms this
/// is treated as unrecoverable — sign-in cannot proceed safely without a
/// real RNG, and a panic surfaces the problem rather than silently
/// emitting a predictable verifier.
fn random_url_safe_id(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS RNG must be available for PKCE");
    base64_url_no_pad(&buf)
}

fn base64_url_no_pad(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((data.len() * 4).div_ceil(3));
    let mut i = 0;
    while i < data.len() {
        let b0 = u32::from(data[i]);
        let b1 = if i + 1 < data.len() {
            u32::from(data[i + 1])
        } else {
            0
        };
        let b2 = if i + 2 < data.len() {
            u32::from(data[i + 2])
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < data.len() {
            out.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        }
        if i + 2 < data.len() {
            out.push(ALPHABET[(triple & 0x3F) as usize] as char);
        }
        i += 3;
    }
    out
}

/// Minimal URL component encoder — keeps unreserved chars, percent-encodes
/// everything else. RFC 3986 unreserved set: A-Z a-z 0-9 - _ . ~
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let allowed = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if allowed {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_nibble(b >> 4));
            out.push(hex_nibble(b & 0x0f));
        }
    }
    out
}

fn url_decode(input: &str) -> String {
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut idx = 0;
    while idx < bytes.len() {
        let byte = bytes[idx];
        if byte == b'+' {
            out.push(b' ');
            idx += 1;
        } else if byte == b'%' && idx + 2 < bytes.len() {
            let hi = hex_value(bytes[idx + 1]);
            let lo = hex_value(bytes[idx + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h << 4) | l);
                idx += 3;
            } else {
                out.push(byte);
                idx += 1;
            }
        } else {
            out.push(byte);
            idx += 1;
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'A' + (n - 10)) as char,
        _ => unreachable!(),
    }
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
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
    let token = load_bearer_token()
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(|| {
            CommandError::new(
                ErrorKind::InvalidInput,
                "cloud connection has no bearer token (sign in first)",
            )
        })?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| CommandError::new(ErrorKind::Internal, format!("http client: {e}")))?;
    Ok((client, cfg.endpoint, token))
}

/// POST `/api/v1/billing/checkout` — mint a Stripe Checkout session
/// for the requested plan (F2.4). Opens the returned URL in the
/// system browser so the user pays in Stripe-hosted UI; the cloud
/// webhook flips `users.plan_id` once payment lands.
#[tauri::command]
pub async fn cloud_billing_checkout(
    app: AppHandle,
    plan: String,
) -> Result<(), CommandError> {
    let (client, endpoint, token) = authed_client(15)?;
    let url = format!("{endpoint}/api/v1/billing/checkout");
    let body = serde_json::json!({ "plan": plan });
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
            format!("checkout returned HTTP {}", resp.status()),
        ));
    }
    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("parse checkout: {e}")))?;
    let session_url = payload
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            CommandError::new(ErrorKind::Io, "checkout response missing url".to_string())
        })?;
    #[allow(deprecated)] // matches cloud_authenticate's TODO; F2.7 migrates to tauri-plugin-opener
    app.shell()
        .open(session_url.to_string(), None)
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("open browser: {e}")))?;
    info!(plan = %plan, "cloud_billing_checkout: stripe session opened in browser");
    Ok(())
}

/// POST `/api/v1/billing/portal` — open the Stripe Customer Portal for
/// invoices, card management, and cancellation (F2.4). Same browser-
/// open pattern as the checkout flow.
#[tauri::command]
pub async fn cloud_billing_portal(app: AppHandle) -> Result<(), CommandError> {
    let (client, endpoint, token) = authed_client(15)?;
    let url = format!("{endpoint}/api/v1/billing/portal");
    let resp = client
        .post(&url)
        .bearer_auth(&token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("post {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(CommandError::new(
            ErrorKind::Io,
            format!("portal returned HTTP {}", resp.status()),
        ));
    }
    let payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("parse portal: {e}")))?;
    let session_url = payload
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            CommandError::new(ErrorKind::Io, "portal response missing url".to_string())
        })?;
    #[allow(deprecated)]
    app.shell()
        .open(session_url.to_string(), None)
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("open browser: {e}")))?;
    info!("cloud_billing_portal: stripe portal opened in browser");
    Ok(())
}

/// GET `/api/v1/billing/usage` — fetch the calling tenant's metered
/// usage (F1.4 sub-bullet 4). Drives the overview-tile badges
/// (F1.4 sub-bullet 5).
///
/// Returns the server's `UsageResponse` payload verbatim — the panel
/// renders against the wire shape rather than a re-mapped local
/// struct so the contract stays in one place.
#[tauri::command]
pub async fn cloud_billing_usage() -> Result<serde_json::Value, CommandError> {
    let (client, endpoint, token) = authed_client(10)?;
    let url = format!("{endpoint}/api/v1/billing/usage");
    let resp = client
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("get {url}: {e}")))?;
    if !resp.status().is_success() {
        return Err(CommandError::new(
            ErrorKind::Io,
            format!("billing usage returned HTTP {}", resp.status()),
        ));
    }
    resp.json()
        .await
        .map_err(|e| CommandError::new(ErrorKind::Io, format!("parse billing usage: {e}")))
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
///
/// `github_installation_id`, when set, drives the F2.1 private-repo path:
/// the cloud mints a short-lived GitHub App installation token and
/// splices it into the clone URL server-side. The token never reaches
/// this command.
#[tauri::command]
pub async fn cloud_clone_repo(
    repo: String,
    branch: Option<String>,
    label: Option<String>,
    github_installation_id: Option<String>,
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
    if let Some(id) = github_installation_id
        && !id.trim().is_empty()
    {
        body["github_installation_id"] = serde_json::Value::String(id);
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
    let (client, endpoint, token) = authed_client(10)?;
    let url = format!("{endpoint}/reindex");
    let body = serde_json::json!({ "corpus_id": corpus_id });
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

#[cfg(test)]
mod tests {
    use super::*;

    /// PKCE verifiers must use the URL-safe base64 alphabet per
    /// RFC 7636 §4.1: `[A-Za-z0-9_-]`.
    fn is_pkce_alphabet(s: &str) -> bool {
        s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    }

    #[test]
    fn random_url_safe_id_uses_pkce_alphabet() {
        let id = random_url_safe_id(32);
        assert!(is_pkce_alphabet(&id), "unexpected chars in {id:?}");
    }

    #[test]
    fn random_url_safe_id_is_unpredictable() {
        // Two back-to-back calls share a nanosecond on a fast machine —
        // the old SHA-256-over-nanos shim would collide here. The OS
        // CSPRNG must not.
        let a = random_url_safe_id(32);
        let b = random_url_safe_id(32);
        assert_ne!(a, b, "OS RNG produced two identical 32-byte ids");
    }

    #[test]
    fn random_url_safe_id_meets_pkce_minimum_length() {
        // RFC 7636 §4.1: verifier MUST be 43-128 base64url chars. 32
        // random bytes encode to 43 chars (no padding) — the minimum.
        let id = random_url_safe_id(32);
        assert!(
            (43..=128).contains(&id.len()),
            "32-byte verifier length {} outside RFC range",
            id.len()
        );
    }

    #[test]
    fn pkce_s256_is_deterministic_for_a_given_verifier() {
        let verifier = "test-verifier-deterministic";
        assert_eq!(pkce_s256(verifier), pkce_s256(verifier));
    }

    /// Round-trip a bearer token through the OS keychain. Gated on
    /// `MINISTR_TEST_KEYCHAIN=1` because hitting the platform keychain
    /// requires a graphical session on Linux (D-Bus secret-service)
    /// and shouldn't be a default-on dependency for `cargo test`.
    /// Developers verifying the keychain wiring run:
    ///   `MINISTR_TEST_KEYCHAIN=1` cargo test -p ministr-app -- --ignored
    #[test]
    #[ignore = "needs MINISTR_TEST_KEYCHAIN=1 + a usable platform keychain"]
    fn bearer_token_round_trips_through_keychain() {
        if std::env::var("MINISTR_TEST_KEYCHAIN").as_deref() != Ok("1") {
            return;
        }
        // Use a unique account so parallel test runs don't collide
        // with each other or with a real signed-in session.
        let account = format!("test-{}", std::process::id());
        let entry = keyring::Entry::new(KEYRING_SERVICE, &account).unwrap();
        entry.set_password("test-token-123").unwrap();
        let got = entry.get_password().unwrap();
        assert_eq!(got, "test-token-123");
        entry.delete_credential().unwrap();
        assert!(matches!(entry.get_password(), Err(keyring::Error::NoEntry)));
    }
}
