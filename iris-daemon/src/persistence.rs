//! Session persistence — save/restore session state across daemon restarts.
//!
//! Stores the restorable components of each session (delivered items,
//! trajectory, budget config, turn counter) in a `SQLite` database alongside
//! the corpus data.

use std::collections::BTreeMap;
use std::path::Path;

use iris_core::session::DeliveredItem;
use iris_core::types::ContentId;
use rusqlite::params;
use tracing::warn;

/// A saved session ready for restoration.
pub struct SavedSession {
    pub session_id: String,
    pub budget_tokens: usize,
    pub current_turn: u32,
    pub delivered: BTreeMap<String, DeliveredItem>,
    pub trajectory: Vec<ContentId>,
}

/// Ensure the session persistence table exists.
///
/// # Errors
///
/// Returns a rusqlite error if the table cannot be created.
pub fn ensure_table(conn: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS daemon_sessions (
            corpus_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            budget_tokens INTEGER NOT NULL,
            current_turn INTEGER NOT NULL,
            delivered_json TEXT NOT NULL,
            trajectory_json TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (corpus_id, session_id)
        );",
    )
}

/// Save or update a session's state.
///
/// # Errors
///
/// Returns a rusqlite error if the upsert fails.
pub fn save_session(
    db_path: &Path,
    corpus_id: &str,
    session_id: &str,
    budget_tokens: usize,
    current_turn: u32,
    delivered: &BTreeMap<String, DeliveredItem>,
    trajectory: &[ContentId],
) -> Result<(), rusqlite::Error> {
    let conn = rusqlite::Connection::open(db_path)?;
    ensure_table(&conn)?;

    let delivered_json = serde_json::to_string(delivered).unwrap_or_default();
    let trajectory_ids: Vec<&str> = trajectory.iter().map(|c| c.0.as_str()).collect();
    let trajectory_json = serde_json::to_string(&trajectory_ids).unwrap_or_default();

    conn.execute(
        "INSERT INTO daemon_sessions (corpus_id, session_id, budget_tokens, current_turn, delivered_json, trajectory_json, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'))
         ON CONFLICT(corpus_id, session_id) DO UPDATE SET
           budget_tokens = excluded.budget_tokens,
           current_turn = excluded.current_turn,
           delivered_json = excluded.delivered_json,
           trajectory_json = excluded.trajectory_json,
           updated_at = excluded.updated_at",
        params![corpus_id, session_id, budget_tokens, current_turn, delivered_json, trajectory_json],
    )?;

    Ok(())
}

/// Load all persisted sessions for a corpus.
///
/// # Errors
///
/// Returns a rusqlite error if the query fails.
pub fn load_sessions(
    db_path: &Path,
    corpus_id: &str,
) -> Result<Vec<SavedSession>, rusqlite::Error> {
    let conn = rusqlite::Connection::open(db_path)?;
    ensure_table(&conn)?;

    let mut stmt = conn.prepare(
        "SELECT session_id, budget_tokens, current_turn, delivered_json, trajectory_json
         FROM daemon_sessions WHERE corpus_id = ?1",
    )?;

    let sessions = stmt
        .query_map(params![corpus_id], |row| {
            let session_id: String = row.get(0)?;
            let budget_tokens: usize = row.get(1)?;
            let current_turn: u32 = row.get(2)?;
            let delivered_json: String = row.get(3)?;
            let trajectory_json: String = row.get(4)?;

            let delivered: BTreeMap<String, DeliveredItem> =
                serde_json::from_str(&delivered_json).unwrap_or_default();
            let trajectory_strs: Vec<String> =
                serde_json::from_str(&trajectory_json).unwrap_or_default();
            let trajectory: Vec<ContentId> = trajectory_strs.into_iter().map(ContentId).collect();

            Ok(SavedSession {
                session_id,
                budget_tokens,
                current_turn,
                delivered,
                trajectory,
            })
        })?
        .filter_map(|r| match r {
            Ok(s) => Some(s),
            Err(e) => {
                warn!(error = %e, "failed to load persisted session");
                None
            }
        })
        .collect();

    Ok(sessions)
}

/// Delete a persisted session.
///
/// # Errors
///
/// Returns a rusqlite error if the delete fails.
pub fn delete_session(
    db_path: &Path,
    corpus_id: &str,
    session_id: &str,
) -> Result<(), rusqlite::Error> {
    let conn = rusqlite::Connection::open(db_path)?;
    ensure_table(&conn)?;
    conn.execute(
        "DELETE FROM daemon_sessions WHERE corpus_id = ?1 AND session_id = ?2",
        params![corpus_id, session_id],
    )?;
    Ok(())
}
