//! F5.4-a — license-key validation primitive for the on-prem
//! Enterprise distribution.
//!
//! ministr-cloud boots in one of two modes:
//!
//! - **Community mode** (default): `MINISTR_LICENSE_KEY` +
//!   `MINISTR_LICENSE_PUBLIC_KEY` unset. The serve runs unrestricted.
//! - **Enterprise mode**: both env vars set. The serve validates the
//!   JWT at boot via [`validate_license_key`]. Invalid / expired /
//!   wrong-signature → boot refuses to start with a clear error.
//!
//! The license-issuing portal (F5.4-e, not yet built) generates an
//! RS256-signed JWT with [`LicenseClaims`] in the body and ships the
//! public key to the customer separately. The customer drops both
//! into their Container App / Docker Compose / Helm values as env
//! vars; the serve checks them at every boot.
//!
//! Why ship the validator now: it's the foundation everyone else
//! builds on. The Helm chart (F5.4-c) just sets the env vars; the
//! portal (F5.4-e) is a separate service that issues JWTs against
//! the same shape. Keeping the public-key-only validator
//! crate-internal (no private-key signing) means the customer's
//! deployment can audit how their key is checked.

use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// JWT payload shape the F5.4-e license portal issues. Each field
/// becomes a top-level claim in the JWT body.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LicenseClaims {
    /// The Enterprise customer this license is bound to. Surfaced
    /// in the boot log so the operator can confirm they pasted the
    /// right key.
    pub enterprise_id: String,
    /// Number of seats the contract allows. Enforcement of seat-cap
    /// at runtime (e.g. /api/v1/orgs/{id}/invites checks the count)
    /// lands as F5.4-b alongside the `ministr-enterprise` crate
    /// scaffold; this chunk only persists the parsed claim.
    pub seat_count: u32,
    /// Unix-seconds expiry. JWT standard `exp` claim; jsonwebtoken's
    /// default `Validation` rejects past-expiry tokens automatically,
    /// but we keep the field public so the boot log can render it.
    pub exp: u64,
    /// Optional feature flags the customer paid for. Empty slice
    /// means "default Enterprise feature set". Specific flag names
    /// land alongside the features that gate on them (F5.5 SLA pool,
    /// F5.6 CMK BYOK, etc).
    #[serde(default)]
    pub enabled_features: Vec<String>,
}

/// Outcomes from [`validate_license_key`]. Boot translates each into
/// a clear stderr message so the operator knows what to fix.
#[derive(Debug, thiserror::Error)]
pub enum LicenseError {
    /// `MINISTR_LICENSE_KEY` set but `MINISTR_LICENSE_PUBLIC_KEY`
    /// (or vice versa) — partial config rejected so the operator
    /// doesn't silently boot in community mode while expecting
    /// Enterprise.
    #[error("license env vars must be set together — found one without the other")]
    PartialConfig,
    /// PEM didn't parse as an RSA public key. Customer pasted the
    /// wrong file (e.g. the cert chain instead of the bare pubkey).
    #[error("public key PEM parse failed: {0}")]
    PubkeyParse(String),
    /// JWT body didn't decode (malformed base64, signature wrong,
    /// or invalid `exp`). The wrapped jsonwebtoken error names the
    /// specific reason.
    #[error("license JWT validation failed: {0}")]
    JwtInvalid(String),
    /// F5.4-e-revoke — the boot-time license's `jwt_id_hash` matched
    /// an entry in `MINISTR_LICENSE_REVOCATIONS`. The operator pushed
    /// a revocation list to the customer (contract termination, key
    /// compromise, etc.) and the serve refuses to boot under a
    /// revoked license even if its `exp` is in the future and the
    /// signature is valid. The error names the matching hash + reason
    /// so the operator sees which entry fired.
    #[error("license revoked at gate: hash={hash} reason={reason}")]
    Revoked {
        /// First 16 hex of `sha256(jwt)` — matches both the audit-log
        /// record from F5.4-e-audit and the revocation-list entry.
        hash: String,
        /// Free-text justification the operator supplied at
        /// `revoke-license` time. Echoed in the boot error so the
        /// customer can confirm the right license was targeted.
        reason: String,
    },
    /// F5.4-e-revoke — `MINISTR_LICENSE_REVOCATIONS` is set but the
    /// path can't be read. Boot refuses rather than silently
    /// proceeding because the operator explicitly asked for
    /// revocation enforcement.
    #[error("revocation list at {path} unreadable: {cause}")]
    RevocationListUnreadable {
        /// Path the operator pointed at.
        path: String,
        /// Underlying IO error message (stringified to keep the enum
        /// variant `Clone + Send + Sync`-friendly without dragging in
        /// `std::io::Error`'s non-clonable shape).
        cause: String,
    },
    /// F5.4-e-revoke-api-fetch — `MINISTR_LICENSE_REVOCATIONS_URL`
    /// is set, the boot-time HTTP fetch failed, AND no within-grace
    /// cache is available. Boot refuses rather than silently
    /// proceeding because the operator explicitly opted into
    /// network-fetched revocation; falling back to "no revocation
    /// check" would mask a deliberately unreachable portal.
    #[error("revocation fetch from {url} failed: {cause}")]
    RevocationFetchFailed {
        /// URL the operator pointed at.
        url: String,
        /// Underlying fetch error message + (when relevant) why the
        /// cache fallback also failed.
        cause: String,
    },
}

