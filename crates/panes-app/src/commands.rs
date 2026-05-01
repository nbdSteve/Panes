use std::path::PathBuf;
use std::sync::Arc;

use panes_core::db::DbHandle;
use panes_core::error::PanesError;
use panes_core::git;
use panes_core::session::{SessionManager, Workspace};
use panes_events::SessionContext;
use panes_memory::manager::MemoryManager;
use panes_memory::{BriefingStore, MemoryStore};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

type SessionState = Arc<Mutex<SessionManager>>;
pub(crate) type DbState = DbHandle;
pub(crate) type MemoryManagerState = Arc<MemoryManager>;

fn resolve_agent_name(agent: Option<String>) -> String {
    agent.filter(|s| !s.is_empty()).unwrap_or_else(|| "claude-code".to_string())
}

pub(crate) fn expand_tilde(path: &str) -> String {
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
    pub budget_cap: Option<f64>,
}

#[tauri::command]
pub async fn add_workspace(
    db: tauri::State<'_, DbState>,
    path: String,
    name: String,
) -> Result<WorkspaceInfo, PanesError> {
    let expanded = expand_tilde(&path);
    let workspace_path = PathBuf::from(&expanded);
    if !workspace_path.exists() {
        return Err(PanesError::ValidationError {
            message: format!("Path does not exist: {expanded}"),
        });
    }

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let id2 = id.clone();
    let expanded2 = expanded.clone();
    let name2 = name.clone();
    db.execute(move |conn| {
        conn.execute(
            "INSERT INTO workspaces (id, path, name, default_agent, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![id2, expanded2, name2, "claude-code", now],
        )?;
        Ok(())
    }).await.map_err(PanesError::from)?;

    Ok(WorkspaceInfo {
        id,
        path: expanded,
        name,
        default_agent: Some("claude-code".to_string()),
        budget_cap: None,
    })
}

#[tauri::command]
pub async fn list_workspaces(
    db: tauri::State<'_, DbState>,
) -> Result<Vec<WorkspaceInfo>, PanesError> {
    db.execute(|conn| {
        let mut stmt = conn
            .prepare("SELECT id, path, name, default_agent, budget_cap FROM workspaces ORDER BY created_at")?;
        let mut workspaces = vec![];
        let rows = stmt.query_map([], |row| {
            Ok(WorkspaceInfo {
                id: row.get(0)?,
                path: row.get(1)?,
                name: row.get(2)?,
                default_agent: row.get(3)?,
                budget_cap: row.get(4)?,
            })
        })?;
        for row in rows {
            workspaces.push(row?);
        }
        Ok(workspaces)
    }).await.map_err(PanesError::from)
}

#[tauri::command]
pub async fn remove_workspace(
    db: tauri::State<'_, DbState>,
    workspace_id: String,
) -> Result<(), PanesError> {
    db.execute(move |conn| {
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM events WHERE thread_id IN (SELECT id FROM threads WHERE workspace_id = ?1)",
            rusqlite::params![workspace_id],
        )?;
        tx.execute(
            "DELETE FROM costs WHERE workspace_id = ?1",
            rusqlite::params![workspace_id],
        )?;
        tx.execute(
            "DELETE FROM threads WHERE workspace_id = ?1",
            rusqlite::params![workspace_id],
        )?;
        tx.execute(
            "DELETE FROM workspaces WHERE id = ?1",
            rusqlite::params![workspace_id],
        )?;
        tx.commit()?;
        Ok(())
    }).await.map_err(PanesError::from)
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
) -> Result<Vec<ThreadInfo>, PanesError> {
    db.execute(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, prompt, status, summary, cost_usd, duration_ms, created_at
             FROM threads WHERE workspace_id = ?1 ORDER BY created_at DESC"
        )?;

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
        })?
        .filter_map(|r| r.ok())
        .collect();

        let mut result = Vec::with_capacity(threads.len());
        for mut thread in threads {
            let mut evt_stmt = conn.prepare(
                "SELECT data FROM events WHERE thread_id = ?1 ORDER BY id ASC"
            )?;

            let events: Vec<serde_json::Value> = evt_stmt.query_map(
                rusqlite::params![thread.id], |row| {
                    let data: String = row.get(0)?;
                    Ok(serde_json::from_str(&data).unwrap_or(serde_json::Value::Null))
                }
            )?
            .filter_map(|r| r.ok())
            .filter(|v| !v.is_null())
            .collect();

            thread.events = events;
            result.push(thread);
        }

        Ok(result)
    }).await.map_err(PanesError::from)
}

