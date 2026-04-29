use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use futures::StreamExt;
use panes_adapters::{AgentAdapter, AgentSession};
use panes_cost::{self, CostTracker};
use panes_events::{AgentEvent, SessionContext, ThreadEvent};
use rusqlite::Connection;
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};
use uuid::Uuid;

use crate::git;

#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub default_agent: Option<String>,
    pub budget_cap: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadStatus {
    Pending,
    Running,
    Gate,
    Completed,
    Error,
    Interrupted,
}

impl std::fmt::Display for ThreadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThreadStatus::Pending => write!(f, "pending"),
            ThreadStatus::Running => write!(f, "running"),
            ThreadStatus::Gate => write!(f, "gate"),
            ThreadStatus::Completed => write!(f, "completed"),
            ThreadStatus::Error => write!(f, "error"),
            ThreadStatus::Interrupted => write!(f, "interrupted"),
        }
    }
}

struct ActiveThread {
    #[allow(dead_code)]
    workspace_id: String,
    session: Box<dyn AgentSession>,
    snapshot: Option<git::SnapshotRef>,
}

pub struct SessionManager {
    active_threads: Arc<Mutex<HashMap<String, ActiveThread>>>,
    session_ids: Arc<Mutex<HashMap<String, String>>>,
    adapters: HashMap<String, Arc<dyn AgentAdapter>>,
    cost_tracker: Arc<CostTracker>,
    event_tx: mpsc::UnboundedSender<ThreadEvent>,
    db: Arc<std::sync::Mutex<Connection>>,
}

impl SessionManager {
    pub fn new(
        cost_tracker: Arc<CostTracker>,
        event_tx: mpsc::UnboundedSender<ThreadEvent>,
        db: Arc<std::sync::Mutex<Connection>>,
    ) -> Self {
        let session_ids = Self::load_session_ids(&db);

        Self {
            active_threads: Arc::new(Mutex::new(HashMap::new())),
            session_ids: Arc::new(Mutex::new(session_ids)),
            adapters: HashMap::new(),
            cost_tracker,
            event_tx,
            db,
        }
    }

