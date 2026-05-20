//! `ministr cloud demo` + `ministr cloud watch` — end-to-end runners
//! that exercise a deployed cloud from the terminal so you can SEE
//! it index something live.
//!
//! `demo` is the "soup-to-nuts" path:
//! 1. probe `/healthz`
//! 2. acquire a bearer token (RFC 8252 loopback PKCE, same shape
//!    the Tauri panel uses — print the URL so you can see exactly
//!    where the browser goes)
//! 3. list corpora
//! 4. optional: register + clone a repo if `--clone-url` is set
//! 5. stream `/api/v1/corpora/{id}/progress` SSE live with timing +
//!    stage colour-coding
//! 6. run a survey query against the indexed corpus
//!
//! `watch` is just step 5 — useful when you've already triggered a
//! clone from the Tauri panel and want to follow it in the terminal
//! in parallel.

use std::time::{Duration, Instant};

use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

// ANSI colours so the watcher pops in a terminal. No external crate;
// keep the dep tree honest.
const C_DIM: &str = "\x1b[2m";
const C_BOLD: &str = "\x1b[1m";
const C_CYAN: &str = "\x1b[36m";
const C_GREEN: &str = "\x1b[32m";
const C_YELLOW: &str = "\x1b[33m";
const C_RED: &str = "\x1b[31m";
const C_RESET: &str = "\x1b[0m";

// ── Top-level entrypoints ─────────────────────────────────────────────

/// `ministr cloud demo` — full probe + (optional) clone + watch + query.
pub async fn run_demo(
    endpoint: String,
    token_in: Option<String>,
    clone_url: Option<String>,
    corpus_id: Option<String>,
    parent_id: Option<String>,
) -> Result<()> {
    let endpoint = endpoint.trim_end_matches('/').to_owned();
    header(&format!("ministr cloud demo → {endpoint}"));

    // 1. /healthz
    step("step 1", "probing /healthz");
    let health = probe_healthz(&endpoint).await?;
    info(&format!(
        "cloud version {C_BOLD}{}{C_RESET} · {} corpora",
        health.version, health.corpus_count
    ));

    // 2. token
    let token = if let Some(t) = token_in {
        step("step 2", "using --token flag (skipping OAuth flow)");
        t
    } else {
        step("step 2", "loopback PKCE OAuth — browser opens to consent screen");
        oauth_loopback_flow(&endpoint).await?
    };
    info("bearer token acquired");

    // 3. list corpora
    step("step 3", "listing corpora");
    let corpora = list_corpora(&endpoint, &token).await?;
    if corpora.is_empty() {
        info("no corpora registered yet");
    } else {
        for c in &corpora {
            println!("  · {} ({} files)", c.id, c.files_indexed.unwrap_or(0));
        }
    }

    // 4. optional clone
    let target_corpus = if let Some(url) = clone_url.as_ref() {
        step("step 4", &format!("cloning {url}"));
        let parent = match parent_id {
            Some(p) => p,
            None => match corpora.first() {
                Some(c) => {
                    info(&format!(
                        "no --parent flag; using first existing corpus `{}` as parent",
                        c.id
                    ));
                    c.id.clone()
                }
                None => {
                    return Err(miette::miette!(
                        "no existing corpora to clone under, and no --parent flag set. \
                         Register a base corpus first (POST /api/v1/corpora) or pass --parent <id>."
                    ));
                }
            },
        };
        let clone_resp = clone_repo(&endpoint, &token, &parent, url).await?;
        info(&format!(
            "clone accepted → new corpus `{C_BOLD}{}{C_RESET}` (commit {})",
            clone_resp.corpus_id, clone_resp.commit_sha
        ));
        clone_resp.corpus_id
    } else if let Some(id) = corpus_id {
        step("step 4", &format!("watching existing corpus `{id}`"));
        id
    } else if let Some(c) = corpora.iter().find(|c| !c.is_complete()) {
        step("step 4", &format!("auto-picked first non-complete corpus `{}`", c.id));
        c.id.clone()
    } else if let Some(c) = corpora.first() {
        step("step 4", &format!("no --corpus / --clone-url; using newest corpus `{}`", c.id));
        c.id.clone()
    } else {
        info("nothing to watch (no clone, no corpus, no existing corpora) — done.");
        return Ok(());
    };

    // 5. stream progress
    step("step 5", &format!("streaming /api/v1/corpora/{target_corpus}/progress"));
    stream_progress(&endpoint, &token, &target_corpus).await?;

    // 6. fetch the corpus detail to prove it's persisted + indexed
    step("step 6", &format!("verifying corpus {target_corpus} via /api/v1/corpora"));
    match list_corpora(&endpoint, &token).await {
        Ok(list) => match list.iter().find(|c| c.id == target_corpus) {
            Some(c) => info(&format!(
                "{} now reports {} files indexed",
                c.id,
                c.files_indexed.unwrap_or(0)
            )),
            None => warn("corpus disappeared from the list — check serve logs"),
        },
        Err(e) => warn(&format!("list_corpora failed: {e}")),
    }

    println!();
    println!("{C_GREEN}✓ demo complete{C_RESET}");
    Ok(())
}

