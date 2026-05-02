use crate::actions;
use crate::executor;
use crate::types::{NotifierRef, Routine, ScheduleAction};
use chrono::{DateTime, Utc};
use cron::Schedule;
use panes_core::db::DbHandle;
use panes_core::session::SessionManager;
use panes_events::{AgentEvent, ThreadEvent};
use panes_memory::manager::MemoryManager;
use rusqlite::params;
use std::str::FromStr;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

const TICK_INTERVAL_SECS: u64 = 60;

pub async fn run_ticker(
    db: DbHandle,
    session_manager: Arc<Mutex<SessionManager>>,
    memory_manager: Arc<MemoryManager>,
    cancel_token: CancellationToken,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(TICK_INTERVAL_SECS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                info!("scheduler ticker shutting down");
                break;
            }
            _ = interval.tick() => {
                if let Err(e) = tick(&db, &session_manager, &memory_manager).await {
                    error!(error = %e, "scheduler tick failed");
                }
            }
        }
    }
}

async fn tick(
    db: &DbHandle,
    session_manager: &Arc<Mutex<SessionManager>>,
    memory_manager: &Arc<MemoryManager>,
) -> anyhow::Result<()> {
    let now = Utc::now();
    let routines = load_enabled_routines(db).await?;

    for routine in routines {
        if is_due(&routine, now) {
            info!(routine_id = %routine.id, prompt = %routine.prompt, "routine is due, firing");
            if let Err(e) =
                executor::fire_routine(&routine, db, session_manager, memory_manager, false).await
            {
                warn!(routine_id = %routine.id, error = %e, "failed to fire routine");
            }
        }
    }

    Ok(())
}

pub fn is_due(routine: &Routine, now: DateTime<Utc>) -> bool {
    let schedule = match Schedule::from_str(&routine.cron_expr) {
        Ok(s) => s,
        Err(e) => {
            warn!(routine_id = %routine.id, cron = %routine.cron_expr, error = %e, "invalid cron expression");
            return false;
        }
    };

    let after = routine.last_run_at.unwrap_or(routine.created_at);
    match schedule.after(&after).next() {
        Some(next_fire) => next_fire <= now,
        None => false,
    }
}

async fn load_enabled_routines(db: &DbHandle) -> anyhow::Result<Vec<Routine>> {
    db.execute(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, prompt, cron_expr, budget_cap,
                    on_complete, on_failure, enabled, last_run_at, created_at
             FROM routines WHERE enabled = 1",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(RoutineRow {
                id: row.get(0)?,
                workspace_id: row.get(1)?,
                prompt: row.get(2)?,
                cron_expr: row.get(3)?,
                budget_cap: row.get(4)?,
                on_complete: row.get(5)?,
                on_failure: row.get(6)?,
                enabled: row.get(7)?,
                last_run_at: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?;

        let mut routines = Vec::new();
        for row in rows {
            let row = row?;
            routines.push(Routine {
                id: row.id,
                workspace_id: row.workspace_id,
                prompt: row.prompt,
                cron_expr: row.cron_expr,
                budget_cap: row.budget_cap,
                on_complete: serde_json::from_str(&row.on_complete)
                    .unwrap_or(ScheduleAction::Notify),
                on_failure: serde_json::from_str(&row.on_failure)
                    .unwrap_or(ScheduleAction::Notify),
                enabled: row.enabled,
                last_run_at: row
                    .last_run_at
                    .and_then(|s: String| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                created_at: DateTime::parse_from_rfc3339(&row.created_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            });
        }
        Ok(routines)
    })
    .await
}

struct RoutineRow {
    id: String,
    workspace_id: String,
    prompt: String,
    cron_expr: String,
    budget_cap: Option<f64>,
    on_complete: String,
    on_failure: String,
    enabled: bool,
    last_run_at: Option<String>,
    created_at: String,
}

pub async fn run_completion_monitor(
    db: DbHandle,
    session_manager: Arc<Mutex<SessionManager>>,
    memory_manager: Arc<MemoryManager>,
    notifier: NotifierRef,
    mut rx: broadcast::Receiver<ThreadEvent>,
    cancel_token: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                info!("completion monitor shutting down");
                break;
            }
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        handle_thread_event(&db, &session_manager, &memory_manager, &notifier, &event).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!(skipped = n, "completion monitor lagged behind broadcast");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        info!("broadcast channel closed, completion monitor exiting");
                        break;
                    }
                }
            }
        }
    }
}

