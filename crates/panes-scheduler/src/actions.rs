use crate::executor;
use crate::types::ScheduleAction;
use panes_core::db::DbHandle;
use panes_core::session::SessionManager;
use panes_memory::manager::MemoryManager;
use rusqlite::params;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

pub async fn dispatch_action(
    action: &ScheduleAction,
    routine_id: &str,
    thread_id: &str,
    error_message: Option<&str>,
    db: &DbHandle,
    session_manager: &Arc<Mutex<SessionManager>>,
    memory_manager: &Arc<MemoryManager>,
) -> anyhow::Result<()> {
    match action {
        ScheduleAction::Notify => {
            send_notification(routine_id, thread_id, error_message);
            Ok(())
        }
        ScheduleAction::RetryOnce => {
            let already_retried = check_has_retry(db, routine_id).await?;
            if already_retried {
                info!(routine_id = %routine_id, "already retried once, sending notification instead");
                send_notification(routine_id, thread_id, error_message);
                Ok(())
            } else {
                info!(routine_id = %routine_id, "retrying routine");
                let routine = load_routine(db, routine_id).await?;
                executor::fire_routine(&routine, db, session_manager, memory_manager, true).await
            }
        }
        ScheduleAction::Chain {
            prompt,
            workspace_id,
        } => {
            info!(
                routine_id = %routine_id,
                chain_prompt = %prompt,
                chain_workspace = ?workspace_id,
                "chaining follow-up prompt"
            );
            let source_routine = load_routine(db, routine_id).await?;
            let target_ws = workspace_id
                .clone()
                .unwrap_or_else(|| source_routine.workspace_id.clone());

            let mut chain_routine = source_routine;
            chain_routine.prompt = prompt.clone();
            chain_routine.workspace_id = target_ws;

            executor::fire_routine(&chain_routine, db, session_manager, memory_manager, false).await
        }
    }
}

fn send_notification(routine_id: &str, thread_id: &str, error_message: Option<&str>) {
    // OS notification will be wired in Phase E via tauri-plugin-notification.
    // For now, log the notification event.
    info!(
        routine_id = %routine_id,
        thread_id = %thread_id,
        error = ?error_message,
        "routine notification"
    );
}

async fn check_has_retry(db: &DbHandle, routine_id: &str) -> anyhow::Result<bool> {
    let rid = routine_id.to_string();
    db.execute(move |conn| {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM routine_executions
             WHERE routine_id = ?1 AND status = 'retrying'",
            params![rid],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    })
    .await
}

async fn load_routine(
    db: &DbHandle,
    routine_id: &str,
) -> anyhow::Result<crate::types::Routine> {
    let rid = routine_id.to_string();
    db.execute(move |conn| {
        conn.query_row(
            "SELECT id, workspace_id, prompt, cron_expr, budget_cap,
                    on_complete, on_failure, enabled, last_run_at, created_at
             FROM routines WHERE id = ?1",
            params![rid],
            |row| {
                let on_complete_str: String = row.get(5)?;
                let on_failure_str: String = row.get(6)?;
                let last_run_at: Option<String> = row.get(8)?;
                let created_at: String = row.get(9)?;

                Ok(crate::types::Routine {
                    id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    prompt: row.get(2)?,
                    cron_expr: row.get(3)?,
                    budget_cap: row.get(4)?,
                    on_complete: serde_json::from_str(&on_complete_str)
                        .unwrap_or(ScheduleAction::Notify),
                    on_failure: serde_json::from_str(&on_failure_str)
                        .unwrap_or(ScheduleAction::Notify),
                    enabled: row.get(7)?,
                    last_run_at: last_run_at
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                        .map(|dt| dt.with_timezone(&chrono::Utc)),
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_at)
                        .map(|dt| dt.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                })
            },
        )
        .map_err(|e| anyhow::anyhow!("routine not found: {e}"))
    })
    .await
}
