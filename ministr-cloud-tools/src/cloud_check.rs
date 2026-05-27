//! `ministr cloud check` — diagnostic that probes every wired cloud
//! integration and prints a tick/cross table.
//!
//! Reads the same env vars `cmd_serve_http` consumes, so a green run
//! here implies `serve --transport http --oauth` will boot cleanly.
//! Conversely a red row tells you exactly which integration needs
//! attention before you waste time on the full serve cycle.
//!
//! # SOLID layout
//!
//! - [`HealthCheck`] trait — one method, `async fn run`. Each concrete
//!   probe is a single-concern impl ([`PostgresCheck`],
//!   [`StripeCheck`], [`GitHubOAuthCheck`], [`GitHubAppCheck`],
//!   [`BaseUrlCheck`], [`BlobBackendCheck`]).
//! - [`CheckResult`] — the per-row outcome the table renders.
//! - Adding a probe = adding an impl + a line in [`build_checks`].
//!   No code outside the new impl changes (OCP).

use std::sync::Arc;

/// One-line outcome of a probe. The status colour and the help-text
/// drive the printed table.
#[derive(Debug)]
pub struct CheckResult {
    /// Short label rendered in the leftmost column.
    pub name: &'static str,
    /// Outcome — `Ok` / `NotConfigured` / `Fail`.
    pub status: CheckStatus,
    /// One-line human summary of what the probe found.
    pub message: String,
}

#[derive(Debug, Clone, Copy)]
pub enum CheckStatus {
    /// Probe passed.
    Ok,
    /// Probe couldn't run because the integration isn't configured.
    /// Not a failure — the cloud will mount without it.
    NotConfigured,
    /// Probe ran but found something off (e.g. credentials present
    /// but unable to authenticate). The cloud will refuse to mount.
    Fail,
}

impl CheckStatus {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Ok => "✓",
            Self::NotConfigured => "·",
            Self::Fail => "✗",
        }
    }
}

/// One probe. Implementations MUST be `Send + Sync` so the runner can
/// hold them in a `Vec<Arc<dyn HealthCheck>>` and stack-allocate a
/// future per probe.
pub trait HealthCheck: Send + Sync {
    fn run(&self) -> std::pin::Pin<Box<dyn Future<Output = CheckResult> + Send + '_>>;
}

// ── Concrete probes ───────────────────────────────────────────────────

/// Postgres reachability + schema-applied probe. Reads
/// `MINISTR_PG_URL`; reports `NotConfigured` when unset.
pub struct PostgresCheck;
impl HealthCheck for PostgresCheck {
    fn run(&self) -> std::pin::Pin<Box<dyn Future<Output = CheckResult> + Send + '_>> {
        Box::pin(async move {
            let Some(url) = trimmed_env("MINISTR_PG_URL") else {
                return CheckResult {
                    name: "postgres",
                    status: CheckStatus::NotConfigured,
                    message: "MINISTR_PG_URL not set — cloud will run in self-hosted (no tenancy) mode".into(),
                };
            };
            let pool = match ministr_cloud::connect(&url) {
                Ok(p) => p,
                Err(e) => {
                    return CheckResult {
                        name: "postgres",
                        status: CheckStatus::Fail,
                        message: format!("pool build failed: {e}"),
                    };
                }
            };
            // Probe: SELECT 1 round-trip + check whether the F1.2
            // `users` table exists (proves migrations have run).
            let client = match pool.get().await {
                Ok(c) => c,
                Err(e) => {
                    return CheckResult {
                        name: "postgres",
                        status: CheckStatus::Fail,
                        message: format!("connection failed: {e}"),
                    };
                }
            };
            match client.query_one("SELECT to_regclass('public.users')::text AS rel", &[]).await {
                Ok(row) => {
                    let rel: Option<String> = row.get("rel");
                    if rel.is_some() {
                        CheckResult {
                            name: "postgres",
                            status: CheckStatus::Ok,
                            message: "reachable + F1.2 schema present".into(),
                        }
                    } else {
                        CheckResult {
                            name: "postgres",
                            status: CheckStatus::Fail,
                            message: "reachable but F1.2 schema missing — start `ministr serve …` once to auto-migrate".into(),
                        }
                    }
                }
                Err(e) => CheckResult {
                    name: "postgres",
                    status: CheckStatus::Fail,
                    message: format!("query failed: {e}"),
                },
            }
        })
    }
}

