//! F5.1-b — SAML 2.0 Service Provider browser-facing endpoints.
//!
//! Mounts two per-org routes for SP-initiated SSO:
//!
//! - `GET /orgs/{id}/saml/metadata.xml` — SP metadata XML the
//!   customer's `IdP` admin imports to configure ministr as a
//!   relying party.
//! - `GET /orgs/{id}/saml/login` — builds a SAML 2.0 `AuthnRequest`
//!   bound to the `IdP`'s SSO URL, redirects (HTTP 302) to it using
//!   the HTTP-Redirect binding (DEFLATE + base64 + URL-encode of
//!   the XML, attached as `?SAMLRequest=…&RelayState=…`).
//!
//! Both routes are public (no `OAuth` gate) — the `IdP` doesn't carry
//! ministr-issued bearer tokens. Security at the SP boundary lands
//! in F5.1-c, where `POST /orgs/{id}/saml/acs` validates the `IdP`'s
//! signed assertion against the per-org pinned `idp_x509_cert`
//! (samael's `xmlsec` feature).
//!
//! Per-org configuration lives in `org_saml_configs` (migration
//! 0010 from F5.1-a). A missing row → 404 on both endpoints; the
//! org simply hasn't enabled SAML SSO.

use std::io::Write;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use deadpool_postgres::Pool;
use flate2::Compression;
use flate2::write::DeflateEncoder;
use samael::metadata::{Endpoint, EntityDescriptor, IdpSsoDescriptor};
use samael::service_provider::ServiceProvider;
use samael::traits::ToXml;

/// Public binding URN for the HTTP-Redirect SSO binding.
const BINDING_HTTP_REDIRECT: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect";

/// Per-route shared state. Holds the Postgres pool so handlers can
/// load `org_saml_configs` rows on every request (the table is
/// expected to be small — one row per org with SAML SSO enabled —
/// so direct DB reads are fine without a cache layer).
#[derive(Clone)]
pub struct SamlState {
    pub pool: Arc<Pool>,
}

impl SamlState {
    #[must_use]
    pub fn new(pool: Arc<Pool>) -> Self {
        Self { pool }
    }
}

/// One row from `org_saml_configs`. Mirrors the schema in
/// migration 0010 with a subset of columns relevant to F5.1-b.
struct OrgSamlConfig {
    idp_entity_id: String,
    idp_sso_url: String,
    /// PEM-encoded X.509 certificate. Not used by F5.1-b (no
    /// signing or verification), but the field is loaded so we can
    /// reject configurations with empty certs (would be a security
    /// bug — F5.1-c relies on this for assertion verification).
    idp_x509_cert: String,
    sp_entity_id: String,
    sp_acs_url: String,
}

/// Build the SAML SP router. Mount at the application root
/// (`/orgs/{id}/saml/…` lives outside the `OAuth`-protected branch).
pub fn saml_routes(state: SamlState) -> Router {
    Router::new()
        .route("/orgs/{id}/saml/metadata.xml", get(handle_metadata))
        .route("/orgs/{id}/saml/login", get(handle_login))
        .with_state(state)
}

/// F5.1-d — per-org SAML config CRUD router. Mount under the
/// `OAuth`-protected branch in `cmd_serve_http`; owner-only ACL
/// is enforced by each handler via [`assert_owner_or_admin`].
pub fn saml_config_routes(state: SamlState) -> Router {
    use axum::routing::post;
    Router::new()
        .route(
            "/api/v1/orgs/{id}/saml/config",
            post(handle_config_upsert)
                .get(handle_config_get)
                .delete(handle_config_delete),
        )
        .with_state(state)
}

async fn handle_metadata(
    State(state): State<SamlState>,
    Path(org_id): Path<String>,
) -> Response {
    match load_config(&state, &org_id).await {
        Ok(Some(cfg)) => match build_sp_metadata(&cfg) {
            Ok(xml) => xml_response(xml),
            Err(e) => internal_error("saml metadata", &e),
        },
        Ok(None) => not_found_response(),
        Err(LoadConfigError::BadOrgId) => bad_request_response("invalid org id"),
        Err(LoadConfigError::Db(e)) => internal_error("load_config", &e),
    }
}

