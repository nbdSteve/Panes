use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use futures::StreamExt;
use panes_adapters::{AgentAdapter, AgentSession};
use panes_cost::{self, CostTracker};
use panes_events::{AgentEvent, SessionContext, ThreadEvent};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{info, warn};
use uuid::Uuid;

use crate::db::DbHandle;
use crate::error::PanesError;
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

/// Drop guard that persists thread costs even if `consume_events` is aborted.
///
/// `try_execute_blocking` calls `blocking_recv()` in Drop, which is safe
/// because the `DbHandle` actor runs on a dedicated `std::thread` (not a
/// tokio task). The `blocking_recv` completes as soon as the actor processes
/// the operation, or returns immediately if the actor has shut down.
struct CostFinalizer {
    thread_id: String,
    cost_tracker: Arc<CostTracker>,
    db: DbHandle,
}

impl Drop for CostFinalizer {
    fn drop(&mut self) {
        if let Some(cost) = self.cost_tracker.finalize(&self.thread_id) {
            let db = self.db.clone();
            let _ = db.try_execute_blocking(move |conn| {
                panes_cost::save_cost(conn, &cost).ok();
                Ok(())
            });
        }
    }
}

struct ActiveThread {
    #[allow(dead_code)]
    workspace_id: String,
    session: Box<dyn AgentSession>,
    snapshot: Option<git::SnapshotRef>,
    gate_tx: GateSender,
}

pub struct SessionManager {
    active_threads: Arc<Mutex<HashMap<String, ActiveThread>>>,
    reservations: Arc<Mutex<HashSet<String>>>,
    session_ids: Arc<Mutex<HashMap<String, String>>>,
    adapters: HashMap<String, Arc<dyn AgentAdapter>>,
    cost_tracker: Arc<CostTracker>,
    event_tx: mpsc::UnboundedSender<ThreadEvent>,
    pub(crate) db: DbHandle,
}

impl SessionManager {
    pub async fn new(
        cost_tracker: Arc<CostTracker>,
        event_tx: mpsc::UnboundedSender<ThreadEvent>,
        db: DbHandle,
    ) -> Self {
        let session_ids = Self::load_session_ids(&db).await;

        Self {
            active_threads: Arc::new(Mutex::new(HashMap::new())),
            reservations: Arc::new(Mutex::new(HashSet::new())),
            session_ids: Arc::new(Mutex::new(session_ids)),
            adapters: HashMap::new(),
            cost_tracker,
            event_tx,
            db,
        }
    }

