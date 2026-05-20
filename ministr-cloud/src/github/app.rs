//! GitHub App installation-token minter (F2.1).
//!
//! Implements [`ministr_api::InstallationTokenMinter`] for the cloud.
//! Cloud deployments configure the App via
//! `MINISTR_GITHUB_APP_ID` + `MINISTR_GITHUB_APP_PRIVATE_KEY` (PEM
//! contents); the daemon's `AppState.installation_minter` slot picks
//! up an `Arc<GitHubAppClient>` and the clone handler awaits it on
//! every `github_installation_id` request.
//!
//! # Auth flow recap
//!
//! 1. Sign a JWT as the App. `alg=RS256`, `iss=<app_id>`,
//!    `iat=<now-60s>`, `exp=<now+540s>`. GitHub allows up to 600s of
//!    JWT lifetime; we use 540s to leave 60s of clock skew. The
//!    `iat-60s` backdate is GitHub's documented recommendation to
//!    survive small clock drift on the cloud-pod side.
//! 2. `POST /app/installations/{installation_id}/access_tokens` with
//!    `Authorization: Bearer <JWT>` returns
//!    `{ "token": "ghs_…", "expires_at": "<RFC3339>" }`. GitHub
//!    issues tokens that expire after about an hour.
//! 3. Cache the `(token, expires_at)` keyed by `installation_id`.
//!    Subsequent calls within the TTL window skip the network
//!    round-trip entirely — important because re-signing a JWT on
//!    every clone is wasteful when 95% of clones in a session re-use
//!    the same installation.
//!
//! # Why the cache TTL is 50 minutes, not 60
//!
//! GitHub returns the expiry verbatim, but the daemon's git clone step
//! takes time (seconds to minutes for large repos). Returning a token
//! that's about to expire risks the clone half-way through. We hold
//! back the last 10 minutes — the cache evicts proactively at
//! `expires_at - 10min` so consumers always see a token good for the
//! whole clone.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{EncodingKey, Header};
use ministr_api::{InstallationTokenMinter, MintError};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

/// JWT lifetime — 9 minutes. GitHub allows up to 10; the buffer leaves
/// room for clock skew between the cloud pod and api.github.com.
const JWT_LIFETIME_SECS: i64 = 540;

/// Backdate the `iat` claim by this much to absorb clock drift in the
/// other direction — matches GitHub's official recommendation in
/// "Generating a JSON Web Token (JWT) for a GitHub App".
const JWT_IAT_BACKDATE_SECS: i64 = 60;

/// How long before a cached token's nominal expiry to evict it. Keeps
/// a long-running clone from racing the actual expiry.
const CACHE_PROACTIVE_EVICT_SECS: u64 = 10 * 60;

/// HTTP timeout for the GitHub access-token exchange. Generous — Azure
/// → api.github.com is usually <200ms but a cold connection on a
/// fresh pod sometimes takes a second.
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// Errors surfaced internally by [`GitHubAppClient`]. Mapped to
/// [`MintError`] at the trait boundary so the daemon never sees this
/// type directly.
#[derive(Debug, thiserror::Error)]
pub enum GitHubAppError {
    /// Empty `app_id`, missing PEM, malformed key, or `reqwest::Client`
    /// build failure.
    #[error("github app setup: {0}")]
    Setup(String),
    /// JWT signing failed — almost always a malformed private key
    /// caught after the build step (e.g. an Ed25519 key passed to an
    /// RS256 signer).
    #[error("github app jwt: {0}")]
    Jwt(String),
    /// Network-layer failure talking to api.github.com.
    #[error("github app transport: {0}")]
    Transport(String),
    /// GitHub returned non-2xx or a malformed access-token body.
    #[error("github app protocol: {0}")]
    Protocol(String),
}

impl From<GitHubAppError> for MintError {
    fn from(e: GitHubAppError) -> Self {
        match e {
            GitHubAppError::Setup(m) | GitHubAppError::Jwt(m) => MintError::Setup(m),
            GitHubAppError::Transport(m) => MintError::Transport(m),
            GitHubAppError::Protocol(m) => MintError::Protocol(m),
        }
    }
}

/// Outbound client for the GitHub App `installations/.../access_tokens`
/// endpoint. Holds the App's RSA private key once and reuses it across
/// requests; the cache layer skips JWT signing when an unexpired token
/// is already on hand.
#[derive(Clone)]
pub struct GitHubAppClient {
    app_id: String,
    encoding_key: Arc<EncodingKey>,
    http: reqwest::Client,
    base_url: String,
    cache: Arc<Mutex<HashMap<String, CachedToken>>>,
}

