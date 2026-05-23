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
/// # Errors
///
/// Surfaces partial config + validation errors so the boot path can
/// refuse to start when the operator's license setup is broken.
pub fn validate_license_from_env() -> Result<Option<LicenseClaims>, LicenseError> {
    let jwt = std::env::var("MINISTR_LICENSE_KEY").ok();
    let pubkey = std::env::var("MINISTR_LICENSE_PUBLIC_KEY").ok();
    match (jwt, pubkey) {
        (None, None) => Ok(None),
        (Some(j), Some(p)) if !j.trim().is_empty() && !p.trim().is_empty() => {
            Ok(Some(validate_license_key(&j, &p)?))
        }
        _ => Err(LicenseError::PartialConfig),
    }
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
}
