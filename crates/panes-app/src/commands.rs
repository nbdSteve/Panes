use std::path::PathBuf;
use std::sync::Arc;

use panes_core::git;
use panes_core::session::{SessionManager, Workspace};
use panes_events::SessionContext;
use panes_memory::sqlite_store::SqliteMemoryStore;
use panes_memory::{BriefingStore, MemoryStore};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

type SessionState = Arc<Mutex<SessionManager>>;
type DbState = Arc<std::sync::Mutex<Connection>>;
type MemoryState = Arc<SqliteMemoryStore>;

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") || path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return path.replacen('~', &home, 1);
        }
    }
    path.to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    pub id: String,
    pub path: String,
    pub name: String,
    pub default_agent: Option<String>,
}

#[tauri::command]
pub async fn add_workspace(
    db: tauri::State<'_, DbState>,
    path: String,
    name: String,
) -> Result<WorkspaceInfo, String> {
    let expanded = expand_tilde(&path);
    let workspace_path = PathBuf::from(&expanded);
    if !workspace_path.exists() {
        return Err(format!("Path does not exist: {expanded}"));
    }

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT INTO workspaces (id, path, name, default_agent, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![id, expanded, name, "claude-code", now],
    ).map_err(|e| e.to_string())?;

    Ok(WorkspaceInfo {
        id,
        path: expanded,
        name,
        default_agent: Some("claude-code".to_string()),
    })
}

#[tauri::command]
pub async fn list_workspaces(
    db: tauri::State<'_, DbState>,
) -> Result<Vec<WorkspaceInfo>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT id, path, name, default_agent FROM workspaces ORDER BY created_at")
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map([], |row| {
            Ok(WorkspaceInfo {
                id: row.get(0)?,
                path: row.get(1)?,
                name: row.get(2)?,
                default_agent: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut workspaces = vec![];
    for row in rows {
        workspaces.push(row.map_err(|e| e.to_string())?);
    }
    Ok(workspaces)
}

#[tauri::command]
pub async fn remove_workspace(
    db: tauri::State<'_, DbState>,
    workspace_id: String,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM events WHERE thread_id IN (SELECT id FROM threads WHERE workspace_id = ?1)",
        rusqlite::params![workspace_id],
    ).map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM costs WHERE workspace_id = ?1",
        rusqlite::params![workspace_id],
    ).map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM threads WHERE workspace_id = ?1",
        rusqlite::params![workspace_id],
    ).map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM workspaces WHERE id = ?1",
        rusqlite::params![workspace_id],
    ).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadInfo {
    pub id: String,
    pub workspace_id: String,
    pub prompt: String,
    pub status: String,
    pub summary: Option<String>,
    pub cost_usd: f64,
    pub duration_ms: Option<i64>,
    pub created_at: String,
    pub events: Vec<serde_json::Value>,
}

