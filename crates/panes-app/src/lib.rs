mod commands;

use std::sync::Arc;

use panes_adapters::claude::ClaudeAdapter;
use panes_adapters::fake::{FakeAdapter, FakeScenario, FakeStep};
use panes_core::db;
use panes_core::session::SessionManager;
use panes_cost::CostTracker;
use panes_events::{RiskLevel, ThreadEvent};
use panes_memory::sqlite_store::SqliteMemoryStore;
use tauri::Emitter;
use tokio::sync::mpsc;
use tracing::info;
use tracing_subscriber::EnvFilter;

fn is_test_mode() -> bool {
    std::env::var("PANES_TEST_MODE").is_ok()
}

fn db_path() -> String {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("dev.panes");
    std::fs::create_dir_all(&data_dir).ok();
    data_dir.join("panes.db").to_string_lossy().to_string()
}

fn memory_db_path() -> String {
    let data_dir = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("dev.panes");
    std::fs::create_dir_all(&data_dir).ok();
    data_dir.join("memory.db").to_string_lossy().to_string()
}

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("panes=debug".parse().unwrap()))
        .with_writer(std::io::stderr)
        .init();

    let test_mode = is_test_mode();
    eprintln!("[panes] app starting (test_mode={})", test_mode);

    let conn = db::initialize(&db_path()).expect("failed to initialize database");
    let db = Arc::new(std::sync::Mutex::new(conn));

    let memory_db_path = memory_db_path();
    let memory_store = Arc::new(SqliteMemoryStore::new(&memory_db_path).expect("failed to initialize memory store"));

    let (event_tx, event_rx) = mpsc::unbounded_channel::<ThreadEvent>();
    let cost_tracker = Arc::new(CostTracker::new());

    let mut session_manager = SessionManager::new(cost_tracker.clone(), event_tx, db.clone());

    if test_mode {
        register_fake_adapters(&mut session_manager);
    } else {
        let adapter = ClaudeAdapter::with_cli_path("/Users/goodhill/.local/bin/claude")
            .env("CLAUDE_CODE_USE_BEDROCK", "1")
            .env("AWS_PROFILE", "bedrock-beta")
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("HOME", std::env::var("HOME").unwrap_or_default());
        session_manager.register_adapter(Arc::new(adapter));
    }

    tauri::Builder::default()
        .manage(Arc::new(tokio::sync::Mutex::new(session_manager)))
        .manage(cost_tracker)
        .manage(db)
        .manage(memory_store)
        .setup(|app| {
            let handle = app.handle().clone();
            let event_rx = Arc::new(tokio::sync::Mutex::new(event_rx));
            tauri::async_runtime::spawn(forward_events(handle, event_rx));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::add_workspace,
            commands::list_workspaces,
            commands::remove_workspace,
            commands::start_thread,
            commands::resume_thread,
            commands::approve_gate,
            commands::reject_gate,
            commands::cancel_thread,
            commands::commit_changes,
            commands::revert_changes,
            commands::get_workspaces,
            commands::list_threads,
            commands::delete_thread,
            commands::extract_memories,
            commands::get_memories,
            commands::search_memories,
            commands::update_memory,
            commands::delete_memory,
            commands::pin_memory,
            commands::get_briefing,
            commands::set_briefing,
            commands::delete_briefing,
        ])
        .run(tauri::generate_context!())
        .expect("error running panes");
}

fn register_fake_adapters(session_manager: &mut SessionManager) {
    // The default "claude-code" adapter in test mode cycles through scenarios
    // based on the prompt content, so tests can trigger specific behaviors.
    session_manager.register_adapter(Arc::new(PromptRoutedFakeAdapter));
}

struct PromptRoutedFakeAdapter;

#[async_trait::async_trait]
impl panes_adapters::AgentAdapter for PromptRoutedFakeAdapter {
    fn name(&self) -> &str {
        "claude-code"
    }

    async fn spawn(
        &self,
        workspace_path: &std::path::Path,
        prompt: &str,
        context: &panes_events::SessionContext,
    ) -> anyhow::Result<Box<dyn panes_adapters::AgentSession>> {
        let scenario = route_prompt(prompt);
        let adapter = FakeAdapter::new(scenario).with_delay(80);
        adapter.spawn(workspace_path, prompt, context).await
    }

    async fn resume(
        &self,
        workspace_path: &std::path::Path,
        session_id: &str,
        prompt: &str,
    ) -> anyhow::Result<Box<dyn panes_adapters::AgentSession>> {
        let scenario = route_prompt(prompt);
        let adapter = FakeAdapter::new(scenario).with_delay(80);
        adapter.resume(workspace_path, session_id, prompt).await
    }
}

fn route_prompt(prompt: &str) -> FakeScenario {
    let lower = prompt.to_lowercase();

    if lower.contains("error") || lower.contains("fail") {
        FakeScenario::Error {
            message: "Simulated error: something went wrong".to_string(),
        }
    } else if lower.contains("gate") || lower.contains("dangerous") || lower.contains("destructive") {
        FakeScenario::GatedAction {
            tool_name: "Bash".to_string(),
            description: "rm -rf /tmp/test-directory".to_string(),
            risk_level: RiskLevel::Critical,
            response: "The dangerous operation has been completed successfully.".to_string(),
        }
    } else if lower.contains("edit") || lower.contains("write") || lower.contains("create file") {
        FakeScenario::FileEdit {
            files: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
            response: "I've made the requested edits to the files.".to_string(),
        }
    } else if lower.contains("read") || lower.contains("explain") || lower.contains("analyze") {
        FakeScenario::ReadAndRespond {
            files: vec!["src/App.tsx".to_string(), "src/styles.css".to_string()],
            response: "Based on my analysis of the files, here is what I found:\n\n- The App component manages thread state centrally\n- Styles use CSS custom properties for theming\n- The architecture follows a unidirectional data flow pattern".to_string(),
        }
    } else if lower.contains("multi") || lower.contains("complex") {
        FakeScenario::MultiStep {
            steps: vec![
                FakeStep {
                    tool_name: "Read".to_string(),
                    description: "Read file: src/App.tsx".to_string(),
                    risk_level: RiskLevel::Low,
                    needs_approval: false,
                    success: true,
                    output: "(file contents)".to_string(),
                },
                FakeStep {
                    tool_name: "Edit".to_string(),
                    description: "Edit file: src/App.tsx".to_string(),
                    risk_level: RiskLevel::Medium,
                    needs_approval: false,
                    success: true,
                    output: "File edited".to_string(),
                },
                FakeStep {
                    tool_name: "Bash".to_string(),
                    description: "Run command: npm test".to_string(),
                    risk_level: RiskLevel::Low,
                    needs_approval: false,
                    success: true,
                    output: "All 42 tests passed".to_string(),
                },
            ],
            response: "I've read the file, made edits, and verified the tests pass.".to_string(),
        }
    } else {
        FakeScenario::TextOnly {
            response: format!("I received your message: \"{}\"\n\nThis is a **fake response** from the test adapter. It supports:\n- `error` / `fail` → error scenario\n- `gate` / `dangerous` → gated action\n- `edit` / `write` → file edit with commit buttons\n- `read` / `explain` → read files then respond\n- `multi` / `complex` → multi-step tool use\n- anything else → this text response", prompt),
        }
    }
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
