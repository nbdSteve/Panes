use crate::types::{ExecutionStatus, Routine};
use chrono::Utc;
use panes_core::db::DbHandle;
use panes_core::error::PanesError;
use panes_core::session::{SessionManager, Workspace};
use panes_events::SessionContext;
use panes_memory::manager::MemoryManager;
use rusqlite::params;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;
use uuid::Uuid;

pub async fn fire_routine(
    routine: &Routine,
    db: &DbHandle,
    session_manager: &Arc<Mutex<SessionManager>>,
    memory_manager: &Arc<MemoryManager>,
    is_retry: bool,
) -> anyhow::Result<()> {
    let ws_row = load_workspace_row(db, &routine.workspace_id).await?;

    let workspace = Workspace {
        id: routine.workspace_id.clone(),
        path: PathBuf::from(&ws_row.path),
        name: ws_row.name.clone(),
        default_agent: ws_row.default_agent.clone(),
        budget_cap: routine.budget_cap.or(ws_row.budget_cap),
    };

    let injected = panes_memory::build_context(
        memory_manager.as_memory_store(),
        memory_manager.as_briefing_store(),
        &routine.prompt,
        &routine.workspace_id,
        2000,
    )
    .await
    .unwrap_or_default();

    let context = SessionContext {
        briefing: injected.briefing,
        memories: injected.memories.iter().map(|m| m.content.clone()).collect(),
        budget_cap: routine.budget_cap,
    };

    let agent_name = workspace
        .default_agent
        .as_deref()
        .unwrap_or("claude-code");

    let sm = session_manager.lock().await;
    let result = sm
        .start_thread(&workspace, &routine.prompt, agent_name, context, None)
        .await;

    let thread_id = match result {
        Ok(tid) => tid,
        Err(PanesError::WorkspaceOccupied { .. }) => {
            info!(
                routine_id = %routine.id,
                workspace_id = %routine.workspace_id,
                "workspace busy, skipping routine execution"
            );
            record_execution(
                db,
                &routine.id,
                None,
                ExecutionStatus::SkippedWorkspaceBusy,
                None,
            )
            .await?;
            return Ok(());
        }
        Err(e) => return Err(anyhow::anyhow!("{e}")),
    };

    drop(sm);

    let status = if is_retry {
        ExecutionStatus::Retrying
    } else {
        ExecutionStatus::Running
    };
    record_execution(db, &routine.id, Some(&thread_id), status, None).await?;

    let rid = routine.id.clone();
    let tid = thread_id.clone();
    let now = Utc::now().to_rfc3339();
    db.execute(move |conn| {
        conn.execute(
            "UPDATE threads SET is_routine = 1, routine_id = ?1 WHERE id = ?2",
            params![rid, tid],
        )?;
        Ok(())
    })
    .await?;

    let rid2 = routine.id.clone();
    db.execute(move |conn| {
        conn.execute(
            "UPDATE routines SET last_run_at = ?1 WHERE id = ?2",
            params![now, rid2],
        )?;
        Ok(())
    })
    .await?;

    info!(routine_id = %routine.id, thread_id = %thread_id, "routine thread started");
    Ok(())
}

pub(crate) async fn record_execution(
    db: &DbHandle,
    routine_id: &str,
    thread_id: Option<&str>,
    status: ExecutionStatus,
    error_message: Option<&str>,
) -> anyhow::Result<()> {
    let id = Uuid::new_v4().to_string();
    let rid = routine_id.to_string();
    let tid = thread_id.map(|s| s.to_string());
    let status_str = status.to_string();
    let err_msg = error_message.map(|s| s.to_string());
    let now = Utc::now().to_rfc3339();
    let completed_at = if status != ExecutionStatus::Running && status != ExecutionStatus::Retrying {
        Some(now.clone())
    } else {
        None
    };

    db.execute(move |conn| {
        conn.execute(
            "INSERT INTO routine_executions (id, routine_id, thread_id, status, started_at, completed_at, error_message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, rid, tid, status_str, now, completed_at, err_msg],
        )?;
        Ok(())
    })
    .await
}

struct WorkspaceDbRow {
    path: String,
    name: String,
    default_agent: Option<String>,
    budget_cap: Option<f64>,
}

async fn load_workspace_row(
    db: &DbHandle,
    workspace_id: &str,
) -> anyhow::Result<WorkspaceDbRow> {
    let wid = workspace_id.to_string();
    db.execute(move |conn| {
        conn.query_row(
            "SELECT path, name, default_agent, budget_cap FROM workspaces WHERE id = ?1",
            params![wid],
            |row| {
                Ok(WorkspaceDbRow {
                    path: row.get(0)?,
                    name: row.get(1)?,
                    default_agent: row.get(2)?,
                    budget_cap: row.get(3)?,
                })
            },
        )
        .map_err(|e| anyhow::anyhow!("workspace not found: {e}"))
    })
    .await
}