#[tauri::command]
pub async fn list_all_threads(
    db: tauri::State<'_, DbState>,
    limit: Option<u32>,
) -> Result<Vec<ThreadInfo>, PanesError> {
    let limit = limit.unwrap_or(100);
    db.execute(move |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, prompt, status, summary, cost_usd, duration_ms, created_at
             FROM threads ORDER BY created_at DESC LIMIT ?1"
        )?;

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
        })?
        .filter_map(|r| r.ok())
        .collect();

        let mut result = Vec::with_capacity(threads.len());
        for mut thread in threads {
            let mut evt_stmt = conn.prepare(
                "SELECT data FROM events WHERE thread_id = ?1 ORDER BY id ASC"
            )?;

            let events: Vec<serde_json::Value> = evt_stmt.query_map(
                rusqlite::params![thread.id], |row| {
                    let data: String = row.get(0)?;
                    Ok(serde_json::from_str(&data).unwrap_or(serde_json::Value::Null))
                }
            )?
            .filter_map(|r| r.ok())
            .filter(|v| !v.is_null())
            .collect();

            thread.events = events;
            result.push(thread);
        }

        Ok(result)
    }).await.map_err(PanesError::from)
}

#[tauri::command]
pub async fn delete_thread(
    db: tauri::State<'_, DbState>,
    thread_id: String,
) -> Result<(), PanesError> {
    db.execute(move |conn| {
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM events WHERE thread_id = ?1",
            rusqlite::params![thread_id],
        )?;
        tx.execute(
            "DELETE FROM costs WHERE thread_id = ?1",
            rusqlite::params![thread_id],
        )?;
        tx.execute(
            "DELETE FROM threads WHERE id = ?1",
            rusqlite::params![thread_id],
        )?;
        tx.commit()?;
        Ok(())
    }).await.map_err(PanesError::from)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartThreadResult {
    pub thread_id: String,
    pub memory_count: usize,
    pub has_briefing: bool,
}

#[tauri::command]
pub async fn start_thread(
    session_manager: tauri::State<'_, SessionState>,
    memory_manager: tauri::State<'_, MemoryManagerState>,
    db: tauri::State<'_, DbState>,
    workspace_id: String,
    workspace_path: String,
    workspace_name: String,
    prompt: String,
    agent: Option<String>,
    model: Option<String>,
) -> Result<StartThreadResult, PanesError> {
    let expanded_path = expand_tilde(&workspace_path);
    let ws_id = workspace_id.clone();
    let budget_cap: Option<f64> = db.execute(move |conn| {
        Ok(conn.query_row(
            "SELECT budget_cap FROM workspaces WHERE id = ?1",
            rusqlite::params![ws_id],
            |row| row.get(0),
        ).unwrap_or(None))
    }).await.map_err(PanesError::from)?;
    let workspace = Workspace {
        id: workspace_id.clone(),
        path: PathBuf::from(&expanded_path),
        name: workspace_name,
        default_agent: agent.clone(),
        budget_cap,
    };

    let injected = panes_memory::build_context(
        memory_manager.as_memory_store(),
        memory_manager.as_briefing_store(),
        &prompt,
        &workspace_id,
        2000,
    )
    .await
    .unwrap_or_default();

    let memory_count = injected.memories.len();
    let has_briefing = injected.briefing.is_some();

    let context = SessionContext {
        briefing: injected.briefing,
        memories: injected.memories.iter().map(|m| m.content.clone()).collect(),
        budget_cap: None,
    };

    let agent_name = resolve_agent_name(agent);

    let mgr = session_manager.lock().await;
    let thread_id = mgr.start_thread(&workspace, &prompt, &agent_name, context, model.as_deref())
        .await?;

    Ok(StartThreadResult {
        thread_id,
        memory_count,
        has_briefing,
    })
}

#[tauri::command]
pub async fn resume_thread(
    session_manager: tauri::State<'_, SessionState>,
    db: tauri::State<'_, DbState>,
    thread_id: String,
    workspace_id: String,
    workspace_path: String,
    workspace_name: String,
    prompt: String,
    agent: Option<String>,
    model: Option<String>,
) -> Result<(), PanesError> {
    let expanded_path = expand_tilde(&workspace_path);
    let ws_id = workspace_id.clone();
    let budget_cap: Option<f64> = db.execute(move |conn| {
        Ok(conn.query_row(
            "SELECT budget_cap FROM workspaces WHERE id = ?1",
            rusqlite::params![ws_id],
            |row| row.get(0),
        ).unwrap_or(None))
    }).await.map_err(PanesError::from)?;
    let workspace = Workspace {
        id: workspace_id,
        path: PathBuf::from(&expanded_path),
        name: workspace_name,
        default_agent: agent.clone(),
        budget_cap,
    };

    let agent_name = resolve_agent_name(agent);

    let mgr = session_manager.lock().await;
    mgr.resume_thread(&thread_id, &workspace, &prompt, &agent_name, model.as_deref())
        .await
}

