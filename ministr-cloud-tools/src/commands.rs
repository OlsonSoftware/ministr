//! Cloud-tool subcommand implementations split from `ministr-cli` in F31.2a.
//!
//! Each `pub(crate)` function corresponds to a CLI subcommand dispatched
//! from [`main`](crate::main). All functions are thin wrappers over
//! library code in `ministr-cloud` / `ministr-atlas` / `ministr-mcp::auth`.

use miette::{IntoDiagnostic, WrapErr};

// ---------------------------------------------------------------------------
// ministr-cloud-tools atlas — F2.6
// ---------------------------------------------------------------------------

/// `atlas reindex` — F2.6 worker entrypoint.
///
/// The Azure Container Apps Job invokes this on the F4.2 weekly cron.
/// F2.6 v0 ships the orchestration with no-op step impls so the
/// command itself is stable: the cron's structured-log dashboard, the
/// dead-letter table, and the alerts all see real data from day one.
///
/// F4.2 replaces the no-op trait impls below with concrete
/// `ministr_core::git::GitFetcher` / corpus-registry / Azure Blob
/// upload paths without changing this function's signature.
pub(crate) async fn cmd_atlas_reindex() -> miette::Result<()> {
    use std::pin::Pin;
    use std::sync::Arc;

    use ministr_atlas::{
        BlobWriter, Cloner, IndexerStep, ReindexError, reindex_once,
    };

    type BoxFut<'a, T> =
        Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

    /// No-op clone step — logs the URL and returns a synthetic path.
    /// F4.2 replaces with a real `ministr_core::git::GitFetcher`.
    #[derive(Debug)]
    struct StubCloner;
    impl Cloner for StubCloner {
        fn clone_to_tmp<'a>(
            &'a self,
            clone_url: &'a str,
        ) -> BoxFut<'a, Result<std::path::PathBuf, ReindexError>> {
            Box::pin(async move {
                tracing::info!(clone_url, "atlas: would clone (stub)");
                Ok(std::path::PathBuf::from(format!(
                    "/tmp/atlas-stub-{}",
                    clone_url.len()
                )))
            })
        }
    }

    /// No-op index step — returns a placeholder bundle handle.
    #[derive(Debug)]
    struct StubIndexer;
    impl IndexerStep for StubIndexer {
        fn index_dir<'a>(
            &'a self,
            path: &'a std::path::Path,
        ) -> BoxFut<'a, Result<String, ReindexError>> {
            Box::pin(async move {
                tracing::info!(path = %path.display(), "atlas: would index (stub)");
                Ok(format!("stub-bundle:{}", path.display()))
            })
        }
    }

    /// No-op blob writer — returns the synthetic blob path the cron
    /// dashboard expects to see in the log.
    #[derive(Debug)]
    struct StubWriter;
    impl BlobWriter for StubWriter {
        fn write_blob<'a>(
            &'a self,
            slug: &'a str,
            _handle: &'a str,
        ) -> BoxFut<'a, Result<String, ReindexError>> {
            Box::pin(async move {
                let blob = format!("atlas/{slug}/latest.idx");
                tracing::info!(blob, "atlas: would write (stub)");
                Ok(blob)
            })
        }
    }

    let cloner: Arc<dyn Cloner> = Arc::new(StubCloner);
    let indexer: Arc<dyn IndexerStep> = Arc::new(StubIndexer);
    let writer: Arc<dyn BlobWriter> = Arc::new(StubWriter);
    let license: Arc<dyn ministr_atlas::LicenseFilter> =
        Arc::new(ministr_atlas::SpdxFilter);
    let optout: Arc<dyn ministr_atlas::OptOutRegistry> =
        Arc::new(ministr_atlas::InMemoryRegistry::new());

    tracing::info!(
        seed_count = ministr_atlas::ATLAS_SEED_REPOS.len(),
        "atlas reindex starting (F2.6 v0 stub orchestration)"
    );
    let outcome = reindex_once(&cloner, &indexer, &writer, &license, &optout).await;
    tracing::info!(
        indexed = outcome.indexed.len(),
        skipped = outcome.skipped.len(),
        failed = outcome.failed.len(),
        "atlas reindex complete"
    );
    if !outcome.failed.is_empty() {
        tracing::warn!("{} step failures recorded", outcome.failed.len());
    }
    Ok(())
}