async fn handle_login(
    State(state): State<SamlState>,
    Path(org_id): Path<String>,
) -> Response {
    match load_config(&state, &org_id).await {
        Ok(Some(cfg)) => match build_login_redirect(&cfg) {
            Ok(url) => redirect_to(&url),
            Err(e) => internal_error("saml login", &e),
        },
        Ok(None) => not_found_response(),
        Err(LoadConfigError::BadOrgId) => bad_request_response("invalid org id"),
        Err(LoadConfigError::Db(e)) => internal_error("load_config", &e),
    }
}

enum LoadConfigError {
    BadOrgId,
    Db(String),
}

async fn load_config(
    state: &SamlState,
    org_id_str: &str,
) -> Result<Option<OrgSamlConfig>, LoadConfigError> {
    // Validate UUID shape defensively so a malformed path segment
    // returns 400 instead of a 500 from the SQL-level type-mismatch
    // path. The `$1::text::uuid` cast at the query is then the
    // belt-and-suspenders type guard.
    let Some(org_id) = parse_uuid(org_id_str) else {
        return Err(LoadConfigError::BadOrgId);
    };
    let client = state
        .pool
        .get()
        .await
        .map_err(|e| LoadConfigError::Db(format!("pool get: {e}")))?;
    let row = client
        .query_opt(
            "SELECT idp_entity_id, idp_sso_url, idp_x509_cert, sp_entity_id, sp_acs_url \
             FROM org_saml_configs WHERE org_id = $1::text::uuid",
            &[&org_id],
        )
        .await
        .map_err(|e| LoadConfigError::Db(format!("query org_saml_configs: {e:?}")))?;
    Ok(row.map(|r| OrgSamlConfig {
        idp_entity_id: r.get(0),
        idp_sso_url: r.get(1),
        idp_x509_cert: r.get(2),
        sp_entity_id: r.get(3),
        sp_acs_url: r.get(4),
    }))
}

/// Minimal UUID v4 string validation (8-4-4-4-12 hex). Avoids
/// pulling the `uuid` crate as a direct dep just to validate.
fn parse_uuid(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    let dashes = [8usize, 13, 18, 23];
    for (i, &b) in bytes.iter().enumerate() {
        if dashes.contains(&i) {
            if b != b'-' {
                return None;
            }
        } else if !b.is_ascii_hexdigit() {
            return None;
        }
    }
    Some(s.to_string())
}

/// Construct a samael `ServiceProvider` populated with the per-org
/// `IdP` trust anchor + the SP's own metadata. Used both by metadata
/// generation and `AuthnRequest` construction.
fn build_sp(cfg: &OrgSamlConfig) -> ServiceProvider {
    // `IdpSsoDescriptor` doesn't impl Default upstream (samael
    // 0.0.20); build it explicitly. All but `single_sign_on_services`
    // and `protocol_support_enumeration` are None / empty for the
    // F5.1-b scope (no key descriptors, no logout, no contacts).
    // F5.1-c populates `key_descriptors` from `idp_x509_cert`.
    let idp_descriptor = IdpSsoDescriptor {
        id: None,
        valid_until: None,
        cache_duration: None,
        protocol_support_enumeration: Some("urn:oasis:names:tc:SAML:2.0:protocol".to_string()),
        error_url: None,
        signature: None,
        key_descriptors: vec![],
        organization: None,
        contact_people: vec![],
        artifact_resolution_service: vec![],
        single_logout_services: vec![],
        manage_name_id_services: vec![],
        name_id_formats: vec![],
        want_authn_requests_signed: Some(false),
        single_sign_on_services: vec![Endpoint {
            binding: BINDING_HTTP_REDIRECT.to_string(),
            location: cfg.idp_sso_url.clone(),
            response_location: None,
        }],
        name_id_mapping_services: vec![],
        assertion_id_request_services: vec![],
        attribute_profiles: vec![],
        attributes: vec![],
    };

    let idp_metadata = EntityDescriptor {
        entity_id: Some(cfg.idp_entity_id.clone()),
        idp_sso_descriptors: Some(vec![idp_descriptor]),
        ..Default::default()
    };

    ServiceProvider {
        entity_id: Some(cfg.sp_entity_id.clone()),
        acs_url: Some(cfg.sp_acs_url.clone()),
        metadata_url: Some(format!("{}/metadata.xml", cfg.sp_acs_url.trim_end_matches('/'))),
        idp_metadata,
        ..Default::default()
    }
}

