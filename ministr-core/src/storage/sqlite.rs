//! SQLite-backed [`Storage`] implementation.
//!
//! All rusqlite calls are wrapped in `tokio::spawn_blocking` to avoid
//! blocking the async runtime. Connections are checked out of a small
//! pool for each call so WAL mode actually buys us concurrent reads —
//! a single shared connection would serialize every query behind one
//! mutex even though WAL permits parallel readers.
//!
//! The pool uses `parking_lot` primitives (no poisoning), so a panic
//! inside one `with_conn` closure doesn't permanently disable the
//! storage layer.

use std::path::Path;
use std::sync::Arc;

use parking_lot::{Condvar, Mutex};
use rusqlite::Connection;
use tracing::instrument;

use super::schema::{configure_connection, run_migrations};
use super::traits::{
    BridgeEndpointRecord, BridgeLinkDetail, BridgeLinkRecord, ClaimRecord, CoAccessRecord,
    CorpusMerkleRecord, CorpusStats, DocumentRecord, FileHashRecord, GitCacheRecord,
    PendingRefRecord, RelatedClaimRecord, SectionAccessStat, SectionRecord, Storage, SymbolFilter,
    SymbolRecord, SymbolRefRecord, WebCacheRecord,
};
use crate::error::StorageError;
use crate::session::{DeliveredItem, Session, SessionId};
use crate::types::{
    ClaimId, ClaimRelationship, ContentId, CorpusRoot, DocumentTree, RefKind, RelationType,
    Resolution, RootKind, Section, SectionId, SymbolId,
};

/// Default file-backed pool size. 8 connections is generous for a single
/// corpus and keeps the blocking-pool contention discussion bounded —
/// anything beyond this and the SQLite write lock becomes the bottleneck.
const DEFAULT_POOL_CAPACITY: usize = 8;

/// A bounded pool of SQLite connections sharing the same underlying
/// database file. Connections are handed out via [`Pool::acquire`]; the
/// returned guard releases the connection on drop.
struct Pool {
    slots: Mutex<Vec<Connection>>,
    available: Condvar,
}

impl Pool {
    /// Check out a connection. Blocks (synchronously, inside a
    /// `spawn_blocking` thread) when the pool is exhausted. The pool
    /// is pre-populated so this path never opens a new connection.
    ///
    /// The wait is *bounded* per iteration: every connection is returned
    /// to the pool on `PoolGuard::drop`, so the only way to wait forever
    /// is a leaked guard (a bug) or a deadlock. An unbounded
    /// `Condvar::wait` would hang the blocking-pool thread silently;
    /// instead we wake every `WAIT_SLICE`, log with escalating severity,
    /// and keep waiting — turning an invisible hang into a diagnosable
    /// one without inventing an error path callers can't recover from.
    fn acquire(self: &Arc<Self>) -> PoolGuard {
        /// How long a single `Condvar` wait may park before we re-check
        /// and log. Short enough that a hang shows up promptly in logs.
        const WAIT_SLICE: std::time::Duration = std::time::Duration::from_secs(5);

        let mut slots = self.slots.lock();
        let mut waited = std::time::Duration::ZERO;
        loop {
            if let Some(conn) = slots.pop() {
                return PoolGuard {
                    conn: Some(conn),
                    pool: Arc::clone(self),
                };
            }
            let res = self.available.wait_for(&mut slots, WAIT_SLICE);
            if res.timed_out() && slots.is_empty() {
                waited += WAIT_SLICE;
                if waited >= std::time::Duration::from_secs(30) {
                    tracing::error!(
                        waited_secs = waited.as_secs(),
                        "SQLite connection pool exhausted for 30s+ — \
                         a PoolGuard is likely leaked or a query is \
                         deadlocked; storage is stalled"
                    );
                } else {
                    tracing::warn!(
                        waited_secs = waited.as_secs(),
                        "SQLite connection pool exhausted; waiting for a \
                         connection to be released"
                    );
                }
            }
        }
    }

    fn release(&self, conn: Connection) {
        self.slots.lock().push(conn);
        self.available.notify_one();
    }
}

/// RAII wrapper that returns a pooled connection when dropped.
struct PoolGuard {
    conn: Option<Connection>,
    pool: Arc<Pool>,
}

impl std::ops::Deref for PoolGuard {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.conn.as_ref().expect("pool guard dropped twice")
    }
}

impl Drop for PoolGuard {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            self.pool.release(conn);
        }
    }
}

/// SQLite-backed storage for a single corpus.
///
/// Internally holds a small pool of connections (default 8) so WAL's
/// concurrent-read guarantee actually translates to parallel reads
/// from the `tokio` blocking pool. A single extra connection is
/// reserved for the embedding cache, which has its own sync API.
#[derive(Clone)]
pub struct SqliteStorage {
    pool: Arc<Pool>,
    /// Dedicated connection for the embedding cache. Kept outside the
    /// pool because the cache exposes a sync API (`Arc<Mutex<Connection>>`)
    /// that callers hold across multiple short operations.
    embedding_conn: Arc<Mutex<Connection>>,
}

impl SqliteStorage {
    /// Open (or create) a content database at the given path.
    ///
    /// Opens `DEFAULT_POOL_CAPACITY` connections against the same file,
    /// configures WAL + busy_timeout + foreign_keys on each, and runs
    /// pending migrations once on the first connection (shared via WAL).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Database`] if the connection cannot be opened,
    /// or [`StorageError::MigrationFailed`] if migrations fail.
    #[instrument(skip_all, fields(path = %path.as_ref().display()))]
    #[must_use = "constructors return a new value"]
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let p = path.as_ref().to_path_buf();

        // `MINISTR_REQUIRE_WAL=0` lets the connection accept the DELETE
        // journal mode when WAL can't be enabled. Required when the DB
        // file lives on SMB/CIFS-mounted shares (e.g. Azure Files) where
        // SQLite's WAL shared-memory file isn't supported. Single-writer
        // deployments (ACA min=1/max=1) tolerate DELETE just fine; the
        // perf cost is losing concurrent-reader-while-writer, which is
        // rare on this workload.
        let require_wal = std::env::var("MINISTR_REQUIRE_WAL").map_or(true, |v| v != "0");

        // Migration pass on the first connection — schema changes are
        // visible to all subsequent pool members via the WAL.
        let mut first = Connection::open(&p).map_err(|e| StorageError::Database {
            reason: format!("failed to open database: {e}"),
        })?;
        configure_connection(&first, require_wal)?;
        run_migrations(&mut first)?;

        let mut pool_conns = Vec::with_capacity(DEFAULT_POOL_CAPACITY);
        pool_conns.push(first);
        for _ in 1..DEFAULT_POOL_CAPACITY {
            let conn = Connection::open(&p).map_err(|e| StorageError::Database {
                reason: format!("failed to open pool connection: {e}"),
            })?;
            configure_connection(&conn, require_wal)?;
            pool_conns.push(conn);
        }

        // Dedicated connection for the embedding cache.
        let embedding_conn = Connection::open(&p).map_err(|e| StorageError::Database {
            reason: format!("failed to open embedding cache connection: {e}"),
        })?;
        configure_connection(&embedding_conn, require_wal)?;

        Ok(Self {
            pool: Arc::new(Pool {
                slots: Mutex::new(pool_conns),
                available: Condvar::new(),
            }),
            embedding_conn: Arc::new(Mutex::new(embedding_conn)),
        })
    }

    /// Open an in-memory database (useful for testing).
    ///
    /// In-memory databases are not shared across `sqlite3_open` calls,
    /// so the pool collapses to a single connection. The embedding-cache
    /// connection is an alias of the same connection, since tests don't
    /// stress concurrency.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if connection setup fails.
    #[must_use = "constructors return a new value"]
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let mut conn = Connection::open_in_memory().map_err(|e| StorageError::Database {
            reason: format!("failed to open in-memory database: {e}"),
        })?;
        configure_connection(&conn, false)?;
        run_migrations(&mut conn)?;

        // For in-memory the pool has a single connection and the
        // embedding cache shares the same handle through a second
        // in-memory connection would be a *different* database.
        let mut embedding_conn =
            Connection::open_in_memory().map_err(|e| StorageError::Database {
                reason: format!("failed to open in-memory embedding cache connection: {e}"),
            })?;
        configure_connection(&embedding_conn, false)?;
        // Embedding cache schema is part of the main migrations — run
        // them on this second in-memory connection too so its tables
        // exist.
        run_migrations(&mut embedding_conn)?;

        Ok(Self {
            pool: Arc::new(Pool {
                slots: Mutex::new(vec![conn]),
                available: Condvar::new(),
            }),
            embedding_conn: Arc::new(Mutex::new(embedding_conn)),
        })
    }

    /// Get a clone of the embedding cache's dedicated connection handle.
    ///
    /// Used by [`crate::embedding::cache::EmbeddingCache`] for synchronous
    /// access outside the pool. Never used by `with_conn`.
    #[must_use]
    pub fn conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.embedding_conn)
    }

    /// Run a blocking closure against a pooled connection inside
    /// `spawn_blocking`. Under contention, the closure waits on the
    /// pool's condvar until a connection is returned.
    async fn with_conn<F, T>(&self, f: F) -> Result<T, StorageError>
    where
        F: FnOnce(&Connection) -> Result<T, StorageError> + Send + 'static,
        T: Send + 'static,
    {
        let pool = Arc::clone(&self.pool);
        tokio::task::spawn_blocking(move || {
            let guard = pool.acquire();
            f(&guard)
        })
        .await
        .map_err(|e| StorageError::Database {
            reason: format!("spawn_blocking join error: {e}"),
        })?
    }

    // ── Answer cache (ministr_ask) ──────────────────────────────────────

    /// Look up a cached answer by query hash.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    pub async fn get_cached_answer(
        &self,
        query_hash: &str,
    ) -> Result<Option<super::traits::CachedAnswer>, StorageError> {
        let hash = query_hash.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT query_hash, query_text, answer, model, token_count, created_at
                     FROM answer_cache WHERE query_hash = ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: format!("prepare get_cached_answer: {e}"),
                })?;
            let row = stmt
                .query_row(rusqlite::params![hash], |row| {
                    Ok(super::traits::CachedAnswer {
                        query_hash: row.get(0)?,
                        query_text: row.get(1)?,
                        answer: row.get(2)?,
                        model: row.get(3)?,
                        token_count: row.get::<_, i64>(4)? as usize,
                        created_at: row.get(5)?,
                    })
                })
                .optional()
                .map_err(|e| StorageError::Database {
                    reason: format!("get_cached_answer: {e}"),
                })?;
            Ok(row)
        })
        .await
    }

    /// Get the source sections for a cached answer.
    pub async fn get_cached_answer_sources(
        &self,
        query_hash: &str,
    ) -> Result<Vec<super::traits::CachedAnswerSource>, StorageError> {
        let hash = query_hash.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT section_id, section_hash
                     FROM answer_cache_sources WHERE query_hash = ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: format!("prepare get_cached_answer_sources: {e}"),
                })?;
            let rows = stmt
                .query_map(rusqlite::params![hash], |row| {
                    Ok(super::traits::CachedAnswerSource {
                        section_id: row.get(0)?,
                        section_hash: row.get(1)?,
                    })
                })
                .map_err(|e| StorageError::Database {
                    reason: format!("get_cached_answer_sources: {e}"),
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| StorageError::Database {
                    reason: format!("get_cached_answer_sources rows: {e}"),
                })?;
            Ok(rows)
        })
        .await
    }

    /// Insert a cached answer with its source sections (upsert semantics).
    #[allow(clippy::cast_possible_wrap)]
    pub async fn insert_cached_answer(
        &self,
        answer: &super::traits::CachedAnswer,
        sources: &[super::traits::CachedAnswerSource],
    ) -> Result<(), StorageError> {
        let answer = answer.clone();
        let sources = sources.to_vec();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO answer_cache
                 (query_hash, query_text, answer, model, token_count)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    answer.query_hash,
                    answer.query_text,
                    answer.answer,
                    answer.model,
                    answer.token_count as i64,
                ],
            )
            .map_err(|e| StorageError::Database {
                reason: format!("insert answer_cache: {e}"),
            })?;

            // CASCADE delete clears old sources, then insert fresh ones.
            for src in &sources {
                conn.execute(
                    "INSERT OR REPLACE INTO answer_cache_sources
                     (query_hash, section_id, section_hash)
                     VALUES (?1, ?2, ?3)",
                    rusqlite::params![answer.query_hash, src.section_id, src.section_hash],
                )
                .map_err(|e| StorageError::Database {
                    reason: format!("insert answer_cache_sources: {e}"),
                })?;
            }
            Ok(())
        })
        .await
    }

    /// Invalidate all cached answers that depend on any of the given section IDs.
    ///
    /// Uses the `answer_cache_sources` reverse index for efficient lookup.
    /// Returns the number of answers invalidated.
    pub async fn invalidate_answers_for_sections(
        &self,
        section_ids: &[String],
    ) -> Result<usize, StorageError> {
        if section_ids.is_empty() {
            return Ok(0);
        }
        let ids = section_ids.to_vec();
        self.with_conn(move |conn| {
            let placeholders: String = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                "DELETE FROM answer_cache WHERE query_hash IN (
                    SELECT DISTINCT query_hash FROM answer_cache_sources
                    WHERE section_id IN ({placeholders})
                )"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = ids
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let deleted =
                conn.execute(&sql, params.as_slice())
                    .map_err(|e| StorageError::Database {
                        reason: format!("invalidate_answers_for_sections: {e}"),
                    })?;
            Ok(deleted)
        })
        .await
    }
}

