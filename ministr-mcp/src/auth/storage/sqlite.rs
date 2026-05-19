//! SQLite-backed OAuth storage.
//!
//! For ACA single-replica deployments: the file lives on the Azure Files
//! mount (or any persistent path), so tokens, codes, and clients survive
//! pod restarts. Atomic `take_code` is implemented via a transaction.
//!
//! # Concurrency
//!
//! One `Connection` behind a `std::sync::Mutex`. OAuth I/O is infrequent
//! (<100 ops/min even under load), so a pool would be over-engineering.
//! Blocking SQL calls run on the `tokio::task::spawn_blocking` thread pool
//! to keep the async runtime free.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OptionalExtension, params};
use tokio::task;
use tracing::debug;

use super::super::types::{AccessToken, AuthorizationCode, RegisteredClient};
use super::{OAuthStorage, StorageError, StorageResult};

/// Persistent OAuth storage. The connection is wrapped in a blocking
/// mutex; all operations dispatch through `spawn_blocking`.
#[derive(Debug, Clone)]
pub(crate) struct SqliteStorage {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStorage {
    /// Open (or create) the OAuth database at `path`. Creates the schema
    /// on first use.
    #[allow(dead_code)] // wired into cmd_serve_http in PR1.4
    pub(crate) fn open(path: &Path) -> StorageResult<Self> {
        let conn = Connection::open(path)
            .map_err(|e| StorageError::Backend(format!("open {}: {e}", path.display())))?;
        configure(&conn)?;
        ensure_schema(&conn)?;
        debug!(path = %path.display(), "opened sqlite oauth store");
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn with_conn<T, F>(&self, op: F) -> impl Future<Output = StorageResult<T>> + Send
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> StorageResult<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        async move {
            task::spawn_blocking(move || {
                let mut guard = conn
                    .lock()
                    .map_err(|e| StorageError::Backend(format!("mutex poisoned: {e}")))?;
                op(&mut guard)
            })
            .await
            .map_err(|e| StorageError::Backend(format!("join: {e}")))?
        }
    }
}

#[allow(dead_code)] // used via SqliteStorage::open once wired in PR1.4
fn configure(conn: &Connection) -> StorageResult<()> {
    // WAL gives concurrent reads during writes; NORMAL sync balances
    // durability vs. latency on the Azure Files mount. A 5s busy timeout
    // tolerates brief SMB contention.
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| StorageError::Backend(format!("journal_mode: {e}")))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| StorageError::Backend(format!("synchronous: {e}")))?;
    conn.pragma_update(None, "busy_timeout", 5_000)
        .map_err(|e| StorageError::Backend(format!("busy_timeout: {e}")))?;
    Ok(())
}

