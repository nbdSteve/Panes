//! ACP session lifecycle primitives — one helper per JSON-RPC method we drive.
//!
//! These are thin async wrappers on top of [`AcpTransport`] that know the
//! shape of each payload and how to interpret the response (or lack thereof,
//! in the case of `session/set_mode` on kiro-cli, which signals readiness via
//! a `_kiro.dev/commands/available` notification instead of a normal reply).
//!
//! Port of `HaroldCLI/src/acp_client.rs:1162-1380`, minus:
//! - Hot-spare preheating
//! - Wasabi protocol variant (integer protocolVersion, different mode names)
//! - Session persistence map (Panes stores this in its own SQLite)
//! - Recording/replay hooks (Panes writes events to its `events` table)

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::Value;

use super::transport::AcpTransport;

/// ACP protocol version we negotiate. Tracks the latest verified-against value.
/// Update when we re-validate against a newer kiro-cli release.
pub const PROTOCOL_VERSION: &str = "2025-08-22";

/// Deadline for the `initialize` handshake. kiro-cli replies in <1s when healthy.
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(10);

/// Deadline for `session/new` / `session/load`. Allows MCP servers to load.
const SESSION_CREATE_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline for the `set_mode` drain loop. kiro-cli is usually <2s here.
///
/// Exposed via a `cfg(test)` setter so tests can exercise the timeout path
/// without waiting a real 10s. Reading via a getter lets us swap in a shorter
/// value during tests without plumbing the override through every caller.
#[cfg(not(test))]
const SET_MODE_TIMEOUT_DEFAULT: Duration = Duration::from_secs(10);
#[cfg(test)]
const SET_MODE_TIMEOUT_DEFAULT: Duration = Duration::from_millis(200);

fn set_mode_timeout() -> Duration {
    SET_MODE_TIMEOUT_DEFAULT
}

/// Deadline for `session/set_model` — normal round-trip response.
const SET_MODEL_TIMEOUT: Duration = Duration::from_secs(5);

/// Deadline for `session/cancel` ack.
const CANCEL_TIMEOUT: Duration = Duration::from_secs(3);

/// Metadata a client presents during `initialize`.
pub struct ClientInfo<'a> {
    pub name: &'a str,
    pub version: &'a str,
}

/// Response from the `initialize` handshake. The protocol version echoed back
/// may differ from what we sent — the agent is authoritative.
#[derive(Debug, Clone, PartialEq)]
pub struct InitResult {
    pub protocol_version: String,
}

/// One agent mode reported by the backend. `id` is what gets sent back via
/// `session/set_mode { modeId }`; `name` is the friendly label (optional —
/// some backends omit it, in which case we surface the id); `description`
/// is a longer blurb shown in the picker.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AcpMode {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Response from `session/new`: the newly-minted session id plus the lists
/// of models and modes the agent reports as available. kiro-cli puts models
/// at `/models/availableModels/*/modelId` and modes at `/modes/*`.
#[derive(Debug, Clone)]
pub struct NewSessionResult {
    pub session_id: String,
    pub available_models: Vec<String>,
    pub available_modes: Vec<AcpMode>,
}

/// Send `initialize`. Returns the agent's echoed protocol version.
pub async fn initialize(
    transport: &mut AcpTransport,
    client_info: &ClientInfo<'_>,
) -> Result<InitResult> {
    let id = transport
        .send_request(
            "initialize",
            serde_json::json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientInfo": {
                    "name": client_info.name,
                    "version": client_info.version,
                },
            }),
        )
        .await?;
    let resp = transport.wait_for_response(id, INITIALIZE_TIMEOUT).await?;
    let protocol_version = resp
        .get("protocolVersion")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    Ok(InitResult { protocol_version })
}

/// Send `session/new`. Returns the new session id and available model list.
pub async fn new_session(transport: &mut AcpTransport, cwd: &Path) -> Result<NewSessionResult> {
    let id = transport
        .send_request(
            "session/new",
            serde_json::json!({
                "cwd": cwd.to_string_lossy(),
                "mcpServers": [],
            }),
        )
        .await?;
    let resp = transport.wait_for_response(id, SESSION_CREATE_TIMEOUT).await?;
    let session_id = resp
        .get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("session/new response missing sessionId: {resp}"))?
        .to_string();
    let available_models = extract_models(&resp);
    let available_modes = extract_modes(&resp);
    Ok(NewSessionResult {
        session_id,
        available_models,
        available_modes,
    })
}

