pub mod actions;
pub mod executor;
pub mod ticker;
pub mod types;

pub use types::{ExecutionStatus, LogNotifier, Notifier, NotifierRef, Routine, RoutineExecution, ScheduleAction};

use panes_core::db::DbHandle;
use panes_core::session::SessionManager;
use panes_events::ThreadEvent;
use panes_memory::manager::MemoryManager;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::info;

pub struct Scheduler {
    db: DbHandle,
    session_manager: Arc<Mutex<SessionManager>>,
    memory_manager: Arc<MemoryManager>,
    broadcast_tx: broadcast::Sender<ThreadEvent>,
    notifier: NotifierRef,
    cancel_token: std::sync::Mutex<CancellationToken>,
    task_handles: std::sync::Mutex<Vec<JoinHandle<()>>>,
}

impl Scheduler {
    pub fn new(
        db: DbHandle,
        session_manager: Arc<Mutex<SessionManager>>,
        memory_manager: Arc<MemoryManager>,
        broadcast_tx: broadcast::Sender<ThreadEvent>,
        notifier: NotifierRef,
    ) -> Self {
        Self {
            db,
            session_manager,
            memory_manager,
            broadcast_tx,
            notifier,
            cancel_token: std::sync::Mutex::new(CancellationToken::new()),
            task_handles: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn start(&self) {
        if self.is_running() {
            return;
        }

        let token = {
            let mut ct = self.cancel_token.lock().unwrap();
            *ct = CancellationToken::new();
            ct.clone()
        };

        let db = self.db.clone();
        let sm = self.session_manager.clone();
        let mm = self.memory_manager.clone();
        let notifier = self.notifier.clone();

        let ticker_handle = tokio::spawn(ticker::run_ticker(
            db.clone(),
            sm.clone(),
            mm.clone(),
            token.clone(),
        ));

        let monitor_rx = self.broadcast_tx.subscribe();
        let monitor_handle = tokio::spawn(ticker::run_completion_monitor(
            db,
            sm,
            mm,
            notifier,
            monitor_rx,
            token,
        ));

        let mut handles = self.task_handles.lock().unwrap();
        *handles = vec![ticker_handle, monitor_handle];

        info!("scheduler started");
    }

    pub async fn stop(&self) {
        {
            let ct = self.cancel_token.lock().unwrap();
            ct.cancel();
        }

        let handles = {
            let mut h = self.task_handles.lock().unwrap();
            std::mem::take(&mut *h)
        };

        for handle in handles {
            let _ = handle.await;
        }

        info!("scheduler stopped");
    }

    pub fn is_running(&self) -> bool {
        let handles = self.task_handles.lock().unwrap();
        !handles.is_empty() && handles.iter().all(|h| !h.is_finished())
    }
}
