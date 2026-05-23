//! ministr-cli — binary entry point for the ministr MCP server.
//!
//! Provides subcommands: `serve` (default), `index`, `status`, `search`,
//! `init`, `export`, `import`, and `hooks test`.
//!
//! This module handles CLI argument parsing and dispatch. Implementation
//! lives in:
//! - [`commands`] — subcommand handlers
//! - [`infra`] — shared infrastructure setup (storage, embedder, index)
//! - [`ingestion`] — corpus ingestion orchestration and file watching

mod cloud_check;
mod cloud_demo;
mod commands;
mod infra;
mod ingestion;
mod worker;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use miette::Result;

/// ministr — a code intelligence MCP server for AI coding agents.
///
/// Runs an MCP server that provides intelligent context retrieval
/// tools (survey, read, extract) for a local document corpus.
/// Supports stdio and Streamable HTTP transports.
#[derive(Parser, Debug)]
#[command(name = "ministr", version, about)]
struct Cli {
    /// Corpus sources: local paths, `https://` URLs, or `github://` URLs.
    ///
    /// Accepts multiple values via repeated flags:
    /// `ministr --corpus ./docs --corpus https://docs.rs/serde`
    #[arg(short, long, global = true)]
    corpus: Vec<String>,

    /// Path to config file (default: ~/.ministr/config.toml).
    #[arg(short = 'C', long, global = true)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Start the MCP server (default when no subcommand is given).
    ///
    /// By default uses stdio transport. Use `--transport http` to start
    /// a Streamable HTTP server for remote/multi-client deployments.
    Serve {
        /// Transport: `stdio` (default) or `http` (Streamable HTTP).
        #[arg(short, long, default_value = "stdio")]
        transport: Transport,

        /// Host to bind the HTTP server to (only used with `--transport http`).
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port for the HTTP server (only used with `--transport http`).
        #[arg(short, long, default_value_t = 8080)]
        port: u16,

        /// Deprecated, no-op (kept for backward compatibility).
        ///
        /// stdio now ALWAYS runs as a thin proxy to the ministr daemon
        /// (auto-spawned if not running); the old monolithic server was
        /// removed. This flag is accepted but has no effect.
        #[arg(long, hide = true)]
        proxy: bool,

        /// Enable OAuth 2.1 authentication for the HTTP transport.
        ///
        /// When enabled, the server exposes OAuth discovery endpoints and
        /// requires Bearer token authentication on the MCP endpoint.
        /// Only effective with `--transport http`.
        #[arg(long)]
        oauth: bool,

        /// OAuth issuer URL (default: `http://<host>:<port>`).
        ///
        /// Used in OAuth metadata discovery responses. Set this to the
        /// public-facing URL when deploying behind a reverse proxy.
        #[arg(long)]
        oauth_issuer: Option<String>,
    },

    /// Run corpus ingestion synchronously and exit (no MCP server).
    ///
    /// Useful for pre-warming the index, debugging ingestion issues,
    /// or running in CI pipelines.
    Index,

    /// Internal: run the headless ministr daemon. Auto-spawned by the
    /// MCP proxy (and desktop app); not intended for direct use.
    #[command(hide = true, name = "__daemon")]
    Daemon,

    /// Show daemon status (requires the ministr daemon to be running).
    Status,

    /// Search the corpus via the daemon (requires the ministr daemon to
    /// be running).
    Search {
        /// Search query.
        query: String,
        /// Maximum results.
        #[arg(short = 'k', long, default_value_t = 10)]
        top_k: usize,
    },

    /// Generate .ministr.toml with auto-detected project settings.
    ///
    /// Scans the current directory for project manifests (Cargo.toml,
    /// package.json, pyproject.toml), detects workspace layouts and
    /// bridge frameworks, and writes a sensible default config.
    Init {
        /// Overwrite existing .ministr.toml if present.
        #[arg(long)]
        force: bool,

        /// Run interactive setup: show the detected project type and
        /// exactly what will be written, then confirm before scaffolding.
        #[arg(long, short)]
        interactive: bool,
    },

    /// Export the corpus index to a portable `.ministr-index` bundle.
    ///
    /// Creates a zstd-compressed archive containing the content database
    /// (with session-local data stripped), HNSW vector index, and metadata
    /// manifest. The bundle can be imported on another machine without
    /// re-parsing or re-embedding.
    Export {
        /// Output file path (default: `<corpus-name>.ministr-index` in current dir).
        #[arg(short, long)]
        output: Option<PathBuf>,
    },

    /// Import a `.ministr-index` bundle into the local corpus store.
    ///
    /// Decompresses the bundle and loads the content database and HNSW
    /// index into the corpus data directory, ready for querying without
    /// re-indexing.
    Import {
        /// Path to the `.ministr-index` bundle file.
        bundle: PathBuf,
    },

    /// Manage ministr agent hooks.
    Hooks {
        #[command(subcommand)]
        action: HooksAction,
    },