/// Response from a successful `session/load`. kiro-cli signals success via
/// the presence of a `modes` array (possibly empty); other fields mirror
/// `NewSessionResult`.
#[derive(Debug, Clone, Default)]
pub struct LoadSessionResult {
    pub resumed: bool,
    pub available_models: Vec<String>,
    pub available_modes: Vec<AcpMode>,
}

/// Send `session/load` to resume an existing session.
///
/// Returns a `LoadSessionResult` — `resumed: true` means the agent confirmed
/// the load (presence of `modes` in the response is how kiro-cli signals
/// success), `resumed: false` means the agent responded but didn't
/// recognise the session, `Err(_)` only on transport failure (which the
/// caller downgrades to `resumed: false` anyway).
///
/// Callers should fall back to [`new_session`] when `resumed` is false.
pub async fn load_session(
    transport: &mut AcpTransport,
    session_id: &str,
    cwd: &Path,
) -> Result<LoadSessionResult> {
    let id = transport
        .send_request(
            "session/load",
            serde_json::json!({
                "sessionId": session_id,
                "cwd": cwd.to_string_lossy(),
                "mcpServers": [],
            }),
        )
        .await?;
    // We don't use `?` on the wait — an error response here is a load failure,
    // not a transport failure. Same rationale as HaroldCLI's fallback logic.
    match transport.wait_for_response(id, SESSION_CREATE_TIMEOUT).await {
        Ok(resp) => {
            let resumed = resp.get("modes").is_some();
            Ok(LoadSessionResult {
                resumed,
                available_models: extract_models(&resp),
                available_modes: extract_modes(&resp),
            })
        }
        Err(e) => {
            tracing::warn!(error = %e, session_id, "session/load failed — caller should fall back");
            Ok(LoadSessionResult::default())
        }
    }
}

/// Send `session/set_mode` and drain events until kiro-cli signals readiness.
///
/// **kiro-cli quirk (verified 2026-02-07):** `session/set_mode` does not emit a
/// normal JSON-RPC response. Instead, the agent sends a
/// `_kiro.dev/commands/available` notification once MCP tools have finished
/// loading and the mode is active. If we naively called `wait_for_response`
/// we'd time out every time.
///
/// We therefore loop, consuming messages until either:
/// - the `_kiro.dev/commands/available` notification arrives → success
/// - the deadline expires → log a warning and return Ok (best-effort)
///
/// Any genuine response to our set_mode id is swallowed so it doesn't leak
/// out as a stale response later.
pub async fn set_mode(
    transport: &mut AcpTransport,
    session_id: &str,
    mode: &str,
) -> Result<()> {
    let req_id = transport
        .send_request(
            "session/set_mode",
            serde_json::json!({
                "sessionId": session_id,
                "modeId": mode,
            }),
        )
        .await?;

    let deadline = tokio::time::Instant::now() + set_mode_timeout();
    let mut got_signal = false;
    let mut got_response = false;

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let step = remaining.min(Duration::from_secs(2));
        match transport.read_message(step).await? {
            Some(msg) if msg.is_method("_kiro.dev/commands/available") => {
                got_signal = true;
                break;
            }
            Some(msg) if msg.is_response_for(req_id) => {
                // Consume the response so it doesn't pollute a later wait.
                got_response = true;
                // Keep looping — we still need the notification.
                tracing::debug!(mode, "consumed set_mode response, awaiting commands/available");
            }
            Some(msg) if msg.method.as_deref() == Some("_kiro.dev/metadata") => {
                // Metadata arriving during init is fine; swallow so it doesn't
                // count as a "real" event the surrounding loop cares about.
            }
            Some(msg) if msg.id.is_some() && msg.method.is_some() => {
                // Server-initiated request during set_mode — buffer for the
                // caller, but don't let it break the drain.
                transport.unread(msg);
                break;
            }
            Some(_other) => {
                // Other notifications / stale messages — drop silently.
            }
            None => continue,
        }
    }

    if !got_signal {
        tracing::warn!(
            mode,
            got_response,
            timeout = ?set_mode_timeout(),
            "set_mode: no commands/available signal — continuing anyway"
        );
    }
    Ok(())
}