    fn load_session_ids(db: &Arc<std::sync::Mutex<Connection>>) -> HashMap<String, String> {
        let mut map = HashMap::new();
        if let Ok(conn) = db.lock() {
            if let Ok(mut stmt) = conn.prepare(
                "SELECT id, session_id FROM threads WHERE session_id IS NOT NULL"
            ) {
                if let Ok(rows) = stmt.query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                }) {
                    for row in rows.flatten() {
                        map.insert(row.0, row.1);
                    }
                }
            }
        }
        if !map.is_empty() {
            info!(count = map.len(), "restored session_ids from database");
        }
        map
    }

    pub fn register_adapter(&mut self, adapter: Arc<dyn AgentAdapter>) {
        self.adapters.insert(adapter.name().to_string(), adapter);
    }

    pub async fn start_thread(
        &self,
        workspace: &Workspace,
        prompt: &str,
        agent_name: &str,
        context: SessionContext,
    ) -> Result<String> {
        let adapter = self
            .adapters
            .get(agent_name)
            .with_context(|| format!("unknown agent: {agent_name}"))?
            .clone();

        // Git snapshot
        let snapshot = if git::is_git_repo(&workspace.path).await {
            let thread_id_preview = "pending";
            match git::snapshot(&workspace.path, thread_id_preview).await {
                Ok(s) => Some(s),
                Err(e) => {
                    warn!(error = %e, "failed to create git snapshot — continuing without rollback");
                    None
                }
            }
        } else {
            warn!(workspace = %workspace.path.display(), "not a git repo — rollback unavailable");
            None
        };

        let thread_id = Uuid::new_v4().to_string();

        // Spawn agent session
        let session = adapter
            .spawn(&workspace.path, prompt, &context)
            .await
            .context("failed to spawn agent session")?;

        let session_id = session.init().session_id.clone();

        // Store the claude session_id for resume
        {
            let mut sids = self.session_ids.lock().await;
            sids.insert(thread_id.clone(), session_id.clone());
        }

        // Persist thread to SQLite
        {
            let now = Utc::now().to_rfc3339();
            if let Ok(conn) = self.db.lock() {
                let _ = conn.execute(
                    "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, session_id, started_at, created_at)
                     VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?6)",
                    rusqlite::params![thread_id, workspace.id, agent_name, prompt, session_id, now],
                );
            }
        }

        self.cost_tracker
            .start_tracking(&thread_id, &workspace.id);

        let active_thread = ActiveThread {
            workspace_id: workspace.id.clone(),
            session,
            snapshot,
        };

        {
            let mut active = self.active_threads.lock().await;
            active.insert(thread_id.clone(), active_thread);
        }

        // Take the event stream out of the session while it's in the map.
        // This lets consume_events own the stream without removing the session,
        // so approve/reject/cancel can still find it.
        let event_stream = {
            let mut active = self.active_threads.lock().await;
            let thread = active.get_mut(&thread_id).expect("just inserted");
            thread.session.events()
        };

        let thread_id_clone = thread_id.clone();
        let event_tx = self.event_tx.clone();
        let cost_tracker = self.cost_tracker.clone();
        let active_threads = self.active_threads.clone();
        let budget_cap = workspace.budget_cap;
        let db = self.db.clone();

        tokio::spawn(async move {
            Self::consume_events(
                thread_id_clone,
                event_tx,
                cost_tracker,
                active_threads,
                budget_cap,
                event_stream,
                db,
            )
            .await;
        });

        info!(thread_id = %thread_id, agent = agent_name, "thread started");
        Ok(thread_id)
    }

    pub async fn resume_thread(
        &self,
        thread_id: &str,
        workspace: &Workspace,
        prompt: &str,
        agent_name: &str,
    ) -> Result<()> {
        let adapter = self
            .adapters
            .get(agent_name)
            .with_context(|| format!("unknown agent: {agent_name}"))?
            .clone();

        let claude_session_id = {
            let sids = self.session_ids.lock().await;
            sids.get(thread_id)
                .cloned()
                .with_context(|| format!("no session_id for thread {thread_id}"))?
        };

        let session = adapter
            .resume(&workspace.path, &claude_session_id, prompt)
            .await
            .context("failed to resume agent session")?;

        // Update stored session_id in case it changed
        let new_session_id = session.init().session_id.clone();
        {
            let mut sids = self.session_ids.lock().await;
            sids.insert(thread_id.to_string(), new_session_id.clone());
        }
        {
            if let Ok(conn) = self.db.lock() {
                let _ = conn.execute(
                    "UPDATE threads SET session_id = ?1, status = 'running' WHERE id = ?2",
                    rusqlite::params![new_session_id, thread_id],
                );
            }
        }

        let active_thread = ActiveThread {
            workspace_id: workspace.id.clone(),
            session,
            snapshot: None,
        };

        {
            let mut active = self.active_threads.lock().await;
            active.insert(thread_id.to_string(), active_thread);
        }

        let event_stream = {
            let mut active = self.active_threads.lock().await;
            let thread = active.get_mut(thread_id).expect("just inserted");
            thread.session.events()
        };

        let thread_id_clone = thread_id.to_string();
        let event_tx = self.event_tx.clone();
        let cost_tracker = self.cost_tracker.clone();
        let active_threads = self.active_threads.clone();
        let budget_cap = workspace.budget_cap;
        let db = self.db.clone();

        tokio::spawn(async move {
            Self::consume_events(
                thread_id_clone,
                event_tx,
                cost_tracker,
                active_threads,
                budget_cap,
                event_stream,
                db,
            )
            .await;
        });

        info!(thread_id = %thread_id, "thread resumed");
        Ok(())
    }

    async fn consume_events(
        thread_id: String,
        event_tx: mpsc::UnboundedSender<ThreadEvent>,
        cost_tracker: Arc<CostTracker>,
        active_threads: Arc<Mutex<HashMap<String, ActiveThread>>>,
        budget_cap: Option<f64>,
        mut events_stream: std::pin::Pin<Box<dyn futures::Stream<Item = AgentEvent> + Send>>,
        db: Arc<std::sync::Mutex<Connection>>,
    ) {
        let mut final_status = "completed";

        while let Some(event) = events_stream.next().await {
            cost_tracker.process_event(&thread_id, &event);

            if let Some(cap) = budget_cap {
                if cost_tracker.check_budget(&thread_id, cap) {
                    warn!(thread_id = %thread_id, cap, "budget cap exceeded — killing session");
                    {
                        let active = active_threads.lock().await;
                        if let Some(thread) = active.get(&thread_id) {
                            let _ = thread.session.cancel().await;
                        }
                    }
                    let error_event = AgentEvent::Error {
                        message: format!("Budget cap of ${cap:.2} exceeded. Session terminated."),
                        recoverable: false,
                    };
                    Self::persist_event(&db, &thread_id, &error_event);
                    let _ = event_tx.send(ThreadEvent {
                        thread_id: thread_id.clone(),
                        timestamp: Utc::now(),
                        event: error_event,
                        parent_tool_use_id: None,
                    });
                    final_status = "error";
                    break;
                }
            }

            Self::persist_event(&db, &thread_id, &event);

            // Update thread status for gates
            if matches!(&event, AgentEvent::ToolRequest { needs_approval: true, .. }) {
                if let Ok(conn) = db.lock() {
                    let _ = conn.execute(
                        "UPDATE threads SET status = 'gate' WHERE id = ?1",
                        rusqlite::params![thread_id],
                    );
                }
            }

            let thread_event = ThreadEvent {
                thread_id: thread_id.clone(),
                timestamp: Utc::now(),
                event: event.clone(),
                parent_tool_use_id: None,
            };

            if event_tx.send(thread_event).is_err() {
                break;
            }

            match &event {
                AgentEvent::Complete { summary, total_cost_usd, duration_ms, .. } => {
                    if let Ok(conn) = db.lock() {
                        let now = Utc::now().to_rfc3339();
                        let _ = conn.execute(
                            "UPDATE threads SET status = 'completed', summary = ?1, cost_usd = ?2, duration_ms = ?3, completed_at = ?4 WHERE id = ?5",
                            rusqlite::params![summary, total_cost_usd, *duration_ms as i64, now, thread_id],
                        );
                    }
                    final_status = "completed";
                    break;
                }
                AgentEvent::Error { recoverable: false, .. } => {
                    final_status = "error";
                    break;
                }
                _ => {}
            }
        }

        // Update final status if we exited without a Complete event
        if final_status == "error" {
            if let Ok(conn) = db.lock() {
                let _ = conn.execute(
                    "UPDATE threads SET status = 'error' WHERE id = ?1",
                    rusqlite::params![thread_id],
                );
            }
        }

        // Finalize cost and persist to SQLite
        if let Some(cost) = cost_tracker.finalize(&thread_id) {
            info!(thread_id = %thread_id, cost = cost.total_usd, "thread completed — persisting cost");
            if let Ok(conn) = db.lock() {
                if let Err(e) = panes_cost::save_cost(&conn, &cost) {
                    warn!(error = %e, "failed to persist cost to SQLite");
                }
            }
        }

        // Clean up — session is done, remove from active map
        // Keep session_ids so the thread can be resumed later
        let mut active = active_threads.lock().await;
        active.remove(&thread_id);
    }

    fn persist_event(db: &Arc<std::sync::Mutex<Connection>>, thread_id: &str, event: &AgentEvent) {
        if let Ok(conn) = db.lock() {
            let event_type = match event {
                AgentEvent::Thinking { .. } => "thinking",
                AgentEvent::Text { .. } => "text",
                AgentEvent::ToolRequest { .. } => "tool_request",
                AgentEvent::ToolResult { .. } => "tool_result",
                AgentEvent::CostUpdate { .. } => "cost_update",
                AgentEvent::Error { .. } => "error",
                AgentEvent::SubAgentSpawned { .. } => "sub_agent_spawned",
                AgentEvent::SubAgentComplete { .. } => "sub_agent_complete",
                AgentEvent::Complete { .. } => "complete",
            };
            let data = serde_json::to_string(event).unwrap_or_default();
            let now = Utc::now().to_rfc3339();
            let _ = conn.execute(
                "INSERT INTO events (thread_id, event_type, timestamp, data) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![thread_id, event_type, now, data],
            );
        }
    }

    pub async fn approve(&self, thread_id: &str, tool_use_id: &str) -> Result<()> {
        let active = self.active_threads.lock().await;
        let thread = active
            .get(thread_id)
            .context("thread not found")?;
        thread.session.approve(tool_use_id).await
    }

    pub async fn reject(&self, thread_id: &str, tool_use_id: &str, reason: &str) -> Result<()> {
        let active = self.active_threads.lock().await;
        let thread = active
            .get(thread_id)
            .context("thread not found")?;
        thread.session.reject(tool_use_id, reason).await
    }

    pub async fn cancel(&self, thread_id: &str) -> Result<()> {
        let active = self.active_threads.lock().await;
        if let Some(thread) = active.get(thread_id) {
            thread.session.cancel().await?;
        }
        Ok(())
    }

    pub async fn get_snapshot(&self, thread_id: &str) -> Option<git::SnapshotRef> {
        let active = self.active_threads.lock().await;
        active
            .get(thread_id)
            .and_then(|t| t.snapshot.clone())
    }

    pub async fn get_workspace_path(&self, _thread_id: &str) -> Option<PathBuf> {
        None
    }

    pub async fn remove_thread(&self, thread_id: &str) {
        let mut active = self.active_threads.lock().await;
        active.remove(thread_id);
    }
}