/// Insert all sections (and their claims) for a document recursively.
fn insert_sections_recursive(
    conn: &Connection,
    document_id: &str,
    sections: &[Section],
    position_offset: &mut i64,
    seen_ids: &mut std::collections::HashSet<String>,
) -> Result<(), StorageError> {
    for section in sections {
        // Deduplicate section IDs — if a heading appears twice in one document
        // (e.g. mdBook sidebar + content both have `<h1>ministr</h1>`), skip the duplicate.
        let section_id = if seen_ids.contains(section.id.as_ref()) {
            // Append position to disambiguate
            let deduped = format!("{}-{}", section.id.as_ref(), *position_offset);
            if seen_ids.contains(&deduped) {
                *position_offset += 1;
                continue; // extremely unlikely third collision — just skip
            }
            deduped
        } else {
            section.id.as_ref().to_string()
        };
        seen_ids.insert(section_id.clone());

        let heading_json = serde_json::to_string(&section.heading_path).map_err(|e| {
            StorageError::Serialization {
                reason: e.to_string(),
            }
        })?;

        conn.execute(
            "INSERT INTO sections (id, document_id, heading_path, depth, text, summary, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                section_id,
                document_id,
                heading_json,
                section.depth,
                section.text,
                section.summary,
                *position_offset,
            ],
        )
        .map_err(|e| StorageError::Database {
            reason: format!("failed to insert section {section_id}: {e}"),
        })?;

        *position_offset += 1;

        // Insert claims for this section.
        // When the section ID was deduplicated, regenerate claim IDs to match
        // (pre-generated claim IDs use the original section ID and would collide).
        let section_was_deduped = section_id != section.id.as_ref();
        for (claim_pos, claim) in section.claims.iter().enumerate() {
            let claim_id = if section_was_deduped {
                format!("{section_id}:c{claim_pos}")
            } else {
                claim.id.as_ref().to_string()
            };
            conn.execute(
                "INSERT INTO claims (id, section_id, text, position) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    claim_id,
                    section_id,
                    claim.text,
                    i64::try_from(claim_pos).unwrap_or(i64::MAX),
                ],
            )
            .map_err(|e| StorageError::Database {
                reason: format!("failed to insert claim {claim_id}: {e}"),
            })?;
        }

        // Recurse into children
        insert_sections_recursive(
            conn,
            document_id,
            &section.children,
            position_offset,
            seen_ids,
        )?;
    }
    Ok(())
}