/// Stripe API-key probe. Performs a no-op `GET /v1/balance` to
/// confirm the key is valid; treats 401 as Fail, transport errors
/// as Fail, 200 as Ok.
pub struct StripeCheck;
impl HealthCheck for StripeCheck {
    fn run(&self) -> std::pin::Pin<Box<dyn Future<Output = CheckResult> + Send + '_>> {
        Box::pin(async move {
            let Some(key) = trimmed_env("MINISTR_STRIPE_SECRET_KEY") else {
                return CheckResult {
                    name: "stripe",
                    status: CheckStatus::NotConfigured,
                    message: "MINISTR_STRIPE_SECRET_KEY not set — billing handlers disabled".into(),
                };
            };
            let client = match reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(8))
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    return CheckResult {
                        name: "stripe",
                        status: CheckStatus::Fail,
                        message: format!("http client build failed: {e}"),
                    };
                }
            };
            let resp = client
                .get("https://api.stripe.com/v1/balance")
                .basic_auth(&key, Some(""))
                .send()
                .await;
            match resp {
                Ok(r) if r.status().is_success() => {
                    let mode = if key.starts_with("sk_test_") {
                        "test mode"
                    } else if key.starts_with("sk_live_") {
                        "LIVE mode"
                    } else {
                        "unknown key prefix"
                    };
                    CheckResult {
                        name: "stripe",
                        status: CheckStatus::Ok,
                        message: format!("authenticated ({mode})"),
                    }
                }
                Ok(r) => CheckResult {
                    name: "stripe",
                    status: CheckStatus::Fail,
                    message: format!("stripe returned HTTP {}", r.status()),
                },
                Err(e) => CheckResult {
                    name: "stripe",
                    status: CheckStatus::Fail,
                    message: format!("transport: {e}"),
                },
            }
        })
    }
}

/// GitHub OAuth App credentials probe. Doesn't actually exchange a
/// token (no live user) — only confirms both env vars are non-empty
/// and the base URL is set (without it the callback can't be built).
pub struct GitHubOAuthCheck;
impl HealthCheck for GitHubOAuthCheck {
    fn run(&self) -> std::pin::Pin<Box<dyn Future<Output = CheckResult> + Send + '_>> {
        Box::pin(async move {
            let cid = trimmed_env("MINISTR_GITHUB_CLIENT_ID");
            let secret = trimmed_env("MINISTR_GITHUB_CLIENT_SECRET");
            let base = trimmed_env("MINISTR_CLOUD_BASE_URL");
            match (cid, secret, base) {
                (None, None, _) => CheckResult {
                    name: "github oauth",
                    status: CheckStatus::NotConfigured,
                    message: "MINISTR_GITHUB_CLIENT_ID/_SECRET not set — sign-in flow disabled".into(),
                },
                (Some(_), Some(_), Some(b)) => CheckResult {
                    name: "github oauth",
                    status: CheckStatus::Ok,
                    message: format!("credentials present; callback {b}/auth/github/callback"),
                },
                (Some(_), Some(_), None) => CheckResult {
                    name: "github oauth",
                    status: CheckStatus::Fail,
                    message: "credentials present but MINISTR_CLOUD_BASE_URL missing — callback URL cannot be built".into(),
                },
                _ => CheckResult {
                    name: "github oauth",
                    status: CheckStatus::Fail,
                    message: "only one of MINISTR_GITHUB_CLIENT_ID / _SECRET is set".into(),
                },
            }
        })
    }
}

/// GitHub App probe — parses the private-key PEM to confirm it's a
/// real RSA key. Doesn't actually call the GitHub App API (no
/// installation ID to test against here).
pub struct GitHubAppCheck;
impl HealthCheck for GitHubAppCheck {
    fn run(&self) -> std::pin::Pin<Box<dyn Future<Output = CheckResult> + Send + '_>> {
        Box::pin(async move {
            let id = trimmed_env("MINISTR_GITHUB_APP_ID");
            let pem = std::env::var("MINISTR_GITHUB_APP_PRIVATE_KEY").ok();
            match (id, pem) {
                (None, None) => CheckResult {
                    name: "github app",
                    status: CheckStatus::NotConfigured,
                    message: "MINISTR_GITHUB_APP_ID/_PRIVATE_KEY not set — private-repo clones via App disabled".into(),
                },
                (Some(_), Some(p)) if p.trim().is_empty() => CheckResult {
                    name: "github app",
                    status: CheckStatus::Fail,
                    message: "MINISTR_GITHUB_APP_PRIVATE_KEY is empty".into(),
                },
                (Some(id), Some(pem)) => {
                    match ministr_cloud::GitHubAppClient::new(id.clone(), &pem) {
                        Ok(_) => CheckResult {
                            name: "github app",
                            status: CheckStatus::Ok,
                            message: format!("app_id={id}; private key parses as RSA"),
                        },
                        Err(e) => CheckResult {
                            name: "github app",
                            status: CheckStatus::Fail,
                            message: format!("private key rejected: {e}"),
                        },
                    }
                }
                _ => CheckResult {
                    name: "github app",
                    status: CheckStatus::Fail,
                    message: "only one of MINISTR_GITHUB_APP_ID / _PRIVATE_KEY is set".into(),
                },
            }
        })
    }
}

