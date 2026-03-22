//! `SQLite` schema definition and migration management.
//!
//! Uses `rusqlite_migration` with the `user_version` pragma to track schema
//! versions. Migrations are forward-only and defined as SQL strings.

use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

use crate::error::StorageError;

/// The current schema version (number of applied migrations).
pub const CURRENT_SCHEMA_VERSION: usize = 5;

/// Returns the migration set for the content database.
///
/// Each migration corresponds to one `user_version` increment.
#[allow(clippy::too_many_lines)]
fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        // V1: Initial schema — documents, sections, claims, summaries, file_hashes
        M::up(
            "
            CREATE TABLE documents (
                id          TEXT PRIMARY KEY NOT NULL,
                title       TEXT NOT NULL,
                source_path TEXT NOT NULL UNIQUE,
                summary     TEXT,
                created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE sections (
                id          TEXT PRIMARY KEY NOT NULL,
                document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
                heading_path TEXT NOT NULL,  -- JSON array of heading strings
                depth       INTEGER NOT NULL,
                text        TEXT NOT NULL,
                summary     TEXT,
                position    INTEGER NOT NULL,  -- ordering within parent
                UNIQUE(document_id, position)
            );

            CREATE INDEX idx_sections_document ON sections(document_id);

            CREATE TABLE claims (
                id         TEXT PRIMARY KEY NOT NULL,
                section_id TEXT NOT NULL REFERENCES sections(id) ON DELETE CASCADE,
                text       TEXT NOT NULL,
                position   INTEGER NOT NULL,  -- ordering within section
                UNIQUE(section_id, position)
            );

            CREATE INDEX idx_claims_section ON claims(section_id);

            CREATE TABLE file_hashes (
                path         TEXT PRIMARY KEY NOT NULL,
                content_hash TEXT NOT NULL,
                last_indexed TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            ",
        ),
        // V2: Session persistence — sessions and delivered items for crash recovery
        M::up(
            "
            CREATE TABLE sessions (
                id             TEXT PRIMARY KEY NOT NULL,
                context_budget INTEGER NOT NULL,
                current_turn   INTEGER NOT NULL DEFAULT 0,
                created_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE session_deliveries (
                session_id     TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                content_id     TEXT NOT NULL,
                resolution     TEXT NOT NULL,
                token_count    INTEGER NOT NULL,
                turn_delivered INTEGER NOT NULL,
                content_hash   TEXT NOT NULL,
                position       INTEGER NOT NULL,
                PRIMARY KEY (session_id, content_id)
            );

            CREATE INDEX idx_session_deliveries_session ON session_deliveries(session_id);
            ",
        ),
        // V3: Claim relationships — cross-references and co-occurring entities
        M::up(
            "
            CREATE TABLE claim_relationships (
                source_claim_id TEXT NOT NULL REFERENCES claims(id) ON DELETE CASCADE,
                target_claim_id TEXT NOT NULL REFERENCES claims(id) ON DELETE CASCADE,
                relation_type   TEXT NOT NULL,
                confidence      REAL NOT NULL DEFAULT 0.0,
                PRIMARY KEY (source_claim_id, target_claim_id, relation_type)
            );

            CREATE INDEX idx_claim_rel_source ON claim_relationships(source_claim_id);
            CREATE INDEX idx_claim_rel_target ON claim_relationships(target_claim_id);
            ",
        ),
        // V4: Cross-session analytics — access frequency and co-access patterns
        M::up(
            "
            CREATE TABLE section_access_stats (
                section_id    TEXT PRIMARY KEY NOT NULL,
                access_count  INTEGER NOT NULL DEFAULT 0,
                last_accessed TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE co_access_patterns (
                section_a TEXT NOT NULL,
                section_b TEXT NOT NULL,
                co_count  INTEGER NOT NULL DEFAULT 1,
                PRIMARY KEY (section_a, section_b)
            );

            CREATE INDEX idx_co_access_a ON co_access_patterns(section_a);
            CREATE INDEX idx_co_access_b ON co_access_patterns(section_b);
            ",
        ),
        // V5: Web cache — fetch metadata per URL for staleness detection
        M::up(
            "
            CREATE TABLE web_cache (
                source_url     TEXT PRIMARY KEY NOT NULL,
                fetch_timestamp TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                etag           TEXT,
                last_modified  TEXT,
                content_hash   TEXT NOT NULL,
                content_type   TEXT
            );
            ",
        ),
    ])
}

/// Configure connection pragmas for performance and correctness.
///
/// - WAL journal mode for concurrent reads during writes
/// - NORMAL synchronous for durability with good performance
/// - 5-second busy timeout for concurrent access
/// - Foreign keys enabled for referential integrity
pub fn configure_connection(conn: &Connection) -> Result<(), StorageError> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| StorageError::Database {
            reason: format!("failed to set WAL mode: {e}"),
        })?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| StorageError::Database {
            reason: format!("failed to set synchronous mode: {e}"),
        })?;
    conn.pragma_update(None, "busy_timeout", 5000)
        .map_err(|e| StorageError::Database {
            reason: format!("failed to set busy_timeout: {e}"),
        })?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| StorageError::Database {
            reason: format!("failed to enable foreign keys: {e}"),
        })?;
    Ok(())
}

/// Run all pending migrations on the given connection.
///
/// # Errors
///
/// Returns [`StorageError::MigrationFailed`] if any migration fails.
pub fn run_migrations(conn: &mut Connection) -> Result<(), StorageError> {
    migrations()
        .to_latest(conn)
        .map_err(|e| StorageError::MigrationFailed {
            reason: e.to_string(),
        })
}

/// Returns the current schema version of the database.
///
/// # Errors
///
/// Returns [`StorageError::Database`] if the pragma query fails.
#[cfg(test)]
pub fn current_version(conn: &Connection) -> Result<usize, StorageError> {
    let version: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| StorageError::Database {
            reason: format!("failed to read user_version: {e}"),
        })?;
    Ok(version as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_valid() {
        migrations().validate().unwrap();
    }

    #[test]
    fn run_migrations_on_fresh_db() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn).unwrap();
        run_migrations(&mut conn).unwrap();

        let version = current_version(&conn).unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn schema_has_expected_tables() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn).unwrap();
        run_migrations(&mut conn).unwrap();

        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();

        assert!(tables.contains(&"documents".to_string()));
        assert!(tables.contains(&"sections".to_string()));
        assert!(tables.contains(&"claims".to_string()));
        assert!(tables.contains(&"file_hashes".to_string()));
        assert!(tables.contains(&"sessions".to_string()));
        assert!(tables.contains(&"session_deliveries".to_string()));
        assert!(tables.contains(&"section_access_stats".to_string()));
        assert!(tables.contains(&"co_access_patterns".to_string()));
        assert!(tables.contains(&"web_cache".to_string()));
    }

    #[test]
    fn wal_mode_is_active() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        configure_connection(&conn).unwrap();

        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn).unwrap();
        run_migrations(&mut conn).unwrap();

        // Inserting a section with a non-existent document_id should fail
        let result = conn.execute(
            "INSERT INTO sections (id, document_id, heading_path, depth, text, position) VALUES ('s1', 'nonexistent', '[]', 1, 'text', 0)",
            [],
        );
        assert!(result.is_err());
    }

    #[test]
    fn migrations_are_idempotent() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn).unwrap();
        run_migrations(&mut conn).unwrap();
        // Running again should be a no-op
        run_migrations(&mut conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
    }
}