#[allow(dead_code)] // used via SqliteStorage::open once wired in PR1.4
fn ensure_schema(conn: &Connection) -> StorageResult<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS oauth_clients (
            client_id TEXT PRIMARY KEY,
            data      TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS oauth_codes (
            code       TEXT PRIMARY KEY,
            expires_at INTEGER NOT NULL,
            data       TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_oauth_codes_expires
            ON oauth_codes(expires_at);
         CREATE TABLE IF NOT EXISTS oauth_tokens (
            token      TEXT PRIMARY KEY,
            expires_at INTEGER NOT NULL,
            data       TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_oauth_tokens_expires
            ON oauth_tokens(expires_at);",
    )
    .map_err(|e| StorageError::Backend(format!("schema: {e}")))?;
    Ok(())
}

impl OAuthStorage for SqliteStorage {
    fn save_client(
        &self,
        client: RegisteredClient,
    ) -> impl Future<Output = StorageResult<()>> + Send {
        self.with_conn(move |conn| {
            let blob = serde_json::to_string(&client)?;
            conn.execute(
                "INSERT INTO oauth_clients (client_id, data) VALUES (?1, ?2)
                 ON CONFLICT(client_id) DO UPDATE SET data = excluded.data",
                params![client.client_id, blob],
            )
            .map_err(|e| StorageError::Backend(format!("save_client: {e}")))?;
            Ok(())
        })
    }

    fn get_client(
        &self,
        client_id: &str,
    ) -> impl Future<Output = StorageResult<Option<RegisteredClient>>> + Send {
        let client_id = client_id.to_owned();
        self.with_conn(move |conn| {
            let blob: Option<String> = conn
                .query_row(
                    "SELECT data FROM oauth_clients WHERE client_id = ?1",
                    params![client_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| StorageError::Backend(format!("get_client: {e}")))?;
            match blob {
                Some(s) => Ok(Some(serde_json::from_str(&s)?)),
                None => Ok(None),
            }
        })
    }

    fn save_code(
        &self,
        code: AuthorizationCode,
    ) -> impl Future<Output = StorageResult<()>> + Send {
        self.with_conn(move |conn| {
            let blob = serde_json::to_string(&code)?;
            conn.execute(
                "INSERT INTO oauth_codes (code, expires_at, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT(code) DO UPDATE SET expires_at = excluded.expires_at, data = excluded.data",
                params![code.code, code.expires_at.cast_signed(), blob],
            )
            .map_err(|e| StorageError::Backend(format!("save_code: {e}")))?;
            Ok(())
        })
    }

    fn take_code(
        &self,
        code: &str,
    ) -> impl Future<Output = StorageResult<Option<AuthorizationCode>>> + Send {
        let code = code.to_owned();
        self.with_conn(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|e| StorageError::Backend(format!("begin: {e}")))?;
            let blob: Option<String> = tx
                .query_row(
                    "SELECT data FROM oauth_codes WHERE code = ?1",
                    params![code],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| StorageError::Backend(format!("take_code select: {e}")))?;
            if blob.is_some() {
                tx.execute(
                    "DELETE FROM oauth_codes WHERE code = ?1",
                    params![code],
                )
                .map_err(|e| StorageError::Backend(format!("take_code delete: {e}")))?;
            }
            tx.commit()
                .map_err(|e| StorageError::Backend(format!("commit: {e}")))?;
            match blob {
                Some(s) => Ok(Some(serde_json::from_str(&s)?)),
                None => Ok(None),
            }
        })
    }

    fn save_token(&self, token: AccessToken) -> impl Future<Output = StorageResult<()>> + Send {
        self.with_conn(move |conn| {
            let blob = serde_json::to_string(&token)?;
            conn.execute(
                "INSERT INTO oauth_tokens (token, expires_at, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT(token) DO UPDATE SET expires_at = excluded.expires_at, data = excluded.data",
                params![token.token, token.expires_at.cast_signed(), blob],
            )
            .map_err(|e| StorageError::Backend(format!("save_token: {e}")))?;
            Ok(())
        })
    }

    fn get_token(
        &self,
        token: &str,
    ) -> impl Future<Output = StorageResult<Option<AccessToken>>> + Send {
        let token = token.to_owned();
        self.with_conn(move |conn| {
            let blob: Option<String> = conn
                .query_row(
                    "SELECT data FROM oauth_tokens WHERE token = ?1",
                    params![token],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| StorageError::Backend(format!("get_token: {e}")))?;
            match blob {
                Some(s) => Ok(Some(serde_json::from_str(&s)?)),
                None => Ok(None),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::util::epoch_now;
    use tempfile::tempdir;

    fn open_in_tempdir() -> (tempfile::TempDir, SqliteStorage) {
        let dir = tempdir().unwrap();
        let path = dir.path().join("oauth.db");
        let storage = SqliteStorage::open(&path).unwrap();
        (dir, storage)
    }

    #[tokio::test]
    async fn token_round_trip() {
        let (_dir, storage) = open_in_tempdir();
        let token = AccessToken {
            token: "tok-1".into(),
            client_id: "client-a".into(),
            scope: "ministr:read".into(),
            expires_at: epoch_now() + 3600,
        };
        storage.save_token(token.clone()).await.unwrap();
        let got = storage.get_token("tok-1").await.unwrap().unwrap();
        assert_eq!(got.client_id, "client-a");
        assert_eq!(got.scope, "ministr:read");
    }

    #[tokio::test]
    async fn take_code_is_atomic_and_idempotent() {
        let (_dir, storage) = open_in_tempdir();
        let code = AuthorizationCode {
            code: "code-1".into(),
            client_id: "client-a".into(),
            redirect_uri: "http://x".into(),
            scope: "ministr:read".into(),
            code_challenge: "abc".into(),
            code_challenge_method: "S256".into(),
            expires_at: epoch_now() + 60,
        };
        storage.save_code(code).await.unwrap();
        assert!(storage.take_code("code-1").await.unwrap().is_some());
        assert!(storage.take_code("code-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn data_survives_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("oauth.db");

        {
            let storage = SqliteStorage::open(&path).unwrap();
            let token = AccessToken {
                token: "survives".into(),
                client_id: "c".into(),
                scope: "ministr:read".into(),
                expires_at: epoch_now() + 3600,
            };
            storage.save_token(token).await.unwrap();
        }

        let storage = SqliteStorage::open(&path).unwrap();
        let got = storage.get_token("survives").await.unwrap();
        assert!(got.is_some(), "token should persist across reopens");
    }

    #[tokio::test]
    async fn upsert_replaces_existing_client() {
        let (_dir, storage) = open_in_tempdir();
        let mut client = RegisteredClient {
            client_id: "c1".into(),
            client_secret: Some("v1".into()),
            redirect_uris: vec!["http://a".into()],
            client_name: None,
            scope: "ministr:read".into(),
            registered_at: epoch_now(),
        };
        storage.save_client(client.clone()).await.unwrap();
        client.client_secret = Some("v2".into());
        storage.save_client(client).await.unwrap();
        let got = storage.get_client("c1").await.unwrap().unwrap();
        assert_eq!(got.client_secret.as_deref(), Some("v2"));
    }
}