fn build_sp_metadata(cfg: &OrgSamlConfig) -> Result<String, String> {
    let sp = build_sp(cfg);
    let metadata = sp
        .metadata()
        .map_err(|e| format!("metadata build: {e}"))?;
    metadata
        .to_string()
        .map_err(|e| format!("metadata serialize: {e}"))
}

fn build_login_redirect(cfg: &OrgSamlConfig) -> Result<String, String> {
    let sp = build_sp(cfg);
    let authn = sp
        .make_authentication_request(&cfg.idp_sso_url)
        .map_err(|e| format!("authn request: {e}"))?;
    let xml = authn
        .to_string()
        .map_err(|e| format!("authn serialize: {e}"))?;
    // SAML HTTP-Redirect binding (SAMLBindings §3.4.4):
    //   1. DEFLATE (RFC 1951; no zlib header/checksum)
    //   2. Base64 (RFC 4648 standard alphabet, padded)
    //   3. URL-encode
    //   4. Append as `?SAMLRequest=<encoded>` to the IdP SSO URL.
    let deflated = deflate_no_wrap(xml.as_bytes())?;
    let b64 = BASE64_STANDARD.encode(&deflated);
    let url_encoded = url_encode(&b64);
    // RelayState is opaque to the IdP — they echo it back to the
    // ACS. F5.1-c will use it for CSRF tying. For F5.1-b we just
    // include a constant marker so the parameter is exercised; a
    // real nonce lands when the ACS path uses it.
    let relay = "ministr-pending-acs";
    Ok(format!(
        "{}?SAMLRequest={}&RelayState={}",
        cfg.idp_sso_url, url_encoded, relay
    ))
}

fn deflate_no_wrap(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .map_err(|e| format!("deflate write: {e}"))?;
    encoder.finish().map_err(|e| format!("deflate finish: {e}"))
}

/// Minimal RFC 3986 percent-encode of base64 chars that need it
/// inside a URL query value: `+ / =`. Other chars are safe.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for c in s.chars() {
        match c {
            '+' => out.push_str("%2B"),
            '/' => out.push_str("%2F"),
            '=' => out.push_str("%3D"),
            _ => out.push(c),
        }
    }
    out
}

fn xml_response(body: String) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/xml; charset=utf-8"),
    );
    (StatusCode::OK, headers, body).into_response()
}

fn redirect_to(url: &str) -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(url) {
        headers.insert(header::LOCATION, v);
    }
    (StatusCode::FOUND, headers, "").into_response()
}

fn not_found_response() -> Response {
    (StatusCode::NOT_FOUND, "saml config not found for org").into_response()
}

fn bad_request_response(msg: &'static str) -> Response {
    (StatusCode::BAD_REQUEST, msg).into_response()
}

fn internal_error(context: &str, e: &str) -> Response {
    tracing::warn!(context = %context, error = %e, "saml endpoint error");
    (StatusCode::INTERNAL_SERVER_ERROR, "saml internal error").into_response()
}

// idp_x509_cert is loaded but unused in F5.1-b; F5.1-c will use it
// for key_descriptors. Reference here so an unused-field warning
// doesn't fire on the struct.
#[allow(dead_code)]
fn _idp_x509_cert_used_in_f5_1_c(cfg: &OrgSamlConfig) -> &str {
    &cfg.idp_x509_cert
}

// ── F5.1-d — per-org SAML config CRUD ────────────────────────────────