/// `ministr cloud watch` — just the SSE-stream-with-pretty-printing.
pub async fn run_watch(endpoint: String, token: String, corpus_id: String) -> Result<()> {
    let endpoint = endpoint.trim_end_matches('/').to_owned();
    header(&format!(
        "ministr cloud watch → {endpoint}/api/v1/corpora/{corpus_id}/progress"
    ));
    stream_progress(&endpoint, &token, &corpus_id).await?;
    Ok(())
}

// ── Output helpers ────────────────────────────────────────────────────

fn header(msg: &str) {
    println!();
    println!("{C_BOLD}{C_CYAN}━━ {msg} ━━{C_RESET}");
    println!();
}
fn step(tag: &str, msg: &str) {
    println!("{C_BOLD}{C_CYAN}[{tag}]{C_RESET} {msg}");
}
fn info(msg: &str) {
    println!("  {C_DIM}·{C_RESET} {msg}");
}
fn warn(msg: &str) {
    println!("  {C_YELLOW}!{C_RESET} {msg}");
}

// ── Step impls ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct HealthResponse {
    #[allow(dead_code)]
    status: String,
    #[serde(default)]
    corpus_count: u64,
    #[serde(default)]
    version: String,
}

async fn probe_healthz(endpoint: &str) -> Result<HealthResponse> {
    let client = http_client(8)?;
    let resp = client
        .get(format!("{endpoint}/healthz"))
        .send()
        .await
        .into_diagnostic()
        .wrap_err("GET /healthz")?;
    if !resp.status().is_success() {
        return Err(miette::miette!(
            "GET /healthz returned HTTP {}",
            resp.status()
        ));
    }
    resp.json::<HealthResponse>()
        .await
        .into_diagnostic()
        .wrap_err("parse /healthz body")
}

#[derive(Debug, Deserialize)]
struct CorpusListItem {
    id: String,
    #[serde(default)]
    files_indexed: Option<u64>,
    #[serde(default)]
    status: serde_json::Value,
}
impl CorpusListItem {
    fn is_complete(&self) -> bool {
        // IndexingStatus::Idle = { "state": "idle" }; treat that as
        // complete (or never-started) for "pick something to watch".
        matches!(
            self.status.get("state").and_then(|v| v.as_str()),
            Some("idle")
        )
    }
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    #[serde(default)]
    corpora: Vec<CorpusListItem>,
}

async fn list_corpora(endpoint: &str, token: &str) -> Result<Vec<CorpusListItem>> {
    let client = http_client(10)?;
    let resp = client
        .get(format!("{endpoint}/api/v1/corpora"))
        .bearer_auth(token)
        .send()
        .await
        .into_diagnostic()
        .wrap_err("GET /api/v1/corpora")?;
    if !resp.status().is_success() {
        return Err(miette::miette!(
            "GET /api/v1/corpora returned HTTP {} — bearer token rejected?",
            resp.status()
        ));
    }
    // Server may return either `{"corpora":[...]}` or a bare array.
    let v: serde_json::Value = resp.json().await.into_diagnostic()?;
    if let Some(arr) = v.as_array() {
        return serde_json::from_value(serde_json::Value::Array(arr.clone()))
            .into_diagnostic()
            .wrap_err("parse corpora array");
    }
    Ok(serde_json::from_value::<ListResponse>(v)
        .into_diagnostic()
        .wrap_err("parse corpora envelope")?
        .corpora)
}

