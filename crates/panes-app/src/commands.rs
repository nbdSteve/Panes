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
pub async fn get_workspaces(
    db: tauri::State<'_, DbState>,
) -> Result<Vec<WorkspaceInfo>, String> {
    list_workspaces(db).await
}

#[tauri::command]
pub async fn remove_workspace(
    db: tauri::State<'_, DbState>,
    workspace_id: String,
) -> Result<(), String> {
    let conn = db.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM workspaces WHERE id = ?1",
        rusqlite::params![workspace_id],
    ).map_err(|e| e.to_string())?;
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
    workspace_path: String,
) -> Result<(), String> {
    let expanded = expand_tilde(&workspace_path);
    let path = PathBuf::from(&expanded);
    let dummy_snapshot = git::SnapshotRef {
        commit_hash: String::new(),
        had_dirty_changes: false,
        stash_ref: None,
    };
    git::revert(&path, &dummy_snapshot)
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