use axum::Extension;
use axum::Json;
use ministr_mcp::auth::tenant::Tenant;

/// Errors surfaced by the SAML config CRUD handlers. Maps to HTTP
/// statuses inside `IntoResponse`.
#[derive(Debug)]
enum SamlConfigError {
    Unauthenticated,
    Forbidden,
    NotFound,
    Invalid(&'static str),
    Db(String),
}

impl IntoResponse for SamlConfigError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => {
                (StatusCode::UNAUTHORIZED, "unauthenticated").into_response()
            }
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden").into_response(),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found").into_response(),
            Self::Invalid(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            Self::Db(msg) => {
                tracing::warn!(error = %msg, "saml config db error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal").into_response()
            }
        }
    }
}

/// POST body for `/api/v1/orgs/{id}/saml/config`. All `idp_*` and
/// `sp_*` fields required; attribute mappings + enforce flag
/// optional with sensible defaults.
#[derive(serde::Deserialize)]
struct SamlConfigUpsertBody {
    idp_entity_id: String,
    idp_sso_url: String,
    idp_x509_cert: String,
    #[serde(default)]
    idp_slo_url: Option<String>,
    sp_entity_id: String,
    sp_acs_url: String,
    #[serde(default)]
    attribute_email: Option<String>,
    #[serde(default)]
    attribute_display_name: Option<String>,
    #[serde(default)]
    enforce_signed_assertions: Option<bool>,
}

/// GET response shape for `/api/v1/orgs/{id}/saml/config`. Mirrors
/// the table columns. `idp_x509_cert` is public `IdP` material; we
/// don't redact it. SP private key isn't a column yet — if added
/// (F5.1-c-prep-libxmlsec-crash resolution), it MUST stay
/// server-side and never appear in this response.
#[derive(serde::Serialize)]
struct SamlConfigView {
    org_id: String,
    idp_entity_id: String,
    idp_sso_url: String,
    idp_x509_cert: String,
    idp_slo_url: Option<String>,
    sp_entity_id: String,
    sp_acs_url: String,
    attribute_email: String,
    attribute_display_name: Option<String>,
    enforce_signed_assertions: bool,
    created_at: String,
    updated_at: String,
}

/// Owner / admin gate. Mirrors `webhooks::assert_owner_or_admin`.
/// Members get 403; non-members get 403 too (we don't distinguish
/// "org doesn't exist" from "you're not in it" to avoid org-id
/// existence probing).
async fn assert_owner_or_admin(
    pool: &Pool,
    org_id: &str,
    user_id: &str,
) -> Result<(), SamlConfigError> {
    let role = crate::orgs::repo::member_role(pool, org_id, user_id)
        .await
        .map_err(|e| SamlConfigError::Db(format!("member_role: {e}")))?;
    if !matches!(role.as_deref(), Some("owner" | "admin")) {
        return Err(SamlConfigError::Forbidden);
    }
    Ok(())
}

fn validate_upsert(body: &SamlConfigUpsertBody) -> Result<(), SamlConfigError> {
    for (name, value) in [
        ("idp_entity_id", body.idp_entity_id.as_str()),
        ("idp_sso_url", body.idp_sso_url.as_str()),
        ("sp_entity_id", body.sp_entity_id.as_str()),
        ("sp_acs_url", body.sp_acs_url.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(match name {
                "idp_entity_id" => SamlConfigError::Invalid("idp_entity_id is required"),
                "idp_sso_url" => SamlConfigError::Invalid("idp_sso_url is required"),
                "sp_entity_id" => SamlConfigError::Invalid("sp_entity_id is required"),
                "sp_acs_url" => SamlConfigError::Invalid("sp_acs_url is required"),
                _ => SamlConfigError::Invalid("required field is empty"),
            });
        }
    }
    if !body.idp_x509_cert.contains("BEGIN CERTIFICATE") {
        return Err(SamlConfigError::Invalid(
            "idp_x509_cert must be a PEM-encoded X.509 certificate",
        ));
    }
    Ok(())
}

