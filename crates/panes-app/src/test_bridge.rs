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

use panes_core::db::DbHandle;
use panes_core::session::SessionManager;
use panes_cost::CostTracker;
use panes_events::{SessionContext, ThreadEvent};
use panes_memory::manager::MemoryManager;
use panes_memory::{BriefingStore, MemoryStore};

pub type SessionState = Arc<Mutex<SessionManager>>;
pub type DbState = DbHandle;
pub type MemoryManagerState = Arc<MemoryManager>;

#[derive(Clone)]
struct BridgeState {
    session_manager: SessionState,
    #[allow(dead_code)]
    cost_tracker: Arc<CostTracker>,
    db: DbState,
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
            let id2 = id.clone();
            let expanded2 = expanded.clone();
            let name2 = name.to_string();
            state.db.execute(move |conn| {
                conn.execute(
                    "INSERT INTO workspaces (id, path, name, default_agent, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![id2, expanded2, name2, "claude-code", now],
                )?;
                Ok(())
            }).await.map_err(|e| e.to_string())?;
            Ok(serde_json::json!({ "id": id, "path": expanded, "name": name, "defaultAgent": "claude-code" }))
        }
        "list_workspaces" => {
            state.db.execute(|conn| {
                let mut stmt = conn.prepare("SELECT id, path, name, default_agent, budget_cap FROM workspaces ORDER BY created_at")?;
                let rows: Vec<Value> = stmt.query_map([], |row| {
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "path": row.get::<_, String>(1)?,
                        "name": row.get::<_, String>(2)?,
                        "defaultAgent": row.get::<_, Option<String>>(3)?,
                        "budgetCap": row.get::<_, Option<f64>>(4).unwrap_or(None),
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
                Ok(Value::Array(rows))
            }).await.map_err(|e| e.to_string())
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
            let tid = thread_id.to_string();
            let snapshot_hash: String = state.db.execute(move |conn| {
                Ok(conn.query_row(
                    "SELECT snapshot_ref FROM threads WHERE id = ?1",
                    rusqlite::params![tid],
                    |row| row.get(0),
                )?)
            }).await.map_err(|e| format!("no snapshot for thread: {e}"))?;
            let expanded = crate::commands::expand_tilde(workspace_path);
            let path = std::path::PathBuf::from(&expanded);
            panes_core::git::revert(&path, &panes_core::git::SnapshotRef { commit_hash: snapshot_hash })
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "list_threads" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?.to_string();
            state.db.execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, workspace_id, prompt, status, cost_usd, created_at, is_routine, routine_id FROM threads WHERE workspace_id = ?1 ORDER BY created_at DESC"
                )?;
                let rows: Vec<Value> = stmt.query_map(rusqlite::params![workspace_id], |row| {
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "workspaceId": row.get::<_, String>(1)?,
                        "prompt": row.get::<_, String>(2)?,
                        "status": row.get::<_, String>(3)?,
                        "costUsd": row.get::<_, f64>(4).unwrap_or(0.0),
                        "createdAt": row.get::<_, String>(5)?,
                        "isRoutine": row.get::<_, i32>(6).unwrap_or(0) != 0,
                        "routineId": row.get::<_, Option<String>>(7).unwrap_or(None),
                        "events": [],
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
                Ok(Value::Array(rows))
            }).await.map_err(|e| e.to_string())
        }
        "list_all_threads" => {
            state.db.execute(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, workspace_id, prompt, status, cost_usd, summary, duration_ms, created_at, is_routine, routine_id FROM threads ORDER BY created_at DESC LIMIT 100"
                )?;
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
                        "isRoutine": row.get::<_, i32>(8).unwrap_or(0) != 0,
                        "routineId": row.get::<_, Option<String>>(9).unwrap_or(None),
                        "events": [],
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();
                Ok(Value::Array(rows))
            }).await.map_err(|e| e.to_string())
        }
        "delete_thread" => {
            let thread_id = args["threadId"].as_str().ok_or("missing threadId")?.to_string();
            state.db.execute(move |conn| {
                let tx = conn.unchecked_transaction()?;
                tx.execute("DELETE FROM events WHERE thread_id = ?1", rusqlite::params![thread_id])?;
                tx.execute("DELETE FROM costs WHERE thread_id = ?1", rusqlite::params![thread_id])?;
                tx.execute("DELETE FROM threads WHERE id = ?1", rusqlite::params![thread_id])?;
                tx.commit()?;
                Ok(())
            }).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "remove_workspace" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?.to_string();
            state.db.execute(move |conn| {
                let tx = conn.unchecked_transaction()?;
                if panes_core::db::routine_tables_exist(conn) {
                    tx.execute("DELETE FROM routine_executions WHERE routine_id IN (SELECT id FROM routines WHERE workspace_id = ?1)", rusqlite::params![workspace_id])?;
                    tx.execute("DELETE FROM routines WHERE workspace_id = ?1", rusqlite::params![workspace_id])?;
                }
                tx.execute("DELETE FROM events WHERE thread_id IN (SELECT id FROM threads WHERE workspace_id = ?1)", rusqlite::params![workspace_id])?;
                tx.execute("DELETE FROM costs WHERE workspace_id = ?1", rusqlite::params![workspace_id])?;
                tx.execute("DELETE FROM threads WHERE workspace_id = ?1", rusqlite::params![workspace_id])?;
                tx.execute("DELETE FROM workspaces WHERE id = ?1", rusqlite::params![workspace_id])?;
                tx.commit()?;
                Ok(())
            }).await.map_err(|e| e.to_string())?;
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
            let total: f64 = state.db.execute(|conn| {
                Ok(conn.query_row(
                    "SELECT COALESCE(SUM(total_usd), 0.0) FROM costs", [], |row| row.get(0),
                )?)
            }).await.map_err(|e| e.to_string())?;
            Ok(serde_json::json!(total))
        }
        "get_workspace_cost" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?.to_string();
            let total: f64 = state.db.execute(move |conn| {
                Ok(conn.query_row(
                    "SELECT COALESCE(SUM(total_usd), 0.0) FROM costs WHERE workspace_id = ?1",
                    rusqlite::params![workspace_id], |row| row.get(0),
                )?)
            }).await.map_err(|e| e.to_string())?;
            Ok(serde_json::json!(total))
        }
        "get_changed_files" => {
            Ok(serde_json::json!([]))
        }
        "extract_memories" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?;
            let thread_id = args["threadId"].as_str().ok_or("missing threadId")?;
            let transcript = args["transcript"].as_str().ok_or("missing transcript")?;
            let memories = state.memory_manager
                .add(transcript, Some(workspace_id), thread_id)
                .await
                .map_err(|e| e.to_string())?;
            let infos: Vec<Value> = memories.into_iter().map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "workspaceId": m.workspace_id,
                    "memoryType": m.memory_type.to_string(),
                    "content": m.content,
                    "sourceThreadId": m.source_thread_id,
                    "pinned": m.pinned,
                    "createdAt": m.created_at.to_rfc3339(),
                })
            }).collect();
            Ok(Value::Array(infos))
        }
        "get_memories" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?;
            let memories = state.memory_manager
                .get_all(Some(workspace_id))
                .await
                .map_err(|e| e.to_string())?;
            let infos: Vec<Value> = memories.into_iter().map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "workspaceId": m.workspace_id,
                    "memoryType": m.memory_type.to_string(),
                    "content": m.content,
                    "sourceThreadId": m.source_thread_id,
                    "pinned": m.pinned,
                    "createdAt": m.created_at.to_rfc3339(),
                })
            }).collect();
            Ok(Value::Array(infos))
        }
        "search_memories" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?;
            let query = args["query"].as_str().ok_or("missing query")?;
            let limit = args["limit"].as_u64().unwrap_or(10) as usize;
            let memories = state.memory_manager
                .search(query, Some(workspace_id), limit)
                .await
                .map_err(|e| e.to_string())?;
            let infos: Vec<Value> = memories.into_iter().map(|m| {
                serde_json::json!({
                    "id": m.id,
                    "workspaceId": m.workspace_id,
                    "memoryType": m.memory_type.to_string(),
                    "content": m.content,
                    "sourceThreadId": m.source_thread_id,
                    "pinned": m.pinned,
                    "createdAt": m.created_at.to_rfc3339(),
                })
            }).collect();
            Ok(Value::Array(infos))
        }
        "update_memory" => {
            let memory_id = args["memoryId"].as_str().ok_or("missing memoryId")?;
            let content = args["content"].as_str().ok_or("missing content")?;
            state.memory_manager
                .update(memory_id, content)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "delete_memory" => {
            let memory_id = args["memoryId"].as_str().ok_or("missing memoryId")?;
            state.memory_manager
                .delete(memory_id)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "pin_memory" => {
            let memory_id = args["memoryId"].as_str().ok_or("missing memoryId")?;
            let pinned = args["pinned"].as_bool().ok_or("missing pinned")?;
            state.memory_manager
                .pin(memory_id, pinned)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "get_briefing" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?;
            let briefing = state.memory_manager
                .get_briefing(workspace_id)
                .await
                .map_err(|e| e.to_string())?;
            match briefing {
                Some(b) => Ok(serde_json::json!({
                    "workspaceId": b.workspace_id,
                    "content": b.content,
                })),
                None => Ok(Value::Null),
            }
        }
        "set_briefing" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?;
            let content = args["content"].as_str().ok_or("missing content")?;
            state.memory_manager
                .set_briefing(workspace_id, content)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "delete_briefing" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?;
            state.memory_manager
                .delete_briefing(workspace_id)
                .await
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "set_workspace_budget_cap" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?.to_string();
            let budget_cap = args["budgetCap"].as_f64();
            state.db.execute(move |conn| {
                conn.execute(
                    "UPDATE workspaces SET budget_cap = ?1 WHERE id = ?2",
                    rusqlite::params![budget_cap, workspace_id],
                )?;
                Ok(())
            }).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "get_memory_backend_status" => {
            Ok(serde_json::json!({
                "backend": state.memory_manager.get_active_backend().to_string(),
                "mem0Configured": state.memory_manager.is_mem0_configured(),
            }))
        }
        "set_memory_backend" => {
            let backend = args["backend"].as_str().ok_or("missing backend")?;
            state.memory_manager.set_active_backend(backend).map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "get_features" => {
            let features: Vec<Value> = state.db.execute(|conn| {
                let features = panes_core::features::list_features(conn)?;
                Ok(features.into_iter().map(|f| serde_json::json!({
                    "id": f.id,
                    "enabled": f.enabled,
                    "label": f.label,
                    "description": f.description,
                })).collect())
            }).await.map_err(|e| e.to_string())?;
            Ok(Value::Array(features))
        }
        "set_feature_enabled" => {
            let feature_id = args["featureId"].as_str().ok_or("missing featureId")?.to_string();
            let enabled = args["enabled"].as_bool().ok_or("missing enabled")?;
            let fid = feature_id.clone();
            state.db.execute(move |conn| {
                panes_core::features::set_feature_enabled(conn, &fid, enabled)?;
                if fid == panes_core::features::FEATURE_ROUTINES && enabled {
                    panes_core::db::create_routine_tables(conn)?;
                }
                Ok(())
            }).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "create_routine" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?.to_string();
            let prompt = args["prompt"].as_str().ok_or("missing prompt")?.to_string();
            let cron_expr = args["cronExpr"].as_str().ok_or("missing cronExpr")?.to_string();
            let budget_cap = args["budgetCap"].as_f64();
            let on_complete = args["onComplete"].as_str().unwrap_or(r#"{"action":"notify"}"#).to_string();
            let on_failure = args["onFailure"].as_str().unwrap_or(r#"{"action":"notify"}"#).to_string();
            let id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            let rid = id.clone();
            let ts = now.clone();
            state.db.execute(move |conn| {
                conn.execute(
                    "INSERT INTO routines (id, workspace_id, prompt, cron_expr, budget_cap, on_complete, on_failure, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![rid, workspace_id, prompt, cron_expr, budget_cap, on_complete, on_failure, ts],
                )?;
                Ok(())
            }).await.map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "id": id,
                "workspaceId": args["workspaceId"],
                "prompt": args["prompt"],
                "cronExpr": args["cronExpr"],
                "budgetCap": args["budgetCap"],
                "onComplete": serde_json::from_str::<Value>(&args["onComplete"].as_str().unwrap_or(r#"{"action":"notify"}"#)).unwrap_or(Value::Null),
                "onFailure": serde_json::from_str::<Value>(&args["onFailure"].as_str().unwrap_or(r#"{"action":"notify"}"#)).unwrap_or(Value::Null),
                "enabled": true,
                "lastRunAt": null,
                "createdAt": now,
            }))
        }
        "list_routines" => {
            let workspace_id = args["workspaceId"].as_str().map(|s| s.to_string());
            let routines: Vec<Value> = state.db.execute(move |conn| {
                let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match &workspace_id {
                    Some(wid) => (
                        "SELECT id, workspace_id, prompt, cron_expr, budget_cap, on_complete, on_failure, enabled, last_run_at, created_at FROM routines WHERE workspace_id = ?1 ORDER BY created_at DESC",
                        vec![Box::new(wid.clone())],
                    ),
                    None => (
                        "SELECT id, workspace_id, prompt, cron_expr, budget_cap, on_complete, on_failure, enabled, last_run_at, created_at FROM routines ORDER BY created_at DESC",
                        vec![],
                    ),
                };
                let mut stmt = conn.prepare(sql)?;
                let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
                let rows = stmt.query_map(params_refs.as_slice(), |row| {
                    let oc: String = row.get(5)?;
                    let of: String = row.get(6)?;
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "workspaceId": row.get::<_, String>(1)?,
                        "prompt": row.get::<_, String>(2)?,
                        "cronExpr": row.get::<_, String>(3)?,
                        "budgetCap": row.get::<_, Option<f64>>(4)?,
                        "onComplete": serde_json::from_str::<Value>(&oc).unwrap_or(Value::Null),
                        "onFailure": serde_json::from_str::<Value>(&of).unwrap_or(Value::Null),
                        "enabled": row.get::<_, bool>(7)?,
                        "lastRunAt": row.get::<_, Option<String>>(8)?,
                        "createdAt": row.get::<_, String>(9)?,
                    }))
                })?;
                let mut result = Vec::new();
                for row in rows { result.push(row?); }
                Ok(result)
            }).await.map_err(|e| e.to_string())?;
            Ok(Value::Array(routines))
        }
        "toggle_routine" => {
            let routine_id = args["routineId"].as_str().ok_or("missing routineId")?.to_string();
            let enabled = args["enabled"].as_bool().ok_or("missing enabled")?;
            state.db.execute(move |conn| {
                conn.execute("UPDATE routines SET enabled = ?1 WHERE id = ?2", rusqlite::params![enabled, routine_id])?;
                Ok(())
            }).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "delete_routine" => {
            let routine_id = args["routineId"].as_str().ok_or("missing routineId")?.to_string();
            state.db.execute(move |conn| {
                let tx = conn.unchecked_transaction()?;
                tx.execute("DELETE FROM routine_executions WHERE routine_id = ?1", rusqlite::params![routine_id])?;
                tx.execute("DELETE FROM routines WHERE id = ?1", rusqlite::params![routine_id])?;
                tx.commit()?;
                Ok(())
            }).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "update_routine" => {
            let routine_id = args["routineId"].as_str().ok_or("missing routineId")?.to_string();
            let prompt = args["prompt"].as_str().map(|s| s.to_string());
            let cron_expr = args["cronExpr"].as_str().map(|s| s.to_string());
            let budget_cap = args["budgetCap"].as_f64();
            let on_complete = args["onComplete"].as_str().map(|s| s.to_string());
            let on_failure = args["onFailure"].as_str().map(|s| s.to_string());
            state.db.execute(move |conn| {
                if let Some(p) = prompt { conn.execute("UPDATE routines SET prompt = ?1 WHERE id = ?2", rusqlite::params![p, routine_id])?; }
                if let Some(ce) = cron_expr { conn.execute("UPDATE routines SET cron_expr = ?1 WHERE id = ?2", rusqlite::params![ce, routine_id])?; }
                if let Some(bc) = budget_cap { conn.execute("UPDATE routines SET budget_cap = ?1 WHERE id = ?2", rusqlite::params![bc, routine_id])?; }
                if let Some(oc) = on_complete { conn.execute("UPDATE routines SET on_complete = ?1 WHERE id = ?2", rusqlite::params![oc, routine_id])?; }
                if let Some(of) = on_failure { conn.execute("UPDATE routines SET on_failure = ?1 WHERE id = ?2", rusqlite::params![of, routine_id])?; }
                Ok(())
            }).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "list_routine_executions" => {
            let routine_id = args["routineId"].as_str().ok_or("missing routineId")?.to_string();
            let limit = args["limit"].as_u64().unwrap_or(50) as u32;
            let execs: Vec<Value> = state.db.execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, routine_id, thread_id, status, cost_usd, started_at, completed_at, error_message
                     FROM routine_executions WHERE routine_id = ?1 ORDER BY started_at DESC LIMIT ?2"
                )?;
                let rows = stmt.query_map(rusqlite::params![routine_id, limit], |row| {
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "routineId": row.get::<_, String>(1)?,
                        "threadId": row.get::<_, Option<String>>(2)?,
                        "status": row.get::<_, String>(3)?,
                        "costUsd": row.get::<_, f64>(4)?,
                        "startedAt": row.get::<_, String>(5)?,
                        "completedAt": row.get::<_, Option<String>>(6)?,
                        "errorMessage": row.get::<_, Option<String>>(7)?,
                    }))
                })?;
                let mut result = Vec::new();
                for row in rows { result.push(row?); }
                Ok(result)
            }).await.map_err(|e| e.to_string())?;
            Ok(Value::Array(execs))
        }
        "get_routine_cost" => {
            let routine_id = args["routineId"].as_str().ok_or("missing routineId")?.to_string();
            let total: f64 = state.db.execute(move |conn| {
                Ok(conn.query_row(
                    "SELECT COALESCE(SUM(cost_usd), 0.0) FROM routine_executions WHERE routine_id = ?1",
                    rusqlite::params![routine_id], |row| row.get(0),
                )?)
            }).await.map_err(|e| e.to_string())?;
            Ok(serde_json::json!(total))
        }
        "set_workspace_default_agent" => {
            let workspace_id = args["workspaceId"].as_str().ok_or("missing workspaceId")?.to_string();
            let agent = args["agent"].as_str().ok_or("missing agent")?.to_string();
            state.db.execute(move |conn| {
                conn.execute(
                    "UPDATE workspaces SET default_agent = ?1 WHERE id = ?2",
                    rusqlite::params![agent, workspace_id],
                )?;
                Ok(())
            }).await.map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        _ => Err(format!("unknown command: {cmd}")),
    }
}
