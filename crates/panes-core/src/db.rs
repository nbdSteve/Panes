use anyhow::{Context, Result};
use rusqlite::Connection;
use tracing::info;

pub fn initialize(db_path: &str) -> Result<Connection> {
    let conn = Connection::open(db_path)
        .with_context(|| format!("failed to open database at {db_path}"))?;

    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    run_migrations(&conn)?;
    recover_stale_threads(&conn)?;

    info!(path = db_path, "database initialized");
    Ok(conn)
}

fn recover_stale_threads(conn: &Connection) -> Result<()> {
    let count = conn.execute(
        "UPDATE threads SET status = 'interrupted' WHERE status IN ('running', 'gate')",
        [],
    )?;
    if count > 0 {
        info!(count, "recovered stale threads from previous crash");
    }
    Ok(())
}

pub(crate) fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS workspaces (
            id TEXT PRIMARY KEY,
            path TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            default_agent TEXT,
            budget_cap REAL,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS threads (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id),
            agent_type TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            prompt TEXT NOT NULL,
            summary TEXT,
            started_at TEXT,
            completed_at TEXT,
            cost_usd REAL DEFAULT 0,
            duration_ms INTEGER,
            snapshot_ref TEXT,
            is_routine INTEGER DEFAULT 0,
            flow_id TEXT,
            flow_step INTEGER,
            created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_threads_workspace ON threads(workspace_id);
        CREATE INDEX IF NOT EXISTS idx_threads_status ON threads(status);

        CREATE TABLE IF NOT EXISTS events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            thread_id TEXT NOT NULL REFERENCES threads(id),
            event_type TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            data TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_events_thread ON events(thread_id);

        CREATE TABLE IF NOT EXISTS costs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            thread_id TEXT NOT NULL REFERENCES threads(id),
            workspace_id TEXT NOT NULL,
            input_tokens INTEGER DEFAULT 0,
            output_tokens INTEGER DEFAULT 0,
            total_usd REAL DEFAULT 0,
            model TEXT,
            timestamp TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_costs_thread ON costs(thread_id);
        CREATE INDEX IF NOT EXISTS idx_costs_workspace ON costs(workspace_id);
        ",
    )
    .context("failed to run database migrations")?;

    // Incremental migrations
    add_column_if_missing(conn, "threads", "session_id", "TEXT")?;

    Ok(())
}

fn add_column_if_missing(conn: &Connection, table: &str, column: &str, col_type: &str) -> Result<()> {
    let columns: Vec<String> = conn
        .prepare(&format!("PRAGMA table_info({table})"))?
        .query_map([], |row| row.get::<_, String>(1))?
        .filter_map(|r| r.ok())
        .collect();
    if !columns.iter().any(|c| c == column) {
        conn.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {col_type}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open(":memory:").unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn insert_workspace(conn: &Connection, id: &str) {
        conn.execute(
            "INSERT INTO workspaces (id, path, name, created_at) VALUES (?1, ?2, ?3, '2024-01-01')",
            rusqlite::params![id, format!("/tmp/{id}"), id],
        ).unwrap();
    }

    fn insert_thread(conn: &Connection, id: &str, workspace_id: &str, status: &str) {
        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, created_at) VALUES (?1, ?2, 'claude-code', ?3, 'test', '2024-01-01')",
            rusqlite::params![id, workspace_id, status],
        ).unwrap();
    }

    fn get_status(conn: &Connection, thread_id: &str) -> String {
        conn.query_row(
            "SELECT status FROM threads WHERE id = ?1",
            rusqlite::params![thread_id],
            |row| row.get(0),
        ).unwrap()
    }

    #[test]
    fn test_recover_stale_running_threads() {
        let conn = setup_db();
        insert_workspace(&conn, "ws1");
        insert_thread(&conn, "t1", "ws1", "running");
        insert_thread(&conn, "t2", "ws1", "gate");

        recover_stale_threads(&conn).unwrap();

        assert_eq!(get_status(&conn, "t1"), "interrupted");
        assert_eq!(get_status(&conn, "t2"), "interrupted");
    }

    #[test]
    fn test_recover_leaves_terminal_states_alone() {
        let conn = setup_db();
        insert_workspace(&conn, "ws1");
        insert_thread(&conn, "t1", "ws1", "completed");
        insert_thread(&conn, "t2", "ws1", "error");
        insert_thread(&conn, "t3", "ws1", "interrupted");

        recover_stale_threads(&conn).unwrap();

        assert_eq!(get_status(&conn, "t1"), "completed");
        assert_eq!(get_status(&conn, "t2"), "error");
        assert_eq!(get_status(&conn, "t3"), "interrupted");
    }

    #[test]
    fn test_initialize_creates_tables() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let conn = initialize(db_path.to_str().unwrap()).unwrap();

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='threads'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_add_column_if_missing_is_idempotent() {
        let conn = setup_db();
        // session_id already added by migrations
        add_column_if_missing(&conn, "threads", "session_id", "TEXT").unwrap();
        // Should not error on second call
        add_column_if_missing(&conn, "threads", "session_id", "TEXT").unwrap();
    }
}
