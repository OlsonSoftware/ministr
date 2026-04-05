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

#[cfg(test)]
mod tests {
    use super::*;
    use iris_core::session::CompressionTier;
    use iris_core::types::Resolution;
    use tempfile::TempDir;

    fn make_delivered(id: &str, tokens: usize, turn: u32) -> (String, DeliveredItem) {
        (
            id.to_string(),
            DeliveredItem {
                content_id: ContentId(id.to_string()),
                resolution: Resolution::Section,
                token_count: tokens,
                turn_delivered: turn,
                content_hash: format!("hash-{id}"),
                compression_tier: CompressionTier::Full,
                compressed_summary: None,
            },
        )
    }

    #[test]
    fn roundtrip_save_and_load() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("sessions.db");

        let mut delivered = BTreeMap::new();
        delivered.insert("sec-1".to_string(), make_delivered("sec-1", 100, 1).1);
        delivered.insert("sec-2".to_string(), make_delivered("sec-2", 200, 2).1);

        let trajectory = vec![
            ContentId("sec-1".to_string()),
            ContentId("sec-2".to_string()),
            ContentId("sec-1".to_string()),
        ];

        save_session(&db, "corpus-a", "sess-1", 50_000, 3, &delivered, &trajectory).unwrap();

        let loaded = load_sessions(&db, "corpus-a").unwrap();
        assert_eq!(loaded.len(), 1);
        let s = &loaded[0];
        assert_eq!(s.session_id, "sess-1");
        assert_eq!(s.budget_tokens, 50_000);
        assert_eq!(s.current_turn, 3);
        assert_eq!(s.delivered.len(), 2);
        assert_eq!(s.delivered["sec-1"].token_count, 100);
        assert_eq!(s.delivered["sec-2"].token_count, 200);
        assert_eq!(s.trajectory.len(), 3);
        assert_eq!(s.trajectory[0].0, "sec-1");
        assert_eq!(s.trajectory[2].0, "sec-1");
    }

    #[test]
    fn save_updates_existing_session() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("sessions.db");

        let mut delivered = BTreeMap::new();
        delivered.insert("sec-1".to_string(), make_delivered("sec-1", 100, 1).1);
        save_session(&db, "corpus-a", "sess-1", 50_000, 1, &delivered, &[]).unwrap();

        // Simulate more deliveries and updated turn.
        delivered.insert("sec-2".to_string(), make_delivered("sec-2", 300, 2).1);
        let trajectory = vec![ContentId("sec-2".to_string())];
        save_session(&db, "corpus-a", "sess-1", 50_000, 2, &delivered, &trajectory).unwrap();

        let loaded = load_sessions(&db, "corpus-a").unwrap();
        assert_eq!(loaded.len(), 1, "should upsert, not duplicate");
        assert_eq!(loaded[0].current_turn, 2);
        assert_eq!(loaded[0].delivered.len(), 2);
    }

    #[test]
    fn multiple_sessions_per_corpus() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("sessions.db");

        let d1 = BTreeMap::from([make_delivered("sec-1", 100, 1)]);
        let d2 = BTreeMap::from([make_delivered("sec-2", 200, 1)]);

        save_session(&db, "corpus-a", "sess-1", 50_000, 1, &d1, &[]).unwrap();
        save_session(&db, "corpus-a", "sess-2", 80_000, 1, &d2, &[]).unwrap();

        let loaded = load_sessions(&db, "corpus-a").unwrap();
        assert_eq!(loaded.len(), 2);

        let ids: Vec<&str> = loaded.iter().map(|s| s.session_id.as_str()).collect();
        assert!(ids.contains(&"sess-1"));
        assert!(ids.contains(&"sess-2"));
    }

    #[test]
    fn sessions_isolated_across_corpora() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("sessions.db");

        let d = BTreeMap::from([make_delivered("sec-1", 100, 1)]);
        save_session(&db, "corpus-a", "sess-1", 50_000, 1, &d, &[]).unwrap();
        save_session(&db, "corpus-b", "sess-1", 80_000, 1, &d, &[]).unwrap();

        let a = load_sessions(&db, "corpus-a").unwrap();
        let b = load_sessions(&db, "corpus-b").unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_eq!(a[0].budget_tokens, 50_000);
        assert_eq!(b[0].budget_tokens, 80_000);
    }

    #[test]
    fn delete_session_removes_from_db() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("sessions.db");

        let d = BTreeMap::from([make_delivered("sec-1", 100, 1)]);
        save_session(&db, "corpus-a", "sess-1", 50_000, 1, &d, &[]).unwrap();
        save_session(&db, "corpus-a", "sess-2", 80_000, 1, &d, &[]).unwrap();

        delete_session(&db, "corpus-a", "sess-1").unwrap();

        let loaded = load_sessions(&db, "corpus-a").unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].session_id, "sess-2");
    }

    #[test]
    fn delete_nonexistent_session_is_noop() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("sessions.db");

        let d = BTreeMap::from([make_delivered("sec-1", 100, 1)]);
        save_session(&db, "corpus-a", "sess-1", 50_000, 1, &d, &[]).unwrap();

        // Deleting a session that doesn't exist should not error.
        delete_session(&db, "corpus-a", "nonexistent").unwrap();

        let loaded = load_sessions(&db, "corpus-a").unwrap();
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn load_from_empty_corpus_returns_empty_vec() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("sessions.db");

        // Ensure table exists by saving to a different corpus.
        let d = BTreeMap::from([make_delivered("sec-1", 100, 1)]);
        save_session(&db, "corpus-a", "sess-1", 50_000, 1, &d, &[]).unwrap();

        let loaded = load_sessions(&db, "corpus-b").unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn full_lifecycle_create_use_evict_destroy() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("sessions.db");

        // 1. Create: save a fresh session.
        let d1 = BTreeMap::from([make_delivered("sec-1", 100, 1)]);
        let traj = vec![ContentId("sec-1".to_string())];
        save_session(&db, "corpus-a", "sess-1", 50_000, 1, &d1, &traj).unwrap();

        // 2. Use: simulate more deliveries across turns.
        let mut d2 = d1.clone();
        d2.insert("sec-2".to_string(), make_delivered("sec-2", 250, 2).1);
        d2.insert("sec-3".to_string(), make_delivered("sec-3", 150, 3).1);
        let traj2 = vec![
            ContentId("sec-1".to_string()),
            ContentId("sec-2".to_string()),
            ContentId("sec-3".to_string()),
        ];
        save_session(&db, "corpus-a", "sess-1", 50_000, 3, &d2, &traj2).unwrap();

        let loaded = load_sessions(&db, "corpus-a").unwrap();
        assert_eq!(loaded[0].delivered.len(), 3);
        assert_eq!(loaded[0].current_turn, 3);

        // 3. Evict: remove sec-2 from delivered (simulate agent eviction).
        let mut d3 = d2.clone();
        d3.remove("sec-2");
        save_session(&db, "corpus-a", "sess-1", 50_000, 4, &d3, &traj2).unwrap();

        let loaded = load_sessions(&db, "corpus-a").unwrap();
        assert_eq!(loaded[0].delivered.len(), 2);
        assert!(!loaded[0].delivered.contains_key("sec-2"));

        // 4. Destroy: delete the session.
        delete_session(&db, "corpus-a", "sess-1").unwrap();
        let loaded = load_sessions(&db, "corpus-a").unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn persistence_survives_simulated_restart() {
        let tmp = TempDir::new().unwrap();
        let db = tmp.path().join("sessions.db");

        // "Process 1": save session state.
        let d = BTreeMap::from([
            make_delivered("sec-1", 100, 1),
            make_delivered("sec-2", 200, 2),
        ]);
        let trajectory = vec![
            ContentId("sec-1".to_string()),
            ContentId("sec-2".to_string()),
        ];
        save_session(&db, "corpus-a", "sess-1", 50_000, 2, &d, &trajectory).unwrap();

        // "Process 2": open same DB (simulates daemon restart), load sessions.
        let loaded = load_sessions(&db, "corpus-a").unwrap();
        assert_eq!(loaded.len(), 1);

        let s = &loaded[0];
        assert_eq!(s.session_id, "sess-1");
        assert_eq!(s.budget_tokens, 50_000);
        assert_eq!(s.current_turn, 2);
        assert_eq!(s.delivered.len(), 2);
        assert_eq!(s.delivered["sec-1"].token_count, 100);
        assert_eq!(s.delivered["sec-2"].content_hash, "hash-sec-2");
        assert_eq!(s.trajectory.len(), 2);
    }
}
