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
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{info, warn};
use uuid::Uuid;

use crate::git;

#[derive(Debug)]
pub enum GateDecision {
    Continue,
    Abort,
}

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

type GateSender = Arc<Mutex<Option<oneshot::Sender<GateDecision>>>>;

struct ActiveThread {
    #[allow(dead_code)]
    workspace_id: String,
    session: Box<dyn AgentSession>,
    snapshot: Option<git::SnapshotRef>,
    gate_tx: GateSender,
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
            match git::snapshot(&workspace.path).await {
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
            let snapshot_hash = snapshot.as_ref().map(|s| s.commit_hash.clone());
            if let Ok(conn) = self.db.lock() {
                let _ = conn.execute(
                    "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, session_id, snapshot_ref, started_at, created_at)
                     VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?7, ?7)",
                    rusqlite::params![thread_id, workspace.id, agent_name, prompt, session_id, snapshot_hash, now],
                );
            }
        }

        self.cost_tracker
            .start_tracking(&thread_id, &workspace.id);

        let gate_tx: GateSender = Arc::new(Mutex::new(None));

        let active_thread = ActiveThread {
            workspace_id: workspace.id.clone(),
            session,
            snapshot,
            gate_tx: gate_tx.clone(),
        };

        {
            let mut active = self.active_threads.lock().await;
            active.insert(thread_id.clone(), active_thread);
        }

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
                gate_tx,
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

        let gate_tx: GateSender = Arc::new(Mutex::new(None));

        let active_thread = ActiveThread {
            workspace_id: workspace.id.clone(),
            session,
            snapshot: None,
            gate_tx: gate_tx.clone(),
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
                gate_tx,
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
        gate_tx: GateSender,
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

            let is_gate = matches!(&event, AgentEvent::ToolRequest { needs_approval: true, .. });

            if is_gate {
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

            // Gate pausing: wait for user decision before consuming more events
            if is_gate {
                let (tx, rx) = oneshot::channel();
                {
                    let mut slot = gate_tx.lock().await;
                    *slot = Some(tx);
                }

                info!(thread_id = %thread_id, "gate paused — waiting for user decision");

                match rx.await {
                    Ok(GateDecision::Continue) => {
                        info!(thread_id = %thread_id, "gate continued — resuming event stream");
                        if let Ok(conn) = db.lock() {
                            let _ = conn.execute(
                                "UPDATE threads SET status = 'running' WHERE id = ?1",
                                rusqlite::params![thread_id],
                            );
                        }
                    }
                    Ok(GateDecision::Abort) | Err(_) => {
                        info!(thread_id = %thread_id, "gate aborted — killing session");
                        {
                            let active = active_threads.lock().await;
                            if let Some(thread) = active.get(&thread_id) {
                                let _ = thread.session.cancel().await;
                            }
                        }
                        final_status = "interrupted";
                        break;
                    }
                }
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

        if final_status == "error" || final_status == "interrupted" {
            if let Ok(conn) = db.lock() {
                let _ = conn.execute(
                    "UPDATE threads SET status = ?1 WHERE id = ?2",
                    rusqlite::params![final_status, thread_id],
                );
            }
        }

        if let Some(cost) = cost_tracker.finalize(&thread_id) {
            info!(thread_id = %thread_id, cost = cost.total_usd, "thread completed — persisting cost");
            if let Ok(conn) = db.lock() {
                if let Err(e) = panes_cost::save_cost(&conn, &cost) {
                    warn!(error = %e, "failed to persist cost to SQLite");
                }
            }
        }

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

    pub async fn approve(&self, thread_id: &str, _tool_use_id: &str) -> Result<()> {
        let active = self.active_threads.lock().await;
        let thread = active
            .get(thread_id)
            .context("thread not found")?;
        let mut slot = thread.gate_tx.lock().await;
        if let Some(tx) = slot.take() {
            let _ = tx.send(GateDecision::Continue);
        }
        Ok(())
    }

    pub async fn reject(&self, thread_id: &str, _tool_use_id: &str, _reason: &str) -> Result<()> {
        let active = self.active_threads.lock().await;
        let thread = active
            .get(thread_id)
            .context("thread not found")?;
        let mut slot = thread.gate_tx.lock().await;
        if let Some(tx) = slot.take() {
            let _ = tx.send(GateDecision::Abort);
        }
        Ok(())
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

    pub async fn remove_thread(&self, thread_id: &str) {
        let mut active = self.active_threads.lock().await;
        active.remove(thread_id);
    }

    pub fn list_adapters(&self) -> Vec<String> {
        self.adapters.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panes_adapters::fake::{FakeAdapter, FakeScenario};

    fn setup_session_manager() -> (SessionManager, mpsc::UnboundedReceiver<ThreadEvent>) {
        let conn = Connection::open(":memory:").unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        crate::db::run_migrations(&conn).unwrap();
        let db = Arc::new(std::sync::Mutex::new(conn));
        let cost_tracker = Arc::new(CostTracker::new());
        let (tx, rx) = mpsc::unbounded_channel();
        (SessionManager::new(cost_tracker, tx, db), rx)
    }

    fn make_workspace() -> Workspace {
        Workspace {
            id: "ws-test".to_string(),
            path: std::env::temp_dir(),
            name: "test-workspace".to_string(),
            default_agent: None,
            budget_cap: None,
        }
    }

    #[tokio::test]
    async fn test_start_thread_unknown_agent() {
        let (mgr, _rx) = setup_session_manager();
        let ws = make_workspace();
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = mgr.start_thread(&ws, "hello", "nonexistent-agent", ctx).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown agent"));
    }

    #[tokio::test]
    async fn test_approve_nonexistent_thread() {
        let (mgr, _rx) = setup_session_manager();
        let result = mgr.approve("no-such-thread", "tool1").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("thread not found"));
    }

    #[tokio::test]
    async fn test_reject_nonexistent_thread() {
        let (mgr, _rx) = setup_session_manager();
        let result = mgr.reject("no-such-thread", "tool1", "reason").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("thread not found"));
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_thread_is_ok() {
        let (mgr, _rx) = setup_session_manager();
        let result = mgr.cancel("no-such-thread").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_start_and_complete_with_fake() {
        let (mut mgr, mut rx) = setup_session_manager();
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hello!".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        {
            let conn = mgr.db.lock().unwrap();
            conn.execute(
                "INSERT INTO workspaces (id, path, name, created_at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![ws.id, ws.path.to_string_lossy(), ws.name, "2024-01-01"],
            ).unwrap();
        }

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx).await.unwrap();
        assert!(!thread_id.is_empty());

        let mut got_complete = false;
        while let Some(te) = rx.recv().await {
            if matches!(te.event, AgentEvent::Complete { .. }) {
                got_complete = true;
                break;
            }
        }
        assert!(got_complete);
    }
}
