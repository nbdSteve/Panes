mod commands;
mod test_bridge;

use std::sync::Arc;

use panes_adapters::claude::ClaudeAdapter;
use panes_adapters::{AcpAdapter, AgentAdapter};
use panes_adapters::fake::{FakeAdapter, FakeScenario, FakeStep};
use panes_core::db;
use panes_core::session::SessionManager;
use panes_cost::CostTracker;
use panes_events::{RiskLevel, ThreadEvent};
use panes_memory::manager::{MemoryConfig, MemoryManager};
use panes_scheduler::Scheduler;
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::{broadcast, mpsc};
use tracing::info;
use tracing_subscriber::EnvFilter;

struct TauriNotifier {
    handle: tauri::AppHandle,
}

impl panes_scheduler::Notifier for TauriNotifier {
    fn send(&self, title: &str, body: &str) {
        if let Err(e) = self.handle.notification()
            .builder()
            .title(title)
            .body(body)
            .show()
        {
            tracing::warn!(error = %e, "failed to send OS notification");
        }
        let _ = self.handle.emit("panes://routine-notification", serde_json::json!({
            "title": title,
            "body": body,
        }));
    }
}

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
    let db = db::DbHandle::new(conn);

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

    let shadow_blob_root = data_dir().join("shadow-blobs");
    let mut session_manager = tauri::async_runtime::block_on(
        SessionManager::new(cost_tracker.clone(), event_tx, db.clone(), shadow_blob_root),
    );

    let (broadcast_tx, _) = broadcast::channel::<ThreadEvent>(256);

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

        // Register kiro-cli if the binary resolves. Graceful no-op otherwise —
        // Panes stays usable with just Claude when kiro-cli isn't installed.
        if let Some(kiro) = AcpAdapter::kiro_cli() {
            tracing::info!(name = kiro.name(), "registering ACP-backed agent");
            session_manager.register_adapter(Arc::new(kiro));
        } else {
            tracing::debug!("kiro-cli not found on PATH — ACP adapter not registered");
        }
    }

    let session_arc = Arc::new(tokio::sync::Mutex::new(session_manager));

    let bridge_session = session_arc.clone();
    let bridge_cost = cost_tracker.clone();
    let bridge_db = db.clone();
    let bridge_memory = memory_manager.clone();
    let setup_session = session_arc.clone();
    let setup_db = db.clone();
    let setup_memory = memory_manager.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .manage(session_arc)
        .manage(cost_tracker)
        .manage(db)
        .manage(memory_manager.clone())
        .setup(move |app| {
            let handle = app.handle().clone();
            let event_rx = Arc::new(tokio::sync::Mutex::new(event_rx));

            let notifier: panes_scheduler::NotifierRef = if test_mode {
                Arc::new(panes_scheduler::LogNotifier)
            } else {
                Arc::new(TauriNotifier { handle: handle.clone() })
            };

            let scheduler = Arc::new(Scheduler::new(
                setup_db.clone(),
                setup_session,
                setup_memory.clone(),
                broadcast_tx.clone(),
                notifier,
            ));
            app.manage(scheduler.clone());

            if test_mode {
                test_bridge::start_test_bridge(
                    bridge_session,
                    bridge_cost,
                    bridge_db,
                    bridge_memory,
                    broadcast_tx.clone(),
                );
            }

            let init_mgr = memory_manager;
            tauri::async_runtime::spawn(async move {
                init_mgr.init().await;
                init_mgr.spawn_health_monitor();
            });

            let startup_scheduler = scheduler;
            let startup_db = setup_db;
            tauri::async_runtime::spawn(async move {
                let enabled = startup_db
                    .execute(|conn| {
                        Ok(panes_core::features::is_feature_enabled(
                            conn,
                            panes_core::features::FEATURE_ROUTINES,
                        )
                        .unwrap_or(false))
                    })
                    .await
                    .unwrap_or(false);
                if enabled {
                    startup_scheduler.start();
                }
            });

            tauri::async_runtime::spawn(forward_events(handle, event_rx, broadcast_tx));
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
            commands::set_thread_model,
            commands::commit_changes,
            commands::revert_changes,
            commands::get_changed_files,
            commands::get_file_diff,
            commands::get_workspace_diff,
            commands::get_files_git_status,
            commands::list_git_repos,
            commands::commit_repos,
            commands::generate_commit_message,
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
            commands::get_cost_timeline,
            commands::get_workspace_cost_breakdown,
            commands::get_memory_backend_status,
            commands::set_memory_backend,
            commands::get_features,
            commands::set_feature_enabled,
            commands::create_routine,
            commands::update_routine,
            commands::delete_routine,
            commands::list_routines,
            commands::toggle_routine,
            commands::list_routine_executions,
            commands::get_routine_cost,
            commands::list_validator_types,
            commands::list_validators,
            commands::add_validator,
            commands::update_validator,
            commands::remove_validator,
        ])
        .run(tauri::generate_context!())
        .expect("error running panes");
}

fn register_fake_adapters(session_manager: &mut SessionManager) {
    // The default "claude-code" adapter in test mode cycles through scenarios
    // based on the prompt content, so tests can trigger specific behaviors.
    session_manager.register_adapter(Arc::new(PromptRoutedFakeAdapter {
        name: "claude-code",
    }));
    // Register the same prompt-routed scenarios under "kiro-cli" so mock E2E
    // tests can exercise the adapter picker without needing a real kiro-cli.
    session_manager.register_adapter(Arc::new(PromptRoutedFakeAdapter {
        name: "kiro-cli",
    }));
}

