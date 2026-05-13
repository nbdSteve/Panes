use anyhow::{Context, Result};
use rusqlite::Connection;
use tokio::sync::{mpsc, oneshot};
use tracing::info;

type DbOp = Box<dyn FnOnce(&Connection) + Send>;

#[derive(Clone)]
pub struct DbHandle {
    tx: mpsc::Sender<DbOp>,
}

impl DbHandle {
    pub fn new(conn: Connection) -> Self {
        let (tx, rx) = mpsc::channel::<DbOp>(256);
        std::thread::spawn(move || Self::actor_loop(conn, rx));
        Self { tx }
    }

    fn actor_loop(conn: Connection, mut rx: mpsc::Receiver<DbOp>) {
        while let Some(op) = rx.blocking_recv() {
            op(&conn);
        }
    }

    pub async fn execute<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .send(Box::new(move |conn| {
                let _ = resp_tx.send(f(conn));
            }))
            .await
            .map_err(|_| anyhow::anyhow!("db actor shut down"))?;
        resp_rx
            .await
            .map_err(|_| anyhow::anyhow!("db actor dropped response"))?
    }

    pub fn try_execute_blocking<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let (resp_tx, resp_rx) = oneshot::channel();
        self.tx
            .try_send(Box::new(move |conn| {
                let _ = resp_tx.send(f(conn));
            }))
            .map_err(|_| anyhow::anyhow!("db actor shut down or full"))?;
        resp_rx
            .blocking_recv()
            .map_err(|_| anyhow::anyhow!("db actor dropped response"))?
    }
}

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

        CREATE TABLE IF NOT EXISTS features (
            id TEXT PRIMARY KEY,
            enabled INTEGER NOT NULL DEFAULT 0
        );
        ",
    )
    .context("failed to run database migrations")?;

    // Incremental migrations
    add_column_if_missing(conn, "threads", "session_id", "TEXT")?;
    add_column_if_missing(conn, "threads", "routine_id", "TEXT")?;
    add_column_if_missing(
        conn,
        "threads",
        "tracker_kind",
        "TEXT NOT NULL DEFAULT 'git'",
    )?;
    // Memory visibility: what was injected at thread start and what was
    // extracted at thread end. Stored as JSON so the camelCase
    // MemoryInfo shape the frontend already consumes survives a round
    // trip. Null on rows that predate this migration — the UI tolerates
    // undefined and simply hides the chip.
    add_column_if_missing(conn, "threads", "injected_memories", "TEXT")?;
    add_column_if_missing(conn, "threads", "injected_briefing", "TEXT")?;
    add_column_if_missing(conn, "threads", "extracted_memories", "TEXT")?;
    // Phase 2: per-thread git worktrees so concurrent threads in the same
    // workspace have isolated file-system state. Null on shadow-tracked
    // (non-git) workspaces and on rows predating this migration. `branch`
    // is the `panes/<thread_id[..8]>` branch created at worktree birth.
    add_column_if_missing(conn, "threads", "worktree_path", "TEXT")?;
    add_column_if_missing(conn, "threads", "worktree_branch", "TEXT")?;

    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_costs_timestamp ON costs(timestamp);

        CREATE TABLE IF NOT EXISTS shadow_edits (
            thread_id    TEXT    NOT NULL,
            file_path    TEXT    NOT NULL,
            pre_existed  INTEGER NOT NULL,
            content_hash TEXT,
            recorded_at  TEXT    NOT NULL,
            PRIMARY KEY (thread_id, file_path)
        );

        CREATE INDEX IF NOT EXISTS idx_shadow_edits_thread ON shadow_edits(thread_id);",
    )
    .context("failed to create costs timestamp index / shadow_edits table")?;

    // mode captures permissions bits from `Metadata::permissions().mode()`.
    // macOS-only product, so unix extension semantics are safe to assume.
    // NULL on tombstone rows (no pre-existing file). Must come AFTER the
    // CREATE TABLE IF NOT EXISTS for shadow_edits above.
    add_column_if_missing(conn, "shadow_edits", "mode", "INTEGER")?;

    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS workspace_validators (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            validator_type TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            config_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_workspace_validators_workspace
            ON workspace_validators(workspace_id);
        ",
    )
    .context("failed to create workspace_validators table")?;

    Ok(())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceValidator {
    pub id: String,
    pub workspace_id: String,
    pub validator_type: String,
    pub enabled: bool,
    pub config_json: String,
    pub created_at: String,
    pub updated_at: String,
}