impl Storage for SqliteStorage {
    async fn insert_document(&self, doc: &DocumentTree) -> Result<(), StorageError> {
        let doc = doc.clone();
        self.with_conn(move |conn| {
            conn.execute("SAVEPOINT insert_doc", [])
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to begin savepoint: {e}"),
                })?;

            let result = (|| {
                // Remove any stale document with the same source_path to avoid
                // UNIQUE constraint violations (can happen if a prior run was
                // interrupted after inserting the doc but before writing the
                // file hash record).
                conn.execute(
                    "DELETE FROM documents WHERE source_path = ?1",
                    rusqlite::params![doc.source_path],
                )
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to clean stale document {}: {e}", doc.source_path),
                })?;

                conn.execute(
                    "INSERT INTO documents (id, title, source_path, summary) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![doc.id.as_ref(), doc.title, doc.source_path, doc.summary,],
                )
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to insert document {}: {e}", doc.id),
                })?;

                let mut pos = 0i64;
                let mut seen_ids = std::collections::HashSet::new();
                insert_sections_recursive(conn, doc.id.as_ref(), &doc.sections, &mut pos, &mut seen_ids)?;
                Ok(())
            })();

            match result {
                Ok(()) => {
                    conn.execute("RELEASE insert_doc", [])
                        .map_err(|e| StorageError::Database {
                            reason: format!("failed to commit: {e}"),
                        })?;
                    Ok(())
                }
                Err(e) => {
                    // Rollback on any error so the connection stays clean
                    let _ = conn.execute("ROLLBACK TO insert_doc", []);
                    let _ = conn.execute("RELEASE insert_doc", []);
                    Err(e)
                }
            }
        })
        .await
    }

    async fn get_document(&self, id: &ContentId) -> Result<Option<DocumentRecord>, StorageError> {
        let id = id.clone();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, title, source_path, summary, root_id FROM documents WHERE id = ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let result = stmt
                .query_row(rusqlite::params![id.as_ref()], |row| {
                    Ok(DocumentRecord {
                        id: ContentId(row.get(0)?),
                        title: row.get(1)?,
                        source_path: row.get(2)?,
                        summary: row.get(3)?,
                        root_id: row.get(4)?,
                    })
                })
                .optional()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            Ok(result)
        })
        .await
    }

    async fn document_count(&self) -> Result<usize, StorageError> {
        self.with_conn(|conn| {
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            Ok(usize::try_from(count).unwrap_or(0))
        })
        .await
    }

    async fn section_count(&self) -> Result<usize, StorageError> {
        self.with_conn(|conn| {
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM sections", [], |row| row.get(0))
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            Ok(usize::try_from(count).unwrap_or(0))
        })
        .await
    }

    async fn symbol_count(&self) -> Result<usize, StorageError> {
        self.with_conn(|conn| {
            let count: i64 = conn
                .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            Ok(usize::try_from(count).unwrap_or(0))
        })
        .await
    }

    async fn list_documents(&self) -> Result<Vec<DocumentRecord>, StorageError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, title, source_path, summary, root_id FROM documents ORDER BY title",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(DocumentRecord {
                        id: ContentId(row.get(0)?),
                        title: row.get(1)?,
                        source_path: row.get(2)?,
                        summary: row.get(3)?,
                        root_id: row.get(4)?,
                    })
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })
        })
        .await
    }

    async fn list_documents_by_root(
        &self,
        root_id: &str,
    ) -> Result<Vec<DocumentRecord>, StorageError> {
        let root_id = root_id.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, title, source_path, summary, root_id
                     FROM documents WHERE root_id = ?1 ORDER BY title",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let rows = stmt
                .query_map(rusqlite::params![root_id], |row| {
                    Ok(DocumentRecord {
                        id: ContentId(row.get(0)?),
                        title: row.get(1)?,
                        source_path: row.get(2)?,
                        summary: row.get(3)?,
                        root_id: row.get(4)?,
                    })
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })
        })
        .await
    }

    async fn delete_document(&self, id: &ContentId) -> Result<bool, StorageError> {
        let id = id.clone();
        self.with_conn(move |conn| {
            let affected = conn
                .execute(
                    "DELETE FROM documents WHERE id = ?1",
                    rusqlite::params![id.as_ref()],
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            Ok(affected > 0)
        })
        .await
    }

    async fn get_section(&self, id: &SectionId) -> Result<Option<SectionRecord>, StorageError> {
        let id = id.clone();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, document_id, heading_path, depth, text, summary, position
                     FROM sections WHERE id = ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let result = stmt
                .query_row(rusqlite::params![id.as_ref()], |row| {
                    let heading_json: String = row.get(2)?;
                    Ok(SectionRecord {
                        id: SectionId(row.get(0)?),
                        document_id: ContentId(row.get(1)?),
                        heading_path: serde_json::from_str(&heading_json).unwrap_or_default(),
                        depth: row.get(3)?,
                        text: row.get(4)?,
                        summary: row.get(5)?,
                        position: row.get(6)?,
                    })
                })
                .optional()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            Ok(result)
        })
        .await
    }

    async fn list_sections(
        &self,
        document_id: &ContentId,
    ) -> Result<Vec<SectionRecord>, StorageError> {
        let document_id = document_id.clone();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, document_id, heading_path, depth, text, summary, position
                     FROM sections WHERE document_id = ?1 ORDER BY position",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let rows = stmt
                .query_map(rusqlite::params![document_id.as_ref()], |row| {
                    let heading_json: String = row.get(2)?;
                    Ok(SectionRecord {
                        id: SectionId(row.get(0)?),
                        document_id: ContentId(row.get(1)?),
                        heading_path: serde_json::from_str(&heading_json).unwrap_or_default(),
                        depth: row.get(3)?,
                        text: row.get(4)?,
                        summary: row.get(5)?,
                        position: row.get(6)?,
                    })
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })
        })
        .await
    }

    async fn get_next_section(
        &self,
        section_id: &SectionId,
    ) -> Result<Option<SectionRecord>, StorageError> {
        let section_id = section_id.clone();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT s2.id, s2.document_id, s2.heading_path, s2.depth, s2.text, s2.summary, s2.position
                     FROM sections s1
                     JOIN sections s2 ON s2.document_id = s1.document_id AND s2.position > s1.position
                     WHERE s1.id = ?1
                     ORDER BY s2.position ASC
                     LIMIT 1",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let result = stmt
                .query_row(rusqlite::params![section_id.as_ref()], |row| {
                    let heading_json: String = row.get(2)?;
                    Ok(SectionRecord {
                        id: SectionId(row.get(0)?),
                        document_id: ContentId(row.get(1)?),
                        heading_path: serde_json::from_str(&heading_json).unwrap_or_default(),
                        depth: row.get(3)?,
                        text: row.get(4)?,
                        summary: row.get(5)?,
                        position: row.get(6)?,
                    })
                })
                .optional()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            Ok(result)
        })
        .await
    }

    async fn get_document_for_section(
        &self,
        section_id: &SectionId,
    ) -> Result<Option<DocumentRecord>, StorageError> {
        let section_id = section_id.clone();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT d.id, d.title, d.source_path, d.summary, d.root_id
                     FROM documents d
                     JOIN sections s ON s.document_id = d.id
                     WHERE s.id = ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let result = stmt
                .query_row(rusqlite::params![section_id.as_ref()], |row| {
                    Ok(DocumentRecord {
                        id: ContentId(row.get(0)?),
                        title: row.get(1)?,
                        source_path: row.get(2)?,
                        summary: row.get(3)?,
                        root_id: row.get(4)?,
                    })
                })
                .optional()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            Ok(result)
        })
        .await
    }

    async fn get_claim(&self, id: &ClaimId) -> Result<Option<ClaimRecord>, StorageError> {
        let id = id.clone();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT id, section_id, text, position FROM claims WHERE id = ?1")
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let result = stmt
                .query_row(rusqlite::params![id.as_ref()], |row| {
                    Ok(ClaimRecord {
                        id: ClaimId(row.get(0)?),
                        section_id: SectionId(row.get(1)?),
                        text: row.get(2)?,
                        position: row.get(3)?,
                    })
                })
                .optional()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            Ok(result)
        })
        .await
    }

    async fn list_claims(&self, section_id: &SectionId) -> Result<Vec<ClaimRecord>, StorageError> {
        let section_id = section_id.clone();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, section_id, text, position
                     FROM claims WHERE section_id = ?1 ORDER BY position",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let rows = stmt
                .query_map(rusqlite::params![section_id.as_ref()], |row| {
                    Ok(ClaimRecord {
                        id: ClaimId(row.get(0)?),
                        section_id: SectionId(row.get(1)?),
                        text: row.get(2)?,
                        position: row.get(3)?,
                    })
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })
        })
        .await
    }

    async fn insert_claim_relationships(
        &self,
        relationships: &[ClaimRelationship],
    ) -> Result<(), StorageError> {
        let relationships = relationships.to_vec();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "INSERT OR REPLACE INTO claim_relationships
                     (source_claim_id, target_claim_id, relation_type, confidence)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to prepare relationship insert: {e}"),
                })?;

            for rel in &relationships {
                stmt.execute(rusqlite::params![
                    rel.source_claim_id.as_ref(),
                    rel.target_claim_id.as_ref(),
                    rel.relation_type.to_string(),
                    rel.confidence,
                ])
                .map_err(|e| StorageError::Database {
                    reason: format!(
                        "failed to insert relationship {} -> {}: {e}",
                        rel.source_claim_id, rel.target_claim_id
                    ),
                })?;
            }

            Ok(())
        })
        .await
    }

    async fn get_related_claims(
        &self,
        claim_id: &ClaimId,
        relation_types: Option<&[RelationType]>,
    ) -> Result<Vec<RelatedClaimRecord>, StorageError> {
        let claim_id = claim_id.clone();
        let relation_types = relation_types.map(<[RelationType]>::to_vec);
        self.with_conn(move |conn| {
            // Query both directions: source→target and target→source
            let sql = "
                SELECT c.id, c.text, cr.relation_type, c.section_id, cr.confidence
                FROM claim_relationships cr
                JOIN claims c ON c.id = cr.target_claim_id
                WHERE cr.source_claim_id = ?1
                UNION ALL
                SELECT c.id, c.text, cr.relation_type, c.section_id, cr.confidence
                FROM claim_relationships cr
                JOIN claims c ON c.id = cr.source_claim_id
                WHERE cr.target_claim_id = ?1
                ORDER BY confidence DESC
            ";

            let mut stmt = conn.prepare(sql).map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })?;

            let rows = stmt
                .query_map(rusqlite::params![claim_id.as_ref()], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, f32>(4)?,
                    ))
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let mut results = Vec::new();
            for row in rows {
                let (cid, text, rel_type_str, sid, confidence) =
                    row.map_err(|e| StorageError::Database {
                        reason: e.to_string(),
                    })?;

                let Some(rel_type) = RelationType::parse(&rel_type_str) else {
                    continue;
                };

                // Filter by relation types if specified
                if let Some(ref types) = relation_types
                    && !types.contains(&rel_type)
                {
                    continue;
                }

                results.push(RelatedClaimRecord {
                    claim_id: ClaimId(cid),
                    text,
                    relation_type: rel_type,
                    section_id: SectionId(sid),
                    confidence,
                });
            }

            Ok(results)
        })
        .await
    }

    async fn delete_relationships_for_section(
        &self,
        section_id: &SectionId,
    ) -> Result<(), StorageError> {
        let section_id = section_id.clone();
        self.with_conn(move |conn| {
            conn.execute(
                "DELETE FROM claim_relationships
                 WHERE source_claim_id IN (SELECT id FROM claims WHERE section_id = ?1)
                    OR target_claim_id IN (SELECT id FROM claims WHERE section_id = ?1)",
                rusqlite::params![section_id.as_ref()],
            )
            .map_err(|e| StorageError::Database {
                reason: format!("failed to delete relationships for section {section_id}: {e}"),
            })?;
            Ok(())
        })
        .await
    }

    async fn upsert_file_hash(&self, record: &FileHashRecord) -> Result<(), StorageError> {
        let record = record.clone();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO file_hashes (path, content_hash, last_indexed, mtime_ns, extractor_version, resolver_version)
                 VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), ?3, ?4, ?5)
                 ON CONFLICT(path) DO UPDATE SET
                    content_hash = excluded.content_hash,
                    last_indexed = excluded.last_indexed,
                    mtime_ns = excluded.mtime_ns,
                    extractor_version = excluded.extractor_version,
                    resolver_version = excluded.resolver_version",
                rusqlite::params![
                    record.path,
                    record.content_hash,
                    record.mtime_ns,
                    record.extractor_version,
                    record.resolver_version,
                ],
            )
            .map_err(|e| StorageError::Database {
                reason: format!("failed to upsert file hash: {e}"),
            })?;
            Ok(())
        })
        .await
    }

    async fn get_file_hash(&self, path: &str) -> Result<Option<FileHashRecord>, StorageError> {
        let path = path.to_owned();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT path, content_hash, mtime_ns, extractor_version, resolver_version \
                     FROM file_hashes WHERE path = ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let result = stmt
                .query_row(rusqlite::params![path], |row| {
                    Ok(FileHashRecord {
                        path: row.get(0)?,
                        content_hash: row.get(1)?,
                        mtime_ns: row.get(2)?,
                        extractor_version: row.get(3)?,
                        resolver_version: row.get(4)?,
                    })
                })
                .optional()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            Ok(result)
        })
        .await
    }

    async fn delete_file_hash(&self, path: &str) -> Result<bool, StorageError> {
        let path = path.to_owned();
        self.with_conn(move |conn| {
            let affected = conn
                .execute(
                    "DELETE FROM file_hashes WHERE path = ?1",
                    rusqlite::params![path],
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            Ok(affected > 0)
        })
        .await
    }

    async fn list_file_hashes(&self) -> Result<Vec<FileHashRecord>, StorageError> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT path, content_hash, mtime_ns, extractor_version, resolver_version \
                     FROM file_hashes ORDER BY path",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let records = stmt
                .query_map([], |row| {
                    Ok(FileHashRecord {
                        path: row.get(0)?,
                        content_hash: row.get(1)?,
                        mtime_ns: row.get(2)?,
                        extractor_version: row.get(3)?,
                        resolver_version: row.get(4)?,
                    })
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            Ok(records)
        })
        .await
    }

    async fn clear_file_hashes(&self) -> Result<usize, StorageError> {
        self.with_conn(|conn| {
            let affected = conn.execute("DELETE FROM file_hashes", []).map_err(|e| {
                StorageError::Database {
                    reason: e.to_string(),
                }
            })?;
            Ok(affected)
        })
        .await
    }

    async fn get_corpus_merkle(
        &self,
        corpus_id: &str,
    ) -> Result<Option<CorpusMerkleRecord>, StorageError> {
        let corpus_id = corpus_id.to_owned();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT corpus_id, root_hash, file_count, last_indexed_ns, extractor_version \
                     FROM corpus_merkle WHERE corpus_id = ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let result = stmt
                .query_row(rusqlite::params![corpus_id], |row| {
                    Ok(CorpusMerkleRecord {
                        corpus_id: row.get(0)?,
                        root_hash: row.get(1)?,
                        file_count: row.get(2)?,
                        last_indexed_ns: row.get(3)?,
                        extractor_version: row.get(4)?,
                    })
                })
                .optional()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            Ok(result)
        })
        .await
    }

    async fn upsert_corpus_merkle(&self, record: &CorpusMerkleRecord) -> Result<(), StorageError> {
        let record = record.clone();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO corpus_merkle \
                    (corpus_id, root_hash, file_count, last_indexed_ns, extractor_version) \
                 VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(corpus_id) DO UPDATE SET \
                    root_hash = excluded.root_hash, \
                    file_count = excluded.file_count, \
                    last_indexed_ns = excluded.last_indexed_ns, \
                    extractor_version = excluded.extractor_version",
                rusqlite::params![
                    record.corpus_id,
                    record.root_hash,
                    record.file_count,
                    record.last_indexed_ns,
                    record.extractor_version,
                ],
            )
            .map_err(|e| StorageError::Database {
                reason: format!("failed to upsert corpus merkle: {e}"),
            })?;
            Ok(())
        })
        .await
    }

    async fn delete_corpus_merkle(&self, corpus_id: &str) -> Result<bool, StorageError> {
        let corpus_id = corpus_id.to_owned();
        self.with_conn(move |conn| {
            let n = conn
                .execute(
                    "DELETE FROM corpus_merkle WHERE corpus_id = ?1",
                    rusqlite::params![corpus_id],
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            Ok(n > 0)
        })
        .await
    }

    async fn save_session(&self, session: &Session) -> Result<(), StorageError> {
        let id = session.id.0.clone();
        let budget = session.agent_context_budget;
        let turn = session.current_turn();
        let items: Vec<DeliveredItem> = session.delivered_items().cloned().collect();
        let trajectory: Vec<ContentId> = session.trajectory().iter().cloned().collect();
        let metrics_json = serde_json::to_string(session.metrics()).unwrap_or_else(|_| "{}".into());
        let recent_queries: Vec<&String> = session.recent_queries().iter().collect();
        let recent_queries_json =
            serde_json::to_string(&recent_queries).unwrap_or_else(|_| "[]".into());

        self.with_conn(move |conn| {
            conn.execute("SAVEPOINT save_session", [])
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to begin savepoint: {e}"),
                })?;

            let result = (|| {
                // Upsert session row
                conn.execute(
                    "INSERT INTO sessions (id, context_budget, current_turn, updated_at, metrics_json, recent_queries_json)
                     VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'), ?4, ?5)
                     ON CONFLICT(id) DO UPDATE SET
                        context_budget = excluded.context_budget,
                        current_turn = excluded.current_turn,
                        updated_at = excluded.updated_at,
                        metrics_json = excluded.metrics_json,
                        recent_queries_json = excluded.recent_queries_json",
                    rusqlite::params![id, budget, turn, metrics_json, recent_queries_json],
                )
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to upsert session: {e}"),
                })?;

                // Clear existing deliveries and re-insert
                conn.execute(
                    "DELETE FROM session_deliveries WHERE session_id = ?1",
                    rusqlite::params![id],
                )
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to clear session deliveries: {e}"),
                })?;

                // Build a position map from trajectory for ordering
                let mut position_map: std::collections::HashMap<String, i64> =
                    std::collections::HashMap::new();
                for (pos, cid) in trajectory.iter().enumerate() {
                    position_map.insert(
                        cid.0.clone(),
                        i64::try_from(pos).unwrap_or(i64::MAX),
                    );
                }

                for item in &items {
                    let position = position_map
                        .get(&item.content_id.0)
                        .copied()
                        .unwrap_or(0);
                    conn.execute(
                        "INSERT INTO session_deliveries
                         (session_id, content_id, resolution, token_count, turn_delivered, content_hash, position, compression_tier, compressed_summary)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        rusqlite::params![
                            id,
                            item.content_id.0,
                            item.resolution.to_string(),
                            item.token_count,
                            item.turn_delivered,
                            item.content_hash,
                            position,
                            item.compression_tier.as_str(),
                            item.compressed_summary,
                        ],
                    )
                    .map_err(|e| StorageError::Database {
                        reason: format!("failed to insert session delivery: {e}"),
                    })?;
                }
                Ok(())
            })();

            match result {
                Ok(()) => {
                    conn.execute("RELEASE save_session", [])
                        .map_err(|e| StorageError::Database {
                            reason: format!("failed to commit: {e}"),
                        })?;
                    Ok(())
                }
                Err(e) => {
                    let _ = conn.execute("ROLLBACK TO save_session", []);
                    let _ = conn.execute("RELEASE save_session", []);
                    Err(e)
                }
            }
        })
        .await
    }

    #[allow(clippy::too_many_lines)]
    async fn load_session(&self, id: &SessionId) -> Result<Option<Session>, StorageError> {
        let id = id.0.clone();
        self.with_conn(move |conn| {
            // Load session metadata
            let mut stmt = conn
                .prepare(
                    "SELECT id, context_budget, current_turn, metrics_json, recent_queries_json \
                     FROM sessions WHERE id = ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let session_row = stmt
                .query_row(rusqlite::params![id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, usize>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                })
                .optional()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let Some((session_id, budget, turn, metrics_json, recent_queries_json)) = session_row
            else {
                return Ok(None);
            };

            // Load delivered items ordered by position (for trajectory reconstruction)
            let mut stmt = conn
                .prepare(
                    "SELECT content_id, resolution, token_count, turn_delivered, content_hash, position, compression_tier, compressed_summary
                     FROM session_deliveries WHERE session_id = ?1 ORDER BY position",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let rows = stmt
                .query_map(rusqlite::params![session_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, usize>(2)?,
                        row.get::<_, u32>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let mut delivered = std::collections::BTreeMap::new();
            let mut trajectory = Vec::new();

            for row in rows {
                let (
                    content_id_str,
                    resolution_str,
                    token_count,
                    turn_delivered,
                    content_hash,
                    compression_tier_str,
                    compressed_summary,
                ) = row.map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

                let content_id = ContentId(content_id_str.clone());
                let resolution = parse_resolution(&resolution_str);
                let compression_tier =
                    crate::session::CompressionTier::from_str_or_full(&compression_tier_str);

                let item = DeliveredItem {
                    content_id: content_id.clone(),
                    resolution,
                    token_count,
                    turn_delivered,
                    content_hash,
                    compression_tier,
                    compressed_summary,
                };

                delivered.insert(content_id_str.clone(), item);
                trajectory.push(content_id);
            }

            // The eviction policy isn't yet part of the persistence schema,
            // so we default to Fifo on restore. Callers that need a
            // different policy should either (a) add an eviction_policy
            // column to the sessions table and thread it through, or
            // (b) load the session and rebuild the UsageTracker with the
            // desired policy.
            let mut session = Session::restore(
                SessionId(session_id),
                budget,
                crate::session::DropPolicy::Fifo,
                delivered,
                trajectory,
                turn,
            );
            if let Some(json) = metrics_json
                && let Ok(metrics) = serde_json::from_str(&json)
            {
                session.set_metrics(metrics);
            }
            if let Some(json) = recent_queries_json
                && let Ok(queries) = serde_json::from_str::<Vec<String>>(&json)
            {
                session.set_recent_queries(queries);
            }
            Ok(Some(session))
        })
        .await
    }

    async fn delete_session(&self, id: &SessionId) -> Result<bool, StorageError> {
        let id = id.0.clone();
        self.with_conn(move |conn| {
            let affected = conn
                .execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![id])
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            Ok(affected > 0)
        })
        .await
    }

    // -- Cross-session analytics --

    async fn record_section_access(&self, section_id: &SectionId) -> Result<(), StorageError> {
        let id = section_id.0.clone();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO section_access_stats (section_id, access_count, last_accessed)
                 VALUES (?1, 1, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                 ON CONFLICT(section_id) DO UPDATE SET
                   access_count = access_count + 1,
                   last_accessed = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
                rusqlite::params![id],
            )
            .map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })?;
            Ok(())
        })
        .await
    }

    async fn record_co_accesses(&self, section_ids: &[SectionId]) -> Result<(), StorageError> {
        let ids: Vec<String> = section_ids.iter().map(|s| s.0.clone()).collect();
        self.with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            // Generate all unique pairs (a, b) where a < b to avoid duplicates.
            // Skip self-pairs defensively — callers normally dedupe via the
            // Analytics wrapper, but the trait is public so a duplicate ID
            // in the input must never produce a `(x, x)` row.
            for i in 0..ids.len() {
                for j in (i + 1)..ids.len() {
                    if ids[i] == ids[j] {
                        continue;
                    }
                    let (a, b) = if ids[i] < ids[j] {
                        (&ids[i], &ids[j])
                    } else {
                        (&ids[j], &ids[i])
                    };
                    tx.execute(
                        "INSERT INTO co_access_patterns (section_a, section_b, co_count)
                         VALUES (?1, ?2, 1)
                         ON CONFLICT(section_a, section_b) DO UPDATE SET
                           co_count = co_count + 1",
                        rusqlite::params![a, b],
                    )
                    .map_err(|e| StorageError::Database {
                        reason: e.to_string(),
                    })?;
                }
            }
            tx.commit().map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })?;
            Ok(())
        })
        .await
    }

    async fn record_co_access_pairs(
        &self,
        pairs: &[(SectionId, SectionId)],
    ) -> Result<(), StorageError> {
        if pairs.is_empty() {
            return Ok(());
        }
        // Normalize ordering (section_a < section_b) and skip self-pairs.
        let normalized: Vec<(String, String)> = pairs
            .iter()
            .filter_map(|(a, b)| {
                if a.0 == b.0 {
                    return None;
                }
                if a.0 < b.0 {
                    Some((a.0.clone(), b.0.clone()))
                } else {
                    Some((b.0.clone(), a.0.clone()))
                }
            })
            .collect();
        if normalized.is_empty() {
            return Ok(());
        }
        self.with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            for (a, b) in &normalized {
                tx.execute(
                    "INSERT INTO co_access_patterns (section_a, section_b, co_count)
                     VALUES (?1, ?2, 1)
                     ON CONFLICT(section_a, section_b) DO UPDATE SET
                       co_count = co_count + 1",
                    rusqlite::params![a, b],
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            }
            tx.commit().map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })?;
            Ok(())
        })
        .await
    }

    async fn get_top_sections(&self, limit: usize) -> Result<Vec<SectionAccessStat>, StorageError> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT section_id, access_count, last_accessed
                     FROM section_access_stats
                     ORDER BY access_count DESC
                     LIMIT ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let rows = stmt
                .query_map(rusqlite::params![limit], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let mut results = Vec::new();
            for row in rows {
                let (section_id, access_count, last_accessed) =
                    row.map_err(|e| StorageError::Database {
                        reason: e.to_string(),
                    })?;
                results.push(SectionAccessStat {
                    section_id: SectionId(section_id),
                    access_count,
                    last_accessed,
                });
            }
            Ok(results)
        })
        .await
    }

    async fn get_co_accessed(
        &self,
        section_id: &SectionId,
        limit: usize,
    ) -> Result<Vec<CoAccessRecord>, StorageError> {
        let id = section_id.0.clone();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT
                       CASE WHEN section_a = ?1 THEN section_b ELSE section_a END AS partner,
                       co_count
                     FROM co_access_patterns
                     WHERE section_a = ?1 OR section_b = ?1
                     ORDER BY co_count DESC
                     LIMIT ?2",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let rows = stmt
                .query_map(rusqlite::params![id, limit], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let mut results = Vec::new();
            for row in rows {
                let (partner, co_count) = row.map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
                results.push(CoAccessRecord {
                    section_id: SectionId(partner),
                    co_count,
                });
            }
            Ok(results)
        })
        .await
    }

    async fn get_corpus_stats(&self) -> Result<CorpusStats, StorageError> {
        self.with_conn(move |conn| {
            let total_accesses: u64 = conn
                .query_row(
                    "SELECT COALESCE(SUM(access_count), 0) FROM section_access_stats",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let unique_sections_accessed: u64 = conn
                .query_row("SELECT COUNT(*) FROM section_access_stats", [], |row| {
                    row.get(0)
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let co_access_pairs: u64 = conn
                .query_row("SELECT COUNT(*) FROM co_access_patterns", [], |row| {
                    row.get(0)
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            Ok(CorpusStats {
                total_accesses,
                unique_sections_accessed,
                co_access_pairs,
            })
        })
        .await
    }

    // -- Web cache --

    async fn upsert_web_cache(&self, record: &WebCacheRecord) -> Result<(), StorageError> {
        let record = record.clone();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO web_cache (source_url, fetch_timestamp, etag, last_modified, content_hash, content_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(source_url) DO UPDATE SET
                    fetch_timestamp = excluded.fetch_timestamp,
                    etag = excluded.etag,
                    last_modified = excluded.last_modified,
                    content_hash = excluded.content_hash,
                    content_type = excluded.content_type",
                rusqlite::params![
                    record.source_url,
                    record.fetch_timestamp,
                    record.etag,
                    record.last_modified,
                    record.content_hash,
                    record.content_type,
                ],
            )
            .map_err(|e| StorageError::Database {
                reason: format!("failed to upsert web cache: {e}"),
            })?;
            Ok(())
        })
        .await
    }

    async fn get_web_cache(&self, url: &str) -> Result<Option<WebCacheRecord>, StorageError> {
        let url = url.to_owned();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT source_url, fetch_timestamp, etag, last_modified, content_hash, content_type
                     FROM web_cache WHERE source_url = ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let result = stmt
                .query_row(rusqlite::params![url], |row| {
                    Ok(WebCacheRecord {
                        source_url: row.get(0)?,
                        fetch_timestamp: row.get(1)?,
                        etag: row.get(2)?,
                        last_modified: row.get(3)?,
                        content_hash: row.get(4)?,
                        content_type: row.get(5)?,
                    })
                })
                .optional()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            Ok(result)
        })
        .await
    }

    async fn list_web_cache(&self) -> Result<Vec<WebCacheRecord>, StorageError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT source_url, fetch_timestamp, etag, last_modified, content_hash, content_type
                     FROM web_cache ORDER BY fetch_timestamp DESC",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(WebCacheRecord {
                        source_url: row.get(0)?,
                        fetch_timestamp: row.get(1)?,
                        etag: row.get(2)?,
                        last_modified: row.get(3)?,
                        content_hash: row.get(4)?,
                        content_type: row.get(5)?,
                    })
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })
        })
        .await
    }

    async fn delete_web_cache(&self, url: &str) -> Result<bool, StorageError> {
        let url = url.to_owned();
        self.with_conn(move |conn| {
            let affected = conn
                .execute(
                    "DELETE FROM web_cache WHERE source_url = ?1",
                    rusqlite::params![url],
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            Ok(affected > 0)
        })
        .await
    }

    // -- Git cache --

    async fn upsert_git_cache(&self, record: &GitCacheRecord) -> Result<(), StorageError> {
        let record = record.clone();
        let paths_json =
            serde_json::to_string(&record.checked_out_paths).unwrap_or_else(|_| "[]".to_string());
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO git_cache (repo_url, branch, commit_sha, clone_timestamp, clone_dir, checked_out_paths)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(repo_url) DO UPDATE SET
                    branch = excluded.branch,
                    commit_sha = excluded.commit_sha,
                    clone_timestamp = excluded.clone_timestamp,
                    clone_dir = excluded.clone_dir,
                    checked_out_paths = excluded.checked_out_paths",
                rusqlite::params![
                    record.repo_url,
                    record.branch,
                    record.commit_sha,
                    record.clone_timestamp,
                    record.clone_dir,
                    paths_json,
                ],
            )
            .map_err(|e| StorageError::Database {
                reason: format!("failed to upsert git cache: {e}"),
            })?;
            Ok(())
        })
        .await
    }

    async fn get_git_cache(&self, repo_url: &str) -> Result<Option<GitCacheRecord>, StorageError> {
        let repo_url = repo_url.to_owned();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT repo_url, branch, commit_sha, clone_timestamp, clone_dir, checked_out_paths
                     FROM git_cache WHERE repo_url = ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let result = stmt
                .query_row(rusqlite::params![repo_url], |row| {
                    let paths_json: String = row.get(5)?;
                    let checked_out_paths: Vec<String> =
                        serde_json::from_str(&paths_json).unwrap_or_default();
                    Ok(GitCacheRecord {
                        repo_url: row.get(0)?,
                        branch: row.get(1)?,
                        commit_sha: row.get(2)?,
                        clone_timestamp: row.get(3)?,
                        clone_dir: row.get(4)?,
                        checked_out_paths,
                    })
                })
                .optional()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            Ok(result)
        })
        .await
    }

    async fn list_git_cache(&self) -> Result<Vec<GitCacheRecord>, StorageError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT repo_url, branch, commit_sha, clone_timestamp, clone_dir, checked_out_paths
                     FROM git_cache ORDER BY clone_timestamp DESC",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let rows = stmt
                .query_map([], |row| {
                    let paths_json: String = row.get(5)?;
                    let checked_out_paths: Vec<String> =
                        serde_json::from_str(&paths_json).unwrap_or_default();
                    Ok(GitCacheRecord {
                        repo_url: row.get(0)?,
                        branch: row.get(1)?,
                        commit_sha: row.get(2)?,
                        clone_timestamp: row.get(3)?,
                        clone_dir: row.get(4)?,
                        checked_out_paths,
                    })
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })
        })
        .await
    }

    async fn delete_git_cache(&self, repo_url: &str) -> Result<bool, StorageError> {
        let repo_url = repo_url.to_owned();
        self.with_conn(move |conn| {
            let affected = conn
                .execute(
                    "DELETE FROM git_cache WHERE repo_url = ?1",
                    rusqlite::params![repo_url],
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            Ok(affected > 0)
        })
        .await
    }

    // -- Symbols --

    async fn insert_symbols(&self, symbols: &[SymbolRecord]) -> Result<(), StorageError> {
        let symbols = symbols.to_vec();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "INSERT OR REPLACE INTO symbols
                     (id, file_path, name, kind, visibility, signature, doc_comment, module_path, line_start, line_end, cyclomatic_complexity)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            for sym in &symbols {
                stmt.execute(rusqlite::params![
                    sym.id.as_ref(),
                    sym.file_path,
                    sym.name,
                    sym.kind,
                    sym.visibility,
                    sym.signature,
                    sym.doc_comment,
                    sym.module_path,
                    sym.line_start,
                    sym.line_end,
                    sym.cyclomatic_complexity,
                ])
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to insert symbol {}: {e}", sym.id),
                })?;
            }
            Ok(())
        })
        .await
    }

    async fn list_symbols(&self, filter: &SymbolFilter) -> Result<Vec<SymbolRecord>, StorageError> {
        let filter = filter.clone();
        self.with_conn(move |conn| {
            let results = query_symbols(conn, &filter, TokenMode::And)?;

            // Fallback: if AND yielded nothing and we have multiple tokens, try OR.
            if results.is_empty()
                && let Some(ref name) = filter.name
            {
                let tokens = tokenize_symbol_query(name);
                if tokens.len() > 1 {
                    return query_symbols(conn, &filter, TokenMode::Or);
                }
            }

            Ok(results)
        })
        .await
    }

    async fn get_symbol(&self, id: &SymbolId) -> Result<Option<SymbolRecord>, StorageError> {
        let id = id.clone();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, file_path, name, kind, visibility, signature, doc_comment, module_path, line_start, line_end, cyclomatic_complexity
                     FROM symbols WHERE id = ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            stmt.query_row(rusqlite::params![id.as_ref()], |row| {
                Ok(SymbolRecord {
                    id: SymbolId(row.get(0)?),
                    file_path: row.get(1)?,
                    name: row.get(2)?,
                    kind: row.get(3)?,
                    visibility: row.get(4)?,
                    signature: row.get(5)?,
                    doc_comment: row.get(6)?,
                    module_path: row.get(7)?,
                    line_start: row.get(8)?,
                    line_end: row.get(9)?,
                    cyclomatic_complexity: row.get(10)?,
                })
            })
            .optional()
            .map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })
        })
        .await
    }

    async fn delete_symbols_for_file(&self, file_path: &str) -> Result<u64, StorageError> {
        let file_path = file_path.to_string();
        self.with_conn(move |conn| {
            let affected = conn
                .execute(
                    "DELETE FROM symbols WHERE file_path = ?1",
                    rusqlite::params![file_path],
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            Ok(u64::try_from(affected).unwrap_or(0))
        })
        .await
    }

    // -- Symbol references --

    async fn insert_symbol_refs(&self, refs: &[SymbolRefRecord]) -> Result<(), StorageError> {
        let refs = refs.to_vec();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "INSERT OR REPLACE INTO symbol_refs (from_symbol_id, to_symbol_id, ref_kind)
                     VALUES (?1, ?2, ?3)",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            for r in &refs {
                stmt.execute(rusqlite::params![
                    r.from_symbol_id.as_ref(),
                    r.to_symbol_id.as_ref(),
                    r.ref_kind.as_str(),
                ])
                .map_err(|e| StorageError::Database {
                    reason: format!(
                        "failed to insert symbol ref {} -> {}: {e}",
                        r.from_symbol_id, r.to_symbol_id
                    ),
                })?;
            }
            Ok(())
        })
        .await
    }

    async fn query_refs(
        &self,
        symbol_id: &SymbolId,
        ref_kind: Option<RefKind>,
    ) -> Result<Vec<SymbolRefRecord>, StorageError> {
        let symbol_id = symbol_id.clone();
        self.with_conn(move |conn| {
            let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
                if let Some(kind) = ref_kind {
                    (
                        "SELECT from_symbol_id, to_symbol_id, ref_kind FROM symbol_refs
                     WHERE (from_symbol_id = ?1 OR to_symbol_id = ?1) AND ref_kind = ?2"
                            .into(),
                        vec![
                            Box::new(symbol_id.0.clone()),
                            Box::new(kind.as_str().to_string()),
                        ],
                    )
                } else {
                    (
                        "SELECT from_symbol_id, to_symbol_id, ref_kind FROM symbol_refs
                     WHERE from_symbol_id = ?1 OR to_symbol_id = ?1"
                            .into(),
                        vec![Box::new(symbol_id.0.clone())],
                    )
                };

            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(std::convert::AsRef::as_ref).collect();

            let mut stmt = conn.prepare(&sql).map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })?;

            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    let kind_str: String = row.get(2)?;
                    Ok(SymbolRefRecord {
                        from_symbol_id: SymbolId(row.get(0)?),
                        to_symbol_id: SymbolId(row.get(1)?),
                        ref_kind: RefKind::parse(&kind_str).unwrap_or(RefKind::Uses),
                    })
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })
        })
        .await
    }

    async fn delete_refs_for_file(&self, file_path: &str) -> Result<(), StorageError> {
        let file_path = file_path.to_string();
        self.with_conn(move |conn| {
            conn.execute(
                "DELETE FROM symbol_refs WHERE from_symbol_id IN (SELECT id FROM symbols WHERE file_path = ?1)
                 OR to_symbol_id IN (SELECT id FROM symbols WHERE file_path = ?1)",
                rusqlite::params![file_path],
            )
            .map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })?;
            Ok(())
        })
        .await
    }

    async fn transitive_caller_counts(
        &self,
        symbol_ids: &[SymbolId],
    ) -> Result<std::collections::HashMap<SymbolId, u32>, StorageError> {
        let symbol_ids = symbol_ids.to_vec();
        self.with_conn(move |conn| {
            let mut result = std::collections::HashMap::new();

            // Use a recursive CTE per symbol to count transitive callers.
            // Bounded to depth 10 to prevent runaway on cycles.
            let sql = "
                WITH RECURSIVE callers(id, depth) AS (
                    SELECT from_symbol_id, 1
                    FROM symbol_refs
                    WHERE to_symbol_id = ?1 AND ref_kind = 'calls'
                    UNION
                    SELECT sr.from_symbol_id, c.depth + 1
                    FROM symbol_refs sr
                    JOIN callers c ON sr.to_symbol_id = c.id
                    WHERE sr.ref_kind = 'calls' AND c.depth < 10
                )
                SELECT COUNT(DISTINCT id) FROM callers
            ";

            let mut stmt = conn.prepare(sql).map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })?;

            for sid in &symbol_ids {
                let count: u32 = stmt
                    .query_row(rusqlite::params![sid.as_ref()], |row| row.get(0))
                    .unwrap_or(0);
                if count > 0 {
                    result.insert(sid.clone(), count);
                }
            }

            Ok(result)
        })
        .await
    }

    async fn insert_bridge_endpoints(
        &self,
        endpoints: &[BridgeEndpointRecord],
    ) -> Result<Vec<i64>, StorageError> {
        let endpoints = endpoints.to_vec();
        self.with_conn(move |conn| {
            let mut ids = Vec::with_capacity(endpoints.len());
            let mut stmt = conn
                .prepare(
                    "INSERT INTO bridge_endpoints
                     (file_path, binding_key, kind, role, language, line, symbol_name, confidence)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            for ep in &endpoints {
                stmt.execute(rusqlite::params![
                    ep.file_path,
                    ep.binding_key,
                    ep.kind,
                    ep.role,
                    ep.language,
                    ep.line,
                    ep.symbol_name,
                    f64::from(ep.confidence),
                ])
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to insert bridge endpoint '{}': {e}", ep.binding_key),
                })?;
                ids.push(conn.last_insert_rowid());
            }
            Ok(ids)
        })
        .await
    }

    async fn insert_bridge_links(&self, links: &[BridgeLinkRecord]) -> Result<(), StorageError> {
        let links = links.to_vec();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "INSERT OR REPLACE INTO bridge_links
                     (export_ep_id, import_ep_id, kind, confidence)
                     VALUES (?1, ?2, ?3, ?4)",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            for link in &links {
                stmt.execute(rusqlite::params![
                    link.export_ep_id,
                    link.import_ep_id,
                    link.kind,
                    f64::from(link.confidence),
                ])
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to insert bridge link: {e}"),
                })?;
            }
            Ok(())
        })
        .await
    }

    async fn query_bridge_links(
        &self,
        file_path: Option<&str>,
        kind: Option<&str>,
    ) -> Result<Vec<BridgeLinkDetail>, StorageError> {
        let file_path = file_path.map(ToString::to_string);
        let kind = kind.map(ToString::to_string);
        self.with_conn(move |conn| {
            let base = "
                SELECT
                    bl.kind, bl.confidence,
                    ex.file_path, ex.binding_key, ex.symbol_name, ex.language, ex.line,
                    im.file_path, im.binding_key, im.symbol_name, im.language, im.line
                FROM bridge_links bl
                JOIN bridge_endpoints ex ON bl.export_ep_id = ex.id
                JOIN bridge_endpoints im ON bl.import_ep_id = im.id
            ";

            let mut conditions = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            let mut param_idx = 1;

            if let Some(ref fp) = file_path {
                conditions.push(format!(
                    "(ex.file_path = ?{param_idx} OR im.file_path = ?{param_idx})"
                ));
                params.push(Box::new(fp.clone()));
                param_idx += 1;
            }

            if let Some(ref k) = kind {
                conditions.push(format!("bl.kind = ?{param_idx}"));
                params.push(Box::new(k.clone()));
            }

            let sql = if conditions.is_empty() {
                format!("{base} ORDER BY bl.confidence DESC")
            } else {
                format!(
                    "{base} WHERE {} ORDER BY bl.confidence DESC",
                    conditions.join(" AND ")
                )
            };

            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(std::convert::AsRef::as_ref).collect();

            let mut stmt = conn.prepare(&sql).map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })?;

            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    Ok(BridgeLinkDetail {
                        kind: row.get(0)?,
                        #[allow(clippy::cast_possible_truncation)]
                        confidence: row.get::<_, f64>(1)? as f32, // SQLite stores f64; truncation to f32 is intentional
                        export_file: row.get(2)?,
                        export_binding_key: row.get(3)?,
                        export_symbol: row.get(4)?,
                        export_language: row.get(5)?,
                        export_line: row.get(6)?,
                        import_file: row.get(7)?,
                        import_binding_key: row.get(8)?,
                        import_symbol: row.get(9)?,
                        import_language: row.get(10)?,
                        import_line: row.get(11)?,
                    })
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })
        })
        .await
    }

    async fn delete_bridge_data_for_file(&self, file_path: &str) -> Result<(), StorageError> {
        let file_path = file_path.to_string();
        self.with_conn(move |conn| {
            // Delete links that reference endpoints in this file, then delete the endpoints.
            conn.execute(
                "DELETE FROM bridge_links WHERE export_ep_id IN
                     (SELECT id FROM bridge_endpoints WHERE file_path = ?1)
                 OR import_ep_id IN
                     (SELECT id FROM bridge_endpoints WHERE file_path = ?1)",
                rusqlite::params![file_path],
            )
            .map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })?;

            conn.execute(
                "DELETE FROM bridge_endpoints WHERE file_path = ?1",
                rusqlite::params![file_path],
            )
            .map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })?;

            Ok(())
        })
        .await
    }

    async fn clear_bridge_data(&self) -> Result<(), StorageError> {
        self.with_conn(|conn| {
            // Delete links first so the FK from links -> endpoints is never
            // dangling mid-transaction (even if enforcement is deferred).
            conn.execute("DELETE FROM bridge_links", [])
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            conn.execute("DELETE FROM bridge_endpoints", [])
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            Ok(())
        })
        .await
    }

    // -- Pending refs (deferred resolution queue) --

    async fn upsert_pending_refs(&self, refs: &[PendingRefRecord]) -> Result<(), StorageError> {
        let refs = refs.to_vec();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "INSERT OR REPLACE INTO pending_refs
                     (from_symbol_id, target_name, kind, file_path, target_crate)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            for r in &refs {
                stmt.execute(rusqlite::params![
                    r.from_symbol_id,
                    r.target_name,
                    r.kind,
                    r.file_path,
                    r.target_crate,
                ])
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to upsert pending ref '{}': {e}", r.target_name),
                })?;
            }
            Ok(())
        })
        .await
    }

    async fn list_pending_refs(&self) -> Result<Vec<PendingRefRecord>, StorageError> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT from_symbol_id, target_name, kind, file_path, target_crate
                     FROM pending_refs",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(PendingRefRecord {
                        from_symbol_id: row.get(0)?,
                        target_name: row.get(1)?,
                        kind: row.get(2)?,
                        file_path: row.get(3)?,
                        target_crate: row.get(4)?,
                    })
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })
        })
        .await
    }

    async fn delete_pending_refs(&self, refs: &[PendingRefRecord]) -> Result<(), StorageError> {
        let refs = refs.to_vec();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "DELETE FROM pending_refs
                     WHERE from_symbol_id = ?1 AND target_name = ?2 AND kind = ?3",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            for r in &refs {
                stmt.execute(rusqlite::params![r.from_symbol_id, r.target_name, r.kind])
                    .map_err(|e| StorageError::Database {
                        reason: format!("failed to delete pending ref '{}': {e}", r.target_name),
                    })?;
            }
            Ok(())
        })
        .await
    }

    // -- Corpus roots --

    async fn upsert_corpus_root(&self, root: &CorpusRoot) -> Result<(), StorageError> {
        let root = root.clone();
        self.with_conn(move |conn| {
            let lang_json = serde_json::to_string(&root.language_stats).unwrap_or_default();
            let sparse_json = serde_json::to_string(&root.sparse_paths).unwrap_or_default();
            conn.execute(
                "INSERT INTO corpus_roots (id, path, kind, display_name, file_count, language_stats,
                     repo_url, branch, commit_sha, clone_timestamp, sparse_paths, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                 ON CONFLICT(id) DO UPDATE SET
                     path = excluded.path,
                     kind = excluded.kind,
                     display_name = excluded.display_name,
                     file_count = excluded.file_count,
                     language_stats = excluded.language_stats,
                     repo_url = excluded.repo_url,
                     branch = excluded.branch,
                     commit_sha = excluded.commit_sha,
                     clone_timestamp = excluded.clone_timestamp,
                     sparse_paths = excluded.sparse_paths,
                     updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
                rusqlite::params![
                    root.id,
                    root.path,
                    root.kind.as_str(),
                    root.display_name,
                    i64::try_from(root.file_count).unwrap_or(i64::MAX),
                    lang_json,
                    root.repo_url,
                    root.branch,
                    root.commit_sha,
                    root.clone_timestamp,
                    sparse_json,
                ],
            )
            .map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })?;
            Ok(())
        })
        .await
    }

    async fn get_corpus_root(&self, id: &str) -> Result<Option<CorpusRoot>, StorageError> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, path, kind, display_name, file_count, language_stats,
                            repo_url, branch, commit_sha, clone_timestamp, sparse_paths
                     FROM corpus_roots WHERE id = ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let result = stmt
                .query_row(rusqlite::params![id], |row| Ok(parse_corpus_root_row(row)))
                .optional()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            Ok(result)
        })
        .await
    }

    async fn list_corpus_roots(&self) -> Result<Vec<CorpusRoot>, StorageError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, path, kind, display_name, file_count, language_stats,
                            repo_url, branch, commit_sha, clone_timestamp, sparse_paths
                     FROM corpus_roots ORDER BY path",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let rows = stmt
                .query_map([], |row| Ok(parse_corpus_root_row(row)))
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })
        })
        .await
    }

    async fn delete_corpus_root(&self, id: &str) -> Result<bool, StorageError> {
        let id = id.to_string();
        self.with_conn(move |conn| {
            let changes = conn
                .execute(
                    "DELETE FROM corpus_roots WHERE id = ?1",
                    rusqlite::params![id],
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            Ok(changes > 0)
        })
        .await
    }

    async fn set_document_root(
        &self,
        doc_id: &ContentId,
        root_id: &str,
    ) -> Result<(), StorageError> {
        let doc_id = doc_id.clone();
        let root_id = root_id.to_string();
        self.with_conn(move |conn| {
            conn.execute(
                "UPDATE documents SET root_id = ?1 WHERE id = ?2",
                rusqlite::params![root_id, doc_id.as_ref()],
            )
            .map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })?;
            Ok(())
        })
        .await
    }
}

