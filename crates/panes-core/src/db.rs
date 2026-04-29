use anyhow::{Context, Result};
use rusqlite::Connection;
use tracing::info;

pub fn initialize(db_path: &str) -> Result<Connection> {
    let conn = Connection::open(db_path)
        .with_context(|| format!("failed to open database at {db_path}"))?;

    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    run_migrations(&conn)?;

    info!(path = db_path, "database initialized");
    Ok(conn)
}

fn run_migrations(conn: &Connection) -> Result<()> {
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