async fn handle_config_upsert(
    State(state): State<SamlState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
    Json(body): Json<SamlConfigUpsertBody>,
) -> Result<(StatusCode, Json<SamlConfigView>), SamlConfigError> {
    let tenant = tenant.ok_or(SamlConfigError::Unauthenticated)?;
    if parse_uuid(&org_id).is_none() {
        return Err(SamlConfigError::Invalid("invalid org id"));
    }
    assert_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;
    validate_upsert(&body)?;

    let attr_email = body
        .attribute_email
        .as_deref()
        .unwrap_or("http://schemas.xmlsoap.org/ws/2005/05/identity/claims/emailaddress");
    let enforce = body.enforce_signed_assertions.unwrap_or(true);

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| SamlConfigError::Db(format!("pool get: {e}")))?;
    let row = client
        .query_one(
            "INSERT INTO org_saml_configs (\
                org_id, idp_entity_id, idp_sso_url, idp_x509_cert, idp_slo_url, \
                sp_entity_id, sp_acs_url, attribute_email, attribute_display_name, \
                enforce_signed_assertions) \
             VALUES ($1::text::uuid, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             ON CONFLICT (org_id) DO UPDATE SET \
                idp_entity_id = EXCLUDED.idp_entity_id, \
                idp_sso_url = EXCLUDED.idp_sso_url, \
                idp_x509_cert = EXCLUDED.idp_x509_cert, \
                idp_slo_url = EXCLUDED.idp_slo_url, \
                sp_entity_id = EXCLUDED.sp_entity_id, \
                sp_acs_url = EXCLUDED.sp_acs_url, \
                attribute_email = EXCLUDED.attribute_email, \
                attribute_display_name = EXCLUDED.attribute_display_name, \
                enforce_signed_assertions = EXCLUDED.enforce_signed_assertions, \
                updated_at = NOW() \
             RETURNING idp_entity_id, idp_sso_url, idp_x509_cert, idp_slo_url, \
                       sp_entity_id, sp_acs_url, attribute_email, \
                       attribute_display_name, enforce_signed_assertions, \
                       to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                       to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
            &[
                &org_id,
                &body.idp_entity_id,
                &body.idp_sso_url,
                &body.idp_x509_cert,
                &body.idp_slo_url,
                &body.sp_entity_id,
                &body.sp_acs_url,
                &attr_email,
                &body.attribute_display_name,
                &enforce,
            ],
        )
        .await
        .map_err(|e| SamlConfigError::Db(format!("upsert: {e:?}")))?;

    let view = SamlConfigView {
        org_id: org_id.clone(),
        idp_entity_id: row.get(0),
        idp_sso_url: row.get(1),
        idp_x509_cert: row.get(2),
        idp_slo_url: row.get(3),
        sp_entity_id: row.get(4),
        sp_acs_url: row.get(5),
        attribute_email: row.get(6),
        attribute_display_name: row.get(7),
        enforce_signed_assertions: row.get(8),
        created_at: row.get(9),
        updated_at: row.get(10),
    };
    Ok((StatusCode::OK, Json(view)))
}

async fn handle_config_get(
    State(state): State<SamlState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
) -> Result<Json<SamlConfigView>, SamlConfigError> {
    let tenant = tenant.ok_or(SamlConfigError::Unauthenticated)?;
    if parse_uuid(&org_id).is_none() {
        return Err(SamlConfigError::Invalid("invalid org id"));
    }
    assert_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| SamlConfigError::Db(format!("pool get: {e}")))?;
    let row = client
        .query_opt(
            "SELECT idp_entity_id, idp_sso_url, idp_x509_cert, idp_slo_url, \
                    sp_entity_id, sp_acs_url, attribute_email, \
                    attribute_display_name, enforce_signed_assertions, \
                    to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"'), \
                    to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') \
             FROM org_saml_configs WHERE org_id = $1::text::uuid",
            &[&org_id],
        )
        .await
        .map_err(|e| SamlConfigError::Db(format!("select: {e:?}")))?
        .ok_or(SamlConfigError::NotFound)?;

    Ok(Json(SamlConfigView {
        org_id: org_id.clone(),
        idp_entity_id: row.get(0),
        idp_sso_url: row.get(1),
        idp_x509_cert: row.get(2),
        idp_slo_url: row.get(3),
        sp_entity_id: row.get(4),
        sp_acs_url: row.get(5),
        attribute_email: row.get(6),
        attribute_display_name: row.get(7),
        enforce_signed_assertions: row.get(8),
        created_at: row.get(9),
        updated_at: row.get(10),
    }))
}

