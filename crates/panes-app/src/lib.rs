mod commands;
mod test_bridge;

use std::sync::Arc;

use panes_adapters::claude::ClaudeAdapter;
use panes_adapters::fake::{FakeAdapter, FakeScenario, FakeStep};
use panes_core::db;
use panes_core::session::SessionManager;
use panes_cost::CostTracker;
use panes_events::{RiskLevel, ThreadEvent};
use panes_memory::manager::{MemoryConfig, MemoryManager};
use tauri::Emitter;
use tokio::sync::{broadcast, mpsc};
use tracing::info;
use tracing_subscriber::EnvFilter;

fn is_test_mode() -> bool {
    std::env::var("PANES_TEST_MODE").is_ok()
}

fn data_dir() -> std::path::PathBuf {
    match std::env::var("PANES_DATA_DIR") {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(_) => dirs::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("dev.panes"),
    }
}

fn db_path() -> String {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).ok();
    dir.join("panes.db").to_string_lossy().to_string()
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

    let memory_config = if test_mode {
        MemoryConfig::for_test()
    } else {
        MemoryConfig::from_env(&data_dir())
    };
    let memory_manager = Arc::new(
        MemoryManager::new(&memory_config).expect("failed to initialize memory manager"),
    );

    let (event_tx, event_rx) = mpsc::unbounded_channel::<ThreadEvent>();
    let cost_tracker = Arc::new(CostTracker::new());

    let mut session_manager = SessionManager::new(cost_tracker.clone(), event_tx, db.clone());

    let bridge_tx: Option<broadcast::Sender<ThreadEvent>> = if test_mode {
        let (tx, _) = broadcast::channel(256);
        Some(tx)
    } else {
        None
    };

    if test_mode {
        register_fake_adapters(&mut session_manager);
    } else {
        let cli_path = std::env::var("PANES_CLAUDE_PATH")
            .unwrap_or_else(|_| "claude".to_string());
        let mut adapter = ClaudeAdapter::with_cli_path(cli_path);
        for key in ["CLAUDE_CODE_USE_BEDROCK", "AWS_PROFILE", "PATH", "HOME"] {
            if let Ok(val) = std::env::var(key) {
                adapter = adapter.env(key, val);
            }
        }
        session_manager.register_adapter(Arc::new(adapter));
    }

    let session_arc = Arc::new(tokio::sync::Mutex::new(session_manager));

    let bridge_session = session_arc.clone();
    let bridge_cost = cost_tracker.clone();
    let bridge_db = db.clone();
    let bridge_memory = memory_manager.clone();

    tauri::Builder::default()
        .manage(session_arc)
        .manage(cost_tracker)
        .manage(db)
        .manage(memory_manager.clone())
        .setup(move |app| {
            let handle = app.handle().clone();
            let event_rx = Arc::new(tokio::sync::Mutex::new(event_rx));

            if let Some(ref tx) = bridge_tx {
                test_bridge::start_test_bridge(
                    bridge_session,
                    bridge_cost,
                    bridge_db,
                    bridge_memory,
                    tx.clone(),
                );
            }

            let init_mgr = memory_manager;
            tauri::async_runtime::spawn(async move {
                init_mgr.init().await;
                init_mgr.spawn_health_monitor();
            });

            tauri::async_runtime::spawn(forward_events(handle, event_rx, bridge_tx));
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
            commands::get_changed_files,
            commands::list_threads,
            commands::list_all_threads,
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
            commands::list_adapters,
            commands::list_agents,
            commands::list_models,
            commands::set_workspace_default_agent,
            commands::set_workspace_budget_cap,
            commands::get_aggregate_cost,
            commands::get_workspace_cost,
            commands::get_memory_backend_status,
            commands::set_memory_backend,
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
        model: Option<&str>,
    ) -> anyhow::Result<Box<dyn panes_adapters::AgentSession>> {
        let lower = prompt.to_lowercase();
        let (scenario, delay) = if lower.contains("slow") {
            (route_prompt(prompt), 500)
        } else {
            (route_prompt(prompt), 80)
        };

        if let FakeScenario::FileEdit { ref files, .. } = scenario {
            for file in files {
                let path = workspace_path.join(file);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                std::fs::write(&path, format!("// modified by panes test\n// file: {file}\n")).ok();
            }
        }

        let adapter = FakeAdapter::new(scenario).with_delay(delay);
        adapter.spawn(workspace_path, prompt, context, model).await
    }

    async fn resume(
        &self,
        workspace_path: &std::path::Path,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
    ) -> anyhow::Result<Box<dyn panes_adapters::AgentSession>> {
        let scenario = route_prompt(prompt);
        let adapter = FakeAdapter::new(scenario).with_delay(80);
        adapter.resume(workspace_path, session_id, prompt, model).await
    }

    async fn list_models(&self) -> anyhow::Result<Vec<panes_adapters::ModelInfo>> {
        Ok(vec![
            panes_adapters::ModelInfo { id: "sonnet".into(), label: "Sonnet".into(), description: "Fast & capable".into() },
            panes_adapters::ModelInfo { id: "opus".into(), label: "Opus".into(), description: "Most capable".into() },
            panes_adapters::ModelInfo { id: "haiku".into(), label: "Haiku".into(), description: "Fastest".into() },
        ])
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
    } else if lower.contains("slow") {
        FakeScenario::MultiStep {
            steps: vec![
                FakeStep {
                    tool_name: "Read".to_string(),
                    description: "Read file: src/main.rs".to_string(),
                    risk_level: RiskLevel::Low,
                    needs_approval: false,
                    success: true,
                    output: "(file contents)".to_string(),
                },
                FakeStep {
                    tool_name: "Edit".to_string(),
                    description: "Edit file: src/main.rs".to_string(),
                    risk_level: RiskLevel::Medium,
                    needs_approval: false,
                    success: true,
                    output: "File edited".to_string(),
                },
                FakeStep {
                    tool_name: "Bash".to_string(),
                    description: "Run command: cargo build".to_string(),
                    risk_level: RiskLevel::Low,
                    needs_approval: false,
                    success: true,
                    output: "Build succeeded".to_string(),
                },
                FakeStep {
                    tool_name: "Bash".to_string(),
                    description: "Run command: cargo test".to_string(),
                    risk_level: RiskLevel::Low,
                    needs_approval: false,
                    success: true,
                    output: "All tests passed".to_string(),
                },
                FakeStep {
                    tool_name: "Read".to_string(),
                    description: "Read file: Cargo.toml".to_string(),
                    risk_level: RiskLevel::Low,
                    needs_approval: false,
                    success: true,
                    output: "(cargo config)".to_string(),
                },
            ],
            response: "Completed the slow multi-step task.".to_string(),
        }
    } else {
        FakeScenario::TextOnly {
            response: format!("I received your message: \"{}\"\n\nThis is a **fake response** from the test adapter. It supports:\n- `error` / `fail` → error scenario\n- `gate` / `dangerous` → gated action\n- `edit` / `write` → file edit with commit buttons\n- `read` / `explain` → read files then respond\n- `multi` / `complex` → multi-step tool use\n- `slow` → slow multi-step (for cancel testing)\n- anything else → this text response", prompt),
        }
    }
}