    /// Add the `ministr` binary's directory to the user's PATH.
    ///
    /// Detects installed shells (bash, zsh, fish, nushell, `PowerShell`, tcsh,
    /// xonsh) and writes the appropriate rc file edits via the `onpath` crate.
    /// On Windows, writes the per-user `HKCU\Environment\PATH` registry entry.
    ///
    /// Idempotent — re-running won't duplicate entries. Used by `install.sh`
    /// and the Tauri desktop app's first-run setup.
    Setup {
        /// Directory to add to (or remove from) PATH.
        ///
        /// Default: parent of the running `ministr` binary.
        #[arg(long)]
        bin_dir: Option<PathBuf>,

        /// Print what would be edited, don't write.
        #[arg(long)]
        dry_run: bool,

        /// Remove the directory from PATH instead of adding it.
        ///
        /// Used by the NSIS uninstaller hook before tearing down the
        /// install dir. Idempotent — no-op if the dir isn't on PATH.
        #[arg(long)]
        uninstall: bool,
    },

    /// Atlas operator commands (F2.6+). Invoked by the weekly cron
    /// in cloud deployments and by developers locally.
    Atlas {
        #[command(subcommand)]
        action: AtlasAction,
    },

    /// Audit-log operator commands (F3.7c+). Invoked by the daily
    /// retention-pruning cron in cloud deployments; runnable
    /// locally for ad-hoc cleanup.
    Audit {
        #[command(subcommand)]
        action: AuditAction,
    },

    /// API-keys operator commands (F3.4c-ii+). Invoked by the weekly
    /// stale-keys cron in cloud deployments; runnable locally for
    /// ad-hoc reviews.
    #[command(name = "api-keys")]
    ApiKeys {
        #[command(subcommand)]
        action: ApiKeysAction,
    },

    /// Cloud operator commands. `check` smoke-tests every wired
    /// integration (Postgres, Stripe, GitHub OAuth, GitHub App, blob
    /// backend) and exits with the number of failed probes — drop
    /// it into CI as `just dev-cloud-check`.
    Cloud {
        #[command(subcommand)]
        action: CloudAction,
    },
}

/// Subcommands for `ministr api-keys` — service-account-key operator
/// commands. Invoked by the Azure Container Apps Job on the weekly
/// cron schedule (F3.4c-ii); also runnable locally during development.
#[derive(Debug, Subcommand)]
enum ApiKeysAction {
    /// Flag service-account API keys whose `last_used_at` (or
    /// `created_at` for never-used keys) is older than
    /// `--threshold-days`. Each flagged key emits an
    /// `api_key.stale` audit event. Idempotent across runs (the
    /// query is deterministic; repeat runs against an unchanged table
    /// emit the same set of events).
    FlagStale {
        /// Staleness threshold in days. Defaults to
        /// `ministr_cloud::DEFAULT_STALE_API_KEY_DAYS` (90), matching
        /// the ROADMAP §F3.4c language. Operators can pass a smaller
        /// value to dry-run the cron against more rows.
        #[arg(long, default_value_t = ministr_cloud::DEFAULT_STALE_API_KEY_DAYS)]
        threshold_days: u32,
    },
}

/// Subcommands for `ministr audit` — audit-log operator commands.
/// Invoked by the Azure Container Apps Job on the daily cron
/// schedule (F3.7c); also runnable locally during development.
#[derive(Debug, Subcommand)]
enum AuditAction {
    /// Drop `audit_events` rows older than `--retention-days`. The
    /// daily cron runs this; manual invocations are idempotent (a
    /// re-run on a freshly-pruned table simply deletes 0 rows).
    Prune {
        /// Retention window in days. Defaults to
        /// `ministr_cloud::DEFAULT_AUDIT_RETENTION_DAYS` (90), which
        /// matches the Team-tier audit retention in §3 of ROADMAP.
        /// F5.3 immutable audit retains forever and skips this cron.
        #[arg(long, default_value_t = ministr_cloud::DEFAULT_AUDIT_RETENTION_DAYS)]
        retention_days: u32,
    },
    /// F5.3-c-ii — extend the `audit_events` quarterly partition
    /// surface forward to `now() + --lookahead-quarters * 3 months`.
    /// `cmd_serve_http` already invokes this at every pod boot;
    /// the CLI form exists for operator-driven catch-up + cron
    /// jobs that don't want to restart the serve to extend the
    /// forward edge.
    EnsurePartitions {
        /// Quarters of runway past the current calendar quarter.
        /// Defaults to `ministr_cloud::DEFAULT_PARTITION_LOOKAHEAD_QUARTERS`
        /// (8 = 2 years).
        #[arg(long, default_value_t = ministr_cloud::DEFAULT_PARTITION_LOOKAHEAD_QUARTERS)]
        lookahead_quarters: u32,
    },
    /// F5.3-c-ii-archive-fs — archive one `audit_events` partition
    /// to a gzipped JSONL file in `--archive-dir`, then DETACH +
    /// DROP it from the live database. The named file becomes the
    /// authoritative copy of the data.
    Archive {
        /// Partition name to archive — must match the
        /// `audit_events_y{YYYY}q{N}` pattern from migration 0013.
        /// Names that don't match are rejected as a defense-in-depth
        /// measure against path traversal.
        #[arg(long)]
        partition: String,
        /// Local directory where the gzipped JSONL file will land.
        /// Created if it doesn't exist. Production deployments
        /// point this at a Container-Apps volume mount;
        /// F5.3-c-ii-archive-blob will add an Azure Blob sink as
        /// the alternative target.
        #[arg(long)]
        archive_dir: std::path::PathBuf,
    },
}