/// `atlas manifest` — emit the F2.6 v0 manifest as JSON on
/// stdout. The cron pipes this into the Atlas storage account so the
/// public mirror at `ministr.ai/atlas/manifest.json` stays in sync.
pub(crate) fn cmd_atlas_manifest() -> miette::Result<()> {
    let manifest = ministr_atlas::ManifestSnapshot::from_seed_list();
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| miette::miette!("serialise atlas manifest: {e}"))?;
    println!("{json}");
    Ok(())
}

// ---------------------------------------------------------------------------
// ministr-cloud-tools api-keys — F3.4c-ii
// ---------------------------------------------------------------------------

/// `api-keys flag-stale --threshold-days N` — F3.4c-ii weekly stale-keys
/// cron entrypoint. Scans `api_keys` for rows whose `last_used_at` (or
/// `created_at` for never-used keys) is older than `threshold_days` days
/// and emits an `api_key.stale` audit event per row.
///
/// Requires `MINISTR_PG_URL`. Idempotent across runs.
pub(crate) async fn cmd_api_keys_flag_stale(threshold_days: u32) -> miette::Result<()> {
    let pg_url = std::env::var("MINISTR_PG_URL").map_err(|_| {
        miette::miette!(
            "api-keys flag-stale requires MINISTR_PG_URL (the cloud Postgres connection string)"
        )
    })?;
    let pool = ministr_cloud::connect(&pg_url)
        .into_diagnostic()
        .wrap_err("open cloud postgres pool")?;
    let pool_arc = std::sync::Arc::new(pool);
    let sink = ministr_cloud::PostgresAuditSink::from_arc(std::sync::Arc::clone(&pool_arc));
    tracing::info!(threshold_days, "api-keys flag-stale starting");
    let outcome = ministr_cloud::flag_stale_api_keys(&pool_arc, threshold_days, &sink)
        .await
        .into_diagnostic()
        .wrap_err("flag stale api_keys")?;
    tracing::info!(
        flagged = outcome.flagged,
        elapsed_ms = u64::try_from(outcome.elapsed.as_millis()).unwrap_or(u64::MAX),
        threshold_days = outcome.threshold_days,
        "api-keys flag-stale complete"
    );

    // F3.4c-iii — send digest emails when a mail provider is configured.
    let mailer = ministr_cloud::build_mail_sender_from_env();
    match ministr_cloud::send_stale_key_digests(&pool_arc, threshold_days, mailer.as_ref()).await {
        Ok(sent) => {
            tracing::info!(digests_sent = sent, "stale-key digest emails dispatched");
        }
        Err(e) => {
            tracing::warn!(error = %e, "stale-key digest query failed — emails not sent");
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// ministr-cloud-tools audit — F3.7c + F5.3-c-ii
// ---------------------------------------------------------------------------

/// `audit prune --retention-days N` — F3.7c daily-retention cron
/// entrypoint. Drops `audit_events` rows older than `retention_days`.
///
/// Requires `MINISTR_PG_URL`.
pub(crate) async fn cmd_audit_prune(retention_days: u32) -> miette::Result<()> {
    let pg_url = std::env::var("MINISTR_PG_URL").map_err(|_| {
        miette::miette!(
            "audit prune requires MINISTR_PG_URL (the cloud Postgres connection string)"
        )
    })?;
    let pool = ministr_cloud::connect(&pg_url)
        .into_diagnostic()
        .wrap_err("open cloud postgres pool")?;
    tracing::info!(retention_days, "audit prune starting");
    let outcome = ministr_cloud::prune_audit_events(&pool, retention_days)
        .await
        .into_diagnostic()
        .wrap_err("prune audit_events")?;
    tracing::info!(
        deleted = outcome.deleted,
        elapsed_ms = u64::try_from(outcome.elapsed.as_millis()).unwrap_or(u64::MAX),
        retention_days = outcome.retention_days,
        "audit prune complete"
    );
    Ok(())
}

/// `audit archive --partition NAME --archive-dir DIR` —
/// F5.3-c-ii-archive-fs operator-driven cold archive. SELECTs all
/// rows from the named partition, writes them as a gzipped JSONL
/// file, then `DETACH PARTITION` + `DROP TABLE` it from the live
/// database. The named file becomes the authoritative copy.
///
/// When `MINISTR_AUDIT_ARCHIVE_BLOB_{ACCOUNT,CONTAINER}` are both set,
/// the Azure Blob sink takes precedence over the FS sink.
pub(crate) async fn cmd_audit_archive(
    partition: &str,
    archive_dir: &std::path::Path,
) -> miette::Result<()> {
    let pg_url = std::env::var("MINISTR_PG_URL").map_err(|_| {
        miette::miette!(
            "audit archive requires MINISTR_PG_URL \
             (the cloud Postgres connection string)"
        )
    })?;
    let pool = ministr_cloud::connect(&pg_url)
        .into_diagnostic()
        .wrap_err("open cloud postgres pool")?;

    let blob_account = std::env::var("MINISTR_AUDIT_ARCHIVE_BLOB_ACCOUNT").ok();
    let blob_container = std::env::var("MINISTR_AUDIT_ARCHIVE_BLOB_CONTAINER").ok();
    let outcome = if let (Some(account), Some(container)) = (blob_account, blob_container) {
        tracing::info!(
            partition,
            account = %account,
            container = %container,
            "audit archive starting (Azure Blob sink)"
        );
        let sink = ministr_cloud::AzureBlobArchiveSink::with_managed_identity(
            &account,
            &container,
        )
        .into_diagnostic()
        .wrap_err(
            "build AzureBlobArchiveSink (requires Managed Identity — run from a \
             Container App or Azure VM, OR fall back to FS sink via --archive-dir)",
        )?;
        ministr_cloud::archive_audit_partition_with_sink(&pool, partition, &sink)
            .await
            .into_diagnostic()
            .wrap_err("archive audit partition (blob)")?
    } else {
        tracing::info!(
            partition,
            archive_dir = %archive_dir.display(),
            "audit archive starting (FS sink)"
        );
        ministr_cloud::archive_audit_partition_to_dir(&pool, partition, archive_dir)
            .await
            .into_diagnostic()
            .wrap_err("archive audit partition (fs)")?
    };
    tracing::info!(
        partition,
        rows = outcome.rows,
        bytes_on_disk = outcome.bytes_on_disk,
        target = %outcome.target,
        "audit archive complete"
    );
    Ok(())
}

/// `audit ensure-partitions --lookahead-quarters N` — F5.3-c-ii CLI
/// surface that mirrors `cmd_serve_http`'s boot-time call. Useful for
/// operator-driven catch-up + cron jobs that don't want to restart the
/// serve to push the forward edge of `audit_events` partitions out.
///
/// Requires `MINISTR_PG_URL`. Idempotent — a re-run with the same
/// lookahead creates 0 new partitions.
pub(crate) async fn cmd_audit_ensure_partitions(
    lookahead_quarters: u32,
) -> miette::Result<()> {
    let pg_url = std::env::var("MINISTR_PG_URL").map_err(|_| {
        miette::miette!(
            "audit ensure-partitions requires MINISTR_PG_URL \
             (the cloud Postgres connection string)"
        )
    })?;
    let pool = ministr_cloud::connect(&pg_url)
        .into_diagnostic()
        .wrap_err("open cloud postgres pool")?;
    tracing::info!(lookahead_quarters, "audit ensure-partitions starting");
    let outcome = ministr_cloud::ensure_audit_partitions(&pool, lookahead_quarters)
        .await
        .into_diagnostic()
        .wrap_err("ensure audit_events partitions")?;
    tracing::info!(
        existing = outcome.existing,
        created = outcome.created,
        target_end_year = outcome.target_end_year,
        target_end_quarter = outcome.target_end_quarter,
        "audit ensure-partitions complete"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// ministr-cloud-tools cloud — F5.4 license + F5.5 SLA + test helpers
// ---------------------------------------------------------------------------

/// F5.4-e-mint shared helper — encode a `LicenseClaims` body as an
/// RS256 JWT using a PKCS#8-PEM private key. Pulled out so both the
/// harness mint (`mint-test-license`, fresh keypair per call) and
/// the operator mint (`mint-license`, persistent on-disk key) share
/// one signing path.
fn sign_license_jwt(
    priv_pem: &[u8],
    claims: &ministr_cloud::LicenseClaims,
) -> miette::Result<String> {
    let enc_key = jsonwebtoken::EncodingKey::from_rsa_pem(priv_pem)
        .into_diagnostic()
        .wrap_err("encoding key from PEM")?;
    let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    jsonwebtoken::encode(&header, claims, &enc_key)
        .into_diagnostic()
        .wrap_err("encode JWT")
}

/// F5.4-b harness helper — generate a fresh RSA-2048 keypair, sign a
/// license JWT with the supplied claims, and print
/// `{jwt, public_key_pem}` JSON on stdout. Pure key-and-JWT generation;
/// does NOT touch Postgres so it works in any environment.
pub(crate) fn cmd_cloud_mint_test_license(
    enterprise_id: &str,
    seat_count: u32,
    valid_days: i64,
) -> miette::Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    // Allow negative valid_days to produce an already-expired JWT.
    let exp_offset = valid_days.saturating_mul(86_400);
    let exp = if exp_offset >= 0 {
        now_secs.saturating_add(exp_offset.unsigned_abs())
    } else {
        now_secs.saturating_sub(exp_offset.unsigned_abs())
    };
    let claims = ministr_cloud::LicenseClaims {
        enterprise_id: enterprise_id.to_string(),
        seat_count,
        exp,
        enabled_features: vec![],
    };
    let rsa = openssl::rsa::Rsa::generate(2048)
        .into_diagnostic()
        .wrap_err("generate RSA-2048")?;
    let pkey = openssl::pkey::PKey::from_rsa(rsa)
        .into_diagnostic()
        .wrap_err("wrap PKey")?;
    let priv_pem = pkey
        .private_key_to_pem_pkcs8()
        .into_diagnostic()
        .wrap_err("private key to PEM")?;
    let pub_pem = pkey
        .public_key_to_pem()
        .into_diagnostic()
        .wrap_err("public key to PEM")?;
    let jwt = sign_license_jwt(&priv_pem, &claims)?;
    let pub_pem_str =
        String::from_utf8(pub_pem).into_diagnostic().wrap_err("public PEM utf-8")?;
    let out = serde_json::json!({
        "jwt": jwt,
        "public_key_pem": pub_pem_str,
        "enterprise_id": enterprise_id,
        "seat_count": seat_count,
        "exp": exp,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&out).into_diagnostic()?
    );
    Ok(())
}

/// F5.4-e-mint operator setup — generate a persistent RSA keypair
/// for license signing. Writes PKCS#8 private key (0600 on POSIX) +
/// SPKI public key.
pub(crate) fn cmd_cloud_generate_license_keypair(
    private_key: &std::path::Path,
    public_key: &std::path::Path,
    bits: u32,
) -> miette::Result<()> {
    if !(2048..=4096).contains(&bits) {
        return Err(miette::miette!(
            "license keypair: --bits must be in [2048, 4096]; got {bits}"
        ));
    }
    if private_key.exists() {
        return Err(miette::miette!(
            "license keypair: private-key path '{}' already exists — refusing to overwrite (move the existing key out of the way first)",
            private_key.display()
        ));
    }
    if public_key.exists() {
        return Err(miette::miette!(
            "license keypair: public-key path '{}' already exists — refusing to overwrite",
            public_key.display()
        ));
    }
    let rsa = openssl::rsa::Rsa::generate(bits)
        .into_diagnostic()
        .wrap_err(format!("generate RSA-{bits}"))?;
    let pkey = openssl::pkey::PKey::from_rsa(rsa)
        .into_diagnostic()
        .wrap_err("wrap PKey")?;
    let priv_pem = pkey
        .private_key_to_pem_pkcs8()
        .into_diagnostic()
        .wrap_err("private key to PEM")?;
    let pub_pem = pkey
        .public_key_to_pem()
        .into_diagnostic()
        .wrap_err("public key to PEM")?;

    std::fs::write(private_key, &priv_pem)
        .into_diagnostic()
        .wrap_err(format!("write private key to {}", private_key.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(private_key, perms)
            .into_diagnostic()
            .wrap_err(format!(
                "chmod 0600 on {}",
                private_key.display()
            ))?;
    }
    std::fs::write(public_key, &pub_pem)
        .into_diagnostic()
        .wrap_err(format!("write public key to {}", public_key.display()))?;

    tracing::info!(
        bits,
        private_key = %private_key.display(),
        public_key = %public_key.display(),
        "license keypair generated; stash the private key in your secrets manager and ship the public key to every Enterprise customer"
    );
    Ok(())
}

/// F5.4-e-mint operator JWT issuance — sign a license JWT against
/// the persistent private key from `generate-license-keypair`.
pub(crate) async fn cmd_cloud_mint_license(
    private_key: &std::path::Path,
    enterprise_id: &str,
    seat_count: u32,
    valid_days: u32,
    out: Option<&std::path::Path>,
    audit_log: Option<&std::path::Path>,
    pg_url_flag: Option<&str>,
) -> miette::Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};
    if valid_days == 0 {
        return Err(miette::miette!(
            "mint-license: --valid-days must be > 0 (use `mint-test-license --valid-days -1` if you need an expired-license fixture)"
        ));
    }
    if enterprise_id.trim().is_empty() {
        return Err(miette::miette!(
            "mint-license: --enterprise-id must be non-empty"
        ));
    }
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let exp = now_secs.saturating_add(u64::from(valid_days).saturating_mul(86_400));
    let claims = ministr_cloud::LicenseClaims {
        enterprise_id: enterprise_id.to_string(),
        seat_count,
        exp,
        enabled_features: vec![],
    };
    let priv_pem = std::fs::read(private_key)
        .into_diagnostic()
        .wrap_err(format!(
            "read private key from {}",
            private_key.display()
        ))?;
    let jwt = sign_license_jwt(&priv_pem, &claims)?;

    // F5.4-e-audit-db — PG dual-write FIRST when configured.
    let pg_url_resolved = pg_url_flag
        .map(str::to_string)
        .or_else(|| std::env::var("MINISTR_PG_URL").ok());
    if let Some(url) = pg_url_resolved.as_deref()
        && !url.trim().is_empty()
    {
        let pool = ministr_cloud::connect(url)
            .into_diagnostic()
            .wrap_err("open cloud postgres pool")?;
        let issuance = ministr_cloud::LicenseIssuance {
            ts_iso: ministr_api::format_unix_secs_iso(now_secs),
            ts_unix: now_secs,
            enterprise_id: enterprise_id.to_string(),
            seat_count,
            valid_days,
            exp,
            jwt_id_hash: ministr_cloud::license_jwt_id_hash(&jwt),
        };
        let inserted = ministr_cloud::persist_issuance(&pool, &issuance)
            .await
            .into_diagnostic()
            .wrap_err("persist issuance to license_issuances")?;
        tracing::info!(
            inserted,
            jwt_id_hash = %issuance.jwt_id_hash,
            "license issuance persisted to PG (F5.4-e-audit-db)"
        );
    }

    if let Some(audit_path) = audit_log {
        append_license_audit_line(audit_path, &claims, &jwt, valid_days)?;
    }

    if let Some(out_path) = out {
        std::fs::write(out_path, &jwt)
            .into_diagnostic()
            .wrap_err(format!("write JWT to {}", out_path.display()))?;
        tracing::info!(
            enterprise_id,
            seat_count,
            valid_days,
            exp,
            out = %out_path.display(),
            audit_log = audit_log.map(|p| p.display().to_string()).unwrap_or_default(),
            "license minted"
        );
    } else {
        println!("{jwt}");
        tracing::info!(
            enterprise_id,
            seat_count,
            valid_days,
            exp,
            audit_log = audit_log.map(|p| p.display().to_string()).unwrap_or_default(),
            "license minted (JWT printed to stdout)"
        );
    }
    Ok(())
}

/// F5.4-e-rotate — re-mint every in-flight license against a new
/// signing keypair.
#[allow(clippy::too_many_lines)] // sequential orchestration; splitting fragments the dedup → filter → re-mint narrative
pub(crate) fn cmd_cloud_rotate_license_keys(
    audit_log: &std::path::Path,
    revocation_list: Option<&std::path::Path>,
    new_private_key: &std::path::Path,
    out_dir: &std::path::Path,
    new_audit_log: &std::path::Path,
    valid_days: u32,
) -> miette::Result<()> {
    use std::collections::HashMap;
    use std::time::{SystemTime, UNIX_EPOCH};
    if valid_days == 0 {
        return Err(miette::miette!(
            "rotate-license-keys: --valid-days must be > 0"
        ));
    }
    let bytes = std::fs::read_to_string(audit_log)
        .into_diagnostic()
        .wrap_err(format!("read audit log {}", audit_log.display()))?;
    let mut latest_by_enterprise: HashMap<String, serde_json::Value> = HashMap::new();
    for raw in bytes.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<serde_json::Value>(raw) else {
            tracing::warn!(line = raw, "skipping malformed audit line");
            continue;
        };
        let Some(eid) = entry
            .get("enterprise_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let ts = entry
            .get("ts_unix")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        match latest_by_enterprise.get(&eid) {
            Some(existing)
                if existing
                    .get("ts_unix")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0)
                    >= ts =>
            {
                // Keep the existing (newer or equal) record.
            }
            _ => {
                latest_by_enterprise.insert(eid, entry);
            }
        }
    }

    let revoked: std::collections::HashSet<String> = match revocation_list {
        Some(p) => ministr_cloud::load_revoked_hashes(p)
            .map_err(|e| miette::miette!("load revocation list {}: {e}", p.display()))?,
        None => std::collections::HashSet::new(),
    };

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let mut survivors: Vec<(String, serde_json::Value)> =
        latest_by_enterprise.into_iter().collect();
    survivors.sort_by(|a, b| a.0.cmp(&b.0));

    std::fs::create_dir_all(out_dir)
        .into_diagnostic()
        .wrap_err(format!("create out-dir {}", out_dir.display()))?;

    let priv_pem = std::fs::read(new_private_key)
        .into_diagnostic()
        .wrap_err(format!(
            "read new private key from {}",
            new_private_key.display()
        ))?;

    let mut reissued = 0usize;
    let mut skipped_revoked = 0usize;
    let mut skipped_expired = 0usize;
    let mut summary = Vec::new();

    for (enterprise_id, entry) in survivors {
        let prev_hash = entry
            .get("jwt_id_hash")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if revoked.contains(prev_hash) {
            skipped_revoked += 1;
            continue;
        }
        let prev_exp = entry
            .get("exp")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if prev_exp <= now_secs {
            skipped_expired += 1;
            continue;
        }
        let seat_count = entry
            .get("seat_count")
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .unwrap_or(0);
        let new_exp = now_secs.saturating_add(u64::from(valid_days).saturating_mul(86_400));
        let claims = ministr_cloud::LicenseClaims {
            enterprise_id: enterprise_id.clone(),
            seat_count,
            exp: new_exp,
            enabled_features: vec![],
        };
        let new_jwt = sign_license_jwt(&priv_pem, &claims)?;
        let new_hash = ministr_cloud::license_jwt_id_hash(&new_jwt);
        let safe_eid: String = enterprise_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let out_file = out_dir.join(format!("{safe_eid}-{new_hash}.jwt"));
        std::fs::write(&out_file, &new_jwt)
            .into_diagnostic()
            .wrap_err(format!("write reissued JWT to {}", out_file.display()))?;
        append_license_audit_line(new_audit_log, &claims, &new_jwt, valid_days)?;
        summary.push((enterprise_id.clone(), out_file.display().to_string()));
        reissued += 1;
    }

    println!(
        "rotation summary — {reissued} re-issued, {skipped_revoked} skipped (revoked), {skipped_expired} skipped (expired)"
    );
    if !summary.is_empty() {
        println!();
        println!("{:<24}  out_file", "enterprise_id");
        println!("{:-<24}  {:-<40}", "", "");
        for (eid, path) in &summary {
            println!("{eid:<24}  {path}");
        }
    }
    tracing::info!(
        reissued,
        skipped_revoked,
        skipped_expired,
        out_dir = %out_dir.display(),
        new_audit_log = %new_audit_log.display(),
        "license keypair rotation complete"
    );
    Ok(())
}

/// F5.5-b-persist-retention — DELETE old `request_latency_snapshots`
/// rows. Refuses `older_than_secs <= 0` to prevent operator typos
/// from nuking the table.
pub(crate) async fn cmd_cloud_sla_prune_snapshots(
    older_than_secs: i64,
) -> miette::Result<()> {
    use std::time::{SystemTime, UNIX_EPOCH};
    if older_than_secs <= 0 {
        return Err(miette::miette!(
            "sla-prune-snapshots: --older-than-secs must be > 0 (got {older_than_secs}). \
             Use `--older-than-secs $((30 * 86400))` for the canonical 30-day retention."
        ));
    }
    let pg_url = std::env::var("MINISTR_PG_URL").map_err(|_| {
        miette::miette!(
            "sla-prune-snapshots requires MINISTR_PG_URL (the cloud Postgres connection string)"
        )
    })?;
    let pool = ministr_cloud::connect(&pg_url)
        .into_diagnostic()
        .wrap_err("open cloud postgres pool")?;
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
    let cutoff = now_secs.saturating_sub(older_than_secs);
    let deleted = ministr_cloud::delete_snapshots_older_than(&pool, cutoff)
        .await
        .into_diagnostic()
        .wrap_err("delete snapshots")?;
    println!(
        "sla-prune-snapshots: deleted {deleted} row(s) older than ts_unix < {cutoff} (now - {older_than_secs}s)"
    );
    tracing::info!(
        deleted,
        cutoff_ts_unix = cutoff,
        older_than_secs,
        "sla snapshot retention complete"
    );
    Ok(())
}

/// F5.4-e-revoke — append one revocation record to the JSONL list
/// the customer's serve consults at boot via
/// `MINISTR_LICENSE_REVOCATIONS`.
pub(crate) fn cmd_cloud_revoke_license(
    jwt_path: Option<&std::path::Path>,
    jwt_id_hash: Option<&str>,
    enterprise_id: &str,
    reason: &str,
    revocation_list: &std::path::Path,
) -> miette::Result<()> {
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    let hash = match (jwt_path, jwt_id_hash) {
        (Some(p), None) => {
            let jwt = std::fs::read_to_string(p)
                .into_diagnostic()
                .wrap_err(format!("read JWT from {}", p.display()))?;
            ministr_cloud::license_jwt_id_hash(jwt.trim())
        }
        (None, Some(h)) => {
            let h = h.trim();
            if h.len() != 16 || !h.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(miette::miette!(
                    "--jwt-id-hash must be 16 hex chars (got {} chars)",
                    h.len()
                ));
            }
            h.to_string()
        }
        (Some(_), Some(_)) => {
            return Err(miette::miette!(
                "pass exactly one of --jwt or --jwt-id-hash, not both"
            ));
        }
        (None, None) => {
            return Err(miette::miette!(
                "pass exactly one of --jwt or --jwt-id-hash"
            ));
        }
    };
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let record = ministr_cloud::RevocationRecord {
        ts_iso: ministr_api::format_unix_secs_iso(now_secs),
        ts_unix: now_secs,
        enterprise_id: enterprise_id.to_string(),
        jwt_id_hash: hash.clone(),
        reason: reason.to_string(),
    };
    let serialized = serde_json::to_string(&record)
        .into_diagnostic()
        .wrap_err("serialize revocation record")?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(revocation_list)
        .into_diagnostic()
        .wrap_err(format!(
            "open revocation list {} for append",
            revocation_list.display()
        ))?;
    writeln!(file, "{serialized}")
        .into_diagnostic()
        .wrap_err(format!(
            "append to revocation list {}",
            revocation_list.display()
        ))?;
    tracing::info!(
        enterprise_id,
        jwt_id_hash = %hash,
        revocation_list = %revocation_list.display(),
        "license revoked"
    );
    Ok(())
}

/// F5.4-e-audit — append one JSONL line to the audit log. JSONL is
/// append-safe on POSIX for writes ≤ `PIPE_BUF` (4 KB); each line is
/// well under that.
fn append_license_audit_line(
    audit_path: &std::path::Path,
    claims: &ministr_cloud::LicenseClaims,
    jwt: &str,
    valid_days: u32,
) -> miette::Result<()> {
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let ts_iso = ministr_api::format_unix_secs_iso(now_secs);
    let jwt_id_hash = ministr_cloud::license_jwt_id_hash(jwt);
    let line = serde_json::json!({
        "ts_iso": ts_iso,
        "ts_unix": now_secs,
        "enterprise_id": claims.enterprise_id,
        "seat_count": claims.seat_count,
        "valid_days": valid_days,
        "exp": claims.exp,
        "jwt_id_hash": jwt_id_hash,
    });
    let serialized = serde_json::to_string(&line)
        .into_diagnostic()
        .wrap_err("serialize audit line")?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(audit_path)
        .into_diagnostic()
        .wrap_err(format!(
            "open audit log {} for append",
            audit_path.display()
        ))?;
    writeln!(file, "{serialized}")
        .into_diagnostic()
        .wrap_err(format!(
            "append to audit log {}",
            audit_path.display()
        ))?;
    Ok(())
}

/// F5.4-e-audit — print the issuance audit log. Reads JSONL line by
/// line, skips malformed lines with a warn, sorts by `ts_unix`
/// descending (most recent first).
pub(crate) async fn cmd_cloud_list_licenses(
    audit_log: Option<&std::path::Path>,
    pg_url_flag: Option<&str>,
    format: &str,
) -> miette::Result<()> {
    let pg_url_resolved = pg_url_flag
        .map(str::to_string)
        .or_else(|| std::env::var("MINISTR_PG_URL").ok())
        .filter(|s| !s.trim().is_empty());

    let mut entries: Vec<serde_json::Value> = if let Some(url) = pg_url_resolved.as_deref() {
        let pool = ministr_cloud::connect(url)
            .into_diagnostic()
            .wrap_err("open cloud postgres pool")?;
        let rows = ministr_cloud::list_issuances(&pool, None)
            .await
            .into_diagnostic()
            .wrap_err("list license_issuances")?;
        rows.into_iter()
            .map(|r| {
                serde_json::json!({
                    "ts_iso": r.ts_iso,
                    "ts_unix": r.ts_unix,
                    "enterprise_id": r.enterprise_id,
                    "seat_count": r.seat_count,
                    "valid_days": r.valid_days,
                    "exp": r.exp,
                    "jwt_id_hash": r.jwt_id_hash,
                })
            })
            .collect()
    } else {
        let Some(audit_path) = audit_log else {
            return Err(miette::miette!(
                "list-licenses: pass either --audit-log PATH (JSONL source) or --pg-url URL (DB source)"
            ));
        };
        let bytes = std::fs::read_to_string(audit_path)
            .into_diagnostic()
            .wrap_err(format!("read audit log {}", audit_path.display()))?;
        bytes
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| match serde_json::from_str::<serde_json::Value>(l) {
                Ok(v) => Some(v),
                Err(e) => {
                    tracing::warn!(error = %e, "skipping malformed audit line");
                    None
                }
            })
            .collect()
    };
    entries.sort_by(|a, b| {
        let ts_a = a.get("ts_unix").and_then(serde_json::Value::as_u64).unwrap_or(0);
        let ts_b = b.get("ts_unix").and_then(serde_json::Value::as_u64).unwrap_or(0);
        ts_b.cmp(&ts_a)
    });

    if format == "json" {
        for entry in &entries {
            println!(
                "{}",
                serde_json::to_string(entry)
                    .into_diagnostic()
                    .wrap_err("serialize entry")?
            );
        }
    } else {
        if entries.is_empty() {
            let source = pg_url_resolved
                .as_deref()
                .map(|u| format!("PG {u}"))
                .or_else(|| audit_log.map(|p| p.display().to_string()))
                .unwrap_or_else(|| "<unspecified>".into());
            println!("(no licenses in {source})");
            return Ok(());
        }
        println!(
            "{:<22}  {:<20}  {:>5}  {:>11}  {:<16}",
            "issued (UTC)", "enterprise_id", "seats", "expires (d)", "jwt_id_hash"
        );
        println!("{:-<22}  {:-<20}  {:->5}  {:->11}  {:-<16}", "", "", "", "", "");
        for entry in &entries {
            let ts = entry.get("ts_iso").and_then(serde_json::Value::as_str).unwrap_or("?");
            let eid = entry.get("enterprise_id").and_then(serde_json::Value::as_str).unwrap_or("?");
            let seats = entry.get("seat_count").and_then(serde_json::Value::as_u64).unwrap_or(0);
            let valid_days = entry.get("valid_days").and_then(serde_json::Value::as_u64).unwrap_or(0);
            let hash = entry.get("jwt_id_hash").and_then(serde_json::Value::as_str).unwrap_or("?");
            println!(
                "{ts:<22}  {eid:<20}  {seats:>5}  {valid_days:>11}  {hash:<16}"
            );
        }
    }
    Ok(())
}