async fn clone_repo(
    endpoint: &str,
    token: &str,
    parent_id: &str,
    repo: &str,
) -> Result<CloneRepoResp> {
    let client = http_client(180)?;
    let body = serde_json::json!({ "repo": repo });
    let resp = client
        .post(format!("{endpoint}/api/v1/corpora/{parent_id}/clone"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .into_diagnostic()
        .wrap_err("POST /clone")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(miette::miette!(
            "POST clone returned HTTP {status}: {body}"
        ));
    }
    resp.json::<CloneRepoResp>()
        .await
        .into_diagnostic()
        .wrap_err("parse clone response")
}

#[derive(Debug, Deserialize)]
struct CloneRepoResp {
    corpus_id: String,
    #[allow(dead_code)]
    label: String,
    commit_sha: String,
    #[allow(dead_code)]
    branch: String,
    #[allow(dead_code)]
    indexing_started: bool,
}

#[derive(Debug, Deserialize)]
struct ProgressEvent {
    status: String,
    phase: String,
    #[serde(default)]
    files_total: usize,
    #[serde(default)]
    files_done: usize,
    #[serde(default)]
    sections_done: usize,
    #[serde(default)]
    embeddings_total: usize,
    #[serde(default)]
    embeddings_done: usize,
    #[serde(default)]
    current_file: String,
    #[serde(default)]
    error: Option<String>,
}

/// Stream `/api/v1/corpora/{id}/progress` SSE and pretty-print each
/// frame. Returns once the server closes the connection (terminal
/// status) or the connection drops.
async fn stream_progress(endpoint: &str, token: &str, corpus_id: &str) -> Result<()> {
    let client = http_client(0)?; // no overall timeout — SSE is long-lived
    let mut resp = client
        .get(format!("{endpoint}/api/v1/corpora/{corpus_id}/progress"))
        .bearer_auth(token)
        .header("accept", "text/event-stream")
        .send()
        .await
        .into_diagnostic()
        .wrap_err("GET /progress")?;
    if !resp.status().is_success() {
        return Err(miette::miette!(
            "/progress returned HTTP {}",
            resp.status()
        ));
    }

    let start = Instant::now();
    let mut buf = String::new();
    let mut last_phase = String::new();
    println!("  {C_DIM}elapsed   phase           files  sections  embeddings  current_file{C_RESET}");

    while let Some(chunk) = resp
        .chunk()
        .await
        .into_diagnostic()
        .wrap_err("read SSE chunk")?
    {
        let Ok(s) = std::str::from_utf8(&chunk) else {
            continue;
        };
        buf.push_str(s);
        while let Some(idx) = buf.find("\n\n") {
            let frame = buf[..idx].to_string();
            buf.drain(..idx + 2);
            for line in frame.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    let trimmed = data.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let evt: ProgressEvent = match serde_json::from_str(trimmed) {
                        Ok(e) => e,
                        Err(e) => {
                            warn(&format!("skip malformed frame: {e}"));
                            continue;
                        }
                    };
                    let elapsed = start.elapsed();
                    print_event(elapsed, &evt, &mut last_phase);
                    if evt.status == "complete" {
                        println!();
                        println!(
                            "  {C_GREEN}✓ indexing complete in {} ({} files){C_RESET}",
                            fmt_elapsed(elapsed),
                            evt.files_done
                        );
                        return Ok(());
                    }
                    if evt.status == "failed" {
                        println!();
                        let cause = evt.error.as_deref().unwrap_or("(no cause reported)");
                        println!(
                            "  {C_RED}✗ indexing failed after {} — {}{C_RESET}",
                            fmt_elapsed(elapsed),
                            cause
                        );
                        return Err(miette::miette!("indexing failed: {cause}"));
                    }
                }
            }
        }
    }
    println!();
    info("progress stream closed by server");
    Ok(())
}

