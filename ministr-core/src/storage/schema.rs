//! `SQLite` schema definition and migration management.
//!
//! Uses `rusqlite_migration` with the `user_version` pragma to track schema
//! versions. Migrations are forward-only and defined as SQL strings.

use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

use crate::error::StorageError;

/// The current schema version (number of applied migrations).
pub const CURRENT_SCHEMA_VERSION: usize = 21;

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
        // V6: Git cache — clone metadata per repo URL for staleness detection
        M::up(
            "
            CREATE TABLE git_cache (
                repo_url          TEXT PRIMARY KEY NOT NULL,
                branch            TEXT,
                commit_sha        TEXT NOT NULL,
                clone_timestamp   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                clone_dir         TEXT NOT NULL,
                checked_out_paths TEXT NOT NULL DEFAULT '[]'
            );
            ",
        ),
        // V7: Code symbols and cross-references
        M::up(
            "
            CREATE TABLE symbols (
                id          TEXT PRIMARY KEY NOT NULL,
                file_path   TEXT NOT NULL,
                name        TEXT NOT NULL,
                kind        TEXT NOT NULL,
                visibility  TEXT NOT NULL DEFAULT '',
                signature   TEXT NOT NULL DEFAULT '',
                doc_comment TEXT,
                module_path TEXT NOT NULL DEFAULT '',
                line_start  INTEGER NOT NULL,
                line_end    INTEGER NOT NULL
            );

            CREATE INDEX idx_symbols_file_path ON symbols(file_path);
            CREATE INDEX idx_symbols_name ON symbols(name);
            CREATE INDEX idx_symbols_kind ON symbols(kind);
            CREATE INDEX idx_symbols_module_path ON symbols(module_path);

            CREATE TABLE symbol_refs (
                from_symbol_id TEXT NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
                to_symbol_id   TEXT NOT NULL REFERENCES symbols(id) ON DELETE CASCADE,
                ref_kind       TEXT NOT NULL,
                PRIMARY KEY (from_symbol_id, to_symbol_id, ref_kind)
            );

            CREATE INDEX idx_symbol_refs_from ON symbol_refs(from_symbol_id);
            CREATE INDEX idx_symbol_refs_to ON symbol_refs(to_symbol_id);
            ",
        ),
        // V8: Cyclomatic complexity metric for code symbols
        M::up(
            "
            ALTER TABLE symbols ADD COLUMN cyclomatic_complexity INTEGER;
            ",
        ),
        // V9: File mtime for fast change detection without re-hashing
        M::up(
            "
            ALTER TABLE file_hashes ADD COLUMN mtime_ns INTEGER;
            ",
        ),
        // V10: Cross-language bridge endpoints and links
        M::up(
            "
            CREATE TABLE bridge_endpoints (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                file_path   TEXT NOT NULL,
                binding_key TEXT NOT NULL,
                kind        TEXT NOT NULL,
                role        TEXT NOT NULL,
                language    TEXT NOT NULL,
                line        INTEGER NOT NULL,
                symbol_name TEXT NOT NULL,
                confidence  REAL NOT NULL
            );

            CREATE INDEX idx_bridge_ep_key ON bridge_endpoints(binding_key, kind);
            CREATE INDEX idx_bridge_ep_file ON bridge_endpoints(file_path);

            CREATE TABLE bridge_links (
                id           INTEGER PRIMARY KEY AUTOINCREMENT,
                export_ep_id INTEGER NOT NULL REFERENCES bridge_endpoints(id) ON DELETE CASCADE,
                import_ep_id INTEGER NOT NULL REFERENCES bridge_endpoints(id) ON DELETE CASCADE,
                kind         TEXT NOT NULL,
                confidence   REAL NOT NULL,
                UNIQUE(export_ep_id, import_ep_id)
            );

            CREATE INDEX idx_bridge_links_kind ON bridge_links(kind);
            ",
        ),
        // V11: Multi-root corpus — per-directory metadata and language stats
        M::up(
            "
            CREATE TABLE corpus_roots (
                id           TEXT PRIMARY KEY NOT NULL,
                path         TEXT NOT NULL UNIQUE,
                kind         TEXT NOT NULL DEFAULT 'local',
                display_name TEXT,
                file_count   INTEGER NOT NULL DEFAULT 0,
                language_stats TEXT NOT NULL DEFAULT '{}',
                created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            ALTER TABLE documents ADD COLUMN root_id TEXT REFERENCES corpus_roots(id);
            CREATE INDEX idx_documents_root ON documents(root_id);
            ",
        ),
        // V12: Git provenance metadata on corpus_roots
        M::up(
            "
            ALTER TABLE corpus_roots ADD COLUMN repo_url TEXT;
            ALTER TABLE corpus_roots ADD COLUMN branch TEXT;
            ALTER TABLE corpus_roots ADD COLUMN commit_sha TEXT;
            ALTER TABLE corpus_roots ADD COLUMN clone_timestamp TEXT;
            ALTER TABLE corpus_roots ADD COLUMN sparse_paths TEXT NOT NULL DEFAULT '[]';
            ",
        ),
        // V13: Content-addressable embedding cache — skip re-embedding unchanged chunks
        M::up(
            "
            CREATE TABLE embedding_cache (
                content_hash TEXT NOT NULL,
                model_name   TEXT NOT NULL,
                vector       BLOB NOT NULL,
                created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now')),
                PRIMARY KEY (content_hash, model_name)
            );
            ",
        ),
        // V14: Deferred reference resolution queue — survives restarts
        M::up(
            "
            CREATE TABLE pending_refs (
                from_symbol_id TEXT NOT NULL,
                target_name    TEXT NOT NULL,
                kind           TEXT NOT NULL,
                file_path      TEXT NOT NULL,
                target_crate   TEXT,
                PRIMARY KEY (from_symbol_id, target_name, kind)
            );
            ",
        ),
        // V15: FSRS memory states — cross-session section importance learning
        M::up(
            "
            CREATE TABLE section_memory_states (
                section_id    TEXT PRIMARY KEY NOT NULL,
                stability     REAL NOT NULL DEFAULT 1.0,
                difficulty    REAL NOT NULL DEFAULT 0.3,
                last_access_turn INTEGER NOT NULL DEFAULT 0,
                access_count  INTEGER NOT NULL DEFAULT 0,
                updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );
            ",
        ),
        // V16: Full-dimension vectors for two-stage Matryoshka retrieval.
        // Stores the un-truncated embedding alongside the truncated one in HNSW,
        // enabling coarse search at low dim + full-dim reranking.
        M::up(
            "
            CREATE TABLE full_dim_vectors (
                vector_id  TEXT PRIMARY KEY NOT NULL,
                vector     BLOB NOT NULL,
                dimension  INTEGER NOT NULL
            );
            ",
        ),
        // V17: Answer cache for ministr_ask sub-inference.
        // answer_cache stores synthesized answers keyed by query hash.
        // answer_cache_sources is a reverse index: given a changed section_id,
        // find all cached answers to invalidate in O(changed_sections).
        M::up(
            "
            CREATE TABLE answer_cache (
                query_hash   TEXT PRIMARY KEY NOT NULL,
                query_text   TEXT NOT NULL,
                answer       TEXT NOT NULL,
                model        TEXT NOT NULL,
                token_count  INTEGER NOT NULL,
                created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))
            );

            CREATE TABLE answer_cache_sources (
                query_hash   TEXT NOT NULL REFERENCES answer_cache(query_hash) ON DELETE CASCADE,
                section_id   TEXT NOT NULL,
                section_hash TEXT NOT NULL,
                PRIMARY KEY (query_hash, section_id)
            );

            CREATE INDEX idx_answer_cache_sources_section ON answer_cache_sources(section_id);
            ",
        ),
        // V18: Persist fields that were silently reset on every daemon
        // restart — compression tier + summary on delivered items, and the
        // cumulative SessionMetrics / recent-query sliding window on the
        // session row. Without these, the compression pipeline's work was
        // discarded and monotonic counters zeroed across restarts.
        M::up(
            "
            ALTER TABLE session_deliveries ADD COLUMN compression_tier TEXT NOT NULL DEFAULT 'full';
            ALTER TABLE session_deliveries ADD COLUMN compressed_summary TEXT;
            ALTER TABLE sessions ADD COLUMN metrics_json TEXT;
            ALTER TABLE sessions ADD COLUMN recent_queries_json TEXT;
            ",
        ),
        // V19: Auto-heal for extractor version drift. `file_hashes` now
        // carries the extractor version that produced its cached refs /
        // symbols, compared against `ingestion::EXTRACTOR_VERSION` on
        // re-ingest. Rows written before this migration get 0, which is
        // below any real version, so they're naturally re-parsed on the
        // first run after upgrade — no manual corpus wipe needed when
        // the symbol-ref extractor logic changes.
        M::up(
            "
            ALTER TABLE file_hashes ADD COLUMN extractor_version INTEGER NOT NULL DEFAULT 0;
            ",
        ),
        // V20: Corpus-root stat-merkle short-circuit. A reindex against
        // an unchanged tree no longer needs to walk + hash every file:
        // we fingerprint the corpus by a sorted BLAKE3 over each file's
        // (rel_path, mtime_ns, size) tuple, store the root hash, and
        // bail out at the top of `ingest_directory_with_embeddings_rooted`
        // when it matches.
        //
        // We deliberately skip content hashing for the fingerprint —
        // hashing 10M LOC of source on every reindex defeats the
        // purpose. mtime+size is what every fast indexer uses (Cursor,
        // CocoIndex). When mtime drifts but content actually matches,
        // the existing per-file `file_hashes.content_hash` cache catches
        // it inside the partial reindex path.
        M::up(
            "
            CREATE TABLE corpus_merkle (
                corpus_id       TEXT PRIMARY KEY NOT NULL,
                root_hash       TEXT NOT NULL,
                file_count      INTEGER NOT NULL,
                last_indexed_ns INTEGER NOT NULL
            );
            ",
        ),
        // V21: Pin the corpus stat-merkle short-circuit to the
        // extractor version that produced the on-disk index. Without
        // this, an `EXTRACTOR_VERSION` bump (e.g. the C++ grammar swap
        // in Phase 2) lets a stat-fingerprint match silently skip the
        // re-extraction the bump was meant to trigger. Default 0 means
        // every existing V20 row reads back as below any real version
        // and forces a full reindex on first run after upgrade.
        M::up(
            "
            ALTER TABLE corpus_merkle ADD COLUMN extractor_version INTEGER NOT NULL DEFAULT 0;
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
///
/// Verifies the journal mode actually took effect — SQLite silently
/// falls back to `DELETE` on filesystems that don't support WAL
/// (tmpfs, some network mounts, `:memory:`). When `require_wal` is
/// true, a fallback is a hard error; when false (in-memory test
/// databases), the fallback is logged at debug level.
pub fn configure_connection(conn: &Connection, require_wal: bool) -> Result<(), StorageError> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| StorageError::Database {
            reason: format!("failed to set WAL mode: {e}"),
        })?;
    let actual_mode: String = conn
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(|e| StorageError::Database {
            reason: format!("failed to read journal_mode: {e}"),
        })?;
    if !actual_mode.eq_ignore_ascii_case("wal") {
        if require_wal {
            return Err(StorageError::Database {
                reason: format!(
                    "journal_mode did not stick — got {actual_mode:?}, wanted WAL \
                     (filesystem may not support WAL — e.g. tmpfs / network mount)"
                ),
            });
        }
        tracing::debug!(
            mode = %actual_mode,
            "journal_mode fell back from WAL (expected for in-memory databases)"
        );
    }
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
        configure_connection(&conn, false).unwrap();
        run_migrations(&mut conn).unwrap();

        let version = current_version(&conn).unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn schema_has_expected_tables() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn, false).unwrap();
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
        assert!(tables.contains(&"git_cache".to_string()));
        assert!(tables.contains(&"co_access_patterns".to_string()));
        assert!(tables.contains(&"web_cache".to_string()));
        assert!(tables.contains(&"symbols".to_string()));
        assert!(tables.contains(&"symbol_refs".to_string()));
        assert!(tables.contains(&"bridge_endpoints".to_string()));
        assert!(tables.contains(&"bridge_links".to_string()));
        assert!(tables.contains(&"corpus_roots".to_string()));
        assert!(tables.contains(&"embedding_cache".to_string()));
        assert!(tables.contains(&"full_dim_vectors".to_string()));
        assert!(tables.contains(&"answer_cache".to_string()));
        assert!(tables.contains(&"answer_cache_sources".to_string()));
        assert!(tables.contains(&"corpus_merkle".to_string()));
    }

    #[test]
    fn wal_mode_is_active() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        configure_connection(&conn, false).unwrap();

        let mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    }

    #[test]
    fn foreign_keys_are_enforced() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure_connection(&conn, false).unwrap();
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
        configure_connection(&conn, false).unwrap();
        run_migrations(&mut conn).unwrap();
        // Running again should be a no-op
        run_migrations(&mut conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), CURRENT_SCHEMA_VERSION);
    }
}