/// Send `session/set_model` and wait for its acknowledgment.
pub async fn set_model(
    transport: &mut AcpTransport,
    session_id: &str,
    model_id: &str,
) -> Result<()> {
    let id = transport
        .send_request(
            "session/set_model",
            serde_json::json!({
                "sessionId": session_id,
                "modelId": model_id,
            }),
        )
        .await?;
    let _ = transport.wait_for_response(id, SET_MODEL_TIMEOUT).await?;
    Ok(())
}

/// Send `session/prompt`. Returns the client-generated request id so the
/// surrounding loop can match the eventual `stopReason` response.
///
/// The message is always wrapped into a single `text` block — image / other
/// content-block support can land later without breaking this signature.
pub async fn prompt(
    transport: &mut AcpTransport,
    session_id: &str,
    text: &str,
) -> Result<u64> {
    let req_id = transport
        .send_request(
            "session/prompt",
            serde_json::json!({
                "sessionId": session_id,
                "prompt": [{ "type": "text", "text": text }],
            }),
        )
        .await?;
    Ok(req_id)
}

/// Send `session/cancel`. Best-effort — the agent may not acknowledge if it's
/// already shutting down. Never blocks longer than [`CANCEL_TIMEOUT`].
pub async fn cancel(transport: &mut AcpTransport, session_id: &str) -> Result<()> {
    let id = transport
        .send_request(
            "session/cancel",
            serde_json::json!({ "sessionId": session_id }),
        )
        .await?;
    match transport.wait_for_response(id, CANCEL_TIMEOUT).await {
        Ok(_) => Ok(()),
        Err(e) => {
            tracing::debug!(error = %e, "session/cancel ack missed — proceeding with shutdown");
            Ok(())
        }
    }
}