impl std::fmt::Debug for GitHubAppClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `encoding_key` + `http` are intentionally omitted — they
        // would leak nothing useful and the `EncodingKey` doesn't
        // implement `Debug` anyway. `finish_non_exhaustive` keeps the
        // contract explicit (and Clippy's `missing_fields_in_debug`
        // happy).
        f.debug_struct("GitHubAppClient")
            .field("app_id", &self.app_id)
            .field("base_url", &self.base_url)
            .field("cached_installations", &self.cache.lock().len())
            .finish_non_exhaustive()
    }
}

/// In-process token cache entry. `expires_at` is the GitHub-reported
/// epoch deadline minus our proactive-evict window.
#[derive(Debug, Clone)]
struct CachedToken {
    token: String,
    usable_until: u64,
}

#[derive(Debug, Serialize)]
struct JwtClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    token: String,
    expires_at: String,
}

impl GitHubAppClient {
    /// Build the client. `app_id` is the numeric GitHub App ID from the
    /// App settings page; `private_key_pem` is the multi-line PEM
    /// contents downloaded from GitHub (begins with `-----BEGIN RSA
    /// PRIVATE KEY-----` or `-----BEGIN PRIVATE KEY-----`).
    ///
    /// # Errors
    ///
    /// [`GitHubAppError::Setup`] for an empty `app_id`, malformed PEM,
    /// or `reqwest::Client` construction failure.
    pub fn new(
        app_id: impl Into<String>,
        private_key_pem: &str,
    ) -> Result<Self, GitHubAppError> {
        Self::with_base_url(app_id, private_key_pem, "https://api.github.com")
    }

