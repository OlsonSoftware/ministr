//! SQLite-backed [`Storage`] implementation.
//!
//! All rusqlite calls are wrapped in `tokio::spawn_blocking` to avoid
//! blocking the async runtime. The [`Connection`] is held behind a
//! `Mutex` to satisfy `Send + Sync` requirements.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use tracing::instrument;

use super::schema::{configure_connection, run_migrations};
use super::traits::{ClaimRecord, DocumentRecord, FileHashRecord, SectionRecord, Storage};
use crate::error::StorageError;
use crate::session::{DeliveredItem, Session, SessionId};
use crate::types::{ClaimId, ContentId, DocumentTree, Resolution, Section, SectionId};

/// SQLite-backed storage for a single corpus.
///
/// The connection is wrapped in `Arc<Mutex<Connection>>` so it can be
/// shared across `spawn_blocking` tasks. The mutex is held only for the
/// duration of each blocking call, never across `.await` points.
#[derive(Clone)]
pub struct SqliteStorage {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStorage {
    /// Open (or create) a content database at the given path.
    ///
    /// Configures WAL mode, runs pending migrations, and returns a ready
    /// storage handle.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Database`] if the connection cannot be opened,
    /// or [`StorageError::MigrationFailed`] if migrations fail.
    #[instrument(skip_all, fields(path = %path.as_ref().display()))]
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let mut conn = Connection::open(path.as_ref()).map_err(|e| StorageError::Database {
            reason: format!("failed to open database: {e}"),
        })?;
        configure_connection(&conn)?;
        run_migrations(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Open an in-memory database (useful for testing).
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] if connection setup fails.
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let mut conn = Connection::open_in_memory().map_err(|e| StorageError::Database {
            reason: format!("failed to open in-memory database: {e}"),
        })?;
        configure_connection(&conn)?;
        run_migrations(&mut conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Run a blocking closure against the connection inside `spawn_blocking`.
    async fn with_conn<F, T>(&self, f: F) -> Result<T, StorageError>
    where
        F: FnOnce(&Connection) -> Result<T, StorageError> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = conn.lock().map_err(|e| StorageError::Database {
                reason: format!("mutex poisoned: {e}"),
            })?;
            f(&guard)
        })
        .await
        .map_err(|e| StorageError::Database {
            reason: format!("spawn_blocking join error: {e}"),
        })?
    }
}

/// Insert all sections (and their claims) for a document recursively.
fn insert_sections_recursive(
    conn: &Connection,
    document_id: &str,
    sections: &[Section],
    position_offset: &mut i64,
) -> Result<(), StorageError> {
    for section in sections {
        let heading_json = serde_json::to_string(&section.heading_path).map_err(|e| {
            StorageError::Serialization {
                reason: e.to_string(),
            }
        })?;

        conn.execute(
            "INSERT INTO sections (id, document_id, heading_path, depth, text, summary, position)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                section.id.as_ref(),
                document_id,
                heading_json,
                section.depth,
                section.text,
                section.summary,
                *position_offset,
            ],
        )
        .map_err(|e| StorageError::Database {
            reason: format!("failed to insert section {}: {e}", section.id),
        })?;

        *position_offset += 1;

        // Insert claims for this section
        for (claim_pos, claim) in section.claims.iter().enumerate() {
            conn.execute(
                "INSERT INTO claims (id, section_id, text, position) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    claim.id.as_ref(),
                    section.id.as_ref(),
                    claim.text,
                    i64::try_from(claim_pos).unwrap_or(i64::MAX),
                ],
            )
            .map_err(|e| StorageError::Database {
                reason: format!("failed to insert claim {}: {e}", claim.id),
            })?;
        }

        // Recurse into children
        insert_sections_recursive(conn, document_id, &section.children, position_offset)?;
    }
    Ok(())
}

