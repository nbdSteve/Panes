mod commands;

use std::sync::Arc;

use panes_adapters::claude::ClaudeAdapter;
use panes_core::session::SessionManager;
use panes_cost::CostTracker;
use panes_events::ThreadEvent;
use tauri::Emitter;
use tokio::sync::mpsc;
use tracing::info;
use tracing_subscriber::EnvFilter;

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("panes=debug".parse().unwrap()))
        .with_writer(std::io::stderr)
        .init();

    eprintln!("[panes] app starting");

    let (event_tx, event_rx) = mpsc::unbounded_channel::<ThreadEvent>();
    let cost_tracker = Arc::new(CostTracker::new());

    let mut session_manager = SessionManager::new(cost_tracker.clone(), event_tx);

    let adapter = ClaudeAdapter::with_cli_path("/Users/goodhill/.local/bin/claude")
        .env("CLAUDE_CODE_USE_BEDROCK", "1")
        .env("AWS_PROFILE", "bedrock-beta")
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", std::env::var("HOME").unwrap_or_default());
    session_manager.register_adapter(Arc::new(adapter));

    tauri::Builder::default()
        .manage(Arc::new(tokio::sync::Mutex::new(session_manager)))
        .manage(cost_tracker)
        .setup(|app| {
            let handle = app.handle().clone();
            let event_rx = Arc::new(tokio::sync::Mutex::new(event_rx));
            tauri::async_runtime::spawn(forward_events(handle, event_rx));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_workspace,
            commands::list_workspaces,
            commands::start_thread,
            commands::resume_thread,
            commands::approve_gate,
            commands::reject_gate,
            commands::cancel_thread,
            commands::get_workspaces,
        ])
        .run(tauri::generate_context!())
        .expect("error running panes");
}

async fn forward_events(
    handle: tauri::AppHandle,
    event_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<ThreadEvent>>>,
) {
    let mut rx = event_rx.lock().await;
    while let Some(event) = rx.recv().await {
        info!(thread_id = %event.thread_id, event = ?event.event, "forwarding event to frontend");
        let _ = handle.emit("panes://thread-event", &event);
    }
}