/// F5.4-a — validate a license JWT against an RSA public key. Pure;
/// pulled out so the boot path + future portal-side test fixtures
/// share one implementation.
///
/// `jwt` is the raw token string (no `Bearer ` prefix). `pubkey_pem`
/// is the RSA public key in PEM format (the `-----BEGIN PUBLIC KEY-----`
/// `SubjectPublicKeyInfo` form, NOT the legacy `-----BEGIN RSA PUBLIC KEY-----`
/// PKCS#1 form — `jsonwebtoken::DecodingKey::from_rsa_pem` accepts both
/// but the portal-issued public keys are SPKI.)
///
/// # Errors
///
/// [`LicenseError::PubkeyParse`] for malformed PEM; [`LicenseError::JwtInvalid`]
/// for anything else (bad signature, expired, missing claim, ...).
pub fn validate_license_key(
    jwt: &str,
    pubkey_pem: &str,
) -> Result<LicenseClaims, LicenseError> {
    let key = jsonwebtoken::DecodingKey::from_rsa_pem(pubkey_pem.as_bytes())
        .map_err(|e| LicenseError::PubkeyParse(e.to_string()))?;
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    // No audience or issuer enforcement at the validator level — the
    // claims carry their own enterprise_id which boot logs. The
    // portal can add aud/iss later without changing this shape.
    validation.validate_exp = true;
    validation.required_spec_claims =
        std::collections::HashSet::from(["exp".to_string()]);
    let decoded = jsonwebtoken::decode::<LicenseClaims>(jwt, &key, &validation)
        .map_err(|e| LicenseError::JwtInvalid(e.to_string()))?;
    Ok(decoded.claims)
}

