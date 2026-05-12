use std::path::PathBuf;
use std::sync::Arc;

use panes_core::db::DbHandle;
use panes_core::error::PanesError;
use panes_core::git;
use panes_core::session::{SessionManager, Workspace};
use panes_events::SessionContext;
use panes_memory::manager::MemoryManager;
use panes_memory::{BriefingStore, MemoryStore};
use panes_scheduler::Scheduler;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;

type SessionState = Arc<Mutex<SessionManager>>;
pub(crate) type DbState = DbHandle;
pub(crate) type MemoryManagerState = Arc<MemoryManager>;
pub(crate) type SchedulerState = Arc<Scheduler>;

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
    session: tauri::State<'_, SessionState>,
    workspace_id: String,
) -> Result<(), PanesError> {
    // GC shadow data for each thread in this workspace before the cascade
    // DELETE removes their rows.
    let ws_for_gc = workspace_id.clone();
    let thread_ids: Vec<String> = db
        .execute(move |conn| {
            let mut stmt = conn.prepare("SELECT id FROM threads WHERE workspace_id = ?1")?;
            let ids = stmt
                .query_map(rusqlite::params![ws_for_gc], |row| row.get::<_, String>(0))?
                .filter_map(|r| r.ok())
                .collect();
            Ok(ids)
        })
        .await
        .map_err(PanesError::from)?;
    let shadow = session.lock().await.shadow_tracker();
    for tid in thread_ids {
        if let Err(e) = shadow.delete_thread_data(&tid).await {
            tracing::warn!(
                error = %e,
                thread_id = %tid,
                "failed to GC shadow data during workspace removal"
            );
        }
    }

    db.execute(move |conn| {
        let tx = conn.unchecked_transaction()?;
        if panes_core::db::routine_tables_exist(conn) {
            tx.execute(
                "DELETE FROM routine_executions WHERE routine_id IN (SELECT id FROM routines WHERE workspace_id = ?1)",
                rusqlite::params![workspace_id],
            )?;
            tx.execute(
                "DELETE FROM routines WHERE workspace_id = ?1",
                rusqlite::params![workspace_id],
            )?;
        }
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
    pub is_routine: bool,
    pub routine_id: Option<String>,
    /// Which version tracker was used for this thread's file edits.
    /// "git" for git-backed workspaces, "shadow" for Panes-managed
    /// blob snapshots in non-git workspaces. The UI uses this to
    /// decide whether to show git-only actions like "Commit".
    #[serde(default)]
    pub tracker_kind: String,
    /// Memory snapshots persisted at thread start / end so the UI can
    /// render the memory-context chip even after a reload. Null for
    /// threads created before the migration that added these columns.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub injected_memories: Option<Vec<MemoryInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub injected_briefing: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extracted_memories: Option<Vec<MemoryInfo>>,
}

/// Columns and row decoder shared between list_threads and list_all_threads.
/// Keep them in sync — callers select this exact prefix.
const THREAD_COLUMNS: &str = "id, workspace_id, prompt, status, summary, cost_usd, duration_ms, created_at, is_routine, routine_id, tracker_kind, injected_memories, injected_briefing, extracted_memories";

fn decode_thread_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadInfo> {
    let injected_json: Option<String> = row.get(11)?;
    let briefing: Option<String> = row.get(12)?;
    let extracted_json: Option<String> = row.get(13)?;
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
        is_routine: row.get::<_, i32>(8).unwrap_or(0) != 0,
        routine_id: row.get(9)?,
        tracker_kind: row.get::<_, Option<String>>(10)?.unwrap_or_else(|| "git".to_string()),
        injected_memories: injected_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<MemoryInfo>>(s).ok()),
        injected_briefing: briefing,
        extracted_memories: extracted_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<MemoryInfo>>(s).ok()),
    })
}