/// Wraps a fake AgentSession and performs the actual on-disk file write
/// AFTER each write-tool ToolRequest is yielded to the consumer. The
/// write happens inline in the stream (before the next event is polled),
/// which matches real agents: a tool_use is announced, then the tool
/// executes, then tool_result arrives. Since SessionManager's version
/// tracker hook runs between consecutive `stream.next()` calls, the hook
/// captures the pre-edit state (tombstone for missing files) strictly
/// before the write lands on disk.
struct WriteInjectingSession {
    inner: Box<dyn panes_adapters::AgentSession>,
    workspace_path: std::path::PathBuf,
}

#[async_trait::async_trait]
impl panes_adapters::AgentSession for WriteInjectingSession {
    fn init(&self) -> &panes_events::SessionInit {
        self.inner.init()
    }

    fn events(
        &mut self,
    ) -> std::pin::Pin<Box<dyn futures::Stream<Item = panes_events::AgentEvent> + Send>> {
        use futures::StreamExt;
        use panes_events::AgentEvent;
        let inner_stream = self.inner.events();
        let workspace_path = self.workspace_path.clone();
        Box::pin(async_stream::stream! {
            let mut s = inner_stream;
            while let Some(event) = s.next().await {
                let maybe_write = if let AgentEvent::ToolRequest {
                    ref tool_name,
                    ref input,
                    ..
                } = event
                {
                    if matches!(tool_name.as_str(), "Write" | "Edit" | "MultiEdit" | "NotebookEdit") {
                        input
                            .get("file_path")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    } else {
                        None
                    }
                } else {
                    None
                };
                yield event;
                // After the consumer has processed the ToolRequest (and
                // the version-tracker hook has tombstoned the path), land
                // the write on disk. A brief yield ensures the consumer
                // task has actually polled the stream again before we
                // produce the next event.
                if let Some(rel) = maybe_write {
                    tokio::task::yield_now().await;
                    let path = workspace_path.join(&rel);
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent).ok();
                    }
                    let _ = std::fs::write(
                        &path,
                        format!("// modified by panes test\n// file: {rel}\n"),
                    );
                }
            }
        })
    }

    async fn approve(&self, tool_use_id: &str) -> anyhow::Result<()> {
        self.inner.approve(tool_use_id).await
    }

    async fn reject(&self, tool_use_id: &str, reason: &str) -> anyhow::Result<()> {
        self.inner.reject(tool_use_id, reason).await
    }

    async fn cancel(&self) -> anyhow::Result<()> {
        self.inner.cancel().await
    }

    async fn set_model(&self, model: &str) -> anyhow::Result<()> {
        self.inner.set_model(model).await
    }
}

struct PromptRoutedFakeAdapter {
    /// Adapter id surfaced to the UI. Same scenario logic for every name —
    /// the only difference is which adapter the frontend picker shows.
    name: &'static str,
}

#[async_trait::async_trait]
impl panes_adapters::AgentAdapter for PromptRoutedFakeAdapter {
    fn name(&self) -> &str {
        self.name
    }

    async fn spawn(
        &self,
        workspace_path: &std::path::Path,
        prompt: &str,
        context: &panes_events::SessionContext,
        model: Option<&str>,
        _agent: Option<&str>,
    ) -> anyhow::Result<Box<dyn panes_adapters::AgentSession>> {
        let lower = prompt.to_lowercase();
        let (scenario, delay) = if lower.contains("slow") {
            (route_prompt(prompt), 500)
        } else {
            (route_prompt(prompt), 80)
        };

        let adapter = FakeAdapter::new(scenario.clone()).with_delay(delay);
        let session = adapter.spawn(workspace_path, prompt, context, model, None).await?;

        // For FileEdit scenarios, wrap the session's event stream so each
        // ToolRequest for a write tool performs the on-disk write *after*
        // the event is yielded. SessionManager's version-tracker hook runs
        // synchronously on ToolRequest before the next event is pulled, so
        // by the time we write, the hook has already captured the pre-edit
        // state (a tombstone for files that didn't exist). This mirrors how
        // real agents behave — tool_use announced before the write hits
        // disk — without the ordering hazards of an unrelated sleep task.
        if let FakeScenario::FileEdit { .. } = &scenario {
            Ok(Box::new(WriteInjectingSession {
                inner: session,
                workspace_path: workspace_path.to_path_buf(),
            }))
        } else {
            Ok(session)
        }
    }

    async fn resume(
        &self,
        workspace_path: &std::path::Path,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
        _agent: Option<&str>,
    ) -> anyhow::Result<Box<dyn panes_adapters::AgentSession>> {
        let scenario = route_prompt(prompt);
        let adapter = FakeAdapter::new(scenario).with_delay(80);
        adapter.resume(workspace_path, session_id, prompt, model, None).await
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

    if lower.contains("validate") {
        // Complete summary references a path that will not exist inside the
        // temp workspaces used by fullstack tests — triggers the citation
        // validator when the FEATURE_VALIDATORS flag and a citation rule
        // are configured.
        FakeScenario::TextOnly {
            response: "See src/missing.rs for the bug.".to_string(),
        }
    } else if lower.contains("error") || lower.contains("fail") {
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
    broadcast_tx: broadcast::Sender<ThreadEvent>,
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
                for event in batch {
                    let _ = broadcast_tx.send(event);
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
                    for event in batch {
                        let _ = broadcast_tx.send(event);
                    }
                }
            }
        }
    }
}
