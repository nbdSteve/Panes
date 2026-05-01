use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures::stream::StreamExt;
use futures::SinkExt;
use serde_json::Value;
use tokio::sync::{broadcast, Mutex};
use tower_http::cors::CorsLayer;

use panes_core::session::SessionManager;
use panes_cost::CostTracker;
use panes_events::{SessionContext, ThreadEvent};
use panes_memory::manager::MemoryManager;

pub type SessionState = Arc<Mutex<SessionManager>>;
pub type DbState = Arc<std::sync::Mutex<rusqlite::Connection>>;
pub type MemoryManagerState = Arc<MemoryManager>;

#[derive(Clone)]
struct BridgeState {
    session_manager: SessionState,
    #[allow(dead_code)]
    cost_tracker: Arc<CostTracker>,
    db: DbState,
    #[allow(dead_code)]
    memory_manager: MemoryManagerState,
    events_tx: broadcast::Sender<ThreadEvent>,
}

pub fn start_test_bridge(
    session_manager: SessionState,
    cost_tracker: Arc<CostTracker>,
    db: DbState,
    memory_manager: MemoryManagerState,
    events_tx: broadcast::Sender<ThreadEvent>,
) {
    let state = BridgeState {
        session_manager,
        cost_tracker,
        db,
        memory_manager,
        events_tx,
    };

    tauri::async_runtime::spawn(async move {
        let app = Router::new()
            .route("/ws", get(ws_handler))
            .layer(CorsLayer::very_permissive())
            .with_state(state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:3001")
            .await
            .expect("failed to bind test bridge port 3001");
        eprintln!("[panes] test bridge (ws) listening on ws://127.0.0.1:3001/ws");
        axum::serve(listener, app).await.ok();
    });
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<BridgeState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(socket: WebSocket, state: BridgeState) {
    let (ws_tx, mut ws_rx) = socket.split();
    let ws_tx = Arc::new(Mutex::new(ws_tx));

    let event_tx = ws_tx.clone();
    let mut event_rx = state.events_tx.subscribe();
    let event_task = tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            if let Ok(json) = serde_json::to_string(&event) {
                let msg = serde_json::json!({
                    "type": "event",
                    "payload": serde_json::from_str::<Value>(&json).unwrap_or(Value::Null),
                });
                let mut tx = event_tx.lock().await;
                if tx.send(Message::Text(msg.to_string().into())).await.is_err() {
                    break;
                }
            }
        }
    });

    while let Some(Ok(msg)) = ws_rx.next().await {
        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Close(_) => break,
            _ => continue,
        };

        let parsed: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = parsed["id"].as_str().unwrap_or("0").to_string();
        let cmd = parsed["cmd"].as_str().unwrap_or("");
        let args = &parsed["args"];

        let result = dispatch_command(cmd, args, &state).await;
        let response = match result {
            Ok(val) => serde_json::json!({ "type": "response", "id": id, "ok": val }),
            Err(e) => serde_json::json!({ "type": "response", "id": id, "error": e }),
        };

        let mut tx = ws_tx.lock().await;
        if tx.send(Message::Text(response.to_string().into())).await.is_err() {
            break;
        }
    }

    event_task.abort();
}