pub fn list_validators(
    conn: &Connection,
    workspace_id: &str,
) -> Result<Vec<WorkspaceValidator>> {
    let mut stmt = conn.prepare(
        "SELECT id, workspace_id, validator_type, enabled, config_json, created_at, updated_at \
         FROM workspace_validators WHERE workspace_id = ?1 ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(rusqlite::params![workspace_id], |row| {
        Ok(WorkspaceValidator {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            validator_type: row.get(2)?,
            enabled: row.get::<_, i64>(3)? != 0,
            config_json: row.get(4)?,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn list_enabled_validators(
    conn: &Connection,
    workspace_id: &str,
) -> Result<Vec<WorkspaceValidator>> {
    Ok(list_validators(conn, workspace_id)?
        .into_iter()
        .filter(|v| v.enabled)
        .collect())
}

pub fn insert_validator(
    conn: &Connection,
    workspace_id: &str,
    validator_type: &str,
    config_json: &str,
) -> Result<WorkspaceValidator> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO workspace_validators \
         (id, workspace_id, validator_type, enabled, config_json, created_at, updated_at) \
         VALUES (?1, ?2, ?3, 1, ?4, ?5, ?5)",
        rusqlite::params![id, workspace_id, validator_type, config_json, now],
    )?;
    Ok(WorkspaceValidator {
        id,
        workspace_id: workspace_id.to_string(),
        validator_type: validator_type.to_string(),
        enabled: true,
        config_json: config_json.to_string(),
        created_at: now.clone(),
        updated_at: now,
    })
}

pub fn update_validator(
    conn: &Connection,
    id: &str,
    enabled: Option<bool>,
    config_json: Option<&str>,
) -> Result<WorkspaceValidator> {
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(e) = enabled {
        conn.execute(
            "UPDATE workspace_validators SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![e as i64, now, id],
        )?;
    }
    if let Some(cfg) = config_json {
        conn.execute(
            "UPDATE workspace_validators SET config_json = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![cfg, now, id],
        )?;
    }
    get_validator(conn, id)
}

pub fn get_validator(conn: &Connection, id: &str) -> Result<WorkspaceValidator> {
    let v = conn.query_row(
        "SELECT id, workspace_id, validator_type, enabled, config_json, created_at, updated_at \
         FROM workspace_validators WHERE id = ?1",
        rusqlite::params![id],
        |row| {
            Ok(WorkspaceValidator {
                id: row.get(0)?,
                workspace_id: row.get(1)?,
                validator_type: row.get(2)?,
                enabled: row.get::<_, i64>(3)? != 0,
                config_json: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        },
    )?;
    Ok(v)
}

pub fn delete_validator(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM workspace_validators WHERE id = ?1",
        rusqlite::params![id],
    )?;
    Ok(())
}

pub fn create_routine_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS routines (
            id TEXT PRIMARY KEY,
            workspace_id TEXT NOT NULL REFERENCES workspaces(id),
            prompt TEXT NOT NULL,
            cron_expr TEXT NOT NULL,
            budget_cap REAL,
            on_complete TEXT NOT NULL DEFAULT '{\"action\":\"notify\"}',
            on_failure TEXT NOT NULL DEFAULT '{\"action\":\"notify\"}',
            enabled INTEGER NOT NULL DEFAULT 1,
            last_run_at TEXT,
            created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_routines_workspace ON routines(workspace_id);
        CREATE INDEX IF NOT EXISTS idx_routines_enabled ON routines(enabled);

        CREATE TABLE IF NOT EXISTS routine_executions (
            id TEXT PRIMARY KEY,
            routine_id TEXT NOT NULL REFERENCES routines(id),
            thread_id TEXT REFERENCES threads(id),
            status TEXT NOT NULL,
            cost_usd REAL DEFAULT 0,
            started_at TEXT NOT NULL,
            completed_at TEXT,
            error_message TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_rexec_routine ON routine_executions(routine_id);
        ",
    )
    .context("failed to create routine tables")?;
    Ok(())
}

pub fn routine_tables_exist(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='routines'",
        [],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyCost {
    pub day: String,
    pub total_usd: f64,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCostBreakdown {
    pub workspace_id: String,
    pub workspace_name: String,
    pub total_usd: f64,
    pub thread_count: u32,
}

pub fn query_cost_timeline(
    conn: &Connection,
    days: u32,
    workspace_id: Option<&str>,
) -> Result<Vec<DailyCost>> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(days));
    let cutoff_str = cutoff.format("%Y-%m-%d").to_string();

    let mut results = Vec::new();
    if let Some(ws_id) = workspace_id {
        let mut stmt = conn.prepare(
            "SELECT DATE(timestamp) as day, SUM(total_usd) as total_usd \
             FROM costs WHERE timestamp >= ?1 AND workspace_id = ?2 \
             GROUP BY DATE(timestamp) ORDER BY day ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![cutoff_str, ws_id], |row| {
            Ok(DailyCost {
                day: row.get(0)?,
                total_usd: row.get(1)?,
            })
        })?;
        for row in rows {
            results.push(row?);
        }
    } else {
        let mut stmt = conn.prepare(
            "SELECT DATE(timestamp) as day, SUM(total_usd) as total_usd \
             FROM costs WHERE timestamp >= ?1 \
             GROUP BY DATE(timestamp) ORDER BY day ASC",
        )?;
        let rows = stmt.query_map(rusqlite::params![cutoff_str], |row| {
            Ok(DailyCost {
                day: row.get(0)?,
                total_usd: row.get(1)?,
            })
        })?;
        for row in rows {
            results.push(row?);
        }
    }
    Ok(results)
}

pub fn query_workspace_cost_breakdown(conn: &Connection) -> Result<Vec<WorkspaceCostBreakdown>> {
    let mut stmt = conn.prepare(
        "SELECT w.id, w.name, COALESCE(SUM(t.cost_usd), 0) as total_usd, COUNT(t.id) as thread_count \
         FROM workspaces w LEFT JOIN threads t ON t.workspace_id = w.id \
         GROUP BY w.id ORDER BY total_usd DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(WorkspaceCostBreakdown {
            workspace_id: row.get(0)?,
            workspace_name: row.get(1)?,
            total_usd: row.get(2)?,
            thread_count: row.get(3)?,
        })
    })?;
    let mut results = Vec::new();
    for row in rows {
        results.push(row?);
    }
    Ok(results)
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

    #[tokio::test]
    async fn test_db_handle_execute() {
        let conn = setup_db();
        let db = DbHandle::new(conn);

        db.execute(|conn| {
            insert_workspace(conn, "ws-actor");
            Ok(())
        })
        .await
        .unwrap();

        let name: String = db
            .execute(|conn| {
                Ok(conn.query_row(
                    "SELECT name FROM workspaces WHERE id = ?1",
                    rusqlite::params!["ws-actor"],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(name, "ws-actor");
    }

    #[test]
    fn test_db_handle_try_execute_blocking() {
        let conn = setup_db();
        let db = DbHandle::new(conn);

        db.try_execute_blocking(|conn| {
            insert_workspace(conn, "ws-blocking");
            Ok(())
        })
        .unwrap();

        let name: String = db
            .try_execute_blocking(|conn| {
                Ok(conn.query_row(
                    "SELECT name FROM workspaces WHERE id = ?1",
                    rusqlite::params!["ws-blocking"],
                    |row| row.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(name, "ws-blocking");
    }

    #[tokio::test]
    async fn test_db_handle_error_propagation() {
        let conn = setup_db();
        let db = DbHandle::new(conn);

        let result = db
            .execute(|conn| {
                conn.execute("INSERT INTO nonexistent_table VALUES (1)", [])?;
                Ok(())
            })
            .await;
        assert!(result.is_err());
    }

    fn insert_cost(conn: &Connection, thread_id: &str, workspace_id: &str, total_usd: f64, timestamp: &str) {
        conn.execute(
            "INSERT INTO costs (thread_id, workspace_id, total_usd, timestamp) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![thread_id, workspace_id, total_usd, timestamp],
        ).unwrap();
    }

    #[test]
    fn test_cost_timeline_groups_by_day() {
        let conn = setup_db();
        insert_workspace(&conn, "ws1");
        insert_thread(&conn, "t1", "ws1", "completed");
        insert_thread(&conn, "t2", "ws1", "completed");

        insert_cost(&conn, "t1", "ws1", 0.05, "2026-05-01T10:00:00Z");
        insert_cost(&conn, "t2", "ws1", 0.03, "2026-05-01T14:00:00Z");
        insert_cost(&conn, "t1", "ws1", 0.10, "2026-05-02T09:00:00Z");

        let result = query_cost_timeline(&conn, 365, None).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].day, "2026-05-01");
        assert!((result[0].total_usd - 0.08).abs() < 0.001);
        assert_eq!(result[1].day, "2026-05-02");
        assert!((result[1].total_usd - 0.10).abs() < 0.001);
    }

    #[test]
    fn test_cost_timeline_respects_date_filter() {
        let conn = setup_db();
        insert_workspace(&conn, "ws1");
        insert_thread(&conn, "t1", "ws1", "completed");

        insert_cost(&conn, "t1", "ws1", 0.50, "2020-01-01T10:00:00Z");
        insert_cost(&conn, "t1", "ws1", 0.10, "2026-05-04T10:00:00Z");

        let result = query_cost_timeline(&conn, 30, None).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].day, "2026-05-04");
    }

    #[test]
    fn test_cost_timeline_filters_by_workspace() {
        let conn = setup_db();
        insert_workspace(&conn, "ws1");
        insert_workspace(&conn, "ws2");
        insert_thread(&conn, "t1", "ws1", "completed");
        insert_thread(&conn, "t2", "ws2", "completed");

        insert_cost(&conn, "t1", "ws1", 0.10, "2026-05-04T10:00:00Z");
        insert_cost(&conn, "t2", "ws2", 0.20, "2026-05-04T11:00:00Z");

        let result = query_cost_timeline(&conn, 30, Some("ws1")).unwrap();
        assert_eq!(result.len(), 1);
        assert!((result[0].total_usd - 0.10).abs() < 0.001);
    }

    #[test]
    fn test_cost_timeline_empty_db_returns_empty_vec() {
        let conn = setup_db();
        let result = query_cost_timeline(&conn, 30, None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_workspace_cost_breakdown_sums_correctly() {
        let conn = setup_db();
        insert_workspace(&conn, "ws1");
        insert_workspace(&conn, "ws2");

        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, cost_usd, created_at) VALUES ('t1', 'ws1', 'claude-code', 'completed', 'test', 0.15, '2024-01-01')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, cost_usd, created_at) VALUES ('t2', 'ws1', 'claude-code', 'completed', 'test', 0.25, '2024-01-01')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, cost_usd, created_at) VALUES ('t3', 'ws2', 'claude-code', 'completed', 'test', 0.10, '2024-01-01')",
            [],
        ).unwrap();

        let result = query_workspace_cost_breakdown(&conn).unwrap();
        assert_eq!(result.len(), 2);
        // Ordered by total_usd DESC
        assert_eq!(result[0].workspace_id, "ws1");
        assert!((result[0].total_usd - 0.40).abs() < 0.001);
        assert_eq!(result[0].thread_count, 2);
        assert_eq!(result[1].workspace_id, "ws2");
        assert!((result[1].total_usd - 0.10).abs() < 0.001);
        assert_eq!(result[1].thread_count, 1);
    }

    #[test]
    fn test_workspace_cost_breakdown_includes_zero_cost_workspaces() {
        let conn = setup_db();
        insert_workspace(&conn, "ws1");
        insert_workspace(&conn, "ws-empty");

        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, cost_usd, created_at) VALUES ('t1', 'ws1', 'claude-code', 'completed', 'test', 0.50, '2024-01-01')",
            [],
        ).unwrap();

        let result = query_workspace_cost_breakdown(&conn).unwrap();
        assert_eq!(result.len(), 2);
        let empty_ws = result.iter().find(|r| r.workspace_id == "ws-empty").unwrap();
        assert!((empty_ws.total_usd - 0.0).abs() < 0.001);
        assert_eq!(empty_ws.thread_count, 0);
    }

    #[test]
    fn test_memory_columns_added_by_migration() {
        let conn = setup_db();
        // Sanity: the three memory columns from the incremental migration
        // must exist on the threads table. pragma_table_info returns one
        // row per column.
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info('threads')")
            .unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        for expected in ["injected_memories", "injected_briefing", "extracted_memories"] {
            assert!(
                cols.contains(&expected.to_string()),
                "expected column `{expected}` on threads table, got {cols:?}"
            );
        }
    }

    #[test]
    fn test_memory_columns_migration_is_idempotent() {
        let conn = setup_db();
        // Running migrations a second time on the same connection should
        // not error (add_column_if_missing checks pragma before ALTER).
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
    }

    #[test]
    fn test_worktree_columns_added_by_migration() {
        let conn = setup_db();
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info('threads')")
            .unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        for expected in ["worktree_path", "worktree_branch"] {
            assert!(
                cols.contains(&expected.to_string()),
                "expected column `{expected}` on threads table, got {cols:?}"
            );
        }
    }

    #[test]
    fn test_workspace_validators_migration_creates_table() {
        let conn = setup_db();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='workspace_validators'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_insert_and_list_validator() {
        let conn = setup_db();
        insert_workspace(&conn, "ws-v");

        let v = insert_validator(&conn, "ws-v", "citation", r#"{"check_line_refs":true}"#).unwrap();
        assert_eq!(v.workspace_id, "ws-v");
        assert_eq!(v.validator_type, "citation");
        assert!(v.enabled);

        let list = list_validators(&conn, "ws-v").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, v.id);
    }

    #[test]
    fn test_update_validator_toggle_and_config() {
        let conn = setup_db();
        insert_workspace(&conn, "ws-v");
        let v = insert_validator(&conn, "ws-v", "citation", "{}").unwrap();

        let updated = update_validator(&conn, &v.id, Some(false), None).unwrap();
        assert!(!updated.enabled);

        let updated2 =
            update_validator(&conn, &v.id, None, Some(r#"{"x":1}"#)).unwrap();
        assert_eq!(updated2.config_json, r#"{"x":1}"#);
    }

    #[test]
    fn test_delete_validator() {
        let conn = setup_db();
        insert_workspace(&conn, "ws-v");
        let v = insert_validator(&conn, "ws-v", "citation", "{}").unwrap();
        delete_validator(&conn, &v.id).unwrap();
        assert!(list_validators(&conn, "ws-v").unwrap().is_empty());
    }

    #[test]
    fn test_list_enabled_validators_filters() {
        let conn = setup_db();
        insert_workspace(&conn, "ws-v");
        let a = insert_validator(&conn, "ws-v", "citation", "{}").unwrap();
        let _ = insert_validator(&conn, "ws-v", "secret_scan", "{}").unwrap();
        update_validator(&conn, &a.id, Some(false), None).unwrap();

        let enabled = list_enabled_validators(&conn, "ws-v").unwrap();
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].validator_type, "secret_scan");
    }

    #[test]
    fn test_validator_cascades_on_workspace_delete() {
        let conn = setup_db();
        insert_workspace(&conn, "ws-v");
        insert_validator(&conn, "ws-v", "citation", "{}").unwrap();

        conn.execute("DELETE FROM workspaces WHERE id = ?1", rusqlite::params!["ws-v"]).unwrap();
        let remaining = list_validators(&conn, "ws-v").unwrap();
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_timestamp_index_exists() {
        let conn = setup_db();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_costs_timestamp'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 1);
    }
}