/// Subcommands for `ministr cloud` — operator diagnostics + future
/// admin tooling.
#[derive(Debug, Subcommand)]
enum CloudAction {
    /// Probe every cloud integration and print a tick/cross table.
    /// Exit code = number of failed probes (so CI can gate on it).
    Check,

    /// End-to-end watchable runner against a deployed cloud. Probes
    /// /healthz, walks the OAuth loopback PKCE flow (browser opens),
    /// lists corpora, optionally registers + clones a repo, then
    /// streams the indexing-progress SSE live to the terminal until
    /// completion, and finishes with a survey query against the
    /// indexed corpus.
    Demo {
        /// Cloud base URL, e.g. `https://my-deployment.example.com`.
        #[arg(long, allow_hyphen_values = true)]
        endpoint: String,
        /// Skip the OAuth flow and use this bearer token instead.
        /// Useful when you already have a token in the keychain
        /// (paste from the Tauri panel's Advanced → "Show token").
        ///
        /// `allow_hyphen_values` is set because the OAuth issuer's
        /// `generate_id` includes `-` in its base64url alphabet —
        /// ~1 in 64 tokens start with `-`, which clap would
        /// otherwise reject as an unknown short flag.
        #[arg(long, allow_hyphen_values = true)]
        token: Option<String>,
        /// Clone this Git URL as part of the demo (cloud-side
        /// `POST /api/v1/corpora/{parent}/clone`). Requires either
        /// an existing parent corpus on the cloud OR `--parent`.
        #[arg(long, allow_hyphen_values = true)]
        clone_url: Option<String>,
        /// Parent corpus ID for the clone. Defaults to the first
        /// listed corpus.
        #[arg(long, allow_hyphen_values = true)]
        parent: Option<String>,
        /// Watch THIS corpus's progress (skip the clone step).
        #[arg(long, allow_hyphen_values = true)]
        corpus: Option<String>,
    },

    /// Trimmed version of `demo`: just stream a specific corpus's
    /// progress to the terminal. Useful when you've triggered a
    /// clone from the Tauri panel and want to follow it in a
    /// terminal in parallel.
    Watch {
        #[arg(long, allow_hyphen_values = true)]
        endpoint: String,
        #[arg(long, allow_hyphen_values = true)]
        token: String,
        #[arg(long, allow_hyphen_values = true)]
        corpus: String,
    },

