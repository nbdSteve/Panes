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
        model: Option<&str>,
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
        model: Option<&str>,
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

            let gate_tool_id = match &event {
                AgentEvent::ToolRequest { id, needs_approval: true, .. } => Some(id.clone()),
                _ => None,
            };

            if gate_tool_id.is_some() {
                if let Ok(conn) = db.lock() {
                    let _ = conn.execute(
                        "UPDATE threads SET status = 'gate' WHERE id = ?1",
                        rusqlite::params![thread_id],
                    );
                }
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
                                thread.session.reject(&tool_id, "rejected by user").await.ok();
                                let _ = thread.session.cancel().await;
                            }
                        }
                        let abort_event = AgentEvent::Error {
                            message: "Gate rejected by user".to_string(),
                            recoverable: false,
                        };
                        Self::persist_event(&db, &thread_id, &abort_event);
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
        let result = mgr.start_thread(&ws, "hello", "nonexistent-agent", ctx, None).await;
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
    fn insert_workspace_row(mgr: &SessionManager, ws: &Workspace) {
        let conn = mgr.db.lock().unwrap();
        conn.execute(
            "INSERT INTO workspaces (id, path, name, created_at) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![ws.id, ws.path.to_string_lossy(), ws.name, "2024-01-01"],
        )
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
        let (mut mgr, mut rx) = setup_session_manager();
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "First reply".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws);

        // Start a thread first so we have a stored session_id
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        // Drain all events from the first run
        let _ = collect_events_until_done(&mut rx).await;

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
        let (mut mgr, _rx) = setup_session_manager();
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
        let (mgr, _rx) = setup_session_manager();
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
        let (mut mgr, mut rx) = setup_session_manager();
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Expensive answer".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace_with_budget(Some(0.001));
        insert_workspace_row(&mgr, &ws);

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
        let (mut mgr, mut rx) = setup_session_manager();
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Cheap answer".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace_with_budget(Some(10.0));
        insert_workspace_row(&mgr, &ws);

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
        let (mut mgr, mut rx) = setup_session_manager();
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws);

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
        let (mut mgr, mut rx) = setup_session_manager();
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws);

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

        // Give the background task time to cancel and clean up
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

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
        let status: String = {
            let conn = mgr.db.lock().unwrap();
            conn.query_row(
                "SELECT status FROM threads WHERE id = ?1",
                rusqlite::params![thread_id],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(status, "interrupted");
    }

    // ---------------------------------------------------------------
    // get_snapshot tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_get_snapshot_nonexistent() {
        let (mgr, _rx) = setup_session_manager();
        let result = mgr.get_snapshot("no-such-thread").await;
        assert!(result.is_none());
    }

    // ---------------------------------------------------------------
    // remove_thread tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_remove_thread() {
        let (mut mgr, mut rx) = setup_session_manager();
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hi".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws);

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
        let (mut mgr, _rx) = setup_session_manager();
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
        let (mut mgr, _rx) = setup_session_manager();
        mgr.register_adapter(Arc::new(
            FakeAdapter::new(FakeScenario::TextOnly { response: "a".into() }),
        ));
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let mut names = mgr.list_adapters();
        names.sort();
        assert_eq!(names, vec!["fake", "gate-test"]);
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
        let (mut mgr, mut rx) = setup_session_manager();
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Stored!".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws);

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        // Wait for thread to complete
        let _ = collect_events_until_done(&mut rx).await;

        // Give the background task a moment to finish persisting
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Query the events table
        let conn = mgr.db.lock().unwrap();
        let event_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM events WHERE thread_id = ?1",
                rusqlite::params![thread_id],
                |row| row.get(0),
            )
            .unwrap();

        // TextOnly emits: Thinking, CostUpdate, Text, Complete = 4 events
        assert_eq!(event_count, 4, "all events should be persisted to DB");

        // Verify specific event types are present
        let mut stmt = conn
            .prepare("SELECT event_type FROM events WHERE thread_id = ?1 ORDER BY id")
            .unwrap();
        let types: Vec<String> = stmt
            .query_map(rusqlite::params![thread_id], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();
        assert_eq!(types, vec!["thinking", "cost_update", "text", "complete"]);
    }

    // ---------------------------------------------------------------
    // DB status updates — verify thread status transitions
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_thread_status_completed_in_db() {
        let (mut mgr, mut rx) = setup_session_manager();
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Done!".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws);

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let _ = collect_events_until_done(&mut rx).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let conn = mgr.db.lock().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM threads WHERE id = ?1",
                rusqlite::params![thread_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "completed");

        // Verify summary and cost_usd were set
        let summary: String = conn
            .query_row(
                "SELECT summary FROM threads WHERE id = ?1",
                rusqlite::params![thread_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(summary, "Done!");
    }

    #[tokio::test]
    async fn test_thread_status_error_in_db() {
        let (mut mgr, mut rx) = setup_session_manager();
        let adapter = FakeAdapter::new(FakeScenario::Error {
            message: "Something went wrong".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws);

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let _ = collect_events_until_done(&mut rx).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let conn = mgr.db.lock().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM threads WHERE id = ?1",
                rusqlite::params![thread_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "error");
    }

    #[tokio::test]
    async fn test_budget_cap_sets_error_status_in_db() {
        let (mut mgr, mut rx) = setup_session_manager();
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Expensive".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace_with_budget(Some(0.001));
        insert_workspace_row(&mgr, &ws);

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let _ = collect_events_until_done(&mut rx).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let conn = mgr.db.lock().unwrap();
        let status: String = conn
            .query_row(
                "SELECT status FROM threads WHERE id = ?1",
                rusqlite::params![thread_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "error");
    }

    // ---------------------------------------------------------------
    // session_id persistence via start_thread + load_session_ids
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_session_id_stored_and_loadable() {
        let (mut mgr, mut rx) = setup_session_manager();
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "x".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws);

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;

        // Verify session_id is in the in-memory map
        {
            let sids = mgr.session_ids.lock().await;
            assert!(sids.contains_key(&thread_id));
        }

        // Verify session_id is in the DB
        let stored_sid: String = {
            let conn = mgr.db.lock().unwrap();
            conn.query_row(
                "SELECT session_id FROM threads WHERE id = ?1",
                rusqlite::params![thread_id],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert!(!stored_sid.is_empty());

        // Verify load_session_ids can reconstruct the map from DB
        let loaded = SessionManager::load_session_ids(&mgr.db);
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
        let (mut mgr, mut rx) = setup_session_manager();
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hi".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws);

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, Some("opus")).await.unwrap();
        assert!(!thread_id.is_empty());

        let events = collect_events_until_done(&mut rx).await;
        assert!(events.iter().any(|te| matches!(&te.event, AgentEvent::Complete { .. })));
    }

    #[tokio::test]
    async fn test_start_thread_model_none_uses_default() {
        let (mut mgr, mut rx) = setup_session_manager();
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hi".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws);

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let _thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let events = collect_events_until_done(&mut rx).await;
        assert!(events.iter().any(|te| matches!(&te.event, AgentEvent::Complete { .. })));
    }

    #[tokio::test]
    async fn test_resume_thread_with_model() {
        let (mut mgr, mut rx) = setup_session_manager();
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "First".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws);

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;

        mgr.resume_thread(&thread_id, &ws, "follow up", "fake", Some("sonnet"))
            .await
            .unwrap();

        let events = collect_events_until_done(&mut rx).await;
        assert!(events.iter().any(|te| matches!(&te.event, AgentEvent::Complete { .. })));
    }

    #[tokio::test]
    async fn test_gate_status_set_in_db() {
        let (mut mgr, mut rx) = setup_session_manager();
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let ws = make_workspace();
        insert_workspace_row(&mgr, &ws);

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr
            .start_thread(&ws, "risky op", "gate-test", ctx, None)
            .await
            .unwrap();

        // Wait for the gate event
        let _tool_use_id = wait_for_gate_event(&mut rx).await;

        // Give consume_events a moment to set the DB status to 'gate'
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        let status: String = {
            let conn = mgr.db.lock().unwrap();
            conn.query_row(
                "SELECT status FROM threads WHERE id = ?1",
                rusqlite::params![thread_id],
                |row| row.get(0),
            )
            .unwrap()
        };
        assert_eq!(status, "gate", "DB should show gate status while paused");

        // Clean up — reject so the background task stops
        mgr.reject(&thread_id, "gate_0", "test cleanup").await.unwrap();
    }
}