/// Base URL probe — required when GitHub sign-in is wired (and
/// useful in general so Stripe success/cancel URLs are well-formed).
pub struct BaseUrlCheck;
impl HealthCheck for BaseUrlCheck {
    fn run(&self) -> std::pin::Pin<Box<dyn Future<Output = CheckResult> + Send + '_>> {
        Box::pin(async move {
            match trimmed_env("MINISTR_CLOUD_BASE_URL") {
                None => CheckResult {
                    name: "base url",
                    status: CheckStatus::NotConfigured,
                    message: "MINISTR_CLOUD_BASE_URL not set".into(),
                },
                Some(url) if url.starts_with("http://") || url.starts_with("https://") => {
                    CheckResult {
                        name: "base url",
                        status: CheckStatus::Ok,
                        message: url,
                    }
                }
                Some(url) => CheckResult {
                    name: "base url",
                    status: CheckStatus::Fail,
                    message: format!("must start with http:// or https://, got `{url}`"),
                },
            }
        })
    }
}

/// Blob backend probe — builds the env-var-selected backend and runs
/// `ensure_container` to confirm the underlying storage is reachable
/// (a directory we can create for filesystem; a container we can
/// access for Azure).
pub struct BlobBackendCheck;
impl HealthCheck for BlobBackendCheck {
    fn run(&self) -> std::pin::Pin<Box<dyn Future<Output = CheckResult> + Send + '_>> {
        Box::pin(async move {
            match ministr_cloud::build_blob_backend_from_env() {
                Ok(None) => CheckResult {
                    name: "blob storage",
                    status: CheckStatus::NotConfigured,
                    message:
                        "MINISTR_BLOB_STORE_KIND / MINISTR_BLOB_FS_ROOT not set — no blob persistence"
                            .into(),
                },
                Ok(Some(backend)) => match backend.ensure_container().await {
                    Ok(()) => {
                        let label = match &backend {
                            ministr_cloud::BlobBackend::Azure(_) => "azure",
                            ministr_cloud::BlobBackend::Filesystem(_) => "filesystem",
                        };
                        CheckResult {
                            name: "blob storage",
                            status: CheckStatus::Ok,
                            message: format!("backend={label}; container/root reachable"),
                        }
                    }
                    Err(e) => CheckResult {
                        name: "blob storage",
                        status: CheckStatus::Fail,
                        message: format!("ensure_container failed: {e}"),
                    },
                },
                Err(e) => CheckResult {
                    name: "blob storage",
                    status: CheckStatus::Fail,
                    message: format!("backend build failed: {e}"),
                },
            }
        })
    }
}

// ── Runner ────────────────────────────────────────────────────────────

/// Build the standard probe set. Adding a probe = adding it here.
fn build_checks() -> Vec<Arc<dyn HealthCheck>> {
    vec![
        Arc::new(BaseUrlCheck),
        Arc::new(PostgresCheck),
        Arc::new(BlobBackendCheck),
        Arc::new(GitHubOAuthCheck),
        Arc::new(GitHubAppCheck),
        Arc::new(StripeCheck),
    ]
}

/// Entrypoint for `ministr cloud check`. Runs every probe sequentially
/// (parallel would only save 1-2 seconds and the output is cleaner
/// sequential) and prints the tick/cross table. Returns the number of
/// failed probes — the CLI uses this as the exit code so CI can
/// gate on `just dev-cloud-check`.
pub async fn run_all() -> usize {
    let checks = build_checks();
    let mut results = Vec::with_capacity(checks.len());
    for check in &checks {
        results.push(check.run().await);
    }
    print_table(&results);
    results
        .iter()
        .filter(|r| matches!(r.status, CheckStatus::Fail))
        .count()
}

fn print_table(results: &[CheckResult]) {
    let name_w = results
        .iter()
        .map(|r| r.name.len())
        .max()
        .unwrap_or(0)
        .max(8);
    println!();
    println!("  {:<width$}      status", "service", width = name_w);
    println!("  {:-<width$}  --  {:-<60}", "", "", width = name_w);
    for r in results {
        println!(
            "  {:<width$}  {}   {}",
            r.name,
            r.status.glyph(),
            r.message,
            width = name_w,
        );
    }
    println!();
    let ok = results.iter().filter(|r| matches!(r.status, CheckStatus::Ok)).count();
    let not_cfg = results
        .iter()
        .filter(|r| matches!(r.status, CheckStatus::NotConfigured))
        .count();
    let fail = results.iter().filter(|r| matches!(r.status, CheckStatus::Fail)).count();
    println!("  {ok} ok · {not_cfg} not configured · {fail} failed");
    println!();
}

fn trimmed_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}