#[tauri::command]
pub async fn list_threads(
    db: tauri::State<'_, DbState>,
    workspace_id: String,
) -> Result<Vec<ThreadInfo>, PanesError> {
    db.execute(move |conn| {
        let sql = format!(
            "SELECT {THREAD_COLUMNS} FROM threads WHERE workspace_id = ?1 ORDER BY created_at DESC"
        );
        let mut stmt = conn.prepare(&sql)?;

        let threads: Vec<ThreadInfo> = stmt.query_map(rusqlite::params![workspace_id], decode_thread_row)?
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
        let sql = format!(
            "SELECT {THREAD_COLUMNS} FROM threads ORDER BY created_at DESC LIMIT ?1"
        );
        let mut stmt = conn.prepare(&sql)?;

        let threads: Vec<ThreadInfo> = stmt.query_map(rusqlite::params![limit], decode_thread_row)?
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
    session: tauri::State<'_, SessionState>,
    thread_id: String,
) -> Result<(), PanesError> {
    // Garbage-collect shadow state before dropping the thread row.
    // Shadow tracking is no-op for git threads, so it's cheap to run
    // unconditionally rather than branching on tracker_kind.
    let shadow = session.lock().await.shadow_tracker();
    if let Err(e) = shadow.delete_thread_data(&thread_id).await {
        tracing::warn!(
            error = %e,
            thread_id = %thread_id,
            "failed to GC shadow data for deleted thread"
        );
    }

    let tid_for_db = thread_id.clone();
    db.execute(move |conn| {
        let tx = conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM events WHERE thread_id = ?1",
            rusqlite::params![tid_for_db],
        )?;
        tx.execute(
            "DELETE FROM costs WHERE thread_id = ?1",
            rusqlite::params![tid_for_db],
        )?;
        tx.execute(
            "DELETE FROM threads WHERE id = ?1",
            rusqlite::params![tid_for_db],
        )?;
        tx.commit()?;
        Ok(())
    }).await.map_err(PanesError::from)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartThreadResult {
    pub thread_id: String,
    pub injected_memories: Vec<MemoryInfo>,
    pub briefing_preview: Option<String>,
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
    // `adapter` selects the AgentAdapter (claude-code, kiro-cli, ...).
    // `agent` is the adapter-specific sub-agent/mode name passed through
    // to the adapter (claude's --agent flag / ACP's set_mode modeId). When
    // `adapter` is absent, `agent` is interpreted as the adapter name for
    // backward compatibility with the pre-kiro-cli IPC shape.
    adapter: Option<String>,
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

    // Resolve the adapter name. SessionManager's overloaded dispatch treats
    // `agent_name` both as adapter lookup key AND as CLI sub-agent — when
    // the caller supplies both we prefer `adapter` for routing and leave
    // `agent` for the per-adapter flag. When only `agent` is supplied we
    // send it as-is for backward compat.
    let agent_for_dispatch = adapter
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| agent.clone().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "claude-code".to_string());

    let workspace = Workspace {
        id: workspace_id.clone(),
        path: PathBuf::from(&expanded_path),
        name: workspace_name,
        default_agent: Some(agent_for_dispatch.clone()),
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

    let briefing_preview = injected.briefing.as_ref().map(|b| truncate_briefing(b));
    let injected_memories: Vec<MemoryInfo> = injected
        .memories
        .iter()
        .cloned()
        .map(MemoryInfo::from)
        .collect();

    let context = SessionContext {
        briefing: injected.briefing,
        memories: injected.memories.iter().map(|m| m.content.clone()).collect(),
        budget_cap: None,
    };

    let injected_json = serde_json::to_string(&injected_memories).unwrap_or_else(|_| "[]".into());
    let briefing_json = briefing_preview.clone();

    let mgr = session_manager.lock().await;
    let thread_id = mgr.start_thread(&workspace, &prompt, &agent_for_dispatch, context, model.as_deref())
        .await?;
    drop(mgr);

    // Persist the injected context against the new thread row so reopened
    // threads can still show what was injected.
    let tid_for_db = thread_id.clone();
    let _ = db.execute(move |conn| {
        conn.execute(
            "UPDATE threads SET injected_memories = ?1, injected_briefing = ?2 WHERE id = ?3",
            rusqlite::params![injected_json, briefing_json, tid_for_db],
        )?;
        Ok(())
    }).await;

    Ok(StartThreadResult {
        thread_id,
        injected_memories,
        briefing_preview,
    })
}

/// Truncate a briefing to 500 chars (not bytes — briefings may contain
/// multi-byte content) and append a horizontal ellipsis if shortened. The
/// preview is purely UI chrome; the full briefing is still used in the
/// prompt sent to the agent.
fn truncate_briefing(b: &str) -> String {
    const MAX_LEN: usize = 500;
    if b.chars().count() <= MAX_LEN {
        b.to_string()
    } else {
        let mut s: String = b.chars().take(MAX_LEN).collect();
        s.push('…');
        s
    }
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

    // The adapter the thread was originally spawned with wins — the frontend's
    // `agent` param is only used when the DB row is somehow missing. Without
    // this, a kiro-cli thread gets routed to claude-code on resume and fails
    // with "failed to resume session" because kiro-cli session ids are
    // meaningless to the claude CLI.
    let tid = thread_id.clone();
    let stored_agent: Option<String> = db.execute(move |conn| {
        Ok(conn.query_row(
            "SELECT agent_type FROM threads WHERE id = ?1",
            rusqlite::params![tid],
            |row| row.get(0),
        ).ok())
    }).await.map_err(PanesError::from)?;

    let workspace = Workspace {
        id: workspace_id,
        path: PathBuf::from(&expanded_path),
        name: workspace_name,
        default_agent: agent.clone(),
        budget_cap,
    };

    let agent_name = resolve_agent_name(stored_agent.or(agent));

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
pub async fn set_thread_model(
    session_manager: tauri::State<'_, SessionState>,
    thread_id: String,
    model: String,
) -> Result<(), PanesError> {
    let mgr = session_manager.lock().await;
    mgr.set_thread_model(&thread_id, &model).await
}

#[tauri::command]
pub async fn commit_changes(
    workspace_path: String,
    message: String,
    files: Option<Vec<String>>,
) -> Result<String, PanesError> {
    let expanded = expand_tilde(&workspace_path);
    let path = PathBuf::from(&expanded);
    git::commit(&path, &message, files.as_deref())
        .await
        .map_err(PanesError::from)
}

/// Inner implementation of `revert_changes`. Split from the tauri
/// command so unit tests can drive it with an `Arc<Mutex<SessionManager>>`
/// without needing a live Tauri `State`. Routes through the
/// thread's recorded tracker — git or shadow.
pub(crate) async fn revert_changes_inner(
    session: &SessionState,
    workspace_path: &str,
    thread_id: &str,
) -> Result<(), PanesError> {
    let mgr = session.lock().await;
    let tracker = mgr.tracker_for_thread(thread_id).await?;
    drop(mgr);

    let expanded = expand_tilde(workspace_path);
    let path = PathBuf::from(&expanded);
    tracker
        .revert(thread_id, &path)
        .await
        .map_err(|e| PanesError::GitError {
            message: format!("revert failed: {e}"),
        })
}

#[tauri::command]
pub async fn revert_changes(
    session: tauri::State<'_, SessionState>,
    workspace_path: String,
    thread_id: String,
) -> Result<(), PanesError> {
    revert_changes_inner(session.inner(), &workspace_path, &thread_id).await
}

/// Inner impl — see `revert_changes_inner` rationale.
pub(crate) async fn get_changed_files_inner(
    session: &SessionState,
    workspace_path: &str,
    thread_id: Option<&str>,
) -> Result<Vec<String>, PanesError> {
    let expanded = expand_tilde(workspace_path);
    let path = PathBuf::from(&expanded);

    // When a thread_id is supplied, route via the tracker so non-git
    // workspaces get their shadow-tracked changes. Otherwise fall through
    // to the original git-only porcelain — preserves compatibility for
    // callers that haven't adopted the new signature yet.
    if let Some(tid) = thread_id {
        let mgr = session.lock().await;
        let tracker = mgr.tracker_for_thread(tid).await?;
        drop(mgr);
        let changed = tracker
            .list_changed_files(tid, &path)
            .await
            .map_err(|e| PanesError::GitError {
                message: format!("list_changed_files failed: {e}"),
            })?;
        // Preserve the "<status> <path>" porcelain string shape that existing
        // callers expect.
        return Ok(changed
            .into_iter()
            .map(|c| {
                let code = match c.action {
                    panes_core::version_tracker::FileAction::Created => " A",
                    panes_core::version_tracker::FileAction::Modified => " M",
                    panes_core::version_tracker::FileAction::Deleted => " D",
                };
                format!("{} {}", code, c.relative_path)
            })
            .collect());
    }

    git::get_changed_files(&path)
        .await
        .map_err(PanesError::from)
}

#[tauri::command]
pub async fn get_changed_files(
    session: tauri::State<'_, SessionState>,
    workspace_path: String,
    thread_id: Option<String>,
) -> Result<Vec<String>, PanesError> {
    get_changed_files_inner(session.inner(), &workspace_path, thread_id.as_deref()).await
}

pub(crate) async fn get_file_diff_inner(
    session: &SessionState,
    workspace_path: &str,
    file_path: &str,
    thread_id: Option<&str>,
) -> Result<String, PanesError> {
    let expanded = expand_tilde(workspace_path);
    let path = PathBuf::from(&expanded);

    if let Some(tid) = thread_id {
        let mgr = session.lock().await;
        let tracker = mgr.tracker_for_thread(tid).await?;
        drop(mgr);
        let file = PathBuf::from(file_path);
        return tracker
            .diff(tid, &path, Some(&[file]))
            .await
            .map_err(|e| PanesError::GitError {
                message: format!("diff failed: {e}"),
            });
    }

    git::get_file_diff(&path, file_path)
        .await
        .map_err(PanesError::from)
}

#[tauri::command]
pub async fn get_file_diff(
    session: tauri::State<'_, SessionState>,
    workspace_path: String,
    file_path: String,
    thread_id: Option<String>,
) -> Result<String, PanesError> {
    get_file_diff_inner(session.inner(), &workspace_path, &file_path, thread_id.as_deref()).await
}

pub(crate) async fn get_workspace_diff_inner(
    session: &SessionState,
    workspace_path: &str,
    files: Option<&[String]>,
    thread_id: Option<&str>,
) -> Result<String, PanesError> {
    let expanded = expand_tilde(workspace_path);
    let path = PathBuf::from(&expanded);

    if let Some(tid) = thread_id {
        let mgr = session.lock().await;
        let tracker = mgr.tracker_for_thread(tid).await?;
        drop(mgr);
        let file_bufs: Option<Vec<PathBuf>> =
            files.map(|fs| fs.iter().map(PathBuf::from).collect());
        return tracker
            .diff(tid, &path, file_bufs.as_deref())
            .await
            .map_err(|e| PanesError::GitError {
                message: format!("diff failed: {e}"),
            });
    }

    git::get_workspace_diff(&path, files)
        .await
        .map_err(PanesError::from)
}

#[tauri::command]
pub async fn get_workspace_diff(
    session: tauri::State<'_, SessionState>,
    workspace_path: String,
    files: Option<Vec<String>>,
    thread_id: Option<String>,
) -> Result<String, PanesError> {
    get_workspace_diff_inner(
        session.inner(),
        &workspace_path,
        files.as_deref(),
        thread_id.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn get_files_git_status(
    file_paths: Vec<String>,
) -> Result<Vec<git::RepoFileStatus>, PanesError> {
    // Intentionally unchanged: this command is only useful in git-backed
    // workspaces (it groups files by their owning repo for the Commit
    // UI). In shadow-tracked workspaces the frontend will short-circuit
    // via `trackerKind` and never issue this call.
    git::get_files_git_status(&file_paths)
        .await
        .map_err(PanesError::from)
}

#[tauri::command]
pub async fn list_git_repos(
    workspace_path: String,
) -> Result<Vec<String>, PanesError> {
    let expanded = expand_tilde(&workspace_path);
    let path = PathBuf::from(&expanded);
    git::list_git_repos(&path)
        .await
        .map_err(PanesError::from)
}

#[tauri::command]
pub async fn commit_repos(
    commits: Vec<git::RepoCommitParams>,
) -> Result<Vec<String>, PanesError> {
    git::commit_repos(&commits)
        .await
        .map_err(PanesError::from)
}

#[tauri::command]
pub async fn generate_commit_message(
    workspace_path: String,
    diff: String,
) -> Result<String, PanesError> {
    git::generate_commit_message(&workspace_path, &diff)
        .await
        .map_err(PanesError::from)
}

// --- Memory extraction ---

#[tauri::command]
pub async fn extract_memories(
    memory_manager: tauri::State<'_, MemoryManagerState>,
    db: tauri::State<'_, DbState>,
    workspace_id: String,
    thread_id: String,
    transcript: String,
) -> Result<Vec<MemoryInfo>, PanesError> {
    let memories = memory_manager
        .add(&transcript, Some(&workspace_id), &thread_id)
        .await
        .map_err(PanesError::from)?;

    let infos: Vec<MemoryInfo> = memories.into_iter().map(MemoryInfo::from).collect();

    // Persist the extracted set on the thread row so the UI can show the
    // "N memories written" chip after a reload. Best-effort — a DB error
    // here must not break the IPC return.
    let json = serde_json::to_string(&infos).unwrap_or_else(|_| "[]".into());
    let tid = thread_id.clone();
    let _ = db.execute(move |conn| {
        conn.execute(
            "UPDATE threads SET extracted_memories = ?1 WHERE id = ?2",
            rusqlite::params![json, tid],
        )?;
        Ok(())
    }).await;

    Ok(infos)
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

#[tauri::command]
pub async fn get_cost_timeline(
    db: tauri::State<'_, DbState>,
    days: Option<u32>,
    workspace_id: Option<String>,
) -> Result<Vec<panes_core::db::DailyCost>, PanesError> {
    let days = days.unwrap_or(30);
    db.execute(move |conn| {
        panes_core::db::query_cost_timeline(conn, days, workspace_id.as_deref())
    })
    .await
    .map_err(PanesError::from)
}

#[tauri::command]
pub async fn get_workspace_cost_breakdown(
    db: tauri::State<'_, DbState>,
) -> Result<Vec<panes_core::db::WorkspaceCostBreakdown>, PanesError> {
    db.execute(|conn| panes_core::db::query_workspace_cost_breakdown(conn))
        .await
        .map_err(PanesError::from)
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
pub async fn list_agents(
    session_manager: tauri::State<'_, SessionState>,
    adapter: String,
) -> Result<Vec<AgentInfo>, PanesError> {
    // Claude's agent list comes from ~/.claude/agents/*.md — panes-app
    // parses this directly because it predates the trait-based discovery.
    if adapter == "claude-code" {
        return list_agents_claude();
    }
    // Every other adapter exposes its agent list via the trait, which ACP
    // adapters populate from the backend's session/new response.
    let adapter_ref = {
        let mgr = session_manager.lock().await;
        mgr.adapter(&adapter)
    };
    list_agents_via_trait(adapter_ref).await
}

/// Shared implementation reachable from both the IPC handler and tests.
/// `adapter_ref` is `None` when the adapter isn't registered; returns an
/// empty list in that case so the UI picker hides gracefully rather than
/// erroring out.
async fn list_agents_via_trait(
    adapter_ref: Option<std::sync::Arc<dyn panes_adapters::AgentAdapter>>,
) -> Result<Vec<AgentInfo>, PanesError> {
    let Some(adapter_ref) = adapter_ref else {
        return Ok(vec![]);
    };
    let raw = adapter_ref
        .list_agents()
        .await
        .map_err(|e| PanesError::Internal { message: e.to_string() })?;
    Ok(raw
        .into_iter()
        .map(|a| AgentInfo {
            name: a.name,
            model: a.model,
            description: a.description,
        })
        .collect())
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

// --- Feature toggle commands ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureInfo {
    pub id: String,
    pub enabled: bool,
    pub label: String,
    pub description: String,
}

#[tauri::command]
pub async fn get_features(
    db: tauri::State<'_, DbState>,
) -> Result<Vec<FeatureInfo>, PanesError> {
    db.execute(|conn| {
        let features = panes_core::features::list_features(conn)?;
        Ok(features.into_iter().map(|f| FeatureInfo {
            id: f.id,
            enabled: f.enabled,
            label: f.label,
            description: f.description,
        }).collect())
    }).await.map_err(PanesError::from)
}

#[tauri::command]
pub async fn set_feature_enabled(
    db: tauri::State<'_, DbState>,
    scheduler: tauri::State<'_, SchedulerState>,
    feature_id: String,
    enabled: bool,
) -> Result<(), PanesError> {
    let fid = feature_id.clone();
    db.execute(move |conn| {
        panes_core::features::set_feature_enabled(conn, &fid, enabled)?;
        if fid == panes_core::features::FEATURE_ROUTINES && enabled {
            panes_core::db::create_routine_tables(conn)?;
        }
        Ok(())
    }).await.map_err(PanesError::from)?;

    if feature_id == panes_core::features::FEATURE_ROUTINES {
        if enabled {
            scheduler.start();
        } else {
            scheduler.stop().await;
        }
    }
    Ok(())
}

// --- Routine CRUD commands ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineInfo {
    pub id: String,
    pub workspace_id: String,
    pub prompt: String,
    pub cron_expr: String,
    pub budget_cap: Option<f64>,
    pub on_complete: serde_json::Value,
    pub on_failure: serde_json::Value,
    pub enabled: bool,
    pub last_run_at: Option<String>,
    pub created_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineExecutionInfo {
    pub id: String,
    pub routine_id: String,
    pub thread_id: Option<String>,
    pub status: String,
    pub cost_usd: f64,
    pub started_at: String,
    pub completed_at: Option<String>,
    pub error_message: Option<String>,
}

fn require_routines_enabled(conn: &rusqlite::Connection) -> anyhow::Result<()> {
    if !panes_core::features::is_feature_enabled(conn, panes_core::features::FEATURE_ROUTINES)? {
        anyhow::bail!("Routines feature is not enabled");
    }
    Ok(())
}

#[tauri::command]
pub async fn create_routine(
    db: tauri::State<'_, DbState>,
    workspace_id: String,
    prompt: String,
    cron_expr: String,
    budget_cap: Option<f64>,
    on_complete: Option<String>,
    on_failure: Option<String>,
) -> Result<RoutineInfo, PanesError> {
    // Normalize 5-field cron (standard) to 6-field (cron crate requires seconds)
    let cron_expr = if cron_expr.split_whitespace().count() == 5 {
        format!("0 {cron_expr}")
    } else {
        cron_expr
    };

    use std::str::FromStr;
    cron::Schedule::from_str(&cron_expr).map_err(|e| PanesError::ValidationError {
        message: format!("Invalid cron expression: {e}"),
    })?;

    // Validate action JSON if provided
    let on_complete_json = on_complete.unwrap_or_else(|| r#"{"action":"notify"}"#.to_string());
    let on_failure_json = on_failure.unwrap_or_else(|| r#"{"action":"notify"}"#.to_string());
    serde_json::from_str::<panes_scheduler::ScheduleAction>(&on_complete_json)
        .map_err(|e| PanesError::ValidationError { message: format!("Invalid on_complete action: {e}") })?;
    serde_json::from_str::<panes_scheduler::ScheduleAction>(&on_failure_json)
        .map_err(|e| PanesError::ValidationError { message: format!("Invalid on_failure action: {e}") })?;

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    let rid = id.clone();
    let wid = workspace_id.clone();
    let p = prompt.clone();
    let ce = cron_expr.clone();
    let bc = budget_cap;
    let oc = on_complete_json.clone();
    let of = on_failure_json.clone();
    let ts = now.clone();

    db.execute(move |conn| {
        require_routines_enabled(conn)?;
        conn.execute(
            "INSERT INTO routines (id, workspace_id, prompt, cron_expr, budget_cap, on_complete, on_failure, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![rid, wid, p, ce, bc, oc, of, ts],
        )?;
        Ok(())
    }).await.map_err(PanesError::from)?;

    Ok(RoutineInfo {
        id,
        workspace_id,
        prompt,
        cron_expr,
        budget_cap,
        on_complete: serde_json::from_str(&on_complete_json).unwrap_or(serde_json::Value::Null),
        on_failure: serde_json::from_str(&on_failure_json).unwrap_or(serde_json::Value::Null),
        enabled: true,
        last_run_at: None,
        created_at: now,
    })
}

#[tauri::command]
pub async fn update_routine(
    db: tauri::State<'_, DbState>,
    routine_id: String,
    prompt: Option<String>,
    cron_expr: Option<String>,
    budget_cap: Option<Option<f64>>,
    on_complete: Option<String>,
    on_failure: Option<String>,
) -> Result<(), PanesError> {
    let cron_expr = cron_expr.map(|ce| {
        if ce.split_whitespace().count() == 5 { format!("0 {ce}") } else { ce }
    });
    if let Some(ref ce) = cron_expr {
        use std::str::FromStr;
        cron::Schedule::from_str(ce).map_err(|e| PanesError::ValidationError {
            message: format!("Invalid cron expression: {e}"),
        })?;
    }

    db.execute(move |conn| {
        require_routines_enabled(conn)?;
        if let Some(p) = prompt {
            conn.execute("UPDATE routines SET prompt = ?1 WHERE id = ?2", rusqlite::params![p, routine_id])?;
        }
        if let Some(ce) = cron_expr {
            conn.execute("UPDATE routines SET cron_expr = ?1 WHERE id = ?2", rusqlite::params![ce, routine_id])?;
        }
        if let Some(bc) = budget_cap {
            conn.execute("UPDATE routines SET budget_cap = ?1 WHERE id = ?2", rusqlite::params![bc, routine_id])?;
        }
        if let Some(oc) = on_complete {
            conn.execute("UPDATE routines SET on_complete = ?1 WHERE id = ?2", rusqlite::params![oc, routine_id])?;
        }
        if let Some(of) = on_failure {
            conn.execute("UPDATE routines SET on_failure = ?1 WHERE id = ?2", rusqlite::params![of, routine_id])?;
        }
        Ok(())
    }).await.map_err(PanesError::from)
}

#[tauri::command]
pub async fn delete_routine(
    db: tauri::State<'_, DbState>,
    routine_id: String,
) -> Result<(), PanesError> {
    db.execute(move |conn| {
        require_routines_enabled(conn)?;
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM routine_executions WHERE routine_id = ?1", rusqlite::params![routine_id])?;
        tx.execute("DELETE FROM routines WHERE id = ?1", rusqlite::params![routine_id])?;
        tx.commit()?;
        Ok(())
    }).await.map_err(PanesError::from)
}

#[tauri::command]
pub async fn list_routines(
    db: tauri::State<'_, DbState>,
    workspace_id: Option<String>,
) -> Result<Vec<RoutineInfo>, PanesError> {
    db.execute(move |conn| {
        require_routines_enabled(conn)?;
        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match &workspace_id {
            Some(wid) => (
                "SELECT id, workspace_id, prompt, cron_expr, budget_cap, on_complete, on_failure, enabled, last_run_at, created_at
                 FROM routines WHERE workspace_id = ?1 ORDER BY created_at DESC",
                vec![Box::new(wid.clone())],
            ),
            None => (
                "SELECT id, workspace_id, prompt, cron_expr, budget_cap, on_complete, on_failure, enabled, last_run_at, created_at
                 FROM routines ORDER BY created_at DESC",
                vec![],
            ),
        };
        let mut stmt = conn.prepare(sql)?;
        let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            let on_complete_str: String = row.get(5)?;
            let on_failure_str: String = row.get(6)?;
            Ok(RoutineInfo {
                id: row.get(0)?,
                workspace_id: row.get(1)?,
                prompt: row.get(2)?,
                cron_expr: row.get(3)?,
                budget_cap: row.get(4)?,
                on_complete: serde_json::from_str(&on_complete_str).unwrap_or(serde_json::Value::Null),
                on_failure: serde_json::from_str(&on_failure_str).unwrap_or(serde_json::Value::Null),
                enabled: row.get(7)?,
                last_run_at: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?;
        let mut routines = Vec::new();
        for row in rows {
            routines.push(row?);
        }
        Ok(routines)
    }).await.map_err(PanesError::from)
}

#[tauri::command]
pub async fn toggle_routine(
    db: tauri::State<'_, DbState>,
    routine_id: String,
    enabled: bool,
) -> Result<(), PanesError> {
    db.execute(move |conn| {
        require_routines_enabled(conn)?;
        conn.execute(
            "UPDATE routines SET enabled = ?1 WHERE id = ?2",
            rusqlite::params![enabled, routine_id],
        )?;
        Ok(())
    }).await.map_err(PanesError::from)
}

#[tauri::command]
pub async fn list_routine_executions(
    db: tauri::State<'_, DbState>,
    routine_id: String,
    limit: Option<u32>,
) -> Result<Vec<RoutineExecutionInfo>, PanesError> {
    let lim = limit.unwrap_or(50);
    db.execute(move |conn| {
        require_routines_enabled(conn)?;
        let mut stmt = conn.prepare(
            "SELECT id, routine_id, thread_id, status, cost_usd, started_at, completed_at, error_message
             FROM routine_executions WHERE routine_id = ?1 ORDER BY started_at DESC LIMIT ?2"
        )?;
        let rows = stmt.query_map(rusqlite::params![routine_id, lim], |row| {
            Ok(RoutineExecutionInfo {
                id: row.get(0)?,
                routine_id: row.get(1)?,
                thread_id: row.get(2)?,
                status: row.get(3)?,
                cost_usd: row.get(4)?,
                started_at: row.get(5)?,
                completed_at: row.get(6)?,
                error_message: row.get(7)?,
            })
        })?;
        let mut execs = Vec::new();
        for row in rows {
            execs.push(row?);
        }
        Ok(execs)
    }).await.map_err(PanesError::from)
}

#[tauri::command]
pub async fn get_routine_cost(
    db: tauri::State<'_, DbState>,
    routine_id: String,
) -> Result<f64, PanesError> {
    db.execute(move |conn| {
        require_routines_enabled(conn)?;
        Ok(conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0.0) FROM routine_executions WHERE routine_id = ?1",
            rusqlite::params![routine_id],
            |row| row.get(0),
        )?)
    }).await.map_err(PanesError::from)
}

// --- Output validator commands ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidatorTypeInfo {
    pub type_id: String,
    pub label: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub correctable: bool,
}

#[tauri::command]
pub async fn list_validator_types(
    session: tauri::State<'_, SessionState>,
) -> Result<Vec<ValidatorTypeInfo>, PanesError> {
    let sm = session.lock().await;
    Ok(sm
        .validators
        .catalog()
        .iter()
        .map(|t| ValidatorTypeInfo {
            type_id: t.type_id.to_string(),
            label: t.label.to_string(),
            description: t.description.to_string(),
            default_config: t.default_config.clone(),
            correctable: t.correctable,
        })
        .collect())
}

#[tauri::command]
pub async fn list_validators(
    db: tauri::State<'_, DbState>,
    workspace_id: String,
) -> Result<Vec<panes_core::db::WorkspaceValidator>, PanesError> {
    db.execute(move |conn| panes_core::db::list_validators(conn, &workspace_id))
        .await
        .map_err(PanesError::from)
}

#[tauri::command]
pub async fn add_validator(
    db: tauri::State<'_, DbState>,
    session: tauri::State<'_, SessionState>,
    workspace_id: String,
    validator_type: String,
    config_json: String,
) -> Result<panes_core::db::WorkspaceValidator, PanesError> {
    // Validate the type_id is known.
    {
        let sm = session.lock().await;
        if sm.validators.get(&validator_type).is_none() {
            return Err(PanesError::ValidationError {
                message: format!("unknown validator type: {validator_type}"),
            });
        }
    }
    // Verify config_json parses.
    if serde_json::from_str::<serde_json::Value>(&config_json).is_err() {
        return Err(PanesError::ValidationError {
            message: "config_json is not valid JSON".to_string(),
        });
    }
    db.execute(move |conn| {
        panes_core::db::insert_validator(conn, &workspace_id, &validator_type, &config_json)
    })
    .await
    .map_err(PanesError::from)
}

#[tauri::command]
pub async fn update_validator(
    db: tauri::State<'_, DbState>,
    id: String,
    enabled: Option<bool>,
    config_json: Option<String>,
) -> Result<panes_core::db::WorkspaceValidator, PanesError> {
    if let Some(ref cfg) = config_json {
        if serde_json::from_str::<serde_json::Value>(cfg).is_err() {
            return Err(PanesError::ValidationError {
                message: "config_json is not valid JSON".to_string(),
            });
        }
    }
    db.execute(move |conn| {
        panes_core::db::update_validator(conn, &id, enabled, config_json.as_deref())
    })
    .await
    .map_err(PanesError::from)
}

#[tauri::command]
pub async fn remove_validator(
    db: tauri::State<'_, DbState>,
    id: String,
) -> Result<(), PanesError> {
    db.execute(move |conn| panes_core::db::delete_validator(conn, &id))
        .await
        .map_err(PanesError::from)
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
                created_at TEXT NOT NULL,
                routine_id TEXT,
                tracker_kind TEXT,
                injected_memories TEXT,
                injected_briefing TEXT,
                extracted_memories TEXT
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

    #[tokio::test]
    async fn test_list_agents_unknown_adapter_returns_empty() {
        // Unknown adapters resolve to `None` — the helper should return an
        // empty list without erroring.
        let result = list_agents_via_trait(None).await.expect("ok");
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_list_agents_via_trait_returns_adapter_agents() {
        // Exercise the trait-dispatch path. AcpAdapter's list_agents runs
        // ensure_probed under the hood — against a broken binary that
        // returns no modes, so we get an empty list but not a crash.
        // Discovery-success assertions live in the adapter's own tests.
        use panes_adapters::AcpAdapter;
        let adapter = std::sync::Arc::new(
            AcpAdapter::new("kiro-cli", "/nonexistent-bin-xyz", vec![]),
        ) as std::sync::Arc<dyn panes_adapters::AgentAdapter>;
        let result = list_agents_via_trait(Some(adapter))
            .await
            .expect("trait dispatch must not propagate discovery failure");
        assert!(
            result.is_empty(),
            "no backend available → empty list, not an error: {result:?}"
        );
    }

    #[tokio::test]
    async fn test_list_agents_never_exposes_acp_as_adapter_name() {
        // Guardrail: the string "acp" is the protocol, never a user-visible
        // adapter id. Even if someone mistakenly registered an adapter under
        // that name, the absence of such a registration means the lookup
        // returns None and we get an empty list.
        let result = list_agents_via_trait(None).await.expect("ok");
        assert!(
            result.is_empty(),
            "'acp' must not be a valid adapter id in the UI — saw: {result:?}"
        );
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
            is_routine: false,
            routine_id: None,
            tracker_kind: "git".to_string(),
            injected_memories: None,
            injected_briefing: None,
            extracted_memories: None,
        };
        let json = serde_json::to_string(&ti).unwrap();
        assert!(json.contains("\"workspaceId\""));
        assert!(json.contains("\"costUsd\""));
        assert!(json.contains("\"durationMs\""));
        assert!(json.contains("\"createdAt\""));
        assert!(json.contains("\"trackerKind\""));
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

    #[test]
    fn test_resume_reads_stored_adapter_from_threads_table() {
        // Regression for: kiro-cli threads failing to resume because
        // `resume_thread` defaulted to claude-code when the frontend didn't
        // pass an `agent` hint. The stored adapter must win.
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");
        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, created_at) VALUES ('t-kiro', 'ws1', 'kiro-cli', 'complete', 'hi', '2024-01-01')",
            [],
        ).unwrap();

        // This is exactly what resume_thread now runs.
        let stored: Option<String> = conn.query_row(
            "SELECT agent_type FROM threads WHERE id = ?1",
            rusqlite::params!["t-kiro"],
            |row| row.get(0),
        ).ok();
        assert_eq!(stored.as_deref(), Some("kiro-cli"));

        // With no frontend hint, we should still resolve to kiro-cli.
        let resolved = resolve_agent_name(stored.or(None));
        assert_eq!(resolved, "kiro-cli");
    }

    #[test]
    fn test_resume_falls_back_to_frontend_agent_when_db_missing() {
        // If somehow the thread row is gone, accept the frontend-provided
        // hint rather than hardwiring to claude-code.
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");

        let stored: Option<String> = conn.query_row(
            "SELECT agent_type FROM threads WHERE id = ?1",
            rusqlite::params!["does-not-exist"],
            |row| row.get(0),
        ).ok();
        assert!(stored.is_none(), "no row, no stored agent");

        let resolved = resolve_agent_name(stored.or(Some("kiro-cli".to_string())));
        assert_eq!(resolved, "kiro-cli");
    }

    #[test]
    fn test_resume_stored_adapter_wins_over_frontend_hint() {
        // Precedence: the DB-stored agent_type is authoritative. A stale
        // frontend hint (e.g. an in-flight request using the workspace's
        // old default) must not be able to re-route a kiro-cli thread.
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/a");
        conn.execute(
            "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, created_at) VALUES ('t-kiro', 'ws1', 'kiro-cli', 'complete', 'hi', '2024-01-01')",
            [],
        ).unwrap();

        let stored: Option<String> = conn.query_row(
            "SELECT agent_type FROM threads WHERE id = ?1",
            rusqlite::params!["t-kiro"],
            |row| row.get(0),
        ).ok();

        // Frontend wrongly hints claude-code; stored kiro-cli must win.
        let resolved = resolve_agent_name(stored.or(Some("claude-code".to_string())));
        assert_eq!(resolved, "kiro-cli");
    }

    // -----------------------------------------------------------------
    // Cov2: IPC tracker-routing unit tests. These exercise the branching
    // inside revert_changes / get_workspace_diff / get_changed_files /
    // get_file_diff — in particular the Option<thread_id> dispatch and
    // the tracker-vs-git fallback. They complement the E2E spec by
    // failing fast when the routing logic drifts.
    // -----------------------------------------------------------------

    use panes_core::db::DbHandle;
    use panes_core::test_support;
    use panes_core::version_tracker::VersionTracker;
    use std::sync::Arc as StdArc;

    struct IpcHarness {
        _tmp: tempfile::TempDir,
        workspace_path: std::path::PathBuf,
        session: SessionState,
        db: DbHandle,
        thread_id: String,
    }

    async fn make_ipc_harness(tracker_kind: &str) -> IpcHarness {
        let tmp = tempfile::tempdir().unwrap();
        let workspace_path = tmp.path().join("ws");
        std::fs::create_dir_all(&workspace_path).unwrap();

        let (mgr, db, _rx) = test_support::test_session_manager().await;
        let session: SessionState = StdArc::new(Mutex::new(mgr));

        // Seed workspace + thread rows directly in the DB so the
        // commands can resolve tracker_for_thread.
        let ws_path_s = workspace_path.to_string_lossy().to_string();
        let kind = tracker_kind.to_string();
        db.execute(move |conn| {
            conn.execute(
                "INSERT INTO workspaces (id, path, name, created_at) VALUES ('ws', ?1, 'ws', '2024-01-01')",
                rusqlite::params![ws_path_s],
            )?;
            conn.execute(
                "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, tracker_kind, created_at) \
                 VALUES ('t1', 'ws', 'claude-code', 'completed', 'p', ?1, '2024-01-01')",
                rusqlite::params![kind],
            )?;
            Ok(())
        }).await.unwrap();

        IpcHarness {
            _tmp: tmp,
            workspace_path,
            session,
            db,
            thread_id: "t1".to_string(),
        }
    }

    async fn shadow_row_count(db: &DbHandle, thread_id: &str) -> i64 {
        let tid = thread_id.to_string();
        db.execute(move |conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM shadow_edits WHERE thread_id = ?1",
                rusqlite::params![tid],
                |row| row.get::<_, i64>(0),
            )?)
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn revert_changes_routes_shadow_tracker_in_non_git_workspace() {
        let h = make_ipc_harness("shadow").await;

        // Arrange shadow state: pre-edit tombstone for a file that the
        // test will "write", then revert must delete it.
        let file = h.workspace_path.join("created.txt");
        let shadow = h.session.lock().await.shadow_tracker();
        shadow
            .record_pre_edit(&h.thread_id, &h.workspace_path, &file)
            .await
            .unwrap();
        std::fs::write(&file, b"agent-produced").unwrap();
        assert!(file.exists());
        assert_eq!(shadow_row_count(&h.db, &h.thread_id).await, 1);

        revert_changes_inner(
            &h.session,
            &h.workspace_path.to_string_lossy(),
            &h.thread_id,
        )
        .await
        .unwrap();

        assert!(!file.exists(), "shadow revert should delete the created file");
    }

    #[tokio::test]
    async fn revert_changes_errors_for_unknown_thread() {
        let h = make_ipc_harness("shadow").await;
        let err = revert_changes_inner(
            &h.session,
            &h.workspace_path.to_string_lossy(),
            "nonexistent",
        )
        .await;
        assert!(err.is_err(), "unknown thread should surface an error");
    }

    #[tokio::test]
    async fn get_workspace_diff_with_thread_id_routes_shadow() {
        let h = make_ipc_harness("shadow").await;
        let file = h.workspace_path.join("a.txt");
        std::fs::write(&file, "before\n").unwrap();

        let shadow = h.session.lock().await.shadow_tracker();
        shadow
            .record_pre_edit(&h.thread_id, &h.workspace_path, &file)
            .await
            .unwrap();
        std::fs::write(&file, "after\n").unwrap();

        let diff = get_workspace_diff_inner(
            &h.session,
            &h.workspace_path.to_string_lossy(),
            None,
            Some(&h.thread_id),
        )
        .await
        .unwrap();

        assert!(diff.contains("a/a.txt"), "shadow diff should appear: {diff}");
        assert!(diff.contains("-before"), "shadow diff should show removed: {diff}");
        assert!(diff.contains("+after"), "shadow diff should show added: {diff}");
    }

    #[tokio::test]
    async fn get_workspace_diff_without_thread_id_falls_back_to_git() {
        // No thread_id → use the git fallback path. In a non-git dir
        // that fallback returns empty (not an error) because
        // `git::get_workspace_diff` silently handles non-repos.
        let h = make_ipc_harness("shadow").await;
        let diff = get_workspace_diff_inner(
            &h.session,
            &h.workspace_path.to_string_lossy(),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(diff.is_empty(), "git fallback in non-git dir should be empty");
    }

    #[tokio::test]
    async fn get_changed_files_with_thread_id_returns_porcelain_shape() {
        let h = make_ipc_harness("shadow").await;
        let file = h.workspace_path.join("new.txt");
        let shadow = h.session.lock().await.shadow_tracker();
        shadow
            .record_pre_edit(&h.thread_id, &h.workspace_path, &file)
            .await
            .unwrap();
        std::fs::write(&file, b"fresh").unwrap();

        let files = get_changed_files_inner(
            &h.session,
            &h.workspace_path.to_string_lossy(),
            Some(&h.thread_id),
        )
        .await
        .unwrap();

        // Expect a single " A new.txt" entry — porcelain shape the
        // frontend consumes via parseGitStatus.
        assert_eq!(files.len(), 1);
        assert_eq!(files[0], " A new.txt");
    }

    #[tokio::test]
    async fn get_file_diff_with_thread_id_scopes_to_one_file() {
        let h = make_ipc_harness("shadow").await;
        let a = h.workspace_path.join("a.txt");
        let b = h.workspace_path.join("b.txt");
        std::fs::write(&a, "old-a\n").unwrap();
        std::fs::write(&b, "old-b\n").unwrap();

        let shadow = h.session.lock().await.shadow_tracker();
        shadow.record_pre_edit(&h.thread_id, &h.workspace_path, &a).await.unwrap();
        shadow.record_pre_edit(&h.thread_id, &h.workspace_path, &b).await.unwrap();
        std::fs::write(&a, "new-a\n").unwrap();
        std::fs::write(&b, "new-b\n").unwrap();

        let diff = get_file_diff_inner(
            &h.session,
            &h.workspace_path.to_string_lossy(),
            &a.to_string_lossy(),
            Some(&h.thread_id),
        )
        .await
        .unwrap();

        // Scoped filter should include a.txt but exclude b.txt.
        assert!(diff.contains("a/a.txt"), "should include a.txt: {diff}");
        assert!(!diff.contains("a/b.txt"), "should not include b.txt: {diff}");
    }

    // --- Memory visibility: persistence + decode roundtrip ---

    /// Round-trip helper: writes a thread row with injected/extracted memory
    /// JSON, then reads it back through the same decode path used by
    /// list_threads/list_all_threads.
    fn decoded_thread(conn: &rusqlite::Connection, tid: &str) -> ThreadInfo {
        let sql = format!("SELECT {THREAD_COLUMNS} FROM threads WHERE id = ?1");
        conn.query_row(&sql, rusqlite::params![tid], decode_thread_row).unwrap()
    }

    fn make_mem(id: &str, mtype: &str, content: &str) -> MemoryInfo {
        MemoryInfo {
            id: id.to_string(),
            workspace_id: Some("ws1".to_string()),
            memory_type: mtype.to_string(),
            content: content.to_string(),
            source_thread_id: "t1".to_string(),
            pinned: false,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_decode_thread_row_null_memory_columns_roundtrip_as_none() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/null-mem");
        insert_test_thread(&conn, "t1", "ws1", None);

        let thread = decoded_thread(&conn, "t1");
        assert!(thread.injected_memories.is_none());
        assert!(thread.injected_briefing.is_none());
        assert!(thread.extracted_memories.is_none());
    }

    #[test]
    fn test_decode_thread_row_parses_persisted_injected_memories() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/inj-mem");
        insert_test_thread(&conn, "t1", "ws1", None);

        let mems = vec![
            make_mem("m1", "decision", "always use rustls"),
            make_mem("m2", "preference", "prefer async"),
        ];
        let json = serde_json::to_string(&mems).unwrap();
        conn.execute(
            "UPDATE threads SET injected_memories = ?1, injected_briefing = ?2 WHERE id = 't1'",
            rusqlite::params![json, "be concise"],
        )
        .unwrap();

        let thread = decoded_thread(&conn, "t1");
        let injected = thread.injected_memories.expect("injected should decode");
        assert_eq!(injected.len(), 2);
        assert_eq!(injected[0].id, "m1");
        assert_eq!(injected[0].memory_type, "decision");
        assert_eq!(injected[1].content, "prefer async");
        assert_eq!(thread.injected_briefing.as_deref(), Some("be concise"));
    }

    #[test]
    fn test_decode_thread_row_parses_extracted_memories() {
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/ext-mem");
        insert_test_thread(&conn, "t1", "ws1", None);

        let mems = vec![make_mem("m1", "pattern", "remembered from this run")];
        let json = serde_json::to_string(&mems).unwrap();
        conn.execute(
            "UPDATE threads SET extracted_memories = ?1 WHERE id = 't1'",
            rusqlite::params![json],
        )
        .unwrap();

        let thread = decoded_thread(&conn, "t1");
        let extracted = thread.extracted_memories.expect("extracted should decode");
        assert_eq!(extracted.len(), 1);
        assert_eq!(extracted[0].memory_type, "pattern");
    }

    #[test]
    fn test_decode_thread_row_tolerates_malformed_json() {
        // Future-proofing: if the column somehow contains non-array JSON or
        // truncated bytes (e.g. from a bad migration), the UI must not crash
        // — decode should degrade to None rather than propagating the error.
        let conn = setup_test_db();
        insert_test_workspace(&conn, "ws1", "/tmp/bad-mem");
        insert_test_thread(&conn, "t1", "ws1", None);
        conn.execute(
            "UPDATE threads SET injected_memories = ?1 WHERE id = 't1'",
            rusqlite::params!["{not valid"],
        )
        .unwrap();

        let thread = decoded_thread(&conn, "t1");
        assert!(thread.injected_memories.is_none());
    }

    // --- Briefing preview truncation ---

    #[test]
    fn test_truncate_briefing_short_is_unchanged() {
        let s = "hello world";
        assert_eq!(truncate_briefing(s), s);
    }

    #[test]
    fn test_truncate_briefing_at_exact_limit_is_unchanged() {
        let s = "a".repeat(500);
        assert_eq!(truncate_briefing(&s), s);
        assert!(!truncate_briefing(&s).ends_with('…'));
    }

    #[test]
    fn test_truncate_briefing_long_gets_ellipsis() {
        let s = "a".repeat(600);
        let out = truncate_briefing(&s);
        assert!(out.ends_with('…'));
        // 500 'a' chars + one '…' = 501 chars total
        assert_eq!(out.chars().count(), 501);
    }

    #[test]
    fn test_truncate_briefing_handles_multibyte_without_splitting() {
        // "🚀" is a 4-byte char but one `char`. A naive `&s[..500]` byte
        // slice would panic on multi-byte boundaries; this asserts the
        // char-based impl stays safe.
        let s = "🚀".repeat(600);
        let out = truncate_briefing(&s);
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().count(), 501);
    }
}