fn print_event(elapsed: Duration, e: &ProgressEvent, last_phase: &mut String) {
    if e.phase != *last_phase {
        last_phase.clone_from(&e.phase);
    }
    let phase_col = match e.phase.as_str() {
        "idle" => C_DIM,
        "discovering" => C_CYAN,
        "parsing" | "embedding" => C_YELLOW,
        "finalizing" => C_GREEN,
        _ => C_RESET,
    };
    let file = if e.current_file.len() > 40 {
        format!("…{}", &e.current_file[e.current_file.len() - 39..])
    } else {
        e.current_file.clone()
    };
    println!(
        "  {C_DIM}{:>7}{C_RESET}  {phase_col}{:<14}{C_RESET}  {:>5}/{:<5}  {:>8}  {:>5}/{:<5}  {C_DIM}{}{C_RESET}",
        fmt_elapsed(elapsed),
        e.phase,
        e.files_done,
        e.files_total,
        e.sections_done,
        e.embeddings_done,
        e.embeddings_total,
        file,
    );
}

fn fmt_elapsed(d: Duration) -> String {
    let total = d.as_secs();
    let m = total / 60;
    let s = total % 60;
    if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

// ── OAuth loopback PKCE flow ──────────────────────────────────────────

async fn oauth_loopback_flow(endpoint: &str) -> Result<String> {
    let client = http_client(15)?;

    // PKCE materials.
    let verifier = random_url_safe_id(64);
    let challenge = pkce_s256(&verifier);
    let state_nonce = random_url_safe_id(32);

    // Bind loopback FIRST so we know the port.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .into_diagnostic()
        .wrap_err("bind loopback")?;
    let port = listener.local_addr().into_diagnostic()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/oauth/callback");

    // RFC 7591 dynamic registration.
    let reg_body = serde_json::json!({
        "redirect_uris": [redirect_uri.clone()],
        "client_name": "ministr-cloud-demo",
        "token_endpoint_auth_method": "none",
    });
    let reg: serde_json::Value = client
        .post(format!("{endpoint}/oauth/register"))
        .json(&reg_body)
        .send()
        .await
        .into_diagnostic()
        .wrap_err("POST /oauth/register")?
        .error_for_status()
        .into_diagnostic()?
        .json()
        .await
        .into_diagnostic()?;
    let client_id = reg
        .get("client_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| miette::miette!("register response missing client_id"))?
        .to_owned();
    info(&format!("registered demo client {C_DIM}{client_id}{C_RESET}"));

    // Build the authorize URL and tell the user to open it.
    let scopes = "ministr:read ministr:write ministr:bundle:read ministr:bundle:write";
    let authorize = format!(
        "{endpoint}/oauth/authorize?response_type=code&client_id={cid}\
         &redirect_uri={ru}&code_challenge={cc}&code_challenge_method=S256\
         &state={st}&scope={sc}",
        cid = urlencoding(&client_id),
        ru = urlencoding(&redirect_uri),
        cc = urlencoding(&challenge),
        st = urlencoding(&state_nonce),
        sc = urlencoding(scopes),
    );
    println!();
    println!("  {C_BOLD}→ Open this URL in a browser to consent:{C_RESET}");
    println!("    {C_CYAN}{authorize}{C_RESET}");
    println!();
    info("(waiting up to 3 minutes for the browser to bounce back to the loopback…)");

    // Try to open it for them; ignore errors since we already printed
    // the URL.
    let _ = try_open_browser(&authorize);

    let (code, state_recv) = tokio::time::timeout(
        Duration::from_secs(180),
        await_callback(listener),
    )
    .await
    .into_diagnostic()
    .wrap_err("OAuth flow timed out (3 min)")??;

    if state_recv != state_nonce {
        return Err(miette::miette!(
            "OAuth state mismatch — possible CSRF, retry"
        ));
    }

    // Exchange.
    let token_body = format!(
        "grant_type=authorization_code&code={c}&redirect_uri={ru}&client_id={cid}\
         &code_verifier={v}",
        c = urlencoding(&code),
        ru = urlencoding(&redirect_uri),
        cid = urlencoding(&client_id),
        v = urlencoding(&verifier),
    );
    let token_resp: serde_json::Value = client
        .post(format!("{endpoint}/oauth/token"))
        .header("content-type", "application/x-www-form-urlencoded")
        .body(token_body)
        .send()
        .await
        .into_diagnostic()?
        .error_for_status()
        .into_diagnostic()?
        .json()
        .await
        .into_diagnostic()?;
    let access_token = token_resp
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| miette::miette!("token response missing access_token"))?
        .to_owned();
    Ok(access_token)
}

async fn await_callback(listener: TcpListener) -> Result<(String, String)> {
    let (mut stream, _) = listener
        .accept()
        .await
        .into_diagnostic()
        .wrap_err("accept callback")?;
    let (read, mut write) = stream.split();
    let mut reader = BufReader::new(read);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .await
        .into_diagnostic()?;
    let mut discard = String::new();
    loop {
        discard.clear();
        let n = reader.read_line(&mut discard).await.into_diagnostic()?;
        if n == 0 || discard == "\r\n" || discard == "\n" {
            break;
        }
    }

    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        let _ = write_response(&mut write, "Malformed request").await;
        return Err(miette::miette!("malformed callback"));
    }
    let pq = parts[1];
    let query = pq.split_once('?').map_or("", |(_, q)| q);
    let mut code: Option<String> = None;
    let mut state: Option<String> = None;
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            let v = urldecode(v);
            match k {
                "code" => code = Some(v),
                "state" => state = Some(v),
                _ => {}
            }
        }
    }
    let (Some(c), Some(s)) = (code, state) else {
        let _ = write_response(&mut write, "Missing code or state").await;
        return Err(miette::miette!("callback missing code/state"));
    };
    let _ = write_response(
        &mut write,
        "Signed in. You can close this window and return to the terminal.",
    )
    .await;
    Ok((c, s))
}