async fn forward_events(
    handle: tauri::AppHandle,
    event_rx: Arc<tokio::sync::Mutex<mpsc::UnboundedReceiver<ThreadEvent>>>,
    bridge_tx: Option<broadcast::Sender<ThreadEvent>>,
) {
    let mut rx = event_rx.lock().await;
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(50));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            Some(first) = rx.recv() => {
                let mut batch = vec![first];
                while let Ok(event) = rx.try_recv() {
                    batch.push(event);
                }
                for event in &batch {
                    info!(thread_id = %event.thread_id, event = ?event.event, "forwarding event to frontend");
                }
                let _ = handle.emit("panes://thread-events", &batch);
                if let Some(ref tx) = bridge_tx {
                    for event in batch {
                        let _ = tx.send(event);
                    }
                }
            }
            _ = interval.tick() => {
                let mut batch = Vec::new();
                while let Ok(event) = rx.try_recv() {
                    batch.push(event);
                }
                if !batch.is_empty() {
                    for event in &batch {
                        info!(thread_id = %event.thread_id, event = ?event.event, "forwarding event to frontend");
                    }
                    let _ = handle.emit("panes://thread-events", &batch);
                    if let Some(ref tx) = bridge_tx {
                        for event in batch {
                            let _ = tx.send(event);
                        }
                    }
                }
            }
        }
    }
}