    /// **Test/operator helper.** Seed a `users` row keyed on
    /// `--github-id` (via the same `upsert_github_user` path the
    /// real GitHub callback uses), then mint a bearer token bound
    /// to the resulting UUID subject. Prints JSON
    /// `{user_id, token, plan_id}` on stdout. Intended for the
    /// `just e2e-cloud-local` harness — production tokens land via
    /// the real GitHub OAuth dance. Requires `MINISTR_PG_URL`.
    MintTestBearer {
        /// Synthetic GitHub user id (any non-zero i64). The `users`
        /// table's UPSERT key is `github_id`, so re-running with the
        /// same value returns the same UUID — idempotent across
        /// harness runs.
        #[arg(long)]
        github_id: i64,
        /// Email address for the test user. Required by
        /// `upsert_github_user`; any non-empty `*@*` string works.
        #[arg(long)]
        email: String,
        /// Scope string for the minted token. Defaults to the
        /// production-equivalent read+write scope set.
        #[arg(long, default_value = "ministr:read ministr:write")]
        scope: String,
    },
    /// F5.4-b harness helper — generate a fresh RSA-2048 keypair,
    /// sign a license JWT carrying the supplied claims, print both
    /// as JSON `{jwt, public_key_pem}` for the harness to capture
    /// and pass to a test serve via `MINISTR_LICENSE_KEY` +
    /// `MINISTR_LICENSE_PUBLIC_KEY` env vars. Intentionally NOT
    /// gated on `MINISTR_PG_URL` — pure key-and-JWT generation.
    MintTestLicense {
        /// `enterprise_id` claim. Echoed in boot logs.
        #[arg(long, default_value = "e2e-test-tenant")]
        enterprise_id: String,
        /// `seat_count` claim — the limit F5.4-b's invite handler
        /// enforces. Default 2 so the harness can prove the cap
        /// fires by inviting 3 emails.
        #[arg(long, default_value_t = 2)]
        seat_count: u32,
        /// Days from now until `exp`. Default 365 (year-long test
        /// license). Set to 0 or negative to produce an already-
        /// expired JWT (used by F5.4-a's expired-license rejection
        /// test path).
        #[arg(long, default_value_t = 365)]
        valid_days: i64,
    },
    /// F5.4-e-mint operator setup — generate a persistent RSA-2048
    /// keypair for license signing. Run ONCE per ministr deployment.
    /// Stash the private key in your secrets manager (Vault / KMS /
    /// 1Password); ship the public key to every Enterprise customer
    /// for their `MINISTR_LICENSE_PUBLIC_KEY` env var.
    GenerateLicenseKeypair {
        /// Output path for the private key (PKCS#8 PEM). chmod 0600.
        #[arg(long)]
        private_key: std::path::PathBuf,
        /// Output path for the public key (SPKI PEM) — ships to
        /// customers verbatim.
        #[arg(long)]
        public_key: std::path::PathBuf,
        /// RSA key size in bits. Default 2048 (NIST SP 800-131A
        /// minimum). 3072 / 4096 are accepted; larger is slower
        /// without meaningful 2026 security uplift.
        #[arg(long, default_value_t = 2048)]
        bits: u32,
    },
    /// F5.4-e-mint operator JWT issuance — sign a license JWT
    /// against the persistent private key generated by
    /// `generate-license-keypair`. Prints the JWT to stdout (or
    /// writes to `--out`) for distribution to the customer.
    MintLicense {
        /// Path to the RSA private key (PKCS#8 PEM). The same key
        /// `generate-license-keypair` wrote to `--private-key`.
        #[arg(long)]
        private_key: std::path::PathBuf,
        /// `enterprise_id` claim — appears in the customer's boot
        /// log when their serve validates the license.
        #[arg(long)]
        enterprise_id: String,
        /// Seat-count limit (F5.4-b enforces on /invites).
        #[arg(long)]
        seat_count: u32,
        /// Days from now until `exp`. Must be > 0 (use
        /// `mint-test-license` for the expired-license test fixture).
        #[arg(long)]
        valid_days: u32,
        /// Optional file path to write the JWT to. Default prints
        /// to stdout. Distribute via your CRM / email / secure-share
        /// to the customer's ops contact.
        #[arg(long)]
        out: Option<std::path::PathBuf>,
        /// F5.4-e-audit — optional path to append one JSONL line
        /// per successful mint. Records who/what/when (NO bearer
        /// material — just a SHA-256 hash of the JWT for unique
        /// identification). Stash alongside your license private
        /// key so the issuance trail survives operator churn.
        #[arg(long)]
        audit_log: Option<std::path::PathBuf>,
        /// F5.4-e-audit-db — optional Postgres connection string
        /// for the multi-operator DB-backed audit mirror. When set,
        /// every successful mint ALSO appends one row to
        /// `license_issuances` (idempotent on the JWT's hash so
        /// retries are safe). Falls through to `MINISTR_PG_URL`
        /// env var when the flag is absent. Pair with
        /// `list-licenses --pg-url URL` to read the unified view.
        #[arg(long)]
        pg_url: Option<String>,
    },
    /// F5.4-e-audit — print the issuance audit log (JSONL written by
    /// `mint-license --audit-log PATH`). Useful for "did I already
    /// issue a license to acme-corp this quarter?" lookups + for
    /// stashing a copy in your CRM at renewal time.
    ListLicenses {
        /// Path to the JSONL audit log. Mutually exclusive with
        /// `--pg-url`; exactly one source is read per invocation.
        #[arg(long, conflicts_with = "pg_url")]
        audit_log: Option<std::path::PathBuf>,
        /// F5.4-e-audit-db — Postgres connection string. When set,
        /// read from `license_issuances` (the DB-backed multi-
        /// operator mirror) instead of the local JSONL. Falls
        /// through to `MINISTR_PG_URL` env var when the flag is
        /// absent — explicit flag wins.
        #[arg(long)]
        pg_url: Option<String>,
        /// Output format. `table` (default) is the human-readable
        /// dashboard view; `json` re-emits the JSONL verbatim
        /// (useful for piping into `jq`).
        #[arg(long, default_value = "table")]
        format: String,
    },
    /// F5.5-b-persist-retention — DELETE `request_latency_snapshots`
    /// rows older than the supplied window. Designed to be wrapped in
    /// an operator cron / Azure Container Apps Job — there is NO
    /// in-process background task because retention cadence is policy
    /// (changes shouldn't require a serve restart). The typical
    /// schedule is daily at low-traffic hour with
    /// `--older-than-secs $((30 * 86400))`.
    ///
    /// Defensive: the flag is required and refuses 0 — the most
    /// common operator typo ("I meant 30 days, typed 0") would
    /// otherwise nuke the entire table.
    SlaPruneSnapshots {
        /// Age threshold in seconds. Rows whose `ts_unix` is strictly
        /// less than `now - older_than_secs` are deleted. Must be > 0.
        #[arg(long)]
        older_than_secs: i64,
    },
    /// F5.4-e-rotate — re-mint every in-flight license against a new
    /// signing keypair. Reads the existing audit log to enumerate
    /// known licenses, skips records whose `jwt_id_hash` matches the
    /// optional revocation list, skips records whose `exp` is already
    /// past, then mints one fresh JWT per surviving enterprise into
    /// `--out-dir`. Writes the new audit log so the rotation cycle's
    /// re-issuances are themselves auditable. The new keypair must be
    /// pre-generated via `generate-license-keypair`; this command
    /// touches only the signing side.
    ///
    /// Usage outline: (1) `generate-license-keypair` to mint
    /// new-private + new-public PEMs; (2) `rotate-license-keys` here;
    /// (3) ship the new public key + the per-customer JWTs in
    /// `--out-dir` to each customer via your CRM / encrypted email,
    /// same channel as the original mint; (4) customers paste both
    /// values into `MINISTR_LICENSE_KEY` + `MINISTR_LICENSE_PUBLIC_KEY`
    /// and restart their pods.
    RotateLicenseKeys {
        /// Path to the existing audit log (JSONL written by
        /// `mint-license --audit-log PATH`). The source of truth for
        /// which licenses exist and need re-issuing.
        #[arg(long)]
        audit_log: std::path::PathBuf,
        /// Optional revocation list (`revoke-license --revocation-list
        /// PATH`); revoked licenses are skipped, not re-issued.
        #[arg(long)]
        revocation_list: Option<std::path::PathBuf>,
        /// Path to the NEW RSA private key (PKCS#8 PEM). Generated by
        /// a separate `generate-license-keypair` call before
        /// invoking this command.
        #[arg(long)]
        new_private_key: std::path::PathBuf,
        /// Directory to write the per-customer JWT files into. Created
        /// if missing. Files are named
        /// `<enterprise_id>-<short_hash>.jwt` so multiple rotations
        /// don't collide.
        #[arg(long)]
        out_dir: std::path::PathBuf,
        /// Path for the new rotation cycle's audit log. Mint records
        /// for the re-issued JWTs land here so the rotation is itself
        /// auditable. Create a fresh file per rotation cycle so the
        /// timestamps are unambiguous.
        #[arg(long)]
        new_audit_log: std::path::PathBuf,
        /// `valid_days` to stamp on every re-issued JWT. Operator
        /// decides the new contract horizon at rotation time.
        #[arg(long, default_value_t = 365)]
        valid_days: u32,
    },
    /// F5.4-e-revoke — append one revocation record to the JSONL
    /// revocation list the customer's serve consults at boot via
    /// `MINISTR_LICENSE_REVOCATIONS`. After revocation, the customer
    /// pulls down the updated list, restarts their pods, and the
    /// serve refuses to boot with a clear "license revoked" error.
    /// Use for contract terminations, key compromises, or any other
    /// situation where you need to invalidate a license before its
    /// `exp` rolls around.
    RevokeLicense {
        /// Path to the customer's JWT file (the same file produced by
        /// `mint-license --out`). The CLI computes the
        /// `jwt_id_hash` from it. Mutually exclusive with
        /// `--jwt-id-hash`.
        #[arg(long, conflicts_with = "jwt_id_hash")]
        jwt: Option<std::path::PathBuf>,
        /// Pre-computed 16-hex-char hash — useful when you no longer
        /// have the JWT but the audit log still has the hash. Mutually
        /// exclusive with `--jwt`.
        #[arg(long)]
        jwt_id_hash: Option<String>,
        /// Human-readable customer label. Matches
        /// `LicenseClaims.enterprise_id`; surfaced in the boot error
        /// so customers can confirm the right license fired.
        #[arg(long)]
        enterprise_id: String,
        /// Free-text justification (contract terminated 2026-12-01,
        /// key compromise, etc.). Echoed in the boot error so the
        /// customer sees why the license was revoked.
        #[arg(long, default_value = "")]
        reason: String,
        /// Path to the JSONL revocation list. Appended atomically;
        /// the customer pulls this file down via the same channel as
        /// the license + public key.
        #[arg(long)]
        revocation_list: std::path::PathBuf,
    },
}