async fn handle_thread_event(
    db: &DbHandle,
    session_manager: &Arc<Mutex<SessionManager>>,
    memory_manager: &Arc<MemoryManager>,
    notifier: &NotifierRef,
    event: &ThreadEvent,
) {
    let thread_id = event.thread_id.clone();

    let routine_info = {
        let tid = thread_id.clone();
        db.execute(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT t.routine_id, r.on_complete, r.on_failure
                     FROM threads t
                     LEFT JOIN routines r ON t.routine_id = r.id
                     WHERE t.id = ?1 AND t.is_routine = 1",
                    params![tid],
                    |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .ok())
        })
        .await
        .ok()
        .flatten()
    };

    let (routine_id, on_complete_json, on_failure_json) = match routine_info {
        Some((Some(rid), oc, of)) => (rid, oc, of),
        _ => return,
    };

    match &event.event {
        AgentEvent::Complete {
            total_cost_usd, ..
        } => {
            let rid = routine_id.clone();
            let tid = thread_id.clone();
            let cost = *total_cost_usd;
            let now = Utc::now().to_rfc3339();
            let _ = db
                .execute(move |conn| {
                    conn.execute(
                        "UPDATE routine_executions SET status = 'completed', cost_usd = ?1, completed_at = ?2
                         WHERE routine_id = ?3 AND thread_id = ?4 AND status = 'running'",
                        params![cost, now, rid, tid],
                    )?;
                    Ok(())
                })
                .await;

            let on_complete: ScheduleAction = on_complete_json
                .and_then(|s: String| serde_json::from_str(&s).ok())
                .unwrap_or(ScheduleAction::Notify);

            if let Err(e) = actions::dispatch_action(
                &on_complete,
                &routine_id,
                &thread_id,
                None,
                db,
                session_manager,
                memory_manager,
                notifier,
            )
            .await
            {
                warn!(routine_id = %routine_id, error = %e, "on_complete action failed");
            }
        }
        AgentEvent::Error { message, .. } => {
            let rid = routine_id.clone();
            let tid = thread_id.clone();
            let msg = message.clone();
            let now = Utc::now().to_rfc3339();

            let is_budget = message.contains("budget") || message.contains("Budget");
            let status = if is_budget {
                "budget_exceeded"
            } else {
                "failed"
            };

            let status_str = status.to_string();
            let _ = db
                .execute(move |conn| {
                    conn.execute(
                        "UPDATE routine_executions SET status = ?1, error_message = ?2, completed_at = ?3
                         WHERE routine_id = ?4 AND thread_id = ?5 AND status = 'running'",
                        params![status_str, msg, now, rid, tid],
                    )?;
                    Ok(())
                })
                .await;

            let on_failure: ScheduleAction = on_failure_json
                .and_then(|s: String| serde_json::from_str(&s).ok())
                .unwrap_or(ScheduleAction::Notify);

            if let Err(e) = actions::dispatch_action(
                &on_failure,
                &routine_id,
                &thread_id,
                Some(message),
                db,
                session_manager,
                memory_manager,
                notifier,
            )
            .await
            {
                warn!(routine_id = %routine_id, error = %e, "on_failure action failed");
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Routine;
    use chrono::{Duration, Utc};

    fn make_routine(cron_expr: &str, last_run_at: Option<DateTime<Utc>>) -> Routine {
        Routine {
            id: "r-1".to_string(),
            workspace_id: "ws-1".to_string(),
            prompt: "test".to_string(),
            cron_expr: cron_expr.to_string(),
            budget_cap: None,
            on_complete: ScheduleAction::Notify,
            on_failure: ScheduleAction::Notify,
            enabled: true,
            last_run_at,
            created_at: Utc::now() - Duration::hours(2),
        }
    }

    #[test]
    fn test_is_due_never_run_hourly() {
        let routine = make_routine("0 0 * * * *", None);
        assert!(is_due(&routine, Utc::now()));
    }

    #[test]
    fn test_is_due_recently_run() {
        let now = Utc::now();
        let routine = make_routine("0 0 * * * *", Some(now));
        assert!(!is_due(&routine, now));
    }

    #[test]
    fn test_is_due_overdue() {
        let now = Utc::now();
        let routine = make_routine("0 0 * * * *", Some(now - Duration::hours(2)));
        assert!(is_due(&routine, now));
    }

    #[test]
    fn test_is_due_invalid_cron() {
        let routine = make_routine("not a cron", None);
        assert!(!is_due(&routine, Utc::now()));
    }
}