/// Tokenize a symbol query into searchable terms.
///
/// Handles whitespace splitting, CamelCase splitting (`ToolRouter` → `["tool", "router"]`),
/// and snake_case splitting (`list_symbols` → `["list", "symbols"]`). Deduplicates and
/// lowercases all tokens. Filters tokens shorter than 2 characters.
fn tokenize_symbol_query(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for word in query.split_whitespace() {
        // Split on underscores (snake_case).
        let snake_parts: Vec<&str> = word.split('_').filter(|s| !s.is_empty()).collect();
        if snake_parts.len() > 1 {
            for part in &snake_parts {
                split_camel_case(part, &mut tokens);
            }
        } else {
            split_camel_case(word, &mut tokens);
        }
    }
    tokens.sort_unstable();
    tokens.dedup();
    tokens.retain(|t| t.len() >= 2);
    tokens
}

/// Split a CamelCase word into lowercase tokens.
fn split_camel_case(word: &str, out: &mut Vec<String>) {
    let mut current = String::new();
    for ch in word.chars() {
        if ch.is_uppercase() && !current.is_empty() {
            out.push(std::mem::take(&mut current).to_lowercase());
        }
        current.push(ch);
    }
    if !current.is_empty() {
        out.push(current.to_lowercase());
    }
}