/// `cloud mint-test-bearer --github-id N --email E [--scope S]` —
/// F-Test-1 helper. Upserts a `users` row via `upsert_github_user`
/// (the same path the real GitHub callback uses), then mints a bearer
/// token bound to the resulting UUID subject. Prints JSON
/// `{user_id, token, plan_id}` on stdout.
pub(crate) async fn cmd_cloud_mint_test_bearer(
    github_id: i64,
    email: &str,
    scope: &str,
) -> miette::Result<()> {
    let pg_url = std::env::var("MINISTR_PG_URL").map_err(|_| {
        miette::miette!(
            "cloud mint-test-bearer requires MINISTR_PG_URL (the cloud Postgres connection string)"
        )
    })?;
    let pool = ministr_cloud::connect(&pg_url)
        .into_diagnostic()
        .wrap_err("open cloud postgres pool")?;
    let identity = ministr_cloud::idp::ResolvedIdentity {
        issuer: ministr_cloud::idp::github::GITHUB_ISSUER.to_string(),
        subject: github_id.to_string(),
        email: Some(email.to_string()),
        display_name: None,
        github_id: Some(github_id),
    };
    let user = ministr_cloud::upsert_github_user(&pool, &identity)
        .await
        .into_diagnostic()
        .wrap_err("upsert test user")?;
    let store = ministr_mcp::auth::OAuthStore::postgres(
        ministr_mcp::auth::OAuthConfig::default(),
        &pg_url,
    )
    .await
    .into_diagnostic()
    .wrap_err("open OAuth store")?;
    let token = store
        .issue_bearer_token(&user.id, scope)
        .await
        .into_diagnostic()
        .wrap_err("issue bearer token")?;
    let out = serde_json::json!({
        "user_id": user.id,
        "token": token,
        "plan_id": user.plan_id,
    });
    println!("{}", serde_json::to_string(&out).unwrap_or_else(|_| "{}".into()));
    Ok(())
}