/// Extract model ids from the `/models/availableModels` array in a response.
fn extract_models(resp: &Value) -> Vec<String> {
    resp.pointer("/models/availableModels")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("modelId").and_then(|v| v.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Extract the mode list from a session/new or session/load response.
///
/// kiro-cli returns modes at `/modes/availableModes`, same envelope as
/// `/models/availableModels`. Each entry has `id`, `name`, and often a
/// `description`. Older HaroldCLI test fixtures showed the list at the
/// top-level `modes` key — we accept both shapes so we stay compatible
/// with any backend that uses the flatter form.
pub(crate) fn extract_modes(resp: &Value) -> Vec<AcpMode> {
    // Preferred shape (current kiro-cli): /modes/availableModes
    if let Some(arr) = resp
        .pointer("/modes/availableModes")
        .and_then(|v| v.as_array())
    {
        return parse_mode_array(arr);
    }
    // Fallback shape: top-level `modes` is directly an array.
    if let Some(arr) = resp.get("modes").and_then(|v| v.as_array()) {
        return parse_mode_array(arr);
    }
    Vec::new()
}

fn parse_mode_array(arr: &[Value]) -> Vec<AcpMode> {
    arr.iter()
        .filter_map(|m| {
            let id = m.get("id").and_then(|v| v.as_str())?.to_string();
            let name = m
                .get("name")
                .and_then(|v| v.as_str())
                .map(String::from);
            let description = m
                .get("description")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            Some(AcpMode {
                id,
                name,
                description,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::transport::{AcpTransport, JsonRpcMessage};
    use std::collections::VecDeque;
    use std::path::PathBuf;

    fn fixture(raw: &str) -> JsonRpcMessage {
        serde_json::from_str(raw).expect("test fixture JSON should be valid")
    }

    fn queue(msgs: &[&str]) -> VecDeque<JsonRpcMessage> {
        msgs.iter().map(|s| fixture(s)).collect()
    }

    fn cwd() -> PathBuf {
        PathBuf::from("/tmp/panes-test-ws")
    }

    // ─── initialize ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn initialize_propagates_protocol_version_from_response() {
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-08-22"}}"#,
        ]));
        let result = initialize(
            &mut transport,
            &ClientInfo {
                name: "panes",
                version: "0.1.0",
            },
        )
        .await
        .expect("initialize should succeed");
        assert_eq!(result.protocol_version, "2025-08-22");
    }

    #[tokio::test]
    async fn initialize_reports_unknown_when_version_missing() {
        let mut transport =
            AcpTransport::mock(queue(&[r#"{"jsonrpc":"2.0","id":1,"result":{}}"#]));
        let result = initialize(
            &mut transport,
            &ClientInfo {
                name: "panes",
                version: "0.1.0",
            },
        )
        .await
        .expect("initialize should still succeed with empty result");
        assert_eq!(result.protocol_version, "unknown");
    }

    #[tokio::test]
    async fn initialize_errors_on_transport_error_response() {
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"Invalid request"}}"#,
        ]));
        let err = initialize(
            &mut transport,
            &ClientInfo {
                name: "panes",
                version: "0.1.0",
            },
        )
        .await
        .expect_err("error response should surface");
        assert!(err.to_string().contains("Invalid request"));
    }

    // ─── new_session ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn new_session_parses_session_id_and_models() {
        let mut transport = AcpTransport::mock(queue(&[
            r#"{
                "jsonrpc":"2.0",
                "id":1,
                "result":{
                    "sessionId":"sess-abc-123",
                    "models":{
                        "availableModels":[
                            {"modelId":"claude-3-5-sonnet"},
                            {"modelId":"claude-3-5-haiku"}
                        ]
                    }
                }
            }"#,
        ]));
        let r = new_session(&mut transport, &cwd())
            .await
            .expect("new_session should succeed");
        assert_eq!(r.session_id, "sess-abc-123");
        assert_eq!(r.available_models, vec!["claude-3-5-sonnet", "claude-3-5-haiku"]);
    }

    #[tokio::test]
    async fn new_session_errors_when_session_id_missing() {
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
        ]));
        let err = new_session(&mut transport, &cwd())
            .await
            .expect_err("missing sessionId is fatal");
        assert!(err.to_string().contains("sessionId"));
    }

    #[tokio::test]
    async fn new_session_handles_missing_models_gracefully() {
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s"}}"#,
        ]));
        let r = new_session(&mut transport, &cwd()).await.expect("ok");
        assert_eq!(r.session_id, "s");
        assert!(r.available_models.is_empty());
        assert!(r.available_modes.is_empty());
    }

    #[tokio::test]
    async fn new_session_parses_modes_array_flat_shape() {
        // Fallback shape: `modes` is a top-level array. Supported so the
        // adapter works against backends that match older test fixtures.
        let mut transport = AcpTransport::mock(queue(&[
            r#"{
                "jsonrpc":"2.0","id":1,
                "result":{
                    "sessionId":"sess-1",
                    "modes":[
                        {"id":"mode-a","name":"Mode A"},
                        {"id":"mode-b","name":"Mode B","description":"does things"},
                        {"id":"mode-c"}
                    ]
                }
            }"#,
        ]));
        let r = new_session(&mut transport, &cwd()).await.expect("ok");
        assert_eq!(r.available_modes.len(), 3);
        assert_eq!(r.available_modes[0].id, "mode-a");
        assert_eq!(r.available_modes[0].name.as_deref(), Some("Mode A"));
        assert_eq!(r.available_modes[1].description.as_deref(), Some("does things"));
        assert_eq!(r.available_modes[2].id, "mode-c");
        assert!(r.available_modes[2].name.is_none());
    }

    #[tokio::test]
    async fn new_session_parses_modes_array_kiro_shape() {
        // Actual kiro-cli shape (verified 2026-05): `/modes/availableModes`
        // nested under a `modes` object alongside `currentModeId`.
        let mut transport = AcpTransport::mock(queue(&[
            r#"{
                "jsonrpc":"2.0","id":1,
                "result":{
                    "sessionId":"sess-1",
                    "modes":{
                        "availableModes":[
                            {"id":"mode-a","name":"Mode A","description":"Does the A things"},
                            {"id":"mode-b","name":"Mode B"}
                        ],
                        "currentModeId":"mode-a"
                    }
                }
            }"#,
        ]));
        let r = new_session(&mut transport, &cwd()).await.expect("ok");
        assert_eq!(r.available_modes.len(), 2);
        assert_eq!(r.available_modes[0].id, "mode-a");
        assert_eq!(r.available_modes[0].name.as_deref(), Some("Mode A"));
        assert_eq!(r.available_modes[0].description.as_deref(), Some("Does the A things"));
        assert_eq!(r.available_modes[1].id, "mode-b");
        assert!(r.available_modes[1].description.is_none());
    }

    #[tokio::test]
    async fn new_session_skips_malformed_modes_entries() {
        // If a mode entry has no id, drop it rather than panic or emit an
        // empty-id entry. Missing name is fine — that stays None.
        let mut transport = AcpTransport::mock(queue(&[
            r#"{
                "jsonrpc":"2.0","id":1,
                "result":{
                    "sessionId":"sess-1",
                    "modes":[
                        {"id":"mode-a"},
                        {"name":"Orphaned"},
                        "not-an-object",
                        {"id":"mode-b","name":"Mode B"}
                    ]
                }
            }"#,
        ]));
        let r = new_session(&mut transport, &cwd()).await.expect("ok");
        assert_eq!(r.available_modes.len(), 2, "malformed entries dropped");
        assert_eq!(r.available_modes[0].id, "mode-a");
        assert_eq!(r.available_modes[1].id, "mode-b");
    }

    #[tokio::test]
    async fn new_session_treats_empty_description_as_none() {
        // kiro-cli ships some modes with `description: ""`. Surface them
        // as None so the UI doesn't render an empty blurb line.
        let mut transport = AcpTransport::mock(queue(&[
            r#"{
                "jsonrpc":"2.0","id":1,
                "result":{
                    "sessionId":"sess-1",
                    "modes":[{"id":"mode-a","name":"Mode A","description":""}]
                }
            }"#,
        ]));
        let r = new_session(&mut transport, &cwd()).await.expect("ok");
        assert!(r.available_modes[0].description.is_none());
    }

    // ─── load_session ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn load_session_returns_true_when_modes_present() {
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","id":1,"result":{"modes":[]}}"#,
        ]));
        let out = load_session(&mut transport, "sess-1", &cwd())
            .await
            .expect("ok");
        assert!(out.resumed);
    }

    #[tokio::test]
    async fn load_session_surfaces_modes_and_models_when_resumed() {
        let mut transport = AcpTransport::mock(queue(&[
            r#"{
                "jsonrpc":"2.0","id":1,
                "result":{
                    "modes":[{"id":"mode-a","name":"Mode A"},{"id":"mode-b"}],
                    "models":{"availableModels":[{"modelId":"claude-3-5-sonnet"}]}
                }
            }"#,
        ]));
        let out = load_session(&mut transport, "sess-1", &cwd())
            .await
            .expect("ok");
        assert!(out.resumed);
        assert_eq!(out.available_models, vec!["claude-3-5-sonnet"]);
        assert_eq!(out.available_modes.len(), 2);
        assert_eq!(out.available_modes[0].id, "mode-a");
        assert_eq!(out.available_modes[0].name.as_deref(), Some("Mode A"));
        assert_eq!(out.available_modes[1].id, "mode-b");
        assert!(out.available_modes[1].name.is_none());
    }

    #[tokio::test]
    async fn load_session_returns_false_when_modes_missing() {
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
        ]));
        let out = load_session(&mut transport, "sess-1", &cwd())
            .await
            .expect("ok");
        assert!(!out.resumed, "missing `modes` field → caller falls back");
    }

    #[tokio::test]
    async fn load_session_returns_false_on_error_response() {
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"not found"}}"#,
        ]));
        let out = load_session(&mut transport, "sess-missing", &cwd())
            .await
            .expect("error response should be downgraded to Ok(false)");
        assert!(!out.resumed);
    }

    // ─── set_mode ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn set_mode_returns_when_commands_available_arrives() {
        // Realistic ordering: our set_mode response first, then the readiness notif.
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
            r#"{"jsonrpc":"2.0","method":"_kiro.dev/commands/available","params":{}}"#,
        ]));
        let start = std::time::Instant::now();
        set_mode(&mut transport, "sess", "mode-a")
            .await
            .expect("set_mode should succeed");
        assert!(
            start.elapsed() < set_mode_timeout(),
            "set_mode must not wait out the full timeout"
        );
    }

    #[tokio::test]
    async fn set_mode_returns_when_commands_available_arrives_first() {
        // Reverse order — notification before response.
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","method":"_kiro.dev/commands/available","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
        ]));
        set_mode(&mut transport, "sess", "mode-a")
            .await
            .expect("set_mode should succeed");
    }

    #[tokio::test]
    async fn set_mode_completes_gracefully_without_commands_available_notification() {
        // Mock queue drains and returns None forever. set_mode must NOT panic
        // and must eventually return Ok once its internal deadline expires.
        // In test builds SET_MODE_TIMEOUT is compressed to 200ms (see top of
        // this file), so this test runs quickly.
        let mut transport = AcpTransport::mock(VecDeque::new());
        let start = std::time::Instant::now();
        set_mode(&mut transport, "sess", "mode-a")
            .await
            .expect("set_mode must tolerate missing commands/available notification");
        let elapsed = start.elapsed();
        assert!(
            elapsed >= set_mode_timeout(),
            "set_mode must wait out the full timeout before giving up, got {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "test compression should keep this under 2s, got {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn set_mode_buffers_server_initiated_requests() {
        // A permission request arrives during set_mode. set_mode should put
        // it back on the buffer so the surrounding loop sees it next.
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","method":"_kiro.dev/commands/available","params":{}}"#,
        ]));

        // Pre-populate the buffer with a server request by using unread —
        // simulates the case where the request arrives first.
        let permission = fixture(
            r#"{"jsonrpc":"2.0","id":"uuid-1","method":"session/request_permission","params":{}}"#,
        );
        transport.unread(permission.clone());

        set_mode(&mut transport, "sess", "mode-a")
            .await
            .expect("set_mode should succeed even with a permission in flight");

        // The permission request should still be available downstream.
        let next = transport
            .read_message(Duration::from_millis(10))
            .await
            .expect("ok");
        let next = next.expect("the buffered permission must survive set_mode");
        assert!(next.is_method("session/request_permission"));
    }

    // ─── set_model ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn set_model_waits_for_response() {
        let mut transport =
            AcpTransport::mock(queue(&[r#"{"jsonrpc":"2.0","id":1,"result":{}}"#]));
        set_model(&mut transport, "sess", "claude-3-5-sonnet")
            .await
            .expect("set_model should ack");
    }

    #[tokio::test]
    async fn set_model_errors_on_error_response() {
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"no such model"}}"#,
        ]));
        let err = set_model(&mut transport, "sess", "unknown")
            .await
            .expect_err("error response should bubble up");
        assert!(err.to_string().contains("no such model"));
    }

    // ─── prompt ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn prompt_returns_request_id_without_waiting_for_response() {
        // Empty queue — prompt must not block on a response; the surrounding
        // event loop handles stopReason.
        let mut transport = AcpTransport::mock(VecDeque::new());
        let id = prompt(&mut transport, "sess", "hello")
            .await
            .expect("prompt should return id");
        assert_eq!(id, 1, "first request id should be 1");
    }

    // ─── cancel ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn cancel_sends_session_id_and_returns_ok_on_ack() {
        let mut transport =
            AcpTransport::mock(queue(&[r#"{"jsonrpc":"2.0","id":1,"result":{}}"#]));
        cancel(&mut transport, "sess-xyz")
            .await
            .expect("cancel should succeed on normal ack");
    }

    #[tokio::test]
    async fn cancel_returns_ok_even_when_ack_missing() {
        // Empty queue → wait_for_response times out → cancel returns Ok.
        let mut transport = AcpTransport::mock(VecDeque::new());
        let start = std::time::Instant::now();
        cancel(&mut transport, "sess-xyz")
            .await
            .expect("cancel should tolerate a missing ack");
        assert!(
            start.elapsed() < CANCEL_TIMEOUT + Duration::from_secs(1),
            "cancel must return within the cancel timeout even without an ack"
        );
    }
}