#[tauri::command]
pub async fn approve_gate(
    session_manager: tauri::State<'_, SessionState>,
    thread_id: String,
    tool_use_id: String,
) -> Result<(), PanesError> {
    let mgr = session_manager.lock().await;
    mgr.approve(&thread_id, &tool_use_id)
        .await
}

#[tauri::command]
pub async fn reject_gate(
    session_manager: tauri::State<'_, SessionState>,
    thread_id: String,
    tool_use_id: String,
    reason: String,
) -> Result<(), PanesError> {
    let mgr = session_manager.lock().await;
    mgr.reject(&thread_id, &tool_use_id, &reason)
        .await
}

#[tauri::command]
pub async fn cancel_thread(
    session_manager: tauri::State<'_, SessionState>,
    thread_id: String,
) -> Result<(), PanesError> {
    let mgr = session_manager.lock().await;
    mgr.cancel(&thread_id)
        .await
}

#[tauri::command]
pub async fn commit_changes(
    workspace_path: String,
    message: String,
) -> Result<String, PanesError> {
    let expanded = expand_tilde(&workspace_path);
    let path = PathBuf::from(&expanded);
    git::commit(&path, &message)
        .await
        .map_err(PanesError::from)
}

#[tauri::command]
pub async fn revert_changes(
    db: tauri::State<'_, DbState>,
    workspace_path: String,
    thread_id: String,
) -> Result<(), PanesError> {
    let tid = thread_id.clone();
    let snapshot_hash: String = db.execute(move |conn| {
        Ok(conn.query_row(
            "SELECT snapshot_ref FROM threads WHERE id = ?1",
            rusqlite::params![tid],
            |row| row.get(0),
        )?)
    }).await.map_err(|e| PanesError::GitError {
        message: format!("no snapshot for thread: {e}"),
    })?;
    let expanded = expand_tilde(&workspace_path);
    let path = PathBuf::from(&expanded);
    git::revert(&path, &git::SnapshotRef { commit_hash: snapshot_hash })
        .await
        .map_err(PanesError::from)
}

#[tauri::command]
pub async fn get_changed_files(
    workspace_path: String,
) -> Result<Vec<String>, PanesError> {
    let expanded = expand_tilde(&workspace_path);
    let path = PathBuf::from(&expanded);
    git::get_changed_files(&path)
        .await
        .map_err(PanesError::from)
}

// --- Memory extraction ---

#[tauri::command]
pub async fn extract_memories(
    memory_manager: tauri::State<'_, MemoryManagerState>,
    workspace_id: String,
    thread_id: String,
    transcript: String,
) -> Result<Vec<MemoryInfo>, PanesError> {
    let memories = memory_manager
        .add(&transcript, Some(&workspace_id), &thread_id)
        .await
        .map_err(PanesError::from)?;

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
    memory_manager: tauri::State<'_, MemoryManagerState>,
    workspace_id: String,
) -> Result<Vec<MemoryInfo>, PanesError> {
    let memories = memory_manager
        .get_all(Some(&workspace_id))
        .await
        .map_err(PanesError::from)?;

    Ok(memories.into_iter().map(MemoryInfo::from).collect())
}