async fn dispatch_command(
    cmd: &str,
    args: &Value,
    state: &BridgeState,
) -> Result<Value, String> {
    match cmd {
        "add_workspace" => {
            let path = args["path"].as_str().ok_or("missing path")?;
            let name = args["name"].as_str().ok_or("missing name")?;
            let expanded = crate::commands::expand_tilde(path);
            let workspace_path = std::path::PathBuf::from(&expanded);
            if !workspace_path.exists() {
                return Err(format!("Path does not exist: {expanded}"));
            }
            let id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT INTO workspaces (id, path, name, default_agent, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![id, expanded, name, "claude-code", now],
            ).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "id": id, "path": expanded, "name": name, "defaultAgent": "claude-code" }))
        }
        "list_workspaces" => {
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            let mut stmt = conn.prepare("SELECT id, path, name, default_agent FROM workspaces ORDER BY created_at")
                .map_err(|e| e.to_string())?;
            let rows: Vec<Value> = stmt.query_map([], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "path": row.get::<_, String>(1)?,
                    "name": row.get::<_, String>(2)?,
                    "defaultAgent": row.get::<_, Option<String>>(3)?,
                }))
            }).map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
            Ok(Value::Array(rows))
        }
        "start_thread" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?;
            let workspace_path = args["workspacePath"].as_str().ok_or("missing workspacePath")?;
            let workspace_name = args["workspaceName"].as_str().ok_or("missing workspaceName")?;
            let prompt = args["prompt"].as_str().ok_or("missing prompt")?;
            let agent = args["agent"].as_str().filter(|s| !s.is_empty()).map(String::from);
            let model = args["model"].as_str().filter(|s| !s.is_empty()).map(String::from);
            let expanded = crate::commands::expand_tilde(workspace_path);

            let workspace = panes_core::session::Workspace {
                id: workspace_id.to_string(),
                path: std::path::PathBuf::from(&expanded),
                name: workspace_name.to_string(),
                default_agent: agent.clone(),
                budget_cap: None,
            };

            let context = SessionContext {
                briefing: None,
                memories: vec![],
                budget_cap: None,
            };

            let agent_name = agent.unwrap_or_else(|| "claude-code".to_string());
            let mgr = state.session_manager.lock().await;
            let thread_id = mgr.start_thread(&workspace, prompt, &agent_name, context, model.as_deref())
                .await
                .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "threadId": thread_id,
                "memoryCount": 0,
                "hasBriefing": false,
            }))
        }
        "resume_thread" => {
            let thread_id = args["threadId"].as_str().ok_or("missing threadId")?;
            let workspace_path = args["workspacePath"].as_str().ok_or("missing workspacePath")?;
            let workspace_name = args["workspaceName"].as_str().ok_or("missing workspaceName")?;
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?;
            let prompt = args["prompt"].as_str().ok_or("missing prompt")?;
            let agent = args["agent"].as_str().filter(|s| !s.is_empty()).map(String::from);
            let model = args["model"].as_str().filter(|s| !s.is_empty()).map(String::from);
            let expanded = crate::commands::expand_tilde(workspace_path);

            let workspace = panes_core::session::Workspace {
                id: workspace_id.to_string(),
                path: std::path::PathBuf::from(&expanded),
                name: workspace_name.to_string(),
                default_agent: agent.clone(),
                budget_cap: None,
            };

            let agent_name = agent.unwrap_or_else(|| "claude-code".to_string());
            let mgr = state.session_manager.lock().await;
            mgr.resume_thread(thread_id, &workspace, prompt, &agent_name, model.as_deref())
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "approve_gate" => {
            let thread_id = args["threadId"].as_str().ok_or("missing threadId")?;
            let tool_use_id = args["toolUseId"].as_str().ok_or("missing toolUseId")?;
            let mgr = state.session_manager.lock().await;
            mgr.approve(thread_id, tool_use_id).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "reject_gate" => {
            let thread_id = args["threadId"].as_str().ok_or("missing threadId")?;
            let tool_use_id = args["toolUseId"].as_str().ok_or("missing toolUseId")?;
            let reason = args["reason"].as_str().unwrap_or("rejected by test");
            let mgr = state.session_manager.lock().await;
            mgr.reject(thread_id, tool_use_id, reason).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "cancel_thread" => {
            let thread_id = args["threadId"].as_str().ok_or("missing threadId")?;
            let mgr = state.session_manager.lock().await;
            mgr.cancel(thread_id).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "commit_changes" => {
            let workspace_path = args["workspacePath"].as_str().ok_or("missing workspacePath")?;
            let message = args["message"].as_str().ok_or("missing message")?;
            let expanded = crate::commands::expand_tilde(workspace_path);
            let path = std::path::PathBuf::from(&expanded);
            let hash = panes_core::git::commit(&path, message)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::String(hash))
        }
        "revert_changes" => {
            let workspace_path = args["workspacePath"].as_str().ok_or("missing workspacePath")?;
            let thread_id = args["threadId"].as_str().ok_or("missing threadId")?;
            let snapshot_hash: String = {
                let conn = state.db.lock().map_err(|e| e.to_string())?;
                conn.query_row(
                    "SELECT snapshot_ref FROM threads WHERE id = ?1",
                    rusqlite::params![thread_id],
                    |row| row.get(0),
                ).map_err(|e| format!("no snapshot for thread: {e}"))?
            };
            let expanded = crate::commands::expand_tilde(workspace_path);
            let path = std::path::PathBuf::from(&expanded);
            panes_core::git::revert(&path, &panes_core::git::SnapshotRef { commit_hash: snapshot_hash })
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "list_threads" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?;
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            let mut stmt = conn.prepare(
                "SELECT id, prompt, status, cost_usd, created_at FROM threads WHERE workspace_id = ?1 ORDER BY created_at DESC"
            ).map_err(|e| e.to_string())?;
            let rows: Vec<Value> = stmt.query_map(rusqlite::params![workspace_id], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "workspaceId": workspace_id,
                    "prompt": row.get::<_, String>(1)?,
                    "status": row.get::<_, String>(2)?,
                    "costUsd": row.get::<_, f64>(3).unwrap_or(0.0),
                    "createdAt": row.get::<_, String>(4)?,
                    "events": [],
                }))
            }).map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
            Ok(Value::Array(rows))
        }
        "list_all_threads" => {
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            let mut stmt = conn.prepare(
                "SELECT id, workspace_id, prompt, status, cost_usd, summary, duration_ms, created_at FROM threads ORDER BY created_at DESC LIMIT 100"
            ).map_err(|e| e.to_string())?;
            let rows: Vec<Value> = stmt.query_map([], |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "workspaceId": row.get::<_, String>(1)?,
                    "prompt": row.get::<_, String>(2)?,
                    "status": row.get::<_, String>(3)?,
                    "costUsd": row.get::<_, f64>(4).unwrap_or(0.0),
                    "summary": row.get::<_, Option<String>>(5).unwrap_or(None),
                    "durationMs": row.get::<_, Option<i64>>(6).unwrap_or(None),
                    "createdAt": row.get::<_, String>(7)?,
                    "events": [],
                }))
            }).map_err(|e| e.to_string())?
            .filter_map(|r| r.ok())
            .collect();
            Ok(Value::Array(rows))
        }
        "delete_thread" => {
            let thread_id = args["threadId"].as_str().ok_or("missing threadId")?;
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM events WHERE thread_id = ?1", rusqlite::params![thread_id]).map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM costs WHERE thread_id = ?1", rusqlite::params![thread_id]).map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM threads WHERE id = ?1", rusqlite::params![thread_id]).map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "remove_workspace" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?;
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM events WHERE thread_id IN (SELECT id FROM threads WHERE workspace_id = ?1)", rusqlite::params![workspace_id]).map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM costs WHERE workspace_id = ?1", rusqlite::params![workspace_id]).map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM threads WHERE workspace_id = ?1", rusqlite::params![workspace_id]).map_err(|e| e.to_string())?;
            tx.execute("DELETE FROM workspaces WHERE id = ?1", rusqlite::params![workspace_id]).map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "list_adapters" => {
            let mgr = state.session_manager.lock().await;
            Ok(Value::Array(mgr.list_adapters().into_iter().map(Value::String).collect()))
        }
        "list_agents" => Ok(serde_json::json!([])),
        "list_models" => {
            let adapter = args["adapter"].as_str().unwrap_or("claude-code");
            let mgr = state.session_manager.lock().await;
            let models = mgr.list_models(adapter).await.map_err(|e| e.to_string())?;
            Ok(serde_json::to_value(models).unwrap_or(Value::Array(vec![])))
        }
        "get_aggregate_cost" => {
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            let total: f64 = conn.query_row(
                "SELECT COALESCE(SUM(total_usd), 0.0) FROM costs", [], |row| row.get(0),
            ).map_err(|e| e.to_string())?;
            Ok(serde_json::json!(total))
        }
        "get_workspace_cost" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?;
            let conn = state.db.lock().map_err(|e| e.to_string())?;
            let total: f64 = conn.query_row(
                "SELECT COALESCE(SUM(total_usd), 0.0) FROM costs WHERE workspace_id = ?1",
                rusqlite::params![workspace_id], |row| row.get(0),
            ).map_err(|e| e.to_string())?;
            Ok(serde_json::json!(total))
        }
        "get_changed_files" => {
            Ok(serde_json::json!([]))
        }
        "get_briefing" | "set_briefing" | "delete_briefing"
        | "get_memories" | "search_memories" | "extract_memories"
        | "update_memory" | "delete_memory" | "pin_memory"
        | "set_workspace_budget_cap"
        | "get_memory_backend_status" | "set_memory_backend" => {
            Ok(Value::Null)
        }
        _ => Err(format!("unknown command: {cmd}")),
    }
}