/// Subcommands for `ministr hooks`.
#[derive(Debug, Subcommand)]
enum HooksAction {
    /// Test installed hooks by simulating tool calls.
    ///
    /// Checks all agent platform hook files, validates their structure,
    /// and simulates common tool calls to report which would be blocked.
    Test,
}

/// Subcommands for `ministr atlas` — Atlas curated-network operator
/// commands. Invoked by the Azure Container Apps Job on the weekly
/// cron schedule (F4.2); also runnable locally during development.
#[derive(Debug, Subcommand)]
enum AtlasAction {
    /// Re-index every seed repo once. F2.6 v0 ships the
    /// orchestration with stubbed step impls (no real clone / index /
    /// blob upload — those land in F4.2); the command logs the
    /// outcome counts so the cron's structured-log dashboard works
    /// end-to-end.
    Reindex,
    /// Emit the public manifest as JSON on stdout. The cron writes
    /// this into the Atlas storage account so `ministr.ai/atlas/
    /// manifest.json` mirrors it (the docs-next route renders the
    /// same source for the public mirror).
    Manifest,
}

/// MCP transport mode.
#[derive(Debug, Clone, clap::ValueEnum)]
enum Transport {
    /// JSON-RPC over stdin/stdout (default for local MCP clients).
    Stdio,
    /// Streamable HTTP (MCP spec 2025-03-26) for remote/multi-client deployments.
    Http,
}