#[tauri::command]
pub async fn search_memories(
    memory_manager: tauri::State<'_, MemoryManagerState>,
    workspace_id: String,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<MemoryInfo>, PanesError> {
    let memories = memory_manager
        .search(&query, Some(&workspace_id), limit.unwrap_or(10))
        .await
        .map_err(PanesError::from)?;

    Ok(memories.into_iter().map(MemoryInfo::from).collect())
}

#[tauri::command]
pub async fn update_memory(
    memory_manager: tauri::State<'_, MemoryManagerState>,
    memory_id: String,
    content: String,
) -> Result<(), PanesError> {
    memory_manager
        .update(&memory_id, &content)
        .await
        .map_err(PanesError::from)
}

#[tauri::command]
pub async fn delete_memory(
    memory_manager: tauri::State<'_, MemoryManagerState>,
    memory_id: String,
) -> Result<(), PanesError> {
    memory_manager
        .delete(&memory_id)
        .await
        .map_err(PanesError::from)
}

#[tauri::command]
pub async fn pin_memory(
    memory_manager: tauri::State<'_, MemoryManagerState>,
    memory_id: String,
    pinned: bool,
) -> Result<(), PanesError> {
    memory_manager
        .pin(&memory_id, pinned)
        .await
        .map_err(PanesError::from)
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
    memory_manager: tauri::State<'_, MemoryManagerState>,
    workspace_id: String,
) -> Result<Option<BriefingInfo>, PanesError> {
    let briefing = memory_manager
        .get_briefing(&workspace_id)
        .await
        .map_err(PanesError::from)?;

    Ok(briefing.map(|b| BriefingInfo {
        workspace_id: b.workspace_id,
        content: b.content,
    }))
}

#[tauri::command]
pub async fn set_briefing(
    memory_manager: tauri::State<'_, MemoryManagerState>,
    workspace_id: String,
    content: String,
) -> Result<(), PanesError> {
    memory_manager
        .set_briefing(&workspace_id, &content)
        .await
        .map_err(PanesError::from)
}

#[tauri::command]
pub async fn delete_briefing(
    memory_manager: tauri::State<'_, MemoryManagerState>,
    workspace_id: String,
) -> Result<(), PanesError> {
    memory_manager
        .delete_briefing(&workspace_id)
        .await
        .map_err(PanesError::from)
}

#[tauri::command]
pub async fn get_aggregate_cost(
    db: tauri::State<'_, DbState>,
) -> Result<f64, PanesError> {
    db.execute(|conn| {
        Ok(conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM threads",
            [],
            |row| row.get(0),
        )?)
    }).await.map_err(PanesError::from)
}

#[tauri::command]
pub async fn get_workspace_cost(
    db: tauri::State<'_, DbState>,
    workspace_id: String,
) -> Result<f64, PanesError> {
    db.execute(move |conn| {
        Ok(conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM threads WHERE workspace_id = ?1",
            rusqlite::params![workspace_id],
            |row| row.get(0),
        )?)
    }).await.map_err(PanesError::from)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBackendStatus {
    pub backend: String,
    pub mem0_configured: bool,
}

#[tauri::command]
pub async fn get_memory_backend_status(
    memory: tauri::State<'_, MemoryManagerState>,
) -> Result<MemoryBackendStatus, PanesError> {
    Ok(MemoryBackendStatus {
        backend: memory.get_active_backend().to_string(),
        mem0_configured: memory.is_mem0_configured(),
    })
}

#[tauri::command]
pub async fn set_memory_backend(
    memory: tauri::State<'_, MemoryManagerState>,
    backend: String,
) -> Result<(), PanesError> {
    memory.set_active_backend(&backend).map_err(PanesError::from)
}

#[tauri::command]
pub async fn list_adapters(
    session_manager: tauri::State<'_, SessionState>,
) -> Result<Vec<String>, PanesError> {
    let mgr = session_manager.lock().await;
    Ok(mgr.list_adapters())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub name: String,
    pub model: Option<String>,
    pub description: Option<String>,
}

#[tauri::command]
pub async fn list_models(
    session_manager: tauri::State<'_, SessionState>,
    adapter: String,
) -> Result<Vec<panes_adapters::ModelInfo>, PanesError> {
    let mgr = session_manager.lock().await;
    mgr.list_models(&adapter)
        .await
}

#[tauri::command]
pub fn list_agents(adapter: String) -> Result<Vec<AgentInfo>, PanesError> {
    match adapter.as_str() {
        "claude-code" => list_agents_claude(),
        _ => Ok(vec![]),
    }
}

fn list_agents_claude() -> Result<Vec<AgentInfo>, PanesError> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let agents_dir = PathBuf::from(&home).join(".claude").join("agents");
    if !agents_dir.is_dir() {
        return Ok(vec![]);
    }
    let mut agents = Vec::new();
    let entries = std::fs::read_dir(&agents_dir).map_err(|e| PanesError::Internal { message: e.to_string() })?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        if let Some(info) = parse_agent_frontmatter(&content) {
            agents.push(info);
        }
    }
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(agents)
}

fn parse_agent_frontmatter(content: &str) -> Option<AgentInfo> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    let after_start = &trimmed[3..];
    let end = after_start.find("\n---")?;
    let frontmatter = &after_start[..end];

    let mut name = None;
    let mut model = None;
    let mut description = None;

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("name:") {
            name = Some(val.trim().trim_matches('"').to_string());
        } else if let Some(val) = line.strip_prefix("model:") {
            let m = val.trim().trim_matches('"').to_string();
            if !m.is_empty() {
                model = Some(m);
            }
        } else if let Some(val) = line.strip_prefix("description:") {
            let d = val.trim().trim_matches('"');
            let short = if d.len() > 100 {
                format!("{}...", &d[..100])
            } else {
                d.to_string()
            };
            description = Some(short);
        }
    }

    Some(AgentInfo {
        name: name?,
        model,
        description,
    })
}