async fn write_response<W: AsyncWriteExt + Unpin>(w: &mut W, msg: &str) -> std::io::Result<()> {
    let body = format!(
        "<!doctype html><html><body style='font-family:system-ui;padding:48px;text-align:center;\
         background:#0a0a0a;color:#e0e0e0'><h1 style='font-size:18px'>{msg}</h1></body></html>"
    );
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    w.write_all(resp.as_bytes()).await?;
    w.flush().await
}

fn try_open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd").args(["/C", "start", url]).spawn()?;
    }
    Ok(())
}

// ── Shared crypto + encoding helpers (small enough to inline) ─────────

fn http_client(timeout_secs: u64) -> Result<reqwest::Client> {
    let mut b = reqwest::Client::builder();
    if timeout_secs > 0 {
        b = b.timeout(Duration::from_secs(timeout_secs));
    }
    b.build().into_diagnostic().wrap_err("build http client")
}

fn random_url_safe_id(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS RNG must be available for PKCE");
    base64_url_no_pad(&buf)
}

fn pkce_s256(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    base64_url_no_pad(&hasher.finalize())
}

fn base64_url_no_pad(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
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
        let t = (b0 << 16) | (b1 << 8) | b2;
        out.push(A[((t >> 18) & 0x3F) as usize] as char);
        out.push(A[((t >> 12) & 0x3F) as usize] as char);
        if i + 1 < data.len() {
            out.push(A[((t >> 6) & 0x3F) as usize] as char);
        }
        if i + 2 < data.len() {
            out.push(A[(t & 0x3F) as usize] as char);
        }
        i += 3;
    }
    out
}

fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_nibble(b >> 4));
            out.push(hex_nibble(b & 0x0f));
        }
    }
    out
}

fn urldecode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'+' {
            out.push(b' ');
            i += 1;
        } else if b == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
            out.push(b);
            i += 1;
        } else {
            out.push(b);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'A' + (n - 10)) as char,
        _ => '0',
    }
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