    /// Test-only constructor that points the client at a local mock
    /// server. Production code calls [`Self::new`].
    ///
    /// # Errors
    ///
    /// Same as [`Self::new`].
    pub fn with_base_url(
        app_id: impl Into<String>,
        private_key_pem: &str,
        base_url: impl Into<String>,
    ) -> Result<Self, GitHubAppError> {
        let app_id = app_id.into();
        if app_id.trim().is_empty() {
            return Err(GitHubAppError::Setup("app_id is empty".into()));
        }
        let encoding_key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
            .map_err(|e| GitHubAppError::Setup(format!("parse private key PEM: {e}")))?;
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent("ministr-cloud-github-app/1 (+https://ministr.ai)")
            .build()
            .map_err(|e| GitHubAppError::Setup(format!("build http: {e}")))?;
        Ok(Self {
            app_id,
            encoding_key: Arc::new(encoding_key),
            http,
            base_url: trim_trailing_slashes(base_url.into()),
            cache: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Sign a fresh App-level JWT. Public for testability — the trait
    /// method also calls it but with an internal cache wrapper.
    ///
    /// # Errors
    ///
    /// [`GitHubAppError::Jwt`] if signing fails (almost always a key
    /// algorithm mismatch).
    pub fn sign_jwt(&self) -> Result<String, GitHubAppError> {
        let now = i64::try_from(epoch_now()).unwrap_or(i64::MAX);
        let claims = JwtClaims {
            iat: now - JWT_IAT_BACKDATE_SECS,
            exp: now + JWT_LIFETIME_SECS,
            iss: self.app_id.clone(),
        };
        let header = Header::new(jsonwebtoken::Algorithm::RS256);
        jsonwebtoken::encode(&header, &claims, &self.encoding_key)
            .map_err(|e| GitHubAppError::Jwt(format!("sign: {e}")))
    }

    /// Inner mint that wraps cache + network. Public for tests; the
    /// trait impl funnels through here.
    ///
    /// # Errors
    ///
    /// Maps from [`GitHubAppError`] at the boundary; see the inner
    /// variants for the failure surface.
    pub async fn mint_installation_token(
        &self,
        installation_id: &str,
    ) -> Result<String, GitHubAppError> {
        if let Some(cached) = self.cache_lookup(installation_id) {
            debug!(installation_id, "github app token cache hit");
            return Ok(cached);
        }

        let jwt = self.sign_jwt()?;
        let url = format!(
            "{}/app/installations/{installation_id}/access_tokens",
            self.base_url
        );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&jwt)
            .header("accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(|e| GitHubAppError::Transport(format!("access_tokens: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(GitHubAppError::Protocol(format!(
                "access_tokens status {status}: {body}"
            )));
        }
        let parsed: AccessTokenResponse = resp
            .json()
            .await
            .map_err(|e| GitHubAppError::Protocol(format!("access_tokens parse: {e}")))?;

        let usable_until = parse_rfc3339_to_epoch(&parsed.expires_at).map_or_else(
            || epoch_now() + 3000, // fallback: assume ~50 min from now
            |epoch| epoch.saturating_sub(CACHE_PROACTIVE_EVICT_SECS),
        );
        self.cache_store(installation_id, parsed.token.clone(), usable_until);
        info!(
            installation_id,
            "github app token minted; cached until {usable_until}"
        );
        Ok(parsed.token)
    }

    fn cache_lookup(&self, installation_id: &str) -> Option<String> {
        let now = epoch_now();
        let mut cache = self.cache.lock();
        if let Some(entry) = cache.get(installation_id) {
            if now < entry.usable_until {
                return Some(entry.token.clone());
            }
            cache.remove(installation_id);
        }
        None
    }

    fn cache_store(&self, installation_id: &str, token: String, usable_until: u64) {
        self.cache.lock().insert(
            installation_id.to_owned(),
            CachedToken {
                token,
                usable_until,
            },
        );
    }

    /// Drop a single installation from the cache — useful when the
    /// caller observes a 401 from GitHub mid-clone (e.g. the
    /// installation was suspended by the repo owner).
    pub fn invalidate(&self, installation_id: &str) {
        if self.cache.lock().remove(installation_id).is_some() {
            warn!(installation_id, "github app token cache invalidated");
        }
    }
}

impl InstallationTokenMinter for GitHubAppClient {
    fn mint<'a>(
        &'a self,
        installation_id: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<String, MintError>> + Send + 'a>> {
        Box::pin(async move {
            self.mint_installation_token(installation_id)
                .await
                .map_err(MintError::from)
        })
    }
}

fn epoch_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn trim_trailing_slashes(mut s: String) -> String {
    while s.ends_with('/') {
        s.pop();
    }
    s
}

/// Parse a GitHub timestamp like `2026-05-19T10:34:50Z` to an epoch
/// second. Returns `None` for malformed input — caller falls back to a
/// safe default. Implemented manually so this crate doesn't grow a
/// `chrono`/`time` dep just for one parse site.
fn parse_rfc3339_to_epoch(s: &str) -> Option<u64> {
    // Expected shape: YYYY-MM-DDTHH:MM:SSZ (optional fractional seconds
    // and offset are ignored — GitHub returns plain `Z`).
    let s = s.trim();
    if s.len() < 20 || !s.ends_with('Z') {
        return None;
    }
    let year: i64 = s[0..4].parse().ok()?;
    let month: u32 = s[5..7].parse().ok()?;
    let day: u32 = s[8..10].parse().ok()?;
    let hour: u32 = s[11..13].parse().ok()?;
    let minute: u32 = s[14..16].parse().ok()?;
    let second: u32 = s[17..19].parse().ok()?;

    // Days from civil (Howard Hinnant's algorithm). Returns days since
    // 1970-01-01, accurate for the entire range of practical interest.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m: i64 = month.into();
    let d: i64 = day.into();
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days_since_epoch = era * 146_097 + doe - 719_468;

    let secs = days_since_epoch * 86_400
        + i64::from(hour) * 3600
        + i64::from(minute) * 60
        + i64::from(second);
    u64::try_from(secs).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2048-bit RSA test key — generated specifically for these unit
    /// tests, never used in production, never imported by anything
    /// outside this module. Kept inline (not under `tests/fixtures/`)
    /// so secret-scanners don't false-positive on a bare `.pem` file
    /// in the tree and so the test surface is self-contained.
    const TEST_PRIVATE_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----\n\
        MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQDYzKfoWY290yvl\n\
        xVHn71J2Frs//oaoAfk+oRopOl4rZCGbHS6fkkobZIkLnjG0Pr7SPRtdJlW23EBI\n\
        zv6aR8AfC3leTZ+yWySVDecqGDmg2mnMcSrDlH4iEgMv3oQl9k10CRzYbymrGWkv\n\
        YIOfnPkCjxNRSkx3AusRnSirvFyC59qqR0llTI3jiLgdBlJANPdeauDsOdSUovUU\n\
        D7jmYZanOTZ0iWmP6vMT/i/6pMPvLxw0tYad+iEEG0EUUXWq2rydXjl/PNwVXaPW\n\
        x1X6EerI0GV3rA3QVxtvTd1b0QpcKh3+QAbZERTPzMc2BZitZMPtkKBFx7tQGRyA\n\
        OqU9l2zLAgMBAAECggEAEuMBXa8vjJybnmYU6e0MygIrneTr5jGwfPxGmIDf400o\n\
        gFLdKkRG9cv0WcbA8xWO73x9/cdxKtT/5K05EPJfPP/K5aRC3U7Ys64vt+2/AicE\n\
        6y07WdPTM99N18W9eBvU84mNBxiSu2J4qgqbwjPynX50DbON+xrjT6LZVYdl/SFR\n\
        7s5nj0vlic3Uj2mL/q4VguHWAt0D8T8uqru49QN9oy8QIgsGeESQmBPennFVrabY\n\
        kFxe56uk0i/25qaNBNgkPtq51gU27NlMSQCWTkk5C1mCZrG7ukVU3biWOdgHpHb5\n\
        UPE/tpHNBiJQuxWWBMnBTumgExlKZ3xi5e5qfBcKAQKBgQD3965YGmpr9Ka4aLGe\n\
        ySStGoam6vZ2njIoazVuARBVyhQkK5H3ZzLOdyPCqdtiGsFpWhHRxyzeZoQYoHra\n\
        0S3Tle7dq0MSSnKffHNkXc1/6dPxu9NJ3T8C3w+ryBBxhsb7wcaWd8humpFPfRBV\n\
        RQOkD2andZxVdTDWDHPs7DjgKwKBgQDf0oHwUchygc2gK/xsyfPDlfItHqlLYzqj\n\
        UmpyzJsmBRwTEopBWKaHDM9djFWxRRmZ5kFz2cvc9WEu2hDKaBdmBmes3PxvqegY\n\
        sjuVHRDLCif0+un+mT/U/SqG12ND+esQqcQkGjDuyj8bDXM9rnUtlqYhnSveWN57\n\
        rTsLEVS14QKBgQCnJy09csEeeNMSKHDjis/QaLswNd9iYo2JNYvU1Z6/VfNx1nUV\n\
        A1n6V9GhXYLnhQWwEOlGMi+K1CxjtXpbmvp7UOyuPM5/u/O8ktXuaFUozuTyZRyv\n\
        BBd/xgH4WGrNPH9SInPN5n0UIdmmbbXe5SDpLQCUDfIOoWsEP2y93xcP9QKBgE05\n\
        gIO+c/6uMphVFN8kPur4zXor3hWYwx6ezQOW/OD9WlZqSzGIuMxX6yRHyzlCsjab\n\
        b3Hdb61pLILR0oFDsO8Ovq6yAJc2dFIxDMXCJY0oj+jCugGSNqfyQb4Mir9ld2lk\n\
        abxbHQ8G0QcweNaLXvq/w8pNRFmPKBRcDMcgz62BAoGBALAoGDV3kGOlWDK6F4sB\n\
        b2NNv1En/vwx4XoPo0dSDlZw47ksbdS6zp3CyqfX1ahWnXSRofQZ0tD1nUp4OYOD\n\
        T82E90bnm6T0fm1CPDTVQTAMsOyl4Moxju0FF8TC1pzK4fX5WnvFTKY0GYh1PADf\n\
        eCDc05OwRBI2jd3lddm/Tgar\n\
        -----END PRIVATE KEY-----\n";

    #[test]
    fn rfc3339_parser_handles_canonical_github_shape() {
        assert_eq!(
            parse_rfc3339_to_epoch("1970-01-01T00:00:00Z"),
            Some(0)
        );
        // 2026-05-19 10:34:50 UTC: 56*365 days + 14 leap days +
        // Jan(31)+Feb(28)+Mar(31)+Apr(30) + 18 days into May + 10h34m50s.
        // Cross-checked against Python's calendar.timegm.
        assert_eq!(
            parse_rfc3339_to_epoch("2026-05-19T10:34:50Z"),
            Some(1_779_186_890)
        );
    }

    #[test]
    fn rfc3339_parser_rejects_malformed_input() {
        assert!(parse_rfc3339_to_epoch("not a date").is_none());
        assert!(parse_rfc3339_to_epoch("2026-05-19").is_none());
        assert!(parse_rfc3339_to_epoch("2026-05-19T10:34:50+02:00").is_none());
    }

    #[test]
    fn new_rejects_empty_app_id() {
        assert!(matches!(
            GitHubAppClient::new("", TEST_PRIVATE_KEY_PEM),
            Err(GitHubAppError::Setup(_))
        ));
    }

    #[test]
    fn new_rejects_malformed_pem() {
        assert!(matches!(
            GitHubAppClient::new("12345", "not a pem"),
            Err(GitHubAppError::Setup(_))
        ));
    }

    #[test]
    fn sign_jwt_produces_three_dot_separated_segments() {
        let client = GitHubAppClient::new("12345", TEST_PRIVATE_KEY_PEM).expect("build client");
        let jwt = client.sign_jwt().expect("sign succeeds");
        let segments: Vec<&str> = jwt.split('.').collect();
        assert_eq!(segments.len(), 3, "jwt must have header.payload.signature");
        // Header MUST decode to a JSON object containing alg=RS256.
        let header_json = base64_url_decode(segments[0]);
        let header_str = String::from_utf8_lossy(&header_json);
        assert!(header_str.contains("RS256"), "header: {header_str}");
        // Payload MUST contain iss = app_id.
        let payload_json = base64_url_decode(segments[1]);
        let payload_str = String::from_utf8_lossy(&payload_json);
        assert!(payload_str.contains("\"iss\":\"12345\""), "payload: {payload_str}");
    }

    fn base64_url_decode(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf: u32 = 0;
        let mut bits: u32 = 0;
        for c in s.bytes() {
            let v: u32 = match c {
                b'A'..=b'Z' => u32::from(c - b'A'),
                b'a'..=b'z' => u32::from(c - b'a') + 26,
                b'0'..=b'9' => u32::from(c - b'0') + 52,
                b'-' => 62,
                b'_' => 63,
                b'=' => break,
                _ => continue,
            };
            buf = (buf << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                #[allow(clippy::cast_possible_truncation)]
                let byte = ((buf >> bits) & 0xff) as u8;
                out.push(byte);
            }
        }
        out
    }

    #[tokio::test]
    async fn mint_installation_token_round_trips_against_local_mock() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = stream.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = "{\"token\":\"ghs_test_token\",\"expires_at\":\"2099-12-31T23:59:59Z\"}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
            req
        });

        let base = format!("http://{addr}");
        let client = GitHubAppClient::with_base_url("12345", TEST_PRIVATE_KEY_PEM, base)
            .expect("build client");
        let token = client
            .mint_installation_token("78910")
            .await
            .expect("mint succeeds");
        assert_eq!(token, "ghs_test_token");

        let req = server.await.unwrap();
        assert!(req.starts_with("POST /app/installations/78910/access_tokens"));
        assert!(req.to_ascii_lowercase().contains("authorization: bearer "));
    }

    #[tokio::test]
    async fn mint_caches_token_within_ttl() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Server answers once. If the cache works, the SECOND mint call
        // doesn't hit the server, the task stays parked, and the join
        // below succeeds because we abort it explicitly.
        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let _ = stream.read(&mut buf).await.unwrap_or(0);
            let body = "{\"token\":\"ghs_cached\",\"expires_at\":\"2099-12-31T23:59:59Z\"}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(resp.as_bytes()).await.unwrap();
            stream.flush().await.unwrap();
        });

        let base = format!("http://{addr}");
        let client = GitHubAppClient::with_base_url("12345", TEST_PRIVATE_KEY_PEM, base)
            .expect("build client");
        let a = client.mint_installation_token("42").await.unwrap();
        let b = client.mint_installation_token("42").await.unwrap();
        assert_eq!(a, "ghs_cached");
        assert_eq!(b, "ghs_cached");
        server.abort();
    }

    #[tokio::test]
    async fn dyn_trait_dispatch_works() {
        // Compile-time + runtime proof the cloud client slots into the
        // open-core trait without surprises.
        let client = GitHubAppClient::new("12345", TEST_PRIVATE_KEY_PEM).expect("build client");
        let minter: Arc<dyn InstallationTokenMinter> = Arc::new(client);
        // Don't actually mint (no server here); just confirm the
        // dynamic dispatch compiles and the future is Send.
        let fut = minter.mint("1");
        // Drop without polling — we only needed the type-check.
        drop(fut);
    }
}