/// How to combine multiple tokens in a name query.
#[derive(Clone, Copy)]
enum TokenMode {
    /// All tokens must match (high precision).
    And,
    /// Any token may match (high recall, used as fallback).
    Or,
}

const SYMBOL_COLUMNS: &str = "id, file_path, name, kind, visibility, signature, doc_comment, module_path, line_start, line_end, cyclomatic_complexity";

/// Build and execute a symbol query against the database.
fn query_symbols(
    conn: &rusqlite::Connection,
    filter: &SymbolFilter,
    mode: TokenMode,
) -> Result<Vec<SymbolRecord>, StorageError> {
    let mut sql = format!("SELECT {SYMBOL_COLUMNS} FROM symbols WHERE 1=1");
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref exact) = filter.name_exact {
        sql.push_str(" AND name = ?");
        params.push(Box::new(exact.clone()));
    } else if let Some(ref name) = filter.name {
        let tokens = tokenize_symbol_query(name);
        if tokens.len() <= 1 {
            // Single token: match against the symbol name only.
            let tok = tokens.first().map_or(name.as_str(), |t| t.as_str());
            sql.push_str(" AND name LIKE ?");
            let pat = format!("%{tok}%");
            params.push(Box::new(pat));
        } else {
            // Multi-token: all (or any) tokens must appear in the name.
            let joiner = match mode {
                TokenMode::And => " AND ",
                TokenMode::Or => " OR ",
            };
            sql.push_str(" AND (");
            for (i, token) in tokens.iter().enumerate() {
                if i > 0 {
                    sql.push_str(joiner);
                }
                sql.push_str("name LIKE ?");
                let pat = format!("%{token}%");
                params.push(Box::new(pat));
            }
            sql.push(')');
        }
    }
    if let Some(ref kind) = filter.kind {
        sql.push_str(" AND kind = ?");
        params.push(Box::new(kind.clone()));
    }
    if let Some(ref visibility) = filter.visibility {
        sql.push_str(" AND visibility = ?");
        params.push(Box::new(visibility.clone()));
    }
    if let Some(ref module) = filter.module {
        sql.push_str(" AND (module_path = ? OR module_path LIKE ? OR file_path LIKE ?)");
        params.push(Box::new(module.clone()));
        params.push(Box::new(format!("{module}::%")));
        params.push(Box::new(format!("%{module}%")));
    }
    if let Some(ref file_path) = filter.file_path {
        sql.push_str(" AND file_path = ?");
        params.push(Box::new(file_path.clone()));
    }

    sql.push_str(" ORDER BY file_path, line_start");

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(std::convert::AsRef::as_ref).collect();

    let mut stmt = conn.prepare(&sql).map_err(|e| StorageError::Database {
        reason: e.to_string(),
    })?;

    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok(SymbolRecord {
                id: SymbolId(row.get(0)?),
                file_path: row.get(1)?,
                name: row.get(2)?,
                kind: row.get(3)?,
                visibility: row.get(4)?,
                signature: row.get(5)?,
                doc_comment: row.get(6)?,
                module_path: row.get(7)?,
                line_start: row.get(8)?,
                line_end: row.get(9)?,
                cyclomatic_complexity: row.get(10)?,
            })
        })
        .map_err(|e| StorageError::Database {
            reason: e.to_string(),
        })?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| StorageError::Database {
            reason: e.to_string(),
        })
}
///
/// Expected column order: `id`, `path`, `kind`, `display_name`, `file_count`,
/// `language_stats`, `repo_url`, `branch`, `commit_sha`, `clone_timestamp`, `sparse_paths`.
fn parse_corpus_root_row(row: &rusqlite::Row<'_>) -> CorpusRoot {
    let lang_json: String = row.get(5).unwrap_or_default();
    let language_stats: std::collections::HashMap<String, usize> =
        serde_json::from_str(&lang_json).unwrap_or_default();
    let sparse_json: String = row.get(10).unwrap_or_default();
    let sparse_paths: Vec<String> = serde_json::from_str(&sparse_json).unwrap_or_default();

    CorpusRoot {
        id: row.get(0).unwrap_or_default(),
        path: row.get(1).unwrap_or_default(),
        kind: RootKind::parse(&row.get::<_, String>(2).unwrap_or_default()),
        display_name: row.get(3).unwrap_or_default(),
        file_count: row.get::<_, i64>(4).unwrap_or(0).try_into().unwrap_or(0),
        language_stats,
        repo_url: row.get(6).unwrap_or_default(),
        branch: row.get(7).unwrap_or_default(),
        commit_sha: row.get(8).unwrap_or_default(),
        clone_timestamp: row.get(9).unwrap_or_default(),
        sparse_paths,
    }
}

