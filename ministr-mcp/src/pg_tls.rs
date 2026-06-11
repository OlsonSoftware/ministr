//! Shared Postgres TLS connector for every deadpool/rustls pool in the
//! workspace (OAuth storage, the admin job queue, and ministr-cloud's
//! tenant pool).
//!
//! One trust policy, one place: the Mozilla CA bundle, plus any roots
//! supplied via `MINISTR_PG_CA_CERT` (PEM contents in the env) for
//! providers with a PRIVATE per-cluster CA — `DigitalOcean` managed
//! Postgres being the motivating case. Azure Postgres Flex / AWS RDS /
//! Google Cloud SQL chain to public roots and need no extra config.
//!
//! Extracted after the third copy-paste connector shipped without the
//! CA hook and silently disabled the cloud `WorkerLoop` — a fourth pool
//! should call this and be done.

use rustls::ClientConfig;
use tokio_postgres_rustls::MakeRustlsConnect;

/// Build the workspace-standard Postgres TLS connector.
#[must_use]
pub fn make_rustls_connector() -> MakeRustlsConnect {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    if let Ok(pem) = std::env::var("MINISTR_PG_CA_CERT") {
        let certs = rustls_pemfile::certs(&mut pem.as_bytes())
            .collect::<Result<Vec<_>, _>>()
            .unwrap_or_default();
        let (added, _ignored) = roots.add_parsable_certificates(certs);
        tracing::info!(
            added,
            "added MINISTR_PG_CA_CERT root(s) to the pg TLS trust store"
        );
    }
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    MakeRustlsConnect::new(config)
}