impl Default for Command {
    fn default() -> Self {
        Self::Serve {
            transport: Transport::Stdio,
            host: "127.0.0.1".to_string(),
            port: 8080,
            proxy: false,
            oauth: false,
            oauth_issuer: None,
        }
    }
}

/// Resolved configuration from CLI args, config.toml, and .ministr.toml.
struct ResolvedConfig {
    config_path: PathBuf,
    config: ministr_core::config::MinistrConfig,
    cwd: PathBuf,
    corpus_paths: Vec<String>,
    /// Projects linked into this workspace via `.ministr.toml` `[[linked]]`.
    linked: Vec<ministr_core::config::ResolvedLinkedProject>,
    git_includes: Vec<ministr_core::config::GitInclude>,
    resolved_model: String,
    repo_config_dir: Option<PathBuf>,
    /// Matryoshka truncation dimension from `.ministr.toml` `[corpus] dimension`.
    resolved_dimension: Option<usize>,
    /// Two-stage rerank depth from `.ministr.toml` `[corpus] rerank_depth`.
    rerank_depth: Option<usize>,
}

/// Load global config, discover per-repo .ministr.toml, and resolve corpus paths.
fn resolve_config(cli: &Cli) -> Result<ResolvedConfig> {
    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(ministr_core::config::MinistrConfig::default_path);
    let config = ministr_core::config::MinistrConfig::load(&config_path).map_err(|e| {
        miette::miette!("failed to load config from {}: {e}", config_path.display())
    })?;

    let cwd = std::env::current_dir()
        .map_err(|e| miette::miette!("failed to get current directory: {e}"))?;
    let corpus_config = ministr_core::config::RepoConfig::discover(&cwd)
        .map_err(|e| miette::miette!("failed to read .ministr.toml: {e}"))?;

    if let Some((ref config_dir, ref cc)) = corpus_config {
        let config_file = config_dir.join(ministr_core::config::CORPUS_CONFIG_FILENAME);
        tracing::info!(
            config = %config_file.display(),
            paths = cc.corpus.paths.len(),
            git_repos = cc.corpus.git.len(),
            ignore_patterns = cc.corpus.ignore.len(),
            "loaded .ministr.toml"
        );
        for w in &cc.validate(config_dir) {
            tracing::warn!("{w}");
        }
    } else {
        tracing::info!("no .ministr.toml found — using CLI args or config.toml defaults");
    }

    // `MINISTR_CORPUS_PATHS` overrides every other source. Used by the
    // cloud deployment so the ACA container can be steered to index a
    // specific path (typically `/data/corpus`) without having to plant a
    // `.ministr.toml` on the Azure Files mount.
    let env_paths: Vec<String> = std::env::var("MINISTR_CORPUS_PATHS")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            s.split(':')
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();

    let corpus_paths: Vec<String> = if !env_paths.is_empty() {
        tracing::info!(
            paths = env_paths.len(),
            "loaded corpus paths from MINISTR_CORPUS_PATHS env var"
        );
        env_paths
    } else if let Some((ref base_dir, ref cc)) = corpus_config {
        cc.resolve_local_paths(base_dir)
    } else if cli.corpus.is_empty() {
        config.corpus_paths.clone()
    } else {
        cli.corpus.clone()
    };

    let linked = corpus_config
        .as_ref()
        .map(|(_, cc)| cc.resolve_linked_projects())
        .unwrap_or_default();

    let repo_config_dir = corpus_config.as_ref().map(|(dir, _)| dir.clone());

    let git_includes = corpus_config
        .as_ref()
        .map(|(_, cc)| cc.corpus.git.clone())
        .unwrap_or_default();

    let resolved_model = ministr_core::config::resolve_model_name(
        corpus_config.as_ref().map(|(_, cc)| cc),
        None,
        &config,
    );

    let resolved_dimension = corpus_config
        .as_ref()
        .and_then(|(_, cc)| cc.corpus.dimension);
    let rerank_depth = corpus_config
        .as_ref()
        .and_then(|(_, cc)| cc.corpus.rerank_depth);

    Ok(ResolvedConfig {
        config_path,
        config,
        cwd,
        corpus_paths,
        linked,
        git_includes,
        resolved_model,
        repo_config_dir,
        resolved_dimension,
        rerank_depth,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = Cli::parse();

    // rustls 0.23 requires the process to install a default
    // CryptoProvider before any TLS work runs. The workspace pulls in
    // both `ring` and `aws-lc-rs` transitively (via reqwest +
    // rustls-platform-verifier + tokio-postgres-rustls), so rustls
    // refuses to auto-pick. Use `aws_lc_rs` — same crypto stack the
    // platform-verifier crate already brings in, so we don't double
    // the binary size with `ring`. Idempotent: the panic is downgraded
    // to a log because a second call (in tests / a re-entrant runtime)
    // would otherwise crash on `unwrap`.
    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        // A provider was already installed by an earlier call site
        // (e.g. a library's `static`-init path). Safe to ignore.
    }

    miette::set_hook(Box::new(|_| {
        Box::new(miette::MietteHandlerOpts::new().build())
    }))
    .expect("miette hook should be set once");

    ministr_core::tracing::init_tracing();

    let command = cli.command.take().unwrap_or_default();

    // `ministr setup` runs *before* resolve_config() so a malformed
    // .ministr.toml in cwd can't lock the user out of the subcommand that
    // gets `ministr` on PATH. Setup needs no corpus paths, no model
    // resolution, no repo config — it just edits shell rc files /
    // HKCU\Environment\PATH.
    if let Command::Setup {
        bin_dir,
        dry_run,
        uninstall,
    } = command
    {
        return commands::cmd_setup(bin_dir.as_deref(), dry_run, uninstall);
    }

    // `ministr __daemon` is the headless daemon host (auto-spawned by the
    // MCP proxy / desktop app). It runs *before* resolve_config() too:
    // it serves every corpus over the IPC endpoint and must not depend on
    // the spawning process's cwd or a `.ministr.toml` there.
    if let Command::Daemon = command {
        let config_path = cli
            .config
            .clone()
            .unwrap_or_else(ministr_core::config::MinistrConfig::default_path);
        // `load` already returns the default when the file is simply
        // absent; an `Err` therefore means the file IS present but
        // unreadable/invalid. Silently defaulting there would start the
        // daemon with the wrong data_dir/model and no clue why — so make
        // that case loud rather than swallowing it.
        let config = match ministr_core::config::MinistrConfig::load(&config_path) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    path = %config_path.display(),
                    error = %e,
                    "config file present but invalid — starting daemon with DEFAULT \
                     settings; fix the config and restart the daemon"
                );
                ministr_core::config::MinistrConfig::default()
            }
        };
        return ministr_daemon::bootstrap::run(config)
            .await
            .map_err(|e| miette::miette!("daemon exited: {e}"));
    }

    let rc = resolve_config(&cli)?;

    dispatch(command, rc).await
}