/// F5.4-a — boot-time helper called from `cmd_serve_http`. Reads the
/// two env vars and dispatches:
///
/// - Both unset → `Ok(None)` (community mode).
/// - Both set + valid → `Ok(Some(LicenseClaims))` (Enterprise mode).
/// - Either set without the other → [`LicenseError::PartialConfig`].
/// - Validation fails → underlying [`LicenseError`].
///
/// F5.4-e-revoke layers a third check on top: when
/// `MINISTR_LICENSE_REVOCATIONS` is set, the file is scanned for a
/// record matching the boot license's `jwt_id_hash`. A match returns
/// [`LicenseError::Revoked`] so the operator sees the explicit
/// revoked-at-gate reason rather than a generic JWT failure. The env
/// var is opt-in — unset means revocation enforcement is off
/// (preserves the F5.4-a boot shape for customers who don't ship a
/// revocation list).
///
/// # Errors
///
/// Surfaces partial config + validation errors + revocation hits so
/// the boot path can refuse to start when the operator's license
/// setup is broken or actively revoked.
pub async fn validate_license_from_env() -> Result<Option<LicenseClaims>, LicenseError> {
    let jwt = std::env::var("MINISTR_LICENSE_KEY").ok();
    let pubkey = std::env::var("MINISTR_LICENSE_PUBLIC_KEY").ok();
    let (jwt_str, claims) = match (jwt, pubkey) {
        (None, None) => return Ok(None),
        (Some(j), Some(p)) if !j.trim().is_empty() && !p.trim().is_empty() => {
            let claims = validate_license_key(&j, &p)?;
            (j, claims)
        }
        _ => return Err(LicenseError::PartialConfig),
    };
    let hash = license_jwt_id_hash(&jwt_str);
    // F5.4-e-revoke-api-fetch — URL takes precedence over the file
    // path so an operator transitioning from file-based to network-
    // fetched revocation can simply set the new env var without
    // unsetting the old one. The fetcher writes the body to
    // cache_path; we then consult that path via the existing
    // file-based is_revoked_by_file.
    if let Some((url, cache_path, grace_secs)) =
        crate::revocation_fetch::revocation_url_config()
    {
        let path =
            crate::revocation_fetch::fetch_revocation_list(&url, &cache_path, grace_secs).await?;
        if let Some(record) = is_revoked_by_file(&path, &hash)? {
            return Err(LicenseError::Revoked {
                hash,
                reason: record.reason,
            });
        }
        return Ok(Some(claims));
    }
    // F5.4-e-revoke — file-based fallback. Opt-in: missing env var
    // preserves the F5.4-a boot shape.
    if let Ok(rev_path) = std::env::var("MINISTR_LICENSE_REVOCATIONS")
        && !rev_path.trim().is_empty()
        && let Some(record) = is_revoked_by_file(std::path::Path::new(&rev_path), &hash)?
    {
        return Err(LicenseError::Revoked {
            hash,
            reason: record.reason,
        });
    }
    Ok(Some(claims))
}

/// F5.4-e-revoke — short unique identifier for a JWT without storing
/// the bearer material. First 16 hex chars of `sha256(jwt)` —
/// sufficient to disambiguate human-readable list output (16 hex = 64
/// bits = collision-free in practice for any realistic operator
/// issuance volume). One canonical home so the F5.4-e-audit mint log,
/// the F5.4-e-revoke CLI, and this module's [`is_revoked_by_file`]
/// boot check all hash the same input the same way.
#[must_use]
pub fn license_jwt_id_hash(jwt: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(jwt.as_bytes());
    let digest = hasher.finalize();
    let bytes = &digest[..8];
    let mut s = String::with_capacity(16);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// F5.4-e-revoke — one revocation record. Operators append these as
/// JSONL lines to the revocation list file; the serve reads them at
/// boot and refuses to start if the boot license's hash matches.
///
/// `ts_iso` and `ts_unix` carry the revocation moment (not the
/// original mint moment — the audit log carries that). `reason` is
/// free-text justification (contract terminated, key compromise,
/// etc.) the operator typed at `revoke-license` time; the boot error
/// echoes it so the customer can confirm the right license fired.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevocationRecord {
    /// ISO-8601 timestamp of the revocation moment.
    pub ts_iso: String,
    /// Unix-seconds timestamp (same moment as `ts_iso`).
    pub ts_unix: u64,
    /// Human-readable customer identifier. Matches `LicenseClaims.enterprise_id`.
    pub enterprise_id: String,
    /// First 16 hex of `sha256(jwt)` — see [`license_jwt_id_hash`].
    pub jwt_id_hash: String,
    /// Operator-supplied justification. Surfaced in the boot error so
    /// customers can confirm the right license was revoked.
    #[serde(default)]
    pub reason: String,
}