/// Parse a resolution string back to the [`Resolution`] enum.
fn parse_resolution(s: &str) -> Resolution {
    match s {
        "summary" => Resolution::Summary,
        "claim" => Resolution::Claim,
        // "section" and any unknown value fall back to Section
        _ => Resolution::Section,
    }
}

/// Extension trait to add `.optional()` to rusqlite results.
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(val) => Ok(Some(val)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

// --- FSRS memory state persistence ---

impl SqliteStorage {
    /// Save FSRS memory states to the database (upsert).
    ///
    /// Used at session end to persist learned section importance across sessions.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the database transaction fails.
    pub async fn save_memory_states(
        &self,
        states: &[(String, crate::session::memory::MemoryState)],
    ) -> Result<(), StorageError> {
        let states = states.to_vec();
        self.with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            for (section_id, state) in &states {
                tx.execute(
                    "INSERT INTO section_memory_states
                       (section_id, stability, difficulty, last_access_turn, access_count, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                     ON CONFLICT(section_id) DO UPDATE SET
                       stability = ?2,
                       difficulty = ?3,
                       last_access_turn = ?4,
                       access_count = ?5,
                       updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now')",
                    rusqlite::params![
                        section_id,
                        state.stability,
                        state.difficulty,
                        state.last_access_turn,
                        state.access_count,
                    ],
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            }
            tx.commit().map_err(|e| StorageError::Database {
                reason: e.to_string(),
            })?;
            Ok(())
        })
        .await
    }

    /// Load FSRS memory states from the database, ordered by access count descending.
    ///
    /// Used at session start to restore learned importance from previous sessions.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the database query fails.
    pub async fn load_memory_states(
        &self,
        limit: usize,
    ) -> Result<Vec<(String, crate::session::memory::MemoryState)>, StorageError> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT section_id, stability, difficulty, last_access_turn, access_count
                     FROM section_memory_states
                     ORDER BY access_count DESC
                     LIMIT ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            let rows = stmt
                .query_map(rusqlite::params![limit], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        crate::session::memory::MemoryState {
                            stability: row.get(1)?,
                            difficulty: row.get(2)?,
                            last_access_turn: row.get(3)?,
                            access_count: row.get(4)?,
                        },
                    ))
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row.map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?);
            }
            Ok(result)
        })
        .await
    }

    // ── Full-dimension vector storage (two-stage Matryoshka retrieval) ──

    /// Store a full-dimension embedding vector for two-stage retrieval.
    ///
    /// Uses `INSERT OR REPLACE` so re-indexing overwrites stale vectors.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the database insert fails.
    pub async fn store_full_dim_vectors(
        &self,
        entries: &[(String, Vec<f32>)],
    ) -> Result<(), StorageError> {
        if entries.is_empty() {
            return Ok(());
        }
        let entries: Vec<(String, Vec<f32>)> = entries.to_vec();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare_cached(
                    "INSERT OR REPLACE INTO full_dim_vectors (vector_id, vector, dimension) \
                     VALUES (?1, ?2, ?3)",
                )
                .map_err(|e| StorageError::Database {
                    reason: format!("prepare failed: {e}"),
                })?;
            for (id, vec) in &entries {
                let blob = encode_f32_blob(vec);
                let dim = i64::try_from(vec.len()).unwrap_or(0);
                stmt.execute(rusqlite::params![id, blob, dim])
                    .map_err(|e| StorageError::Database {
                        reason: format!("insert full_dim_vectors failed for {id}: {e}"),
                    })?;
            }
            Ok(())
        })
        .await
    }

    /// Retrieve full-dimension vectors for a batch of IDs.
    ///
    /// Returns `(vector_id, Vec<f32>)` pairs for IDs that exist in the table.
    /// Missing IDs are silently skipped.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the database query fails.
    pub async fn get_full_dim_vectors(
        &self,
        ids: &[&str],
    ) -> Result<Vec<(String, Vec<f32>)>, StorageError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<String> = ids.iter().map(|s| (*s).to_owned()).collect();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT vector_id, vector FROM full_dim_vectors WHERE vector_id = ?1",
                )
                .map_err(|e| StorageError::Database {
                    reason: format!("prepare failed: {e}"),
                })?;
            let mut results = Vec::with_capacity(ids.len());
            for id in &ids {
                if let Ok(row) = stmt.query_row(rusqlite::params![id], |row| {
                    let vec_id: String = row.get(0)?;
                    let blob: Vec<u8> = row.get(1)?;
                    Ok((vec_id, blob))
                }) {
                    results.push((row.0, decode_f32_blob(&row.1)));
                }
            }
            Ok(results)
        })
        .await
    }

    /// Delete full-dimension vectors by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if the database delete fails.
    pub async fn delete_full_dim_vectors(&self, ids: &[&str]) -> Result<(), StorageError> {
        if ids.is_empty() {
            return Ok(());
        }
        let ids: Vec<String> = ids.iter().map(|s| (*s).to_owned()).collect();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare_cached("DELETE FROM full_dim_vectors WHERE vector_id = ?1")
                .map_err(|e| StorageError::Database {
                    reason: format!("prepare failed: {e}"),
                })?;
            for id in &ids {
                stmt.execute(rusqlite::params![id])
                    .map_err(|e| StorageError::Database {
                        reason: format!("delete full_dim_vectors failed for {id}: {e}"),
                    })?;
            }
            Ok(())
        })
        .await
    }
}