    async fn load_session_ids(db: &DbHandle) -> HashMap<String, String> {
        let map = db
            .execute(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id FROM threads WHERE session_id IS NOT NULL",
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .filter_map(|r| r.ok())
                    .collect::<HashMap<String, String>>();
                Ok(rows)
            })
            .await
            .unwrap_or_default();
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
        model: Option<&str>,
    ) -> Result<String, PanesError> {
        let adapter = self
            .adapters
            .get(agent_name)
            .ok_or_else(|| PanesError::AdapterNotFound {
                adapter: agent_name.to_string(),
                message: format!("unknown agent: {agent_name}"),
            })?
            .clone();

        {
            let active = self.active_threads.lock().await;
            let mut reserved = self.reservations.lock().await;
            if active.values().any(|t| t.workspace_id == workspace.id)
                || reserved.contains(&workspace.id)
            {
                return Err(PanesError::WorkspaceOccupied {
                    workspace_id: workspace.id.clone(),
                    message: "A thread is already running in this workspace. Wait for it to complete or cancel it first.".to_string(),
                });
            }
            reserved.insert(workspace.id.clone());
        }

        let result = self
            .start_thread_inner(workspace, prompt, agent_name, adapter, context, model)
            .await
            .map_err(PanesError::from);

        if result.is_err() {
            self.reservations.lock().await.remove(&workspace.id);
        }

        result
    }

    async fn start_thread_inner(
        &self,
        workspace: &Workspace,
        prompt: &str,
        agent_name: &str,
        adapter: Arc<dyn AgentAdapter>,
        context: SessionContext,
        model: Option<&str>,
    ) -> Result<String> {
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
            .spawn(&workspace.path, prompt, &context, model)
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
            let tid = thread_id.clone();
            let wid = workspace.id.clone();
            let agent = agent_name.to_string();
            let p = prompt.to_string();
            let sid = session_id.clone();
            let _ = self.db.execute(move |conn| {
                conn.execute(
                    "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, session_id, snapshot_ref, started_at, created_at)
                     VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?7, ?7)",
                    rusqlite::params![tid, wid, agent, p, sid, snapshot_hash, now],
                )?;
                Ok(())
            }).await;
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
            self.reservations.lock().await.remove(&workspace.id);
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

        info!(thread_id = %thread_id, "thread started");
        Ok(thread_id)
    }

    pub async fn resume_thread(
        &self,
        thread_id: &str,
        workspace: &Workspace,
        prompt: &str,
        agent_name: &str,
        model: Option<&str>,
    ) -> Result<(), PanesError> {
        let adapter = self
            .adapters
            .get(agent_name)
            .ok_or_else(|| PanesError::AdapterNotFound {
                adapter: agent_name.to_string(),
                message: format!("unknown agent: {agent_name}"),
            })?
            .clone();

        {
            let active = self.active_threads.lock().await;
            let mut reserved = self.reservations.lock().await;
            if active.contains_key(thread_id) {
                return Err(PanesError::WorkspaceOccupied {
                    workspace_id: workspace.id.clone(),
                    message: format!("thread {thread_id} is still active. Wait for it to complete first."),
                });
            }
            if active.iter().any(|(id, t)| t.workspace_id == workspace.id && id != thread_id)
                || reserved.contains(&workspace.id)
            {
                return Err(PanesError::WorkspaceOccupied {
                    workspace_id: workspace.id.clone(),
                    message: "A thread is already running in this workspace. Wait for it to complete or cancel it first.".to_string(),
                });
            }
            reserved.insert(workspace.id.clone());
        }

        let result = self
            .resume_thread_inner(thread_id, workspace, prompt, adapter, model)
            .await
            .map_err(PanesError::from);

        if result.is_err() {
            self.reservations.lock().await.remove(&workspace.id);
        }

        result
    }

    async fn resume_thread_inner(
        &self,
        thread_id: &str,
        workspace: &Workspace,
        prompt: &str,
        adapter: Arc<dyn AgentAdapter>,
        model: Option<&str>,
    ) -> Result<()> {
        let claude_session_id = {
            let sids = self.session_ids.lock().await;
            sids.get(thread_id)
                .cloned()
                .with_context(|| format!("no session_id for thread {thread_id}"))?
        };

        let session = adapter
            .resume(&workspace.path, &claude_session_id, prompt, model)
            .await
            .context("failed to resume agent session")?;

        // Update stored session_id in case it changed
        let new_session_id = session.init().session_id.clone();
        {
            let mut sids = self.session_ids.lock().await;
            sids.insert(thread_id.to_string(), new_session_id.clone());
        }
        {
            let sid = new_session_id.clone();
            let tid = thread_id.to_string();
            let _ = self.db.execute(move |conn| {
                conn.execute(
                    "UPDATE threads SET session_id = ?1, status = 'running' WHERE id = ?2",
                    rusqlite::params![sid, tid],
                )?;
                Ok(())
            }).await;
        }

        self.cost_tracker
            .start_tracking(thread_id, &workspace.id);

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
            self.reservations.lock().await.remove(&workspace.id);
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
        db: DbHandle,
        gate_tx: GateSender,
    ) {
        let _cost_guard = CostFinalizer {
            thread_id: thread_id.clone(),
            cost_tracker: cost_tracker.clone(),
            db: db.clone(),
        };

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
                    Self::persist_event(&db, &thread_id, &error_event).await;
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

            Self::persist_event(&db, &thread_id, &event).await;

            let gate_tool_id = match &event {
                AgentEvent::ToolRequest { id, needs_approval: true, .. } => Some(id.clone()),
                _ => None,
            };

            if gate_tool_id.is_some() {
                let tid = thread_id.clone();
                let _ = db.execute(move |conn| {
                    conn.execute(
                        "UPDATE threads SET status = 'gate' WHERE id = ?1",
                        rusqlite::params![tid],
                    )?;
                    Ok(())
                }).await;
            }

            // Set up gate oneshot BEFORE sending the event so approve/reject
            // can find it immediately after the frontend receives the event.
            let gate_rx = if gate_tool_id.is_some() {
                let (tx, rx) = oneshot::channel();
                {
                    let mut slot = gate_tx.lock().await;
                    *slot = Some(tx);
                }
                Some(rx)
            } else {
                None
            };

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
            if let Some(rx) = gate_rx {
                let tool_id = gate_tool_id.unwrap();
                info!(thread_id = %thread_id, "gate paused — waiting for user decision");

                match rx.await {
                    Ok(GateDecision::Continue) => {
                        info!(thread_id = %thread_id, "gate continued — resuming event stream");
                        {
                            let active = active_threads.lock().await;
                            if let Some(thread) = active.get(&thread_id) {
                                thread.session.approve(&tool_id).await.ok();
                            }
                        }
                        let tid = thread_id.clone();
                        let _ = db.execute(move |conn| {
                            conn.execute(
                                "UPDATE threads SET status = 'running' WHERE id = ?1",
                                rusqlite::params![tid],
                            )?;
                            Ok(())
                        }).await;
                    }
                    Ok(GateDecision::Abort) | Err(_) => {
                        info!(thread_id = %thread_id, "gate aborted — killing session");
                        {
                            let active = active_threads.lock().await;
                            if let Some(thread) = active.get(&thread_id) {
                                thread.session.reject(&tool_id, "rejected by user").await.ok();
                                let _ = thread.session.cancel().await;
                            }
                        }
                        let abort_event = AgentEvent::Error {
                            message: "Gate rejected by user".to_string(),
                            recoverable: false,
                        };
                        Self::persist_event(&db, &thread_id, &abort_event).await;
                        let _ = event_tx.send(ThreadEvent {
                            thread_id: thread_id.clone(),
                            timestamp: Utc::now(),
                            event: abort_event,
                            parent_tool_use_id: None,
                        });
                        final_status = "interrupted";
                        break;
                    }
                }
            }

            // threads.cost_usd comes from the Complete event (authoritative per-run
            // cost from the agent). The costs table (CostFinalizer/CostTracker) is an
            // independent audit log — slight divergence is expected and by design.
            match &event {
                AgentEvent::Complete { summary, total_cost_usd, duration_ms, .. } => {
                    let now = Utc::now().to_rfc3339();
                    let s = summary.clone();
                    let cost = *total_cost_usd;
                    let dur = *duration_ms as i64;
                    let tid = thread_id.clone();
                    let _ = db.execute(move |conn| {
                        conn.execute(
                            "UPDATE threads SET status = 'completed', summary = ?1, cost_usd = cost_usd + ?2, duration_ms = ?3, completed_at = ?4 WHERE id = ?5",
                            rusqlite::params![s, cost, dur, now, tid],
                        )?;
                        Ok(())
                    }).await;
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
            let status = final_status.to_string();
            let tid = thread_id.clone();
            let _ = db.execute(move |conn| {
                conn.execute(
                    "UPDATE threads SET status = ?1 WHERE id = ?2",
                    rusqlite::params![status, tid],
                )?;
                Ok(())
            }).await;
        }

        let mut active = active_threads.lock().await;
        active.remove(&thread_id);
    }

    async fn persist_event(db: &DbHandle, thread_id: &str, event: &AgentEvent) {
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
        }
        .to_string();
        let data = serde_json::to_string(event).unwrap_or_default();
        let now = Utc::now().to_rfc3339();
        let tid = thread_id.to_string();
        let _ = db.execute(move |conn| {
            conn.execute(
                "INSERT INTO events (thread_id, event_type, timestamp, data) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![tid, event_type, now, data],
            )?;
            Ok(())
        }).await;
    }

    pub async fn approve(&self, thread_id: &str, _tool_use_id: &str) -> Result<(), PanesError> {
        let active = self.active_threads.lock().await;
        let thread = active
            .get(thread_id)
            .ok_or_else(|| PanesError::ThreadNotFound {
                thread_id: thread_id.to_string(),
                message: "thread not found".to_string(),
            })?;
        let mut slot = thread.gate_tx.lock().await;
        match slot.take() {
            Some(tx) => { let _ = tx.send(GateDecision::Continue); Ok(()) }
            None => Err(PanesError::NoGatePending {
                thread_id: thread_id.to_string(),
                message: format!("no gate pending for thread {thread_id}"),
            }),
        }
    }

    pub async fn reject(&self, thread_id: &str, _tool_use_id: &str, _reason: &str) -> Result<(), PanesError> {
        let active = self.active_threads.lock().await;
        let thread = active
            .get(thread_id)
            .ok_or_else(|| PanesError::ThreadNotFound {
                thread_id: thread_id.to_string(),
                message: "thread not found".to_string(),
            })?;
        let mut slot = thread.gate_tx.lock().await;
        match slot.take() {
            Some(tx) => { let _ = tx.send(GateDecision::Abort); Ok(()) }
            None => Err(PanesError::NoGatePending {
                thread_id: thread_id.to_string(),
                message: format!("no gate pending for thread {thread_id}"),
            }),
        }
    }

    pub async fn cancel(&self, thread_id: &str) -> Result<(), PanesError> {
        let active = self.active_threads.lock().await;
        if let Some(thread) = active.get(thread_id) {
            thread.session.cancel().await
                .map_err(|e| PanesError::Internal { message: e.to_string() })?;
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

    pub async fn list_models(&self, adapter_name: &str) -> Result<Vec<panes_adapters::ModelInfo>, PanesError> {
        let adapter = self
            .adapters
            .get(adapter_name)
            .ok_or_else(|| PanesError::AdapterNotFound {
                adapter: adapter_name.to_string(),
                message: format!("unknown adapter: {adapter_name}"),
            })?;
        adapter.list_models().await
            .map_err(|e| PanesError::Internal { message: e.to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panes_adapters::fake::{FakeAdapter, FakeScenario};

    async fn setup_session_manager() -> (SessionManager, mpsc::UnboundedReceiver<ThreadEvent>) {
        let conn = rusqlite::Connection::open(":memory:").unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        crate::db::run_migrations(&conn).unwrap();
        let db = crate::db::DbHandle::new(conn);
        let cost_tracker = Arc::new(CostTracker::new());
        let (tx, rx) = mpsc::unbounded_channel();
        (SessionManager::new(cost_tracker, tx, db).await, rx)
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

    async fn wait_for_thread_cleanup(mgr: &SessionManager, thread_id: &str, timeout_ms: u64) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if !mgr.active_threads.lock().await.contains_key(thread_id) {
                return;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("thread {} not cleaned up within {}ms", thread_id, timeout_ms);
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    async fn wait_for_db_status(mgr: &SessionManager, thread_id: &str, expected: &str, timeout_ms: u64) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let tid = thread_id.to_string();
            if let Ok(status) = mgr.db.execute(move |conn| {
                Ok(conn.query_row(
                    "SELECT status FROM threads WHERE id = ?1",
                    rusqlite::params![tid],
                    |row| row.get::<_, String>(0),
                )?)
            }).await {
                if status == expected {
                    return;
                }
            }
            if tokio::time::Instant::now() > deadline {
                panic!("thread {} did not reach status '{}' within {}ms", thread_id, expected, timeout_ms);
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn test_start_thread_unknown_agent() {
        let (mgr, _rx) = setup_session_manager().await;
        let ws = make_workspace();
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = mgr.start_thread(&ws, "hello", "nonexistent-agent", ctx, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown agent"));
    }

    #[tokio::test]
    async fn test_start_thread_empty_agent_name_rejected() {
        let (mgr, _rx) = setup_session_manager().await;
        let ws = make_workspace();
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = mgr.start_thread(&ws, "hello", "", ctx, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown agent"));
    }

    #[tokio::test]
    async fn test_resume_thread_empty_agent_name_rejected() {
        let (mgr, _rx) = setup_session_manager().await;
        let ws = make_workspace();
        let result = mgr.resume_thread("t1", &ws, "hello", "", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown agent"));
    }

    #[tokio::test]
    async fn test_approve_nonexistent_thread() {
        let (mgr, _rx) = setup_session_manager().await;
        let result = mgr.approve("no-such-thread", "tool1").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("thread not found"));
    }

    #[tokio::test]
    async fn test_reject_nonexistent_thread() {
        let (mgr, _rx) = setup_session_manager().await;
        let result = mgr.reject("no-such-thread", "tool1", "reason").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("thread not found"));
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_thread_is_ok() {
        let (mgr, _rx) = setup_session_manager().await;
        let result = mgr.cancel("no-such-thread").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_start_and_complete_with_fake() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hello!".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
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

    // ---------------------------------------------------------------
    // Helper: insert workspace row into the DB (needed for FK)
    // ---------------------------------------------------------------
    async fn insert_workspace_row(mgr: &SessionManager, ws: &Workspace) {
        let id = ws.id.clone();
        let path = ws.path.to_string_lossy().to_string();
        let name = ws.name.clone();
        mgr.db
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO workspaces (id, path, name, created_at) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![id, path, name, "2024-01-01"],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    fn make_workspace_with_budget(cap: Option<f64>) -> Workspace {
        Workspace {
            id: "ws-test".to_string(),
            path: std::env::temp_dir(),
            name: "test-workspace".to_string(),
            default_agent: None,
            budget_cap: cap,
        }
    }

    async fn query_thread_status(mgr: &SessionManager, thread_id: &str) -> String {
        let tid = thread_id.to_string();
        mgr.db
            .execute(move |conn| {
                Ok(conn.query_row(
                    "SELECT status FROM threads WHERE id = ?1",
                    rusqlite::params![tid],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap()
    }

    /// Collect all events until Complete, Error, or timeout.
    async fn collect_events_until_done(
        rx: &mut mpsc::UnboundedReceiver<ThreadEvent>,
    ) -> Vec<ThreadEvent> {
        let mut events = vec![];
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(te)) => {
                    let done = matches!(
                        &te.event,
                        AgentEvent::Complete { .. } | AgentEvent::Error { recoverable: false, .. }
                    );
                    events.push(te);
                    if done {
                        break;
                    }
                }
                _ => break,
            }
        }
        events
    }

    /// Wait for a gated ToolRequest and return its tool_use_id.
    async fn wait_for_gate_event(
        rx: &mut mpsc::UnboundedReceiver<ThreadEvent>,
    ) -> String {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(te)) => {
                    if let AgentEvent::ToolRequest {
                        id,
                        needs_approval: true,
                        ..
                    } = &te.event
                    {
                        return id.clone();
                    }
                }
                _ => panic!("timed out waiting for gate event"),
            }
        }
    }

    // ---------------------------------------------------------------
    // A gate-compatible test adapter whose event stream does NOT
    // have its own internal pausing — it freely yields all events.
    // This lets us test SessionManager's gate logic in isolation
    // without fighting FakeSession's gate_notify mechanism.
    // ---------------------------------------------------------------
    mod gate_test_adapter {
        use std::path::Path;
        use std::pin::Pin;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        use anyhow::Result;
        use async_trait::async_trait;
        use futures::Stream;
        use panes_events::{AgentEvent, RiskLevel, SessionContext, SessionInit};
        use tokio::sync::Notify;
        use futures::stream::unfold;

        use panes_adapters::{AgentAdapter, AgentSession};

        /// An adapter that emits a gated ToolRequest. After yielding the
        /// gate event the underlying stream pauses on a Notify, which the
        /// session's approve/reject/cancel methods signal. This lets
        /// SessionManager's own oneshot-based gate logic interleave
        /// correctly with the stream.
        pub struct GateTestAdapter;

        #[async_trait]
        impl AgentAdapter for GateTestAdapter {
            fn name(&self) -> &str { "gate-test" }

            async fn spawn(
                &self,
                _workspace_path: &Path,
                _prompt: &str,
                _context: &SessionContext,
                _model: Option<&str>,
            ) -> Result<Box<dyn AgentSession>> {
                let cancelled = Arc::new(AtomicBool::new(false));
                let resume_notify = Arc::new(Notify::new());

                // Build a channel-based stream. A background task sends
                // events into the channel, pausing at the gate until
                // resume_notify is signalled.
                let (tx, rx) = tokio::sync::mpsc::channel::<AgentEvent>(16);
                let c = cancelled.clone();
                let n = resume_notify.clone();
                tokio::spawn(async move {
                    let _ = tx.send(AgentEvent::Thinking {
                        text: "Thinking about risky operation...".to_string(),
                    }).await;

                    let _ = tx.send(AgentEvent::ToolRequest {
                        id: "gate_0".to_string(),
                        tool_name: "Bash".to_string(),
                        description: "rm -rf /important".to_string(),
                        input: serde_json::json!({"command": "rm -rf /important"}),
                        needs_approval: true,
                        risk_level: RiskLevel::Critical,
                    }).await;

                    // Pause here until approve/reject/cancel signals us.
                    n.notified().await;

                    if c.load(Ordering::Relaxed) {
                        return; // stream ends — no more events
                    }

                    let _ = tx.send(AgentEvent::ToolResult {
                        id: "gate_0".to_string(),
                        tool_name: "Bash".to_string(),
                        success: true,
                        output: "Executed successfully".to_string(),
                        raw_output: None,
                        duration_ms: 500,
                    }).await;

                    let _ = tx.send(AgentEvent::Complete {
                        summary: "Risky operation completed".to_string(),
                        total_cost_usd: 0.01,
                        duration_ms: 3000,
                        turns: 2,
                    }).await;
                });

                Ok(Box::new(GateTestSession {
                    init_data: SessionInit {
                        session_id: uuid::Uuid::new_v4().to_string(),
                        model: "gate-test-model".to_string(),
                        cwd: "/tmp".to_string(),
                        tools: vec!["Bash".into()],
                    },
                    cancelled,
                    resume_notify,
                    rx: tokio::sync::Mutex::new(Some(rx)),
                }))
            }

            async fn resume(
                &self,
                workspace_path: &Path,
                _session_id: &str,
                prompt: &str,
                model: Option<&str>,
            ) -> Result<Box<dyn AgentSession>> {
                self.spawn(
                    workspace_path,
                    prompt,
                    &SessionContext { briefing: None, memories: vec![], budget_cap: None },
                    model,
                ).await
            }
        }

        struct GateTestSession {
            init_data: SessionInit,
            cancelled: Arc<AtomicBool>,
            resume_notify: Arc<Notify>,
            rx: tokio::sync::Mutex<Option<tokio::sync::mpsc::Receiver<AgentEvent>>>,
        }

        #[async_trait]
        impl AgentSession for GateTestSession {
            fn init(&self) -> &SessionInit { &self.init_data }

            fn events(&mut self) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
                let rx = self.rx.get_mut().take().expect("events() called twice");
                Box::pin(unfold(rx, |mut rx| async move {
                    rx.recv().await.map(|event| (event, rx))
                }))
            }

            async fn approve(&self, _tool_use_id: &str) -> Result<()> {
                self.resume_notify.notify_one();
                Ok(())
            }

            async fn reject(&self, _tool_use_id: &str, _reason: &str) -> Result<()> {
                self.cancelled.store(true, Ordering::Relaxed);
                self.resume_notify.notify_one();
                Ok(())
            }

            async fn cancel(&self) -> Result<()> {
                self.cancelled.store(true, Ordering::Relaxed);
                self.resume_notify.notify_one();
                Ok(())
            }
        }
    }

    // ---------------------------------------------------------------
    // resume_thread tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_resume_thread_success() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "First reply".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        // Start a thread first so we have a stored session_id
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        // Drain all events from the first run
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        // Now resume the same thread
        mgr.resume_thread(&thread_id, &ws, "follow up", "fake", None)
            .await
            .unwrap();

        let events = collect_events_until_done(&mut rx).await;
        assert!(
            events.iter().any(|te| matches!(&te.event, AgentEvent::Complete { .. })),
            "resumed thread should complete"
        );
    }

    #[tokio::test]
    async fn test_resume_thread_nonexistent() {
        let (mut mgr, _rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "x".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        let result = mgr.resume_thread("no-such-thread", &ws, "prompt", "fake", None).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("no session_id"),
            "should fail because no session_id was stored"
        );
    }

    #[tokio::test]
    async fn test_resume_thread_unknown_agent() {
        let (mgr, _rx) = setup_session_manager().await;
        let ws = make_workspace();
        let result = mgr.resume_thread("t1", &ws, "prompt", "nonexistent-agent", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown agent"));
    }

    // ---------------------------------------------------------------
    // Budget cap tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_budget_cap_exceeded() {
        // FakeScenario::TextOnly emits CostUpdate with total_usd: 0.003
        // Set budget_cap to 0.001 — well below the cost
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Expensive answer".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace_with_budget(Some(0.001));
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let _thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let events = collect_events_until_done(&mut rx).await;

        let has_budget_error = events.iter().any(|te| {
            matches!(&te.event, AgentEvent::Error { message, .. } if message.contains("Budget cap"))
        });
        assert!(has_budget_error, "should have a budget cap error event");

        // Should NOT have a Complete event — session was killed
        let has_complete = events
            .iter()
            .any(|te| matches!(&te.event, AgentEvent::Complete { .. }));
        assert!(!has_complete, "session should be terminated before completion");
    }

    #[tokio::test]
    async fn test_budget_cap_not_exceeded() {
        // Set budget_cap to 10.0 — well above the 0.003 cost
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Cheap answer".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace_with_budget(Some(10.0));
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let _thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let events = collect_events_until_done(&mut rx).await;

        let has_complete = events
            .iter()
            .any(|te| matches!(&te.event, AgentEvent::Complete { .. }));
        assert!(has_complete, "thread should complete normally when under budget");

        let has_budget_error = events.iter().any(|te| {
            matches!(&te.event, AgentEvent::Error { message, .. } if message.contains("Budget cap"))
        });
        assert!(!has_budget_error, "no budget error expected");
    }

    // ---------------------------------------------------------------
    // Gate approve / reject tests (using GateTestAdapter)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_gate_approve_completes_thread() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr
            .start_thread(&ws, "do something risky", "gate-test", ctx, None)
            .await
            .unwrap();

        // Receive events until we see the gated ToolRequest
        let tool_use_id = wait_for_gate_event(&mut rx).await;

        // Approve the gate — this unblocks both consume_events and the underlying session
        mgr.approve(&thread_id, &tool_use_id).await.unwrap();

        // Collect remaining events — should see ToolResult + Complete
        let events = collect_events_until_done(&mut rx).await;
        let has_tool_result = events
            .iter()
            .any(|te| matches!(&te.event, AgentEvent::ToolResult { .. }));
        let has_complete = events
            .iter()
            .any(|te| matches!(&te.event, AgentEvent::Complete { .. }));
        assert!(has_tool_result, "approved gate should produce ToolResult");
        assert!(has_complete, "approved gate should lead to Complete");
    }

    #[tokio::test]
    async fn test_gate_reject_interrupts_thread() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr
            .start_thread(&ws, "do something risky", "gate-test", ctx, None)
            .await
            .unwrap();

        // Receive events until we see the gated ToolRequest
        let tool_use_id = wait_for_gate_event(&mut rx).await;

        // Reject the gate
        mgr.reject(&thread_id, &tool_use_id, "too dangerous")
            .await
            .unwrap();

        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        // Drain any remaining events — should NOT have a Complete
        let mut remaining = vec![];
        while let Ok(Some(te)) =
            tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
        {
            remaining.push(te);
        }

        let has_complete = remaining
            .iter()
            .any(|te| matches!(&te.event, AgentEvent::Complete { .. }));
        assert!(!has_complete, "rejected gate should NOT lead to Complete");

        // Verify DB status is interrupted
        let status = query_thread_status(&mgr, &thread_id).await;
        assert_eq!(status, "interrupted");
    }

    // ---------------------------------------------------------------
    // get_snapshot tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_get_snapshot_nonexistent() {
        let (mgr, _rx) = setup_session_manager().await;
        let result = mgr.get_snapshot("no-such-thread").await;
        assert!(result.is_none());
    }

    // ---------------------------------------------------------------
    // remove_thread tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_remove_thread() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hi".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        // Wait for completion — consume_events removes thread from active map itself
        let _ = collect_events_until_done(&mut rx).await;

        // Even after auto-removal, calling remove_thread should be a no-op (not panic)
        mgr.remove_thread(&thread_id).await;

        // Verify it's definitely gone
        let active = mgr.active_threads.lock().await;
        assert!(
            !active.contains_key(&thread_id),
            "thread should be removed from active map"
        );
    }

    // ---------------------------------------------------------------
    // list_adapters tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_list_adapters() {
        let (mut mgr, _rx) = setup_session_manager().await;
        assert!(mgr.list_adapters().is_empty(), "no adapters registered yet");

        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "x".to_string(),
        });
        mgr.register_adapter(Arc::new(adapter));

        let names = mgr.list_adapters();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"fake".to_string()));
    }

    #[tokio::test]
    async fn test_list_adapters_multiple() {
        let (mut mgr, _rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(
            FakeAdapter::new(FakeScenario::TextOnly { response: "a".into() }),
        ));
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let mut names = mgr.list_adapters();
        names.sort();
        assert_eq!(names, vec!["fake", "gate-test"]);
    }

    // ---------------------------------------------------------------
    // list_models tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_list_models_unknown_adapter() {
        let (mgr, _rx) = setup_session_manager().await;
        let result = mgr.list_models("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_models_fake_adapter() {
        let (mut mgr, _rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(FakeAdapter::new(FakeScenario::TextOnly {
            response: "x".into(),
        })));
        let models = mgr.list_models("fake").await.unwrap();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id == "sonnet"));
        assert!(models.iter().any(|m| m.id == "opus"));
        assert!(models.iter().any(|m| m.id == "haiku"));
    }

    // ---------------------------------------------------------------
    // ThreadStatus Display impl
    // ---------------------------------------------------------------

    #[test]
    fn test_thread_status_display() {
        assert_eq!(format!("{}", ThreadStatus::Pending), "pending");
        assert_eq!(format!("{}", ThreadStatus::Running), "running");
        assert_eq!(format!("{}", ThreadStatus::Gate), "gate");
        assert_eq!(format!("{}", ThreadStatus::Completed), "completed");
        assert_eq!(format!("{}", ThreadStatus::Error), "error");
        assert_eq!(format!("{}", ThreadStatus::Interrupted), "interrupted");
    }

    // ---------------------------------------------------------------
    // persist_event — verify events are stored in SQLite
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_events_persisted_to_db() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Stored!".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        // Wait for thread to complete
        let _ = collect_events_until_done(&mut rx).await;

        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        // Query the events table
        let tid = thread_id.clone();
        let (event_count, types) = mgr.db
            .execute(move |conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM events WHERE thread_id = ?1",
                    rusqlite::params![tid],
                    |row| row.get(0),
                )?;
                let mut stmt = conn.prepare("SELECT event_type FROM events WHERE thread_id = ?1 ORDER BY id")?;
                let types: Vec<String> = stmt
                    .query_map(rusqlite::params![tid], |row| row.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok((count, types))
            })
            .await
            .unwrap();

        // TextOnly emits: Thinking, CostUpdate, Text, Complete = 4 events
        assert_eq!(event_count, 4, "all events should be persisted to DB");
        assert_eq!(types, vec!["thinking", "cost_update", "text", "complete"]);
    }

    // ---------------------------------------------------------------
    // DB status updates — verify thread status transitions
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_thread_status_completed_in_db() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Done!".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        let status = query_thread_status(&mgr, &thread_id).await;
        assert_eq!(status, "completed");

        // Verify summary and cost_usd were set
        let tid = thread_id.clone();
        let summary: String = mgr.db
            .execute(move |conn| {
                Ok(conn.query_row(
                    "SELECT summary FROM threads WHERE id = ?1",
                    rusqlite::params![tid],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(summary, "Done!");
    }

    #[tokio::test]
    async fn test_thread_status_error_in_db() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::Error {
            message: "Something went wrong".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        let status = query_thread_status(&mgr, &thread_id).await;
        assert_eq!(status, "error");
    }

    #[tokio::test]
    async fn test_budget_cap_sets_error_status_in_db() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Expensive".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace_with_budget(Some(0.001));
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        let status = query_thread_status(&mgr, &thread_id).await;
        assert_eq!(status, "error");
    }

    // ---------------------------------------------------------------
    // session_id persistence via start_thread + load_session_ids
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_session_id_stored_and_loadable() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "x".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;

        // Verify session_id is in the in-memory map
        {
            let sids = mgr.session_ids.lock().await;
            assert!(sids.contains_key(&thread_id));
        }

        // Verify session_id is in the DB
        let tid = thread_id.clone();
        let stored_sid: String = mgr.db
            .execute(move |conn| {
                Ok(conn.query_row(
                    "SELECT session_id FROM threads WHERE id = ?1",
                    rusqlite::params![tid],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        assert!(!stored_sid.is_empty());

        // Verify load_session_ids can reconstruct the map from DB
        let loaded = SessionManager::load_session_ids(&mgr.db).await;
        assert_eq!(loaded.get(&thread_id).unwrap(), &stored_sid);
    }

    // ---------------------------------------------------------------
    // Gate pausing sets gate status in DB
    // ---------------------------------------------------------------

    // ---------------------------------------------------------------
    // Model selection tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_start_thread_with_model() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hi".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, Some("opus")).await.unwrap();
        assert!(!thread_id.is_empty());

        let events = collect_events_until_done(&mut rx).await;
        assert!(events.iter().any(|te| matches!(&te.event, AgentEvent::Complete { .. })));
    }

    #[tokio::test]
    async fn test_start_thread_model_none_uses_default() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hi".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let _thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let events = collect_events_until_done(&mut rx).await;
        assert!(events.iter().any(|te| matches!(&te.event, AgentEvent::Complete { .. })));
    }

    #[tokio::test]
    async fn test_resume_thread_with_model() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "First".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        mgr.resume_thread(&thread_id, &ws, "follow up", "fake", Some("sonnet"))
            .await
            .unwrap();

        let events = collect_events_until_done(&mut rx).await;
        assert!(events.iter().any(|te| matches!(&te.event, AgentEvent::Complete { .. })));
    }

    #[tokio::test]
    async fn test_gate_status_set_in_db() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr
            .start_thread(&ws, "risky op", "gate-test", ctx, None)
            .await
            .unwrap();

        // Wait for the gate event
        let _tool_use_id = wait_for_gate_event(&mut rx).await;

        wait_for_db_status(&mgr, &thread_id, "gate", 2000).await;

        let status = query_thread_status(&mgr, &thread_id).await;
        assert_eq!(status, "gate", "DB should show gate status while paused");

        // Clean up — reject so the background task stops
        mgr.reject(&thread_id, "gate_0", "test cleanup").await.unwrap();
    }

    // ---------------------------------------------------------------
    // One-thread-per-workspace guard tests
    // ---------------------------------------------------------------

    fn make_workspace_with_id(id: &str) -> Workspace {
        Workspace {
            id: id.to_string(),
            path: std::env::temp_dir().join(id),
            name: format!("test-{id}"),
            default_agent: None,
            budget_cap: None,
        }
    }

    #[tokio::test]
    async fn test_start_blocks_second_start_same_workspace() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let ws = make_workspace_with_id("ws-guard-1");
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let _tid_a = mgr.start_thread(&ws, "first", "gate-test", ctx, None).await.unwrap();
        let _gate_id = wait_for_gate_event(&mut rx).await;

        let ctx2 = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = mgr.start_thread(&ws, "second", "gate-test", ctx2, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already running in this workspace"));

        mgr.reject(&_tid_a, &_gate_id, "cleanup").await.unwrap();
    }

    #[tokio::test]
    async fn test_start_different_workspace_concurrent_ok() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let ws1 = make_workspace_with_id("ws-guard-2a");
        let ws2 = make_workspace_with_id("ws-guard-2b");
        insert_workspace_row(&mgr, &ws1).await;
        insert_workspace_row(&mgr, &ws2).await;

        let ctx1 = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid_a = mgr.start_thread(&ws1, "first", "gate-test", ctx1, None).await.unwrap();
        let gate_a = wait_for_gate_event(&mut rx).await;

        let ctx2 = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = mgr.start_thread(&ws2, "second", "gate-test", ctx2, None).await;
        assert!(result.is_ok(), "different workspace should succeed");

        let tid_b = result.unwrap();
        let gate_b = wait_for_gate_event(&mut rx).await;

        mgr.reject(&tid_a, &gate_a, "cleanup").await.unwrap();
        mgr.reject(&tid_b, &gate_b, "cleanup").await.unwrap();
    }

    #[tokio::test]
    async fn test_resume_blocks_if_other_thread_active() {
        let (mut mgr, mut rx) = setup_session_manager().await;

        let fake = FakeAdapter::new(FakeScenario::TextOnly {
            response: "done".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(fake));
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let ws = make_workspace_with_id("ws-guard-3");
        insert_workspace_row(&mgr, &ws).await;

        // Start and complete thread A
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid_a = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &tid_a, 2000).await;

        // Start thread C (gate-test) in same workspace — it will be active at the gate
        let ctx2 = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid_c = mgr.start_thread(&ws, "risky", "gate-test", ctx2, None).await.unwrap();
        let gate_id = wait_for_gate_event(&mut rx).await;

        // Try to resume thread A while C is active — should fail
        let result = mgr.resume_thread(&tid_a, &ws, "follow up", "fake", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already running in this workspace"));

        mgr.reject(&tid_c, &gate_id, "cleanup").await.unwrap();
    }

    #[tokio::test]
    async fn test_resume_succeeds_same_thread_only() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "reply".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace_with_id("ws-guard-4");
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &tid, 2000).await;

        // Resume same thread in same workspace — no other active thread
        let result = mgr.resume_thread(&tid, &ws, "follow up", "fake", None).await;
        assert!(result.is_ok(), "resume of own thread should succeed");
        let _ = collect_events_until_done(&mut rx).await;
    }

    // ---------------------------------------------------------------
    // Fix #1: Gate approve/reject robustness
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_approve_no_pending_gate() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "done".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &tid, 2000).await;

        let result = mgr.approve(&tid, "fake-tool-id").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("thread not found"));
    }

    #[tokio::test]
    async fn test_double_approve_returns_error() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let ws = make_workspace_with_id("ws-double-approve");
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid = mgr.start_thread(&ws, "gate me", "gate-test", ctx, None).await.unwrap();
        let gate_id = wait_for_gate_event(&mut rx).await;

        mgr.approve(&tid, &gate_id).await.unwrap();

        let result = mgr.approve(&tid, &gate_id).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no gate pending") || err.contains("thread not found"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn test_reject_no_pending_gate() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "done".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &tid, 2000).await;

        let result = mgr.reject(&tid, "fake-tool-id", "test").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("thread not found"));
    }
}