/// F5.4-e-revoke — scan a JSONL revocation list for a record matching
/// `target_hash`. Returns `Ok(Some(record))` on first match,
/// `Ok(None)` when no entry matches (or the file is empty),
/// `Err(LicenseError::RevocationListUnreadable)` when IO fails.
///
/// Malformed lines (partial writes, hand-edits) are skipped silently
/// — the boot check is best-effort defensive: a single corrupt line
/// shouldn't mask a legitimate revocation entry further down. Operators
/// who care can `jq -c .` the file to validate.
///
/// # Errors
///
/// Returns [`LicenseError::RevocationListUnreadable`] when the file
/// can't be opened or read. Returns [`LicenseError::Revoked`] is the
/// boot wrapper's job — this function only signals whether a match
/// was found.
pub fn is_revoked_by_file(
    path: &std::path::Path,
    target_hash: &str,
) -> Result<Option<RevocationRecord>, LicenseError> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).map_err(|e| LicenseError::RevocationListUnreadable {
        path: path.display().to_string(),
        cause: e.to_string(),
    })?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let Ok(line) = line else {
            // Mid-line IO error — treat as malformed and continue.
            continue;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<RevocationRecord>(trimmed) else {
            continue;
        };
        if record.jwt_id_hash == target_hash {
            return Ok(Some(record));
        }
    }
    Ok(None)
}

/// F5.4-e-rotate — batch sibling to [`is_revoked_by_file`]. Reads
/// every well-formed record from a JSONL revocation list and returns
/// the set of revoked `jwt_id_hash` strings. Used by the rotation
/// flow to skip licenses that are already revoked before re-minting
/// them against a new keypair.
///
/// Malformed lines are skipped silently — the same defensive posture
/// `is_revoked_by_file` takes — so partial writes or hand-edits don't
/// mask legitimate entries. An empty file or missing entries returns
/// an empty set.
///
/// # Errors
///
/// Returns [`LicenseError::RevocationListUnreadable`] when the file
/// cannot be opened.
pub fn load_revoked_hashes(
    path: &std::path::Path,
) -> Result<std::collections::HashSet<String>, LicenseError> {
    use std::io::{BufRead, BufReader};
    let file = std::fs::File::open(path).map_err(|e| LicenseError::RevocationListUnreadable {
        path: path.display().to_string(),
        cause: e.to_string(),
    })?;
    let reader = BufReader::new(file);
    let mut out = std::collections::HashSet::new();
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<RevocationRecord>(trimmed) else {
            continue;
        };
        out.insert(record.jwt_id_hash);
    }
    Ok(out)
}

