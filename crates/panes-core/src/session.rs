use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use futures::StreamExt;
use panes_adapters::{AgentAdapter, AgentSession};
use panes_cost::CostTracker;
use panes_events::{AgentEvent, SessionContext, ThreadEvent};
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
}

impl SessionManager {
    pub fn new(
        cost_tracker: Arc<CostTracker>,
        event_tx: mpsc::UnboundedSender<ThreadEvent>,
    ) -> Self {
        Self {
            active_threads: Arc::new(Mutex::new(HashMap::new())),
            session_ids: Arc::new(Mutex::new(HashMap::new())),
            adapters: HashMap::new(),
            cost_tracker,
            event_tx,
        }
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

        // Store the claude session_id for resume
        {
            let mut sids = self.session_ids.lock().await;
            sids.insert(thread_id.clone(), session.init().session_id.clone());
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

        // Start consuming the event stream in a background task
        let thread_id_clone = thread_id.clone();
        let event_tx = self.event_tx.clone();
        let cost_tracker = self.cost_tracker.clone();
        let active_threads = self.active_threads.clone();
        let budget_cap = workspace.budget_cap;

        tokio::spawn(async move {
            Self::consume_events(
                thread_id_clone,
                event_tx,
                cost_tracker,
                active_threads,
                budget_cap,
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
        {
            let mut sids = self.session_ids.lock().await;
            sids.insert(thread_id.to_string(), session.init().session_id.clone());
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

        let thread_id_clone = thread_id.to_string();
        let event_tx = self.event_tx.clone();
        let cost_tracker = self.cost_tracker.clone();
        let active_threads = self.active_threads.clone();
        let budget_cap = workspace.budget_cap;

        tokio::spawn(async move {
            Self::consume_events(
                thread_id_clone,
                event_tx,
                cost_tracker,
                active_threads,
                budget_cap,
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
    ) {
        // Take the session out of the map so we own it (and can call events())
        let mut thread = {
            let mut active = active_threads.lock().await;
            match active.remove(&thread_id) {
                Some(t) => t,
                None => return,
            }
        };

        let mut events_stream = thread.session.events();

        while let Some(event) = events_stream.next().await {
            // Update cost tracker
            cost_tracker.process_event(&thread_id, &event);

            // Check budget cap
            if let Some(cap) = budget_cap {
                if cost_tracker.check_budget(&thread_id, cap) {
                    warn!(thread_id = %thread_id, cap, "budget cap exceeded — killing session");
                    let _ = event_tx.send(ThreadEvent {
                        thread_id: thread_id.clone(),
                        timestamp: Utc::now(),
                        event: AgentEvent::Error {
                            message: format!("Budget cap of ${cap:.2} exceeded. Session terminated."),
                            recoverable: false,
                        },
                        parent_tool_use_id: None,
                    });
                    break;
                }
            }

            // Forward to frontend
            let thread_event = ThreadEvent {
                thread_id: thread_id.clone(),
                timestamp: Utc::now(),
                event: event.clone(),
                parent_tool_use_id: None,
            };

            if event_tx.send(thread_event).is_err() {
                break;
            }

            // Check if session completed
            if matches!(event, AgentEvent::Complete { .. } | AgentEvent::Error { recoverable: false, .. }) {
                break;
            }
        }

        // Finalize cost
        if let Some(cost) = cost_tracker.finalize(&thread_id) {
            info!(
                thread_id = %thread_id,
                cost = cost.total_usd,
                "thread completed"
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
