//! `ministr-cloud-tools` — operator CLI for ministr-cloud deployments.
//!
//! Hosts the four cloud-only subcommand groups split out of `ministr`
//! in F31.2a:
//!
//! - `atlas`     — F2.6 curated-network reindex + manifest emission
//! - `audit`     — F3.7c / F5.3-c retention + partitioning + archive
//! - `api-keys`  — F3.4c-ii stale-key weekly sweep
//! - `cloud`     — F5.4-e license mint/rotate/revoke + deployment diagnostics
//!
//! The public `ministr` CLI (MIT) no longer carries any of these so
//! the open-core split stays honest — operators run this proprietary
//! binary on the Container Apps Job rather than the public CLI.

mod cloud_check;
mod cloud_demo;
mod commands;

use clap::{Parser, Subcommand};
use miette::Result;

/// ministr-cloud-tools — operator CLI for ministr-cloud.
#[derive(Parser, Debug)]
#[command(name = "ministr-cloud-tools", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
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

/// Subcommands for `api-keys` — service-account-key operator
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

/// Subcommands for `audit` — audit-log operator commands.
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

/// Subcommands for `cloud` — operator diagnostics + admin tooling.
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

/// Subcommands for `atlas` — Atlas curated-network operator
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

#[tokio::main]
async fn main() -> Result<()> {
    // rustls 0.23 requires the process to install a default
    // CryptoProvider before any TLS work runs. Same rationale as
    // ministr-cli's main: reqwest + tokio-postgres-rustls pull in
    // both ring and aws-lc-rs transitively, so rustls refuses to
    // auto-pick. Use aws_lc_rs — same crypto stack the platform
    // verifier crate already brings in.
    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        // A provider was already installed by an earlier call site.
    }

    miette::set_hook(Box::new(|_| {
        Box::new(miette::MietteHandlerOpts::new().build())
    }))
    .expect("miette hook should be set once");

    ministr_core::tracing::init_tracing();

    let cli = Cli::parse();
    dispatch(cli.command).await
}

#[allow(clippy::too_many_lines)]
async fn dispatch(command: Command) -> Result<()> {
    match command {
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
            } => {
                commands::cmd_cloud_mint_license(
                    &private_key,
                    &enterprise_id,
                    seat_count,
                    valid_days,
                    out.as_deref(),
                    audit_log.as_deref(),
                    pg_url.as_deref(),
                )
                .await
            }
            CloudAction::ListLicenses {
                audit_log,
                pg_url,
                format,
            } => {
                commands::cmd_cloud_list_licenses(
                    audit_log.as_deref(),
                    pg_url.as_deref(),
                    &format,
                )
                .await
            }
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
    }
}