/// Encode a `f32` slice as little-endian bytes.
fn encode_f32_blob(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for &val in vector {
        bytes.extend_from_slice(&val.to_le_bytes());
    }
    bytes
}

/// Decode little-endian bytes back into a `f32` vector.
fn decode_f32_blob(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::traits::Storage;
    use crate::types::SymbolId;

    // ── Tokenizer tests ──────────────────────────────────────────────

    #[test]
    fn tokenize_single_word() {
        assert_eq!(tokenize_symbol_query("Session"), vec!["session"]);
    }

    #[test]
    fn tokenize_camel_case() {
        let tokens = tokenize_symbol_query("ToolRouter");
        assert_eq!(tokens, vec!["router", "tool"]);
    }

    #[test]
    fn tokenize_snake_case() {
        let tokens = tokenize_symbol_query("list_symbols");
        assert_eq!(tokens, vec!["list", "symbols"]);
    }

    #[test]
    fn tokenize_multi_word_with_camel_and_snake() {
        let tokens = tokenize_symbol_query("ToolRouter list_symbols");
        assert_eq!(tokens, vec!["list", "router", "symbols", "tool"]);
    }

    #[test]
    fn tokenize_filters_short_tokens() {
        let tokens = tokenize_symbol_query("a fn");
        assert_eq!(tokens, vec!["fn"]);
    }

    #[test]
    fn tokenize_deduplicates() {
        let tokens = tokenize_symbol_query("tool_list ToolList");
        assert_eq!(tokens, vec!["list", "tool"]);
    }

    // ── CamelCase splitter edge cases ────────────────────────────────

    #[test]
    fn split_camel_uppercase_run() {
        // After tokenize_symbol_query filters <2 chars: only "parser" survives.
        let tokens = tokenize_symbol_query("HTMLParser");
        assert!(tokens.contains(&"parser".to_string()));
    }

    #[test]
    fn split_camel_all_lowercase() {
        let mut out = Vec::new();
        split_camel_case("session", &mut out);
        assert_eq!(out, vec!["session"]);
    }

    // ── Integration: query_symbols with AND/OR ───────────────────────

    #[tokio::test]
    async fn query_symbols_single_token_matches_name_only() {
        let storage = test_storage().await;
        insert_test_symbol(
            &storage,
            "sym-1",
            "create_user",
            "function",
            Some("Create a new user account."),
            "pub fn create_user(name: &str) -> User",
        )
        .await;

        // "account" appears in doc_comment but not in the name — should NOT match.
        let filter = SymbolFilter {
            name: Some("account".into()),
            ..Default::default()
        };
        let results = storage.list_symbols(&filter).await.unwrap();
        assert_eq!(
            results.len(),
            0,
            "name filter should not search doc_comment"
        );

        // "create" appears in the name — should match.
        let filter = SymbolFilter {
            name: Some("create".into()),
            ..Default::default()
        };
        let results = storage.list_symbols(&filter).await.unwrap();
        assert_eq!(results.len(), 1, "name filter should match symbol name");
        assert_eq!(results[0].name, "create_user");
    }

    #[tokio::test]
    async fn query_symbols_multi_token_and_fallback_to_or() {
        let storage = test_storage().await;
        insert_test_symbol(
            &storage,
            "sym-1",
            "create_user",
            "function",
            Some("Create a new user."),
            "pub fn create_user()",
        )
        .await;
        insert_test_symbol(
            &storage,
            "sym-2",
            "delete_session",
            "function",
            Some("Delete a session."),
            "pub fn delete_session()",
        )
        .await;

        // "create session" — no single symbol has BOTH tokens.
        // AND should return 0, OR fallback should return both.
        let filter = SymbolFilter {
            name: Some("create session".into()),
            ..Default::default()
        };
        let results = storage.list_symbols(&filter).await.unwrap();
        assert_eq!(results.len(), 2, "OR fallback should find both");
    }

    #[tokio::test]
    async fn query_symbols_camel_case_splitting() {
        let storage = test_storage().await;
        insert_test_symbol(
            &storage,
            "sym-1",
            "build_router",
            "function",
            Some("Build the HTTP router."),
            "pub fn build_router() -> Router",
        )
        .await;

        // "ToolRouter" splits to ["tool", "router"] — should match "router" in name.
        let filter = SymbolFilter {
            name: Some("ToolRouter".into()),
            ..Default::default()
        };
        let results = storage.list_symbols(&filter).await.unwrap();
        assert!(
            results.iter().any(|r| r.name == "build_router"),
            "should find build_router via 'router' token (OR fallback)"
        );
    }

    #[tokio::test]
    async fn query_symbols_and_succeeds_no_fallback() {
        let storage = test_storage().await;
        insert_test_symbol(
            &storage,
            "sym-1",
            "create_session",
            "function",
            Some("Create a new session."),
            "pub fn create_session()",
        )
        .await;

        // "create session" — both tokens in name.
        let filter = SymbolFilter {
            name: Some("create session".into()),
            ..Default::default()
        };
        let results = storage.list_symbols(&filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "create_session");
    }

    // ── Session persistence round-trip ───────────────────────────────

    #[tokio::test]
    async fn save_load_preserves_compression_tier() {
        // Regression: SqliteStorage::save_session used to drop
        // `compression_tier` and `compressed_summary` because the
        // `session_deliveries` schema had no columns for them; load_session
        // hardcoded `Full`. Every daemon restart wiped the compression
        // pipeline's work. V18 added the missing columns.
        use crate::session::{CompressionTier, DropPolicy, SessionId};
        use crate::types::{ContentId, Resolution};

        let storage = test_storage().await;
        let mut session = crate::session::Session::new(
            SessionId::from("persist-tier".to_string()),
            100_000,
            DropPolicy::Fifo,
        );
        session.record_delivery(
            &ContentId::from("s1".to_string()),
            Resolution::Section,
            1000,
            1,
            "h1".to_string(),
        );
        // Simulate the compression pipeline promoting s1 to Extractive.
        session.set_compressed_summary(
            &ContentId::from("s1".to_string()),
            "summary of s1".to_string(),
            CompressionTier::Extractive,
            300,
        );

        storage.save_session(&session).await.unwrap();
        let loaded = storage
            .load_session(&SessionId::from("persist-tier".to_string()))
            .await
            .unwrap()
            .expect("session should load");

        let item = loaded
            .get_delivered(&ContentId::from("s1".to_string()))
            .expect("item should persist");
        assert_eq!(
            item.compression_tier,
            CompressionTier::Extractive,
            "compression_tier must survive round-trip (got {:?})",
            item.compression_tier
        );
        assert_eq!(
            item.compressed_summary.as_deref(),
            Some("summary of s1"),
            "compressed_summary must survive round-trip",
        );
    }

    #[tokio::test]
    async fn save_load_preserves_session_metrics() {
        // Regression: the `sessions` table had no columns for cumulative
        // SessionMetrics (total_deliveries, cumulative_tokens_delivered, …),
        // so every daemon restart zeroed them. V18 added a metrics_json
        // column and restore now re-seeds metrics.
        use crate::session::{DropPolicy, SessionId};
        use crate::types::{ContentId, Resolution};

        let storage = test_storage().await;
        let mut session = crate::session::Session::new(
            SessionId::from("persist-metrics".to_string()),
            100_000,
            DropPolicy::Fifo,
        );
        for i in 0..5_u32 {
            session.record_delivery(
                &ContentId::from(format!("s{i}")),
                Resolution::Section,
                100,
                i + 1,
                format!("h{i}"),
            );
        }
        assert_eq!(session.metrics().total_deliveries, 5);
        assert_eq!(session.metrics().cumulative_tokens_delivered, 500);

        storage.save_session(&session).await.unwrap();
        let loaded = storage
            .load_session(&SessionId::from("persist-metrics".to_string()))
            .await
            .unwrap()
            .expect("session should load");

        assert_eq!(loaded.metrics().total_deliveries, 5);
        assert_eq!(loaded.metrics().cumulative_tokens_delivered, 500);
    }

    #[tokio::test]
    async fn save_load_preserves_recent_queries() {
        // Regression: recent_queries drive the DropRanker's task-salience
        // factor. V18 persists them as JSON so salience doesn't evaporate on
        // daemon restart mid-task.
        use crate::session::{DropPolicy, SessionId};

        let storage = test_storage().await;
        let mut session = crate::session::Session::new(
            SessionId::from("persist-queries".to_string()),
            100_000,
            DropPolicy::Fifo,
        );
        session.record_query("authentication flow");
        session.record_query("JWT tokens");

        storage.save_session(&session).await.unwrap();
        let loaded = storage
            .load_session(&SessionId::from("persist-queries".to_string()))
            .await
            .unwrap()
            .expect("session should load");

        let queries: Vec<&str> = loaded.recent_queries().iter().map(String::as_str).collect();
        assert_eq!(queries, vec!["authentication flow", "JWT tokens"]);
    }

    // ── Test helpers ─────────────────────────────────────────────────

    #[allow(clippy::unused_async)]
    async fn test_storage() -> SqliteStorage {
        SqliteStorage::open_in_memory().expect("in-memory storage")
    }

    async fn insert_test_symbol(
        storage: &SqliteStorage,
        id: &str,
        name: &str,
        kind: &str,
        doc: Option<&str>,
        sig: &str,
    ) {
        let record = SymbolRecord {
            id: SymbolId(id.to_string()),
            file_path: "test.rs".to_string(),
            name: name.to_string(),
            kind: kind.to_string(),
            visibility: "pub".to_string(),
            signature: sig.to_string(),
            doc_comment: doc.map(String::from),
            module_path: "test".to_string(),
            line_start: 1,
            line_end: 10,
            cyclomatic_complexity: None,
        };
        Storage::insert_symbols(storage, &[record]).await.unwrap();
    }
}