#[allow(clippy::too_many_lines)]
async fn dispatch(command: Command, rc: ResolvedConfig) -> Result<()> {
    match command {
        Command::Serve {
            transport,
            host,
            port,
            proxy,
            oauth,
            oauth_issuer,
        } => {
            // Create-only: a routine MCP start must never silently
            // rewrite an existing .claude/settings.json hooks block.
            // Healing is reserved for explicit `ministr init` / the
            // desktop Repair action (run with a known-current binary).
            ministr_core::scaffold::scaffold_agent_config_with(
                &rc.cwd,
                ministr_core::scaffold::ScaffoldMode::CreateOnly,
            );

            match transport {
                Transport::Stdio => {
                    // Always a thin proxy. The proxy auto-starts the
                    // headless daemon (self-exec `ministr __daemon`) if
                    // none is running, so shared-daemon is the single
                    // architecture — there is no separate monolithic
                    // per-corpus server. `--proxy` is now implicit; the
                    // flag is retained only for backward compatibility.
                    let _ = proxy;
                    commands::cmd_serve_proxy_stdio(&rc.corpus_paths, &rc.linked).await
                }
                Transport::Http => {
                    let oauth_config = if oauth {
                        Some(ministr_mcp::auth::OAuthConfig {
                            issuer: oauth_issuer.unwrap_or_else(|| format!("http://{host}:{port}")),
                            ..ministr_mcp::auth::OAuthConfig::default()
                        })
                    } else {
                        None
                    };
                    commands::cmd_serve_http(
                        &rc.corpus_paths,
                        &rc.git_includes,
                        &rc.config_path,
                        &rc.config,
                        &host,
                        port,
                        oauth_config,
                        &rc.resolved_model,
                        rc.repo_config_dir.as_deref(),
                        rc.resolved_dimension,
                        rc.rerank_depth,
                    )
                    .await
                }
            }
        }
        Command::Index => {
            commands::cmd_index(
                &rc.corpus_paths,
                &rc.git_includes,
                &rc.config_path,
                &rc.config,
                &rc.resolved_model,
                rc.resolved_dimension,
                rc.rerank_depth,
            )
            .await
        }
        Command::Status => commands::cmd_daemon_status().await,
        Command::Search { query, top_k } => {
            commands::cmd_daemon_search(&rc.corpus_paths, &query, top_k).await
        }
        Command::Init { force, interactive } => {
            if interactive {
                commands::cmd_init_interactive(&rc.cwd, force)
            } else {
                commands::cmd_init(&rc.cwd, force)
            }
        }
        Command::Export { output } => {
            commands::cmd_export(
                &rc.corpus_paths,
                &rc.config,
                &rc.resolved_model,
                output.as_deref(),
            )
            .await
        }
        Command::Import { bundle } => commands::cmd_import(&rc.corpus_paths, &rc.config, &bundle),
        Command::Hooks { action } => match action {
            HooksAction::Test => {
                commands::cmd_hooks_test(&rc.cwd);
                Ok(())
            }
        },
        Command::Atlas { action } => match action {
            AtlasAction::Reindex => commands::cmd_atlas_reindex().await,
            AtlasAction::Manifest => commands::cmd_atlas_manifest(),
        },
        Command::Audit { action } => match action {
            AuditAction::Prune { retention_days } => {
                commands::cmd_audit_prune(retention_days).await
            }
            AuditAction::EnsurePartitions { lookahead_quarters } => {
                commands::cmd_audit_ensure_partitions(lookahead_quarters).await
            }
            AuditAction::Archive {
                partition,
                archive_dir,
            } => commands::cmd_audit_archive(&partition, &archive_dir).await,
        },
        Command::ApiKeys { action } => match action {
            ApiKeysAction::FlagStale { threshold_days } => {
                commands::cmd_api_keys_flag_stale(threshold_days).await
            }
        },
        Command::Cloud { action } => match action {
            CloudAction::Check => {
                let failed = cloud_check::run_all().await;
                if failed > 0 {
                    std::process::exit(i32::try_from(failed).unwrap_or(1));
                }
                Ok(())
            }
            CloudAction::Demo {
                endpoint,
                token,
                clone_url,
                parent,
                corpus,
            } => cloud_demo::run_demo(endpoint, token, clone_url, corpus, parent).await,
            CloudAction::Watch {
                endpoint,
                token,
                corpus,
            } => cloud_demo::run_watch(endpoint, token, corpus).await,
            CloudAction::MintTestBearer {
                github_id,
                email,
                scope,
            } => commands::cmd_cloud_mint_test_bearer(github_id, &email, &scope).await,
            CloudAction::MintTestLicense {
                enterprise_id,
                seat_count,
                valid_days,
            } => commands::cmd_cloud_mint_test_license(&enterprise_id, seat_count, valid_days),
            CloudAction::GenerateLicenseKeypair {
                private_key,
                public_key,
                bits,
            } => commands::cmd_cloud_generate_license_keypair(&private_key, &public_key, bits),
            CloudAction::MintLicense {
                private_key,
                enterprise_id,
                seat_count,
                valid_days,
                out,
                audit_log,
                pg_url,
            } => commands::cmd_cloud_mint_license(
                &private_key,
                &enterprise_id,
                seat_count,
                valid_days,
                out.as_deref(),
                audit_log.as_deref(),
                pg_url.as_deref(),
            ).await,
            CloudAction::ListLicenses {
                audit_log,
                pg_url,
                format,
            } => commands::cmd_cloud_list_licenses(
                audit_log.as_deref(),
                pg_url.as_deref(),
                &format,
            ).await,
            CloudAction::RevokeLicense {
                jwt,
                jwt_id_hash,
                enterprise_id,
                reason,
                revocation_list,
            } => commands::cmd_cloud_revoke_license(
                jwt.as_deref(),
                jwt_id_hash.as_deref(),
                &enterprise_id,
                &reason,
                &revocation_list,
            ),
            CloudAction::SlaPruneSnapshots { older_than_secs } => {
                commands::cmd_cloud_sla_prune_snapshots(older_than_secs).await
            }
            CloudAction::RotateLicenseKeys {
                audit_log,
                revocation_list,
                new_private_key,
                out_dir,
                new_audit_log,
                valid_days,
            } => commands::cmd_cloud_rotate_license_keys(
                &audit_log,
                revocation_list.as_deref(),
                &new_private_key,
                &out_dir,
                &new_audit_log,
                valid_days,
            ),
        },
        Command::Setup { .. } => {
            unreachable!("ministr setup is dispatched before resolve_config in main()")
        }
        Command::Daemon => {
            unreachable!("ministr __daemon is dispatched before resolve_config in main()")
        }
    }
}