impl Storage for SqliteStorage {
    async fn insert_document(&self, doc: &DocumentTree) -> Result<(), StorageError> {
        let doc = doc.clone();
        self.with_conn(move |conn| {
            conn.execute("BEGIN", [])
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to begin transaction: {e}"),
                })?;

            conn.execute(
                "INSERT INTO documents (id, title, source_path, summary) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![doc.id.as_ref(), doc.title, doc.source_path, doc.summary,],
            )
            .map_err(|e| StorageError::Database {
                reason: format!("failed to insert document {}: {e}", doc.id),
            })?;

            let mut pos = 0i64;
            insert_sections_recursive(conn, doc.id.as_ref(), &doc.sections, &mut pos)?;

            conn.execute("COMMIT", [])
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to commit: {e}"),
                })?;
            Ok(())
        })
        .await
    }

    async fn get_document(&self, id: &ContentId) -> Result<Option<DocumentRecord>, StorageError> {
        let id = id.clone();
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT id, title, source_path, summary FROM documents WHERE id = ?1")
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

    async fn list_documents(&self) -> Result<Vec<DocumentRecord>, StorageError> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id, title, source_path, summary FROM documents ORDER BY title")
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

    async fn upsert_file_hash(&self, record: &FileHashRecord) -> Result<(), StorageError> {
        let record = record.clone();
        self.with_conn(move |conn| {
            conn.execute(
                "INSERT INTO file_hashes (path, content_hash, last_indexed)
                 VALUES (?1, ?2, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                 ON CONFLICT(path) DO UPDATE SET
                    content_hash = excluded.content_hash,
                    last_indexed = excluded.last_indexed",
                rusqlite::params![record.path, record.content_hash],
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
                .prepare("SELECT path, content_hash FROM file_hashes WHERE path = ?1")
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let result = stmt
                .query_row(rusqlite::params![path], |row| {
                    Ok(FileHashRecord {
                        path: row.get(0)?,
                        content_hash: row.get(1)?,
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

    async fn save_session(&self, session: &Session) -> Result<(), StorageError> {
        let id = session.id.0.clone();
        let budget = session.agent_context_budget;
        let turn = session.current_turn();
        let items: Vec<DeliveredItem> = session.delivered_items().cloned().collect();
        let trajectory: Vec<ContentId> = session.trajectory().to_vec();

        self.with_conn(move |conn| {
            conn.execute("BEGIN", [])
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to begin transaction: {e}"),
                })?;

            // Upsert session row
            conn.execute(
                "INSERT INTO sessions (id, context_budget, current_turn, updated_at)
                 VALUES (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
                 ON CONFLICT(id) DO UPDATE SET
                    context_budget = excluded.context_budget,
                    current_turn = excluded.current_turn,
                    updated_at = excluded.updated_at",
                rusqlite::params![id, budget, turn],
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
                // Last occurrence wins — gives the most recent position
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
                     (session_id, content_id, resolution, token_count, turn_delivered, content_hash, position)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        id,
                        item.content_id.0,
                        item.resolution.to_string(),
                        item.token_count,
                        item.turn_delivered,
                        item.content_hash,
                        position,
                    ],
                )
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to insert session delivery: {e}"),
                })?;
            }

            conn.execute("COMMIT", [])
                .map_err(|e| StorageError::Database {
                    reason: format!("failed to commit: {e}"),
                })?;
            Ok(())
        })
        .await
    }

    async fn load_session(&self, id: &SessionId) -> Result<Option<Session>, StorageError> {
        let id = id.0.clone();
        self.with_conn(move |conn| {
            // Load session metadata
            let mut stmt = conn
                .prepare("SELECT id, context_budget, current_turn FROM sessions WHERE id = ?1")
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let session_row = stmt
                .query_row(rusqlite::params![id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, usize>(1)?,
                        row.get::<_, u32>(2)?,
                    ))
                })
                .optional()
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let Some((session_id, budget, turn)) = session_row else {
                return Ok(None);
            };

            // Load delivered items ordered by position (for trajectory reconstruction)
            let mut stmt = conn
                .prepare(
                    "SELECT content_id, resolution, token_count, turn_delivered, content_hash, position
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
                    ))
                })
                .map_err(|e| StorageError::Database {
                    reason: e.to_string(),
                })?;

            let mut delivered = std::collections::BTreeMap::new();
            let mut trajectory = Vec::new();

            for row in rows {
                let (content_id_str, resolution_str, token_count, turn_delivered, content_hash) =
                    row.map_err(|e| StorageError::Database {
                        reason: e.to_string(),
                    })?;

                let content_id = ContentId(content_id_str.clone());
                let resolution = parse_resolution(&resolution_str);

                let item = DeliveredItem {
                    content_id: content_id.clone(),
                    resolution,
                    token_count,
                    turn_delivered,
                    content_hash,
                };

                delivered.insert(content_id_str.clone(), item);
                trajectory.push(content_id);
            }

            Ok(Some(Session::restore(
                SessionId(session_id),
                budget,
                delivered,
                trajectory,
                turn,
            )))
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