/// Convenience: render the seat-count + expiry as a structured-log
/// triple. Boot calls this so operators see a clear "license OK"
/// line at startup.
#[must_use]
pub fn render_license_summary(claims: &LicenseClaims) -> String {
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days_left = if claims.exp > now_secs {
        (claims.exp - now_secs) / 86_400
    } else {
        0
    };
    format!(
        "enterprise_id={} seat_count={} exp_days_left={} features={:?}",
        claims.enterprise_id, claims.seat_count, days_left, claims.enabled_features
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: generate a fresh RSA-2048 keypair + a license JWT
    // signed with the private key. Returns (jwt, public_key_pem).
    fn mint_test_license(claims: &LicenseClaims) -> (String, String) {
        use openssl::pkey::PKey;
        use openssl::rsa::Rsa;
        let rsa = Rsa::generate(2048).expect("generate rsa-2048");
        let pkey = PKey::from_rsa(rsa).expect("wrap pkey");
        let priv_pem = pkey.private_key_to_pem_pkcs8().expect("priv pem");
        let pub_pem = pkey.public_key_to_pem().expect("pub pem");
        let enc_key = jsonwebtoken::EncodingKey::from_rsa_pem(&priv_pem)
            .expect("encoding key");
        let header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        let jwt = jsonwebtoken::encode(&header, claims, &enc_key)
            .expect("encode jwt");
        (jwt, String::from_utf8(pub_pem).expect("pub pem utf8"))
    }

    fn future_exp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 86_400 * 365
    }

    fn past_exp() -> u64 {
        // jsonwebtoken's default `Validation.leeway` is 60s — push
        // the expiry well past that so the test's intent isn't lost
        // in clock-skew tolerance.
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - 86_400
    }

    #[test]
    fn validate_license_key_accepts_freshly_minted_jwt() {
        let claims = LicenseClaims {
            enterprise_id: "acme-corp".to_string(),
            seat_count: 50,
            exp: future_exp(),
            enabled_features: vec!["sla_pool".to_string(), "cmk".to_string()],
        };
        let (jwt, pub_pem) = mint_test_license(&claims);
        let validated = validate_license_key(&jwt, &pub_pem).expect("must validate");
        assert_eq!(validated.enterprise_id, "acme-corp");
        assert_eq!(validated.seat_count, 50);
        assert_eq!(validated.enabled_features, vec!["sla_pool", "cmk"]);
    }

    #[test]
    fn validate_license_key_rejects_expired_jwt() {
        let claims = LicenseClaims {
            enterprise_id: "acme-corp".to_string(),
            seat_count: 50,
            exp: past_exp(),
            enabled_features: vec![],
        };
        let (jwt, pub_pem) = mint_test_license(&claims);
        let err = validate_license_key(&jwt, &pub_pem)
            .expect_err("must reject past-exp");
        let is_expired = matches!(
            &err,
            LicenseError::JwtInvalid(msg) if msg.to_lowercase().contains("expired")
        );
        assert!(is_expired, "expected `ExpiredSignature`, got {err:?}");
    }

    #[test]
    fn validate_license_key_rejects_wrong_signature() {
        let claims = LicenseClaims {
            enterprise_id: "acme-corp".to_string(),
            seat_count: 50,
            exp: future_exp(),
            enabled_features: vec![],
        };
        let (jwt, _good_pub) = mint_test_license(&claims);
        // Generate a DIFFERENT keypair; verify against its public key.
        // Signature won't match → JwtInvalid.
        let (_, attacker_pub) = mint_test_license(&claims);
        let err = validate_license_key(&jwt, &attacker_pub)
            .expect_err("must reject wrong-signature");
        assert!(matches!(err, LicenseError::JwtInvalid(_)), "got {err:?}");
    }

    #[test]
    fn validate_license_key_rejects_garbage_pubkey() {
        let claims = LicenseClaims {
            enterprise_id: "acme-corp".to_string(),
            seat_count: 50,
            exp: future_exp(),
            enabled_features: vec![],
        };
        let (jwt, _) = mint_test_license(&claims);
        let err = validate_license_key(&jwt, "this-is-not-pem")
            .expect_err("must reject garbage pubkey");
        assert!(matches!(err, LicenseError::PubkeyParse(_)), "got {err:?}");
    }

    #[test]
    fn render_license_summary_includes_enterprise_id_and_seat_count() {
        let claims = LicenseClaims {
            enterprise_id: "acme-corp".to_string(),
            seat_count: 100,
            exp: future_exp(),
            enabled_features: vec!["cmk".to_string()],
        };
        let s = render_license_summary(&claims);
        assert!(s.contains("acme-corp"), "rendered: {s}");
        assert!(s.contains("seat_count=100"), "rendered: {s}");
        assert!(s.contains("cmk"), "rendered: {s}");
    }

    // ── F5.4-e-revoke ─────────────────────────────────────────────────

    #[test]
    fn license_jwt_id_hash_is_16_hex_chars() {
        let h = license_jwt_id_hash("any.jwt.string");
        assert_eq!(h.len(), 16);
        assert!(
            h.chars().all(|c| c.is_ascii_hexdigit()),
            "non-hex output: {h}"
        );
    }

    #[test]
    fn license_jwt_id_hash_is_deterministic() {
        let a = license_jwt_id_hash("abc");
        let b = license_jwt_id_hash("abc");
        assert_eq!(a, b);
        let c = license_jwt_id_hash("abd");
        assert_ne!(a, c, "different input must produce different hash");
    }

    fn write_revocation_list(records: &[RevocationRecord]) -> tempfile::NamedTempFile {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().expect("temp file");
        for r in records {
            let line = serde_json::to_string(r).expect("serialize");
            writeln!(f, "{line}").expect("write");
        }
        f
    }

    fn rev(hash: &str, reason: &str) -> RevocationRecord {
        RevocationRecord {
            ts_iso: "2026-05-22T00:00:00Z".into(),
            ts_unix: 1_779_408_000,
            enterprise_id: "acme-corp".into(),
            jwt_id_hash: hash.into(),
            reason: reason.into(),
        }
    }

    #[test]
    fn is_revoked_by_file_returns_match_on_hit() {
        let f = write_revocation_list(&[
            rev("aaaaaaaaaaaaaaaa", "unrelated"),
            rev("bbbbbbbbbbbbbbbb", "contract terminated"),
        ]);
        let hit = is_revoked_by_file(f.path(), "bbbbbbbbbbbbbbbb")
            .expect("read ok")
            .expect("hit");
        assert_eq!(hit.reason, "contract terminated");
    }

    #[test]
    fn is_revoked_by_file_returns_none_on_miss() {
        let f = write_revocation_list(&[rev("aaaaaaaaaaaaaaaa", "x")]);
        let miss = is_revoked_by_file(f.path(), "ffffffffffffffff").expect("read ok");
        assert!(miss.is_none());
    }

    #[test]
    fn is_revoked_by_file_skips_malformed_lines() {
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().expect("temp");
        // First line is garbage; second is a valid match. The boot
        // check must not let one corrupt line mask a legitimate
        // revocation further down.
        writeln!(f, "{{not valid json").expect("write");
        let rec = rev("ccccccccccccccc1", "key compromise");
        let line = serde_json::to_string(&rec).expect("ser");
        writeln!(f, "{line}").expect("write");
        let hit = is_revoked_by_file(f.path(), "ccccccccccccccc1")
            .expect("read ok")
            .expect("hit");
        assert_eq!(hit.reason, "key compromise");
    }

    #[test]
    fn is_revoked_by_file_errors_on_missing_file() {
        let err = is_revoked_by_file(
            std::path::Path::new("/tmp/this-path-must-not-exist-ministr-revoke-test"),
            "00",
        )
        .expect_err("missing file is an error");
        assert!(
            matches!(err, LicenseError::RevocationListUnreadable { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn revocation_record_round_trips_json() {
        let rec = rev("abcdef0123456789", "test reason");
        let s = serde_json::to_string(&rec).expect("ser");
        let back: RevocationRecord = serde_json::from_str(&s).expect("de");
        assert_eq!(back.jwt_id_hash, rec.jwt_id_hash);
        assert_eq!(back.reason, rec.reason);
        assert_eq!(back.enterprise_id, rec.enterprise_id);
    }

    #[test]
    fn load_revoked_hashes_collects_unique_hashes() {
        // F5.4-e-rotate — batch helper. Same record appearing twice
        // (re-revocation on a different reason) collapses to one
        // entry; malformed lines are skipped.
        use std::io::Write;
        let mut f = tempfile::NamedTempFile::new().expect("temp");
        let r1 = rev("1111111111111111", "first");
        let r2 = rev("2222222222222222", "second");
        let r1_dup = rev("1111111111111111", "redundant — same hash");
        for r in [&r1, &r2, &r1_dup] {
            writeln!(f, "{}", serde_json::to_string(r).expect("ser")).expect("write");
        }
        writeln!(f, "not-a-json-line").expect("write");
        let set = load_revoked_hashes(f.path()).expect("load");
        assert_eq!(set.len(), 2, "duplicate hash should collapse: {set:?}");
        assert!(set.contains("1111111111111111"));
        assert!(set.contains("2222222222222222"));
    }

    #[test]
    fn load_revoked_hashes_empty_file_returns_empty_set() {
        let f = tempfile::NamedTempFile::new().expect("temp");
        let set = load_revoked_hashes(f.path()).expect("load");
        assert!(set.is_empty());
    }
}