#[tauri::command]
pub async fn list_threads(
    db: tauri::State<'_, DbState>,
    workspace_id: String,
) -> Result<Vec<ThreadInfo>, String> {
    let conn = db.lock().map_err(|e| e.to_string())?;

    let mut stmt = conn.prepare(
        "SELECT id, workspace_id, prompt, status, summary, cost_usd, duration_ms, created_at
         FROM threads WHERE workspace_id = ?1 ORDER BY created_at DESC"
    ).map_err(|e| e.to_string())?;

    let threads: Vec<ThreadInfo> = stmt.query_map(rusqlite::params![workspace_id], |row| {
        Ok(ThreadInfo {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            prompt: row.get(2)?,
            status: row.get(3)?,
            summary: row.get(4)?,
            cost_usd: row.get::<_, f64>(5).unwrap_or(0.0),
            duration_ms: row.get(6)?,
            created_at: row.get(7)?,
            events: vec![],
        })
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();

    let mut result = Vec::with_capacity(threads.len());
    for mut thread in threads {
        let mut evt_stmt = conn.prepare(
            "SELECT data FROM events WHERE thread_id = ?1 ORDER BY id ASC"
        ).map_err(|e| e.to_string())?;

        let events: Vec<serde_json::Value> = evt_stmt.query_map(
            rusqlite::params![thread.id], |row| {
                let data: String = row.get(0)?;
                Ok(serde_json::from_str(&data).unwrap_or(serde_json::Value::Null))
            }
        ).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .filter(|v| !v.is_null())
        .collect();

        thread.events = events;
        result.push(thread);
    }

    Ok(result)
}

#[tauri::command]
pub async fn list_all_threads(
    db: tauri::State<'_, DbState>,
    limit: Option<u32>,
) -> Result<Vec<ThreadInfo>, String> {
    let limit = limit.unwrap_or(100);
    let conn = db.lock().map_err(|e| e.to_string())?;

    let mut stmt = conn.prepare(
        "SELECT id, workspace_id, prompt, status, summary, cost_usd, duration_ms, created_at
         FROM threads ORDER BY created_at DESC LIMIT ?1"
    ).map_err(|e| e.to_string())?;

    let threads: Vec<ThreadInfo> = stmt.query_map(rusqlite::params![limit], |row| {
        Ok(ThreadInfo {
            id: row.get(0)?,
            workspace_id: row.get(1)?,
            prompt: row.get(2)?,
            status: row.get(3)?,
            summary: row.get(4)?,
            cost_usd: row.get::<_, f64>(5).unwrap_or(0.0),
            duration_ms: row.get(6)?,
            created_at: row.get(7)?,
            events: vec![],
        })
    }).map_err(|e| e.to_string())?
    .filter_map(|r| r.ok())
    .collect();

    let mut result = Vec::with_capacity(threads.len());
    for mut thread in threads {
        let mut evt_stmt = conn.prepare(
            "SELECT data FROM events WHERE thread_id = ?1 ORDER BY id ASC"
        ).map_err(|e| e.to_string())?;

        let events: Vec<serde_json::Value> = evt_stmt.query_map(
            rusqlite::params![thread.id], |row| {
                let data: String = row.get(0)?;
                Ok(serde_json::from_str(&data).unwrap_or(serde_json::Value::Null))
            }
        ).map_err(|e| e.to_string())?
        .filter_map(|r| r.ok())
        .filter(|v| !v.is_null())
        .collect();

        thread.events = events;
        result.push(thread);
    }

    Ok(result)
}

#[tauri::command]
pub async fn delete_thread(
    db: tauri::State<'_, DbState>,
    thread_id: String,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM events WHERE thread_id = ?1",
        rusqlite::params![thread_id],
    ).map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM costs WHERE thread_id = ?1",
        rusqlite::params![thread_id],
    ).map_err(|e| e.to_string())?;
    tx.execute(
        "DELETE FROM threads WHERE id = ?1",
        rusqlite::params![thread_id],
    ).map_err(|e| e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn start_thread(
    session_manager: tauri::State<'_, SessionState>,
    memory_store: tauri::State<'_, MemoryState>,
    workspace_id: String,
    workspace_path: String,
    workspace_name: String,
    prompt: String,
    agent: Option<String>,
) -> Result<String, String> {
    let expanded_path = expand_tilde(&workspace_path);
    let workspace = Workspace {
        id: workspace_id.clone(),
        path: PathBuf::from(&expanded_path),
        name: workspace_name,
        default_agent: agent.clone(),
        budget_cap: None,
    };

    let injected = panes_memory::build_context(
        memory_store.as_ref(),
        memory_store.as_ref(),
        &prompt,
        &workspace_id,
        2000,
    )
    .await
    .unwrap_or_default();

    let context = SessionContext {
        briefing: injected.briefing,
        memories: injected.memories.iter().map(|m| m.content.clone()).collect(),
        budget_cap: None,
    };

    let agent_name = agent.unwrap_or_else(|| "claude-code".to_string());

    let mgr = session_manager.lock().await;
    mgr.start_thread(&workspace, &prompt, &agent_name, context)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resume_thread(
    session_manager: tauri::State<'_, SessionState>,
    thread_id: String,
    workspace_id: String,
    workspace_path: String,
    workspace_name: String,
    prompt: String,
    agent: Option<String>,
) -> Result<(), String> {
    let expanded_path = expand_tilde(&workspace_path);
    let workspace = Workspace {
        id: workspace_id,
        path: PathBuf::from(&expanded_path),
        name: workspace_name,
        default_agent: agent.clone(),
        budget_cap: None,
    };

    let agent_name = agent.unwrap_or_else(|| "claude-code".to_string());

    let mgr = session_manager.lock().await;
    mgr.resume_thread(&thread_id, &workspace, &prompt, &agent_name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn approve_gate(
    session_manager: tauri::State<'_, SessionState>,
    thread_id: String,
    tool_use_id: String,
) -> Result<(), String> {
    let mgr = session_manager.lock().await;
    mgr.approve(&thread_id, &tool_use_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn reject_gate(
    session_manager: tauri::State<'_, SessionState>,
    thread_id: String,
    tool_use_id: String,
    reason: String,
) -> Result<(), String> {
    let mgr = session_manager.lock().await;
    mgr.reject(&thread_id, &tool_use_id, &reason)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn cancel_thread(
    session_manager: tauri::State<'_, SessionState>,
    thread_id: String,
) -> Result<(), String> {
    let mgr = session_manager.lock().await;
    mgr.cancel(&thread_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn commit_changes(
    workspace_path: String,
    message: String,
) -> Result<String, String> {
    let expanded = expand_tilde(&workspace_path);
    let path = PathBuf::from(&expanded);
    git::commit(&path, &message)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn revert_changes(
    db: tauri::State<'_, DbState>,
    workspace_path: String,
    thread_id: String,
) -> Result<(), String> {
    let snapshot_hash: String = {
        let conn = db.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT snapshot_ref FROM threads WHERE id = ?1",
            rusqlite::params![thread_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("no snapshot for thread: {e}"))?
    };
    let expanded = expand_tilde(&workspace_path);
    let path = PathBuf::from(&expanded);
    git::revert(&path, &git::SnapshotRef { commit_hash: snapshot_hash })
        .await
        .map_err(|e| e.to_string())
}

// --- Memory extraction ---

#[tauri::command]
pub async fn extract_memories(
    memory_store: tauri::State<'_, MemoryState>,
    workspace_id: String,
    thread_id: String,
    transcript: String,
) -> Result<Vec<MemoryInfo>, String> {
    let memories = memory_store
        .add(&transcript, Some(&workspace_id), &thread_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(memories.into_iter().map(MemoryInfo::from).collect())
}

// --- Memory CRUD ---

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MemoryInfo {
    pub id: String,
    pub workspace_id: Option<String>,
    pub memory_type: String,
    pub content: String,
    pub source_thread_id: String,
    pub pinned: bool,
    pub created_at: String,
}

impl From<panes_memory::types::Memory> for MemoryInfo {
    fn from(m: panes_memory::types::Memory) -> Self {
        Self {
            id: m.id,
            workspace_id: m.workspace_id,
            memory_type: m.memory_type.to_string(),
            content: m.content,
            source_thread_id: m.source_thread_id,
            pinned: m.pinned,
            created_at: m.created_at.to_rfc3339(),
        }
    }
}

#[tauri::command]
pub async fn get_memories(
    memory_store: tauri::State<'_, MemoryState>,
    workspace_id: String,
) -> Result<Vec<MemoryInfo>, String> {
    let memories = memory_store
        .get_all(Some(&workspace_id))
        .await
        .map_err(|e| e.to_string())?;

    Ok(memories.into_iter().map(MemoryInfo::from).collect())
}

#[tauri::command]
pub async fn search_memories(
    memory_store: tauri::State<'_, MemoryState>,
    workspace_id: String,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<MemoryInfo>, String> {
    let memories = memory_store
        .search(&query, Some(&workspace_id), limit.unwrap_or(10))
        .await
        .map_err(|e| e.to_string())?;

    Ok(memories.into_iter().map(MemoryInfo::from).collect())
}

#[tauri::command]
pub async fn update_memory(
    memory_store: tauri::State<'_, MemoryState>,
    memory_id: String,
    content: String,
) -> Result<(), String> {
    memory_store
        .update(&memory_id, &content)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_memory(
    memory_store: tauri::State<'_, MemoryState>,
    memory_id: String,
) -> Result<(), String> {
    memory_store
        .delete(&memory_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pin_memory(
    memory_store: tauri::State<'_, MemoryState>,
    memory_id: String,
    pinned: bool,
) -> Result<(), String> {
    memory_store
        .pin(&memory_id, pinned)
        .await
        .map_err(|e| e.to_string())
}

// --- Briefing CRUD ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BriefingInfo {
    pub workspace_id: String,
    pub content: String,
}

#[tauri::command]
pub async fn get_briefing(
    memory_store: tauri::State<'_, MemoryState>,
    workspace_id: String,
) -> Result<Option<BriefingInfo>, String> {
    let briefing = memory_store
        .get_briefing(&workspace_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(briefing.map(|b| BriefingInfo {
        workspace_id: b.workspace_id,
        content: b.content,
    }))
}

#[tauri::command]
pub async fn set_briefing(
    memory_store: tauri::State<'_, MemoryState>,
    workspace_id: String,
    content: String,
) -> Result<(), String> {
    memory_store
        .set_briefing(&workspace_id, &content)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_briefing(
    memory_store: tauri::State<'_, MemoryState>,
    workspace_id: String,
) -> Result<(), String> {
    memory_store
        .delete_briefing(&workspace_id)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use panes_memory::types::{Memory, MemoryType};

    #[test]
    fn test_expand_tilde() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_tilde("~/projects"), format!("{home}/projects"));
        assert_eq!(expand_tilde("~"), home);
        assert_eq!(expand_tilde("/absolute/path"), "/absolute/path");
        assert_eq!(expand_tilde("relative/path"), "relative/path");
        assert_eq!(expand_tilde("~user/not-home"), "~user/not-home");
    }

    #[test]
    fn test_memory_info_from_memory() {
        let now = Utc::now();
        let memory = Memory {
            id: "mem-1".to_string(),
            workspace_id: Some("ws-1".to_string()),
            memory_type: MemoryType::Decision,
            content: "Use pnpm".to_string(),
            source_thread_id: "t-1".to_string(),
            created_at: now,
            edited_at: None,
            pinned: true,
        };

        let info = MemoryInfo::from(memory);
        assert_eq!(info.id, "mem-1");
        assert_eq!(info.workspace_id, Some("ws-1".to_string()));
        assert_eq!(info.memory_type, "decision");
        assert_eq!(info.content, "Use pnpm");
        assert_eq!(info.source_thread_id, "t-1");
        assert!(info.pinned);
    }

    #[test]
    fn test_memory_info_preserves_all_types() {
        let make = |mt: MemoryType| {
            Memory {
                id: "x".to_string(),
                workspace_id: None,
                memory_type: mt,
                content: "c".to_string(),
                source_thread_id: "t".to_string(),
                created_at: Utc::now(),
                edited_at: None,
                pinned: false,
            }
        };

        assert_eq!(MemoryInfo::from(make(MemoryType::Decision)).memory_type, "decision");
        assert_eq!(MemoryInfo::from(make(MemoryType::Preference)).memory_type, "preference");
        assert_eq!(MemoryInfo::from(make(MemoryType::Constraint)).memory_type, "constraint");
        assert_eq!(MemoryInfo::from(make(MemoryType::Pattern)).memory_type, "pattern");
    }

    #[test]
    fn test_memory_info_camel_case_serialization() {
        let info = MemoryInfo {
            id: "1".to_string(),
            workspace_id: Some("ws".to_string()),
            memory_type: "pattern".to_string(),
            content: "test".to_string(),
            source_thread_id: "t1".to_string(),
            pinned: false,
            created_at: "2024-01-01T00:00:00+00:00".to_string(),
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"workspaceId\""));
        assert!(json.contains("\"memoryType\""));
        assert!(json.contains("\"sourceThreadId\""));
        assert!(json.contains("\"createdAt\""));
        assert!(!json.contains("\"workspace_id\""));
    }

    #[test]
    fn test_briefing_info_camel_case_serialization() {
        let info = BriefingInfo {
            workspace_id: "ws".to_string(),
            content: "Always use TS".to_string(),
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"workspaceId\""));
        assert!(!json.contains("\"workspace_id\""));
    }

    fn setup_test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("
            PRAGMA foreign_keys=ON;
            CREATE TABLE workspaces (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL,
                default_agent TEXT,
                budget_cap REAL,
                created_at TEXT NOT NULL
            );
            CREATE TABLE threads (
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
            CREATE TABLE events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                thread_id TEXT NOT NULL REFERENCES threads(id),
                event_type TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                data TEXT NOT NULL
            );
            CREATE TABLE costs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                thread_id TEXT NOT NULL REFERENCES threads(id),
                workspace_id TEXT NOT NULL,
                input_tokens INTEGER DEFAULT 0,
                output_tokens INTEGER DEFAULT 0,
                total_usd REAL DEFAULT 0,
                model TEXT,
                timestamp TEXT NOT NULL
            );
        ").unwrap();
        conn
    }

    fn insert_test_workspace(conn: &rusqlite::Connection, id: &str, path: &str) {
        conn.execute(
            "INSERT INTO workspaces (id, path, name, created_at) VALUES (?1, ?2, ?3, '2024-01-01')",
            rusqlite::params![id, path, format!("ws-{id}")],
        ).unwrap();
    }

    fn insert_test_thread(conn: &rusqlite::Connection, id: &str, ws_id: &str, snapshot: Option<&str>) {
        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, created_at, snapshot_ref)
             VALUES (?1, ?2, 'claude', 'completed', 'test', '2024-01-01', ?3)",
            rusqlite::params![id, ws_id, snapshot],
        ).unwrap();
    }

    #[test]
    fn test_revert_query_nonexistent_thread() {
        let conn = setup_test_db();
        let result: Result<String, _> = conn.query_row(
            "SELECT snapshot_ref FROM threads WHERE id = ?1",
            rusqlite::params!["nonexistent"],
            |row| row.get(0),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_revert_query_null_snapshot() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/test");
        insert_test_thread(&conn, "t1", "ws1", None);

        let result: Result<String, _> = conn.query_row(
            "SELECT snapshot_ref FROM threads WHERE id = ?1",
            rusqlite::params!["t1"],
            |row| row.get(0),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_revert_query_valid_snapshot() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/test");
        insert_test_thread(&conn, "t1", "ws1", Some("abc123"));

        let hash: String = conn.query_row(
            "SELECT snapshot_ref FROM threads WHERE id = ?1",
            rusqlite::params!["t1"],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(hash, "abc123");
    }

    #[test]
    fn test_delete_nonexistent_ids_succeeds() {
        let conn = setup_test_db();
        let tx = conn.unchecked_transaction().unwrap();
        tx.execute("DELETE FROM events WHERE thread_id = ?1", rusqlite::params!["nope"]).unwrap();
        tx.execute("DELETE FROM costs WHERE thread_id = ?1", rusqlite::params!["nope"]).unwrap();
        tx.execute("DELETE FROM threads WHERE id = ?1", rusqlite::params!["nope"]).unwrap();
        tx.commit().unwrap();
    }

    #[test]
    fn test_list_all_threads_multi_workspace() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/ws1");
        insert_test_workspace(&conn, "ws2", "/tmp/ws2");
        insert_test_thread(&conn, "t1", "ws1", None);
        insert_test_thread(&conn, "t2", "ws2", None);

        let mut stmt = conn.prepare(
            "SELECT id FROM threads ORDER BY created_at DESC"
        ).unwrap();
        let ids: Vec<String> = stmt.query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"t1".to_string()));
        assert!(ids.contains(&"t2".to_string()));
    }

    #[test]
    fn test_list_all_threads_ordering() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/ws1");
        // Insert with different timestamps
        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, created_at)
             VALUES ('t_old', 'ws1', 'claude', 'completed', 'old', '2024-01-01T00:00:00Z')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, created_at)
             VALUES ('t_new', 'ws1', 'claude', 'completed', 'new', '2024-06-01T00:00:00Z')",
            [],
        ).unwrap();

        let mut stmt = conn.prepare(
            "SELECT id FROM threads ORDER BY created_at DESC LIMIT 100"
        ).unwrap();
        let ids: Vec<String> = stmt.query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(ids[0], "t_new");
        assert_eq!(ids[1], "t_old");
    }

    #[test]
    fn test_list_all_threads_empty() {
        let conn = setup_test_db();
        let mut stmt = conn.prepare(
            "SELECT id FROM threads ORDER BY created_at DESC LIMIT 100"
        ).unwrap();
        let ids: Vec<String> = stmt.query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert!(ids.is_empty());
    }

    #[test]
    fn test_add_workspace_duplicate_path() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/project");
        let result = conn.execute(
            "INSERT INTO workspaces (id, path, name, created_at) VALUES (?1, ?2, ?3, '2024-01-01')",
            rusqlite::params!["ws2", "/tmp/project", "duplicate"],
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("UNIQUE"), "should be UNIQUE constraint error: {err}");
    }
}