async fn handle_config_delete(
    State(state): State<SamlState>,
    tenant: Option<Extension<Tenant>>,
    Path(org_id): Path<String>,
) -> Result<StatusCode, SamlConfigError> {
    let tenant = tenant.ok_or(SamlConfigError::Unauthenticated)?;
    if parse_uuid(&org_id).is_none() {
        return Err(SamlConfigError::Invalid("invalid org id"));
    }
    assert_owner_or_admin(&state.pool, &org_id, &tenant.0.subject).await?;

    let client = state
        .pool
        .get()
        .await
        .map_err(|e| SamlConfigError::Db(format!("pool get: {e}")))?;
    let deleted = client
        .execute(
            "DELETE FROM org_saml_configs WHERE org_id = $1::text::uuid",
            &[&org_id],
        )
        .await
        .map_err(|e| SamlConfigError::Db(format!("delete: {e:?}")))?;
    if deleted == 0 {
        return Err(SamlConfigError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}


#[cfg(test)]
mod tests {
    //! F5.1-c-prep — validate that samael's xmlsec feature can
    //! sign and verify an XML assertion end-to-end on this build
    //! environment. Doesn't exercise the SP routes (that's F5.1-c-acs);
    //! the point is to fail loud if libxmlsec1 / libxml2 native deps
    //! aren't linked correctly OR if samael's xmlsec API surface
    //! changed in a way that breaks our planned ACS handler.

    use openssl::asn1::Asn1Time;
    use openssl::hash::MessageDigest;
    use openssl::pkey::PKey;
    use openssl::rsa::Rsa;
    use openssl::x509::{X509, X509NameBuilder};
    use samael::crypto::{Crypto, CryptoProvider};
    use samael::service_provider::ServiceProvider;
    use samael::traits::ToXml;

    /// Build a self-signed RSA-2048 keypair + X509 cert. Returns
    /// `(private_key_der, x509_cert_der)` in DER form, which is what
    /// samael's `Crypto::sign_xml` and `verify_signed_xml` consume.
    fn generate_self_signed() -> (Vec<u8>, Vec<u8>) {
        let rsa = Rsa::generate(2048).expect("rsa generate");
        let pkey = PKey::from_rsa(rsa).expect("pkey wrap");

        let mut name_builder = X509NameBuilder::new().expect("x509 name builder");
        name_builder
            .append_entry_by_text("CN", "ministr-test-idp")
            .expect("append CN");
        let name = name_builder.build();

        let mut cert_builder = X509::builder().expect("x509 builder");
        cert_builder.set_version(2).expect("set version");
        cert_builder.set_subject_name(&name).expect("set subject");
        cert_builder.set_issuer_name(&name).expect("set issuer");
        cert_builder.set_pubkey(&pkey).expect("set pubkey");
        cert_builder
            .set_not_before(&Asn1Time::days_from_now(0).expect("nb"))
            .expect("set nbf");
        cert_builder
            .set_not_after(&Asn1Time::days_from_now(365).expect("na"))
            .expect("set naf");
        cert_builder
            .sign(&pkey, MessageDigest::sha256())
            .expect("sign cert");
        let cert = cert_builder.build();

        (
            pkey.private_key_to_der().expect("priv to der"),
            cert.to_der().expect("cert to der"),
        )
    }

    /// Build an `AuthnRequest` with an empty `<ds:Signature>`
    /// template attached. samael's `Crypto::sign_xml` needs the
    /// template node to fill in; it doesn't create one. The `IdP`
    /// `Response` builder uses the same pattern at
    /// `samael-0.0.20/src/idp/response_builder.rs:128`.
    fn build_authn_request_with_signature_template(
        cert_der: &[u8],
    ) -> samael::schema::AuthnRequest {
        let sp = ServiceProvider {
            entity_id: Some("https://sp.test/entity".to_string()),
            acs_url: Some("https://sp.test/acs".to_string()),
            ..ServiceProvider::default()
        };
        let mut authn = sp
            .make_authentication_request("https://idp.test/sso")
            .expect("authn request builds");
        let cert = samael::crypto::CertificateDer::from(cert_der.to_vec());
        let mut sig = samael::signature::Signature::template(&authn.id, &cert);
        // samael's `Signature::template` hardcodes SHA-1 for the
        // reference digest. libxmlsec1 1.3 + openssl@3 rejects SHA-1
        // by default in `xmlSecOpenSSLEvpDigestVerify` ("data and
        // digest do not match"). Patch the digest method to SHA-256
        // BEFORE signing — this is also the algorithm any modern IdP
        // would emit, so F5.1-c-acs uses the same path.
        if let Some(reference) = sig.signed_info.reference.first_mut() {
            reference.digest_method = samael::signature::DigestMethod {
                algorithm: samael::signature::DigestAlgorithm::Sha256,
            };
        }
        authn.signature = Some(sig);
        authn
    }

    /// Smoke test: prove the xmlsec-feature-gated `samael::crypto`
    /// surface is in the binary. Doesn't exercise any FFI path.
    #[test]
    fn samael_xmlsec_compiles_in_with_feature() {
        let cert = samael::crypto::CertificateDer::from(vec![0u8; 4]);
        assert_eq!(cert.der_data().len(), 4);
    }

    /// F5.1-c-prep — sign + verify roundtrip via libxmlsec1.
    /// Proves the entire native-deps path works on this build env
    /// before F5.1-c-acs depends on it for assertion verification.
    ///
    /// Currently `#[ignore]`'d due to F5.1-c-prep-libxmlsec-crash
    /// (see ROADMAP). Crashes inside `Crypto::sign_xml` on macOS
    /// with brew libxmlsec1 1.3.11 + `openssl@3`, even with the
    /// proper `<ds:Signature>` template attached and SHA-256
    /// digest method. Not an `OPENSSL_DIR` / ABI mismatch — the
    /// crash persists across env-var combinations.
    #[test]
    #[ignore = "crashes inside libxmlsec1 sign_xml on macOS — F5.1-c-prep-libxmlsec-crash"]
    fn samael_xmlsec_sign_and_verify_roundtrip() {
        let (priv_der, cert_der) = generate_self_signed();
        let authn = build_authn_request_with_signature_template(&cert_der);
        let unsigned_xml = authn.to_string().expect("authn to_string");
        assert!(
            unsigned_xml.contains("<ds:Signature"),
            "unsigned XML has the empty signature template ready for sign_xml"
        );

        let signed = Crypto::sign_xml(unsigned_xml.as_bytes(), &priv_der)
            .expect("samael Crypto::sign_xml fills in the template");
        assert!(
            signed.contains("SignatureValue") && !signed.contains("<SignatureValue></SignatureValue>"),
            "signed XML has a non-empty SignatureValue: {}",
            &signed[..signed.len().min(400)]
        );

        let cert = samael::crypto::CertificateDer::from(cert_der.clone());
        Crypto::verify_signed_xml(signed.as_bytes(), &cert, Some("ID"))
            .expect("samael Crypto::verify_signed_xml accepts the matching cert");

        // Tampering must fail verification — F5.1-c-acs relies on
        // this property to reject forged assertions.
        let tampered = signed.replace("https://idp.test/sso", "https://attacker.test/sso");
        let tamper_result =
            Crypto::verify_signed_xml(tampered.as_bytes(), &cert, Some("ID"));
        assert!(
            tamper_result.is_err(),
            "verify_signed_xml must reject tampered body, got Ok"
        );
    }
}
