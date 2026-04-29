use std::path::PathBuf;
use std::sync::Arc;

use panes_core::session::{SessionManager, Workspace};
use panes_events::SessionContext;
use serde::Serialize;
use tokio::sync::Mutex;
use uuid::Uuid;

type SessionState = Arc<Mutex<SessionManager>>;

fn expand_tilde(path: &str) -> String {
    if path.starts_with("~/") || path == "~" {
        if let Ok(home) = std::env::var("HOME") {
            return path.replacen('~', &home, 1);
        }
    }
    path.to_string()
}

#[derive(Serialize)]
pub struct WorkspaceInfo {
    pub id: String,
    pub path: String,
    pub name: String,
    pub default_agent: Option<String>,
}

#[tauri::command]
pub async fn add_workspace(
    path: String,
    name: String,
) -> Result<WorkspaceInfo, String> {
    let expanded = expand_tilde(&path);
    let workspace_path = PathBuf::from(&expanded);
    if !workspace_path.exists() {
        return Err(format!("Path does not exist: {expanded}"));
    }

    let id = Uuid::new_v4().to_string();
    // TODO: persist to SQLite
    Ok(WorkspaceInfo {
        id,
        path,
        name,
        default_agent: Some("claude-code".to_string()),
    })
}

#[tauri::command]
pub async fn list_workspaces() -> Result<Vec<WorkspaceInfo>, String> {
    // TODO: read from SQLite
    Ok(vec![])
}

#[tauri::command]
pub async fn get_workspaces() -> Result<Vec<WorkspaceInfo>, String> {
    list_workspaces().await
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

    eprintln!("[panes] start_thread called: workspace={}, agent={}, prompt={}", workspace.name, agent_name, &prompt[..prompt.len().min(50)]);

    let mgr = session_manager.lock().await;
    let result = mgr.start_thread(&workspace, &prompt, &agent_name, context)
        .await
        .map_err(|e| {
            eprintln!("[panes] start_thread error: {e:#}");
            e.to_string()
        });

    eprintln!("[panes] start_thread result: {:?}", result);
    result
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

    eprintln!("[panes] resume_thread called: thread={}, workspace={}, prompt={}", thread_id, workspace.name, &prompt[..prompt.len().min(50)]);

    let mgr = session_manager.lock().await;
    mgr.resume_thread(&thread_id, &workspace, &prompt, &agent_name)
        .await
        .map_err(|e| {
            eprintln!("[panes] resume_thread error: {e:#}");
            e.to_string()
        })
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