#[tauri::command]
pub async fn set_workspace_default_agent(
    db: tauri::State<'_, DbState>,
    workspace_id: String,
    agent: String,
) -> Result<(), PanesError> {
    db.execute(move |conn| {
        conn.execute(
            "UPDATE workspaces SET default_agent = ?1 WHERE id = ?2",
            rusqlite::params![agent, workspace_id],
        )?;
        Ok(())
    }).await.map_err(PanesError::from)
}

#[tauri::command]
pub async fn set_workspace_budget_cap(
    db: tauri::State<'_, DbState>,
    workspace_id: String,
    budget_cap: Option<f64>,
) -> Result<(), PanesError> {
    db.execute(move |conn| {
        conn.execute(
            "UPDATE workspaces SET budget_cap = ?1 WHERE id = ?2",
            rusqlite::params![budget_cap, workspace_id],
        )?;
        Ok(())
    }).await.map_err(PanesError::from)
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

    #[test]
    fn test_get_aggregate_cost_empty() {
        let conn = setup_test_db();
        let total: f64 = conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM threads",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(total, 0.0);
    }

    #[test]
    fn test_get_aggregate_cost_sums() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/ws1");
        insert_test_thread(&conn, "t1", "ws1", None);
        insert_test_thread(&conn, "t2", "ws1", None);
        conn.execute(
            "UPDATE threads SET cost_usd = 0.05 WHERE id = 't1'",
            [],
        ).unwrap();
        conn.execute(
            "UPDATE threads SET cost_usd = 0.03 WHERE id = 't2'",
            [],
        ).unwrap();
        let total: f64 = conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM threads",
            [],
            |row| row.get(0),
        ).unwrap();
        assert!((total - 0.08).abs() < 1e-10);
    }

    #[test]
    fn test_set_workspace_default_agent() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/ws1");
        conn.execute(
            "UPDATE workspaces SET default_agent = ?1 WHERE id = ?2",
            rusqlite::params!["new-agent", "ws1"],
        ).unwrap();
        let agent: Option<String> = conn.query_row(
            "SELECT default_agent FROM workspaces WHERE id = ?1",
            rusqlite::params!["ws1"],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(agent.unwrap(), "new-agent");
    }

    #[test]
    fn test_parse_agent_frontmatter_full() {
        let content = r#"---
name: my-agent
model: opus
description: "Does cool things"
---
Body text here
"#;
        let info = parse_agent_frontmatter(content).unwrap();
        assert_eq!(info.name, "my-agent");
        assert_eq!(info.model.as_deref(), Some("opus"));
        assert_eq!(info.description.as_deref(), Some("Does cool things"));
    }

    #[test]
    fn test_parse_agent_frontmatter_no_model() {
        let content = "---\nname: basic-agent\ndescription: Simple\n---\nBody";
        let info = parse_agent_frontmatter(content).unwrap();
        assert_eq!(info.name, "basic-agent");
        assert!(info.model.is_none());
    }

    #[test]
    fn test_parse_agent_frontmatter_full_model_id() {
        let content = "---\nname: opus-agent\nmodel: claude-opus-4.6\n---\n";
        let info = parse_agent_frontmatter(content).unwrap();
        assert_eq!(info.model.as_deref(), Some("claude-opus-4.6"));
    }

    #[test]
    fn test_parse_agent_frontmatter_quoted_model() {
        let content = "---\nname: quoted\nmodel: \"sonnet\"\n---\n";
        let info = parse_agent_frontmatter(content).unwrap();
        assert_eq!(info.model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn test_parse_agent_frontmatter_missing_name() {
        let content = "---\nmodel: opus\ndescription: No name\n---\n";
        assert!(parse_agent_frontmatter(content).is_none());
    }

    #[test]
    fn test_parse_agent_frontmatter_no_frontmatter() {
        assert!(parse_agent_frontmatter("Just plain text").is_none());
    }

    #[test]
    fn test_parse_agent_frontmatter_no_closing_fence() {
        assert!(parse_agent_frontmatter("---\nname: broken\n").is_none());
    }

    #[test]
    fn test_parse_agent_frontmatter_empty_model() {
        let content = "---\nname: empty-model\nmodel: \n---\n";
        let info = parse_agent_frontmatter(content).unwrap();
        assert!(info.model.is_none());
    }

    #[test]
    fn test_parse_agent_frontmatter_long_description_truncated() {
        let long_desc = "x".repeat(200);
        let content = format!("---\nname: verbose\ndescription: {long_desc}\n---\n");
        let info = parse_agent_frontmatter(&content).unwrap();
        assert_eq!(info.description.as_ref().unwrap().len(), 103); // 100 + "..."
        assert!(info.description.unwrap().ends_with("..."));
    }

    #[test]
    fn test_list_agents_unknown_adapter() {
        let result = list_agents_claude();
        // Just verify it doesn't panic — the actual agent list depends on filesystem
        assert!(result.is_ok());
    }

    #[test]
    fn test_agent_info_camel_case_serialization() {
        let info = AgentInfo {
            name: "test".to_string(),
            model: Some("opus".to_string()),
            description: Some("desc".to_string()),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\""));
        assert!(json.contains("\"model\""));
        assert!(json.contains("\"description\""));
    }

    #[test]
    fn test_list_agents_unknown_adapter_returns_empty() {
        let result = list_agents("unknown-adapter".to_string());
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_workspace_info_camel_case_serialization() {
        let ws = WorkspaceInfo {
            id: "ws1".to_string(),
            path: "/tmp".to_string(),
            name: "test".to_string(),
            default_agent: Some("claude-code".to_string()),
            budget_cap: Some(5.0),
        };
        let json = serde_json::to_string(&ws).unwrap();
        assert!(json.contains("\"defaultAgent\""));
        assert!(!json.contains("\"default_agent\""));
        assert!(json.contains("\"budgetCap\""));
        assert!(!json.contains("\"budget_cap\""));
    }

    #[test]
    fn test_thread_info_camel_case_serialization() {
        let ti = ThreadInfo {
            id: "t1".to_string(),
            workspace_id: "ws1".to_string(),
            prompt: "hello".to_string(),
            status: "running".to_string(),
            summary: None,
            cost_usd: 0.05,
            duration_ms: Some(1000),
            created_at: "2024-01-01".to_string(),
            events: vec![],
        };
        let json = serde_json::to_string(&ti).unwrap();
        assert!(json.contains("\"workspaceId\""));
        assert!(json.contains("\"costUsd\""));
        assert!(json.contains("\"durationMs\""));
        assert!(json.contains("\"createdAt\""));
    }

    #[test]
    fn test_add_and_list_workspaces() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");
        insert_test_workspace(&conn, "ws2", "/tmp/b");

        let mut stmt = conn.prepare(
            "SELECT id, path, name, default_agent FROM workspaces ORDER BY created_at"
        ).unwrap();
        let rows: Vec<(String, String, String)> = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        }).unwrap().filter_map(|r| r.ok()).collect();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "ws1");
        assert_eq!(rows[1].0, "ws2");
    }

    #[test]
    fn test_remove_workspace_cascades() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");
        insert_test_thread(&conn, "t1", "ws1", None);
        conn.execute(
            "INSERT INTO events (thread_id, event_type, timestamp, data) VALUES ('t1', 'text', '2024-01-01', '{}')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO costs (thread_id, workspace_id, total_usd, timestamp) VALUES ('t1', 'ws1', 0.01, '2024-01-01')",
            [],
        ).unwrap();

        let tx = conn.unchecked_transaction().unwrap();
        tx.execute("DELETE FROM events WHERE thread_id IN (SELECT id FROM threads WHERE workspace_id = ?1)", rusqlite::params!["ws1"]).unwrap();
        tx.execute("DELETE FROM costs WHERE workspace_id = ?1", rusqlite::params!["ws1"]).unwrap();
        tx.execute("DELETE FROM threads WHERE workspace_id = ?1", rusqlite::params!["ws1"]).unwrap();
        tx.execute("DELETE FROM workspaces WHERE id = ?1", rusqlite::params!["ws1"]).unwrap();
        tx.commit().unwrap();

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM workspaces", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM threads", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM costs", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_list_threads_for_workspace() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");
        insert_test_workspace(&conn, "ws2", "/tmp/b");
        insert_test_thread(&conn, "t1", "ws1", None);
        insert_test_thread(&conn, "t2", "ws1", None);
        insert_test_thread(&conn, "t3", "ws2", None);

        let mut stmt = conn.prepare(
            "SELECT id FROM threads WHERE workspace_id = ?1 ORDER BY created_at DESC"
        ).unwrap();
        let ids: Vec<String> = stmt.query_map(rusqlite::params!["ws1"], |row| row.get(0))
            .unwrap().filter_map(|r| r.ok()).collect();
        assert_eq!(ids.len(), 2);
        assert!(!ids.contains(&"t3".to_string()));
    }

    #[test]
    fn test_list_threads_empty_workspace() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");

        let mut stmt = conn.prepare(
            "SELECT id FROM threads WHERE workspace_id = ?1"
        ).unwrap();
        let ids: Vec<String> = stmt.query_map(rusqlite::params!["ws1"], |row| row.get(0))
            .unwrap().filter_map(|r| r.ok()).collect();
        assert!(ids.is_empty());
    }

    #[test]
    fn test_list_threads_includes_events() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");
        insert_test_thread(&conn, "t1", "ws1", None);
        conn.execute(
            "INSERT INTO events (thread_id, event_type, timestamp, data) VALUES ('t1', 'text', '2024-01-01', '{\"event_type\":\"text\",\"text\":\"hello\"}')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO events (thread_id, event_type, timestamp, data) VALUES ('t1', 'complete', '2024-01-01', '{\"event_type\":\"complete\",\"summary\":\"done\"}')",
            [],
        ).unwrap();

        let mut evt_stmt = conn.prepare("SELECT data FROM events WHERE thread_id = ?1 ORDER BY id ASC").unwrap();
        let events: Vec<serde_json::Value> = evt_stmt.query_map(rusqlite::params!["t1"], |row| {
            let data: String = row.get(0)?;
            Ok(serde_json::from_str(&data).unwrap_or(serde_json::Value::Null))
        }).unwrap().filter_map(|r| r.ok()).collect();

        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["event_type"], "text");
        assert_eq!(events[1]["event_type"], "complete");
    }

    #[test]
    fn test_delete_thread_removes_all_related() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");
        insert_test_thread(&conn, "t1", "ws1", None);
        conn.execute(
            "INSERT INTO events (thread_id, event_type, timestamp, data) VALUES ('t1', 'text', '2024-01-01', '{}')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO costs (thread_id, workspace_id, total_usd, timestamp) VALUES ('t1', 'ws1', 0.01, '2024-01-01')",
            [],
        ).unwrap();

        let tx = conn.unchecked_transaction().unwrap();
        tx.execute("DELETE FROM events WHERE thread_id = ?1", rusqlite::params!["t1"]).unwrap();
        tx.execute("DELETE FROM costs WHERE thread_id = ?1", rusqlite::params!["t1"]).unwrap();
        tx.execute("DELETE FROM threads WHERE id = ?1", rusqlite::params!["t1"]).unwrap();
        tx.commit().unwrap();

        let count: i64 = conn.query_row("SELECT COUNT(*) FROM threads", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_set_and_read_budget_cap() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");

        conn.execute(
            "UPDATE workspaces SET budget_cap = ?1 WHERE id = ?2",
            rusqlite::params![5.0, "ws1"],
        ).unwrap();

        let cap: Option<f64> = conn.query_row(
            "SELECT budget_cap FROM workspaces WHERE id = ?1",
            rusqlite::params!["ws1"],
            |row| row.get(0),
        ).unwrap();
        assert!((cap.unwrap() - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_budget_cap_null_by_default() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");

        let cap: Option<f64> = conn.query_row(
            "SELECT budget_cap FROM workspaces WHERE id = ?1",
            rusqlite::params!["ws1"],
            |row| row.get(0),
        ).unwrap();
        assert!(cap.is_none());
    }

    #[test]
    fn test_set_budget_cap_round_trip() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");

        // Set a cap
        conn.execute(
            "UPDATE workspaces SET budget_cap = ?1 WHERE id = ?2",
            rusqlite::params![Some(2.50), "ws1"],
        ).unwrap();

        let cap: Option<f64> = conn.query_row(
            "SELECT budget_cap FROM workspaces WHERE id = ?1",
            rusqlite::params!["ws1"],
            |row| row.get(0),
        ).unwrap();
        assert!((cap.unwrap() - 2.50).abs() < f64::EPSILON);

        // Clear the cap
        conn.execute(
            "UPDATE workspaces SET budget_cap = ?1 WHERE id = ?2",
            rusqlite::params![None::<f64>, "ws1"],
        ).unwrap();

        let cap: Option<f64> = conn.query_row(
            "SELECT budget_cap FROM workspaces WHERE id = ?1",
            rusqlite::params!["ws1"],
            |row| row.get(0),
        ).unwrap();
        assert!(cap.is_none());
    }

    #[test]
    fn test_list_workspaces_includes_budget_cap() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");
        conn.execute(
            "UPDATE workspaces SET budget_cap = ?1 WHERE id = ?2",
            rusqlite::params![10.0, "ws1"],
        ).unwrap();
        insert_test_workspace(&conn, "ws2", "/tmp/b");

        let mut stmt = conn.prepare(
            "SELECT id, path, name, default_agent, budget_cap FROM workspaces ORDER BY created_at"
        ).unwrap();
        let rows: Vec<(String, Option<f64>)> = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<f64>>(4)?))
        }).unwrap().filter_map(|r| r.ok()).collect();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "ws1");
        assert!((rows[0].1.unwrap() - 10.0).abs() < f64::EPSILON);
        assert_eq!(rows[1].0, "ws2");
        assert!(rows[1].1.is_none());
    }

    #[test]
    fn test_budget_cap_read_for_thread_start() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");
        conn.execute(
            "UPDATE workspaces SET budget_cap = ?1 WHERE id = ?2",
            rusqlite::params![3.50, "ws1"],
        ).unwrap();

        // Simulate what start_thread/resume_thread do: read budget_cap from DB
        let budget_cap: Option<f64> = conn.query_row(
            "SELECT budget_cap FROM workspaces WHERE id = ?1",
            rusqlite::params!["ws1"],
            |row| row.get(0),
        ).unwrap_or(None);

        assert!((budget_cap.unwrap() - 3.50).abs() < f64::EPSILON);
    }

    #[test]
    fn test_budget_cap_read_returns_none_when_unset() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");

        let budget_cap: Option<f64> = conn.query_row(
            "SELECT budget_cap FROM workspaces WHERE id = ?1",
            rusqlite::params!["ws1"],
            |row| row.get(0),
        ).unwrap_or(None);

        assert!(budget_cap.is_none());
    }

    #[test]
    fn test_budget_cap_read_returns_none_for_missing_workspace() {
        let conn = setup_test_db();

        let budget_cap: Option<f64> = conn.query_row(
            "SELECT budget_cap FROM workspaces WHERE id = ?1",
            rusqlite::params!["nonexistent"],
            |row| row.get(0),
        ).unwrap_or(None);

        assert!(budget_cap.is_none());
    }

    #[test]
    fn test_workspace_cost_with_multiple_threads() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");
        insert_test_thread(&conn, "t1", "ws1", None);
        insert_test_thread(&conn, "t2", "ws1", None);

        conn.execute("UPDATE threads SET cost_usd = 0.10 WHERE id = 't1'", []).unwrap();
        conn.execute("UPDATE threads SET cost_usd = 0.25 WHERE id = 't2'", []).unwrap();

        let total: f64 = conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM threads WHERE workspace_id = ?1",
            rusqlite::params!["ws1"],
            |row| row.get(0),
        ).unwrap();
        assert!((total - 0.35).abs() < 1e-10);
    }

    #[test]
    fn test_expand_tilde_empty_string() {
        assert_eq!(expand_tilde(""), "");
    }

    #[test]
    fn test_list_all_threads_respects_limit() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");
        for i in 0..5 {
            conn.execute(
                "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, created_at) VALUES (?1, 'ws1', 'claude', 'completed', 'test', ?2)",
                rusqlite::params![format!("t{i}"), format!("2024-0{}-01", i + 1)],
            ).unwrap();
        }

        let mut stmt = conn.prepare(
            "SELECT id FROM threads ORDER BY created_at DESC LIMIT ?1"
        ).unwrap();
        let ids: Vec<String> = stmt.query_map(rusqlite::params![3], |row| row.get(0))
            .unwrap().filter_map(|r| r.ok()).collect();
        assert_eq!(ids.len(), 3);
    }

    #[test]
    fn test_thread_with_cost_and_duration() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");
        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, cost_usd, duration_ms, created_at)
             VALUES ('t1', 'ws1', 'claude', 'completed', 'test', 0.05, 5000, '2024-01-01')",
            [],
        ).unwrap();

        let (cost, duration): (f64, Option<i64>) = conn.query_row(
            "SELECT cost_usd, duration_ms FROM threads WHERE id = 't1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).unwrap();
        assert!((cost - 0.05).abs() < f64::EPSILON);
        assert_eq!(duration, Some(5000));
    }

    #[test]
    fn test_resolve_agent_name_none_returns_default() {
        assert_eq!(resolve_agent_name(None), "claude-code");
    }

    #[test]
    fn test_resolve_agent_name_empty_string_returns_default() {
        assert_eq!(resolve_agent_name(Some("".to_string())), "claude-code");
    }

    #[test]
    fn test_resolve_agent_name_explicit_value_preserved() {
        assert_eq!(resolve_agent_name(Some("custom-agent".to_string())), "custom-agent");
    }

    #[test]
    fn test_resolve_agent_name_claude_code_preserved() {
        assert_eq!(resolve_agent_name(Some("claude-code".to_string())), "claude-code");
    }
}
