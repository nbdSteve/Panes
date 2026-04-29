use std::path::PathBuf;
use std::sync::Arc;

use panes_core::git;
use panes_core::session::{SessionManager, Workspace};
use panes_events::SessionContext;
use rusqlite::Connection;
use serde::Serialize;
use tokio::sync::Mutex;
use uuid::Uuid;

type SessionState = Arc<Mutex<SessionManager>>;
type DbState = Arc<std::sync::Mutex<Connection>>;

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
    workspace_id: String,
    workspace_path: String,
    workspace_name: String,
    prompt: String,
    agent: Option<String>,
) -> Result<String, String> {
    let expanded_path = expand_tilde(&workspace_path);
    let workspace = Workspace {
        id: workspace_id,
        path: PathBuf::from(&expanded_path),
        name: workspace_name,
        default_agent: agent.clone(),
        budget_cap: None,
    };

    let agent_name = agent.unwrap_or_else(|| "claude-code".to_string());
    let context = SessionContext {
        briefing: None,
        memories: vec![],
        budget_cap: None,
    };

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
