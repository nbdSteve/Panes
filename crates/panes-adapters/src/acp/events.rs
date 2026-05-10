//! Translate ACP JSON-RPC messages into Panes [`AgentEvent`]s.
//!
//! This is the bridge layer that lets SessionManager treat an ACP-speaking
//! backend the same as Claude. Each inbound message produces zero or more
//! `AgentEvent`s — a single notification can emit multiple events (e.g. a
//! `tool_call_update` with a status change plus a text chunk), so the
//! translator returns a `Vec` rather than an `Option`.
//!
//! Two special cases the translator owns:
//! - **Text coalescing**: `agent_message_chunk` arrives in tiny pieces. We
//!   buffer chunks that arrive within 50ms of each other into a single
//!   `AgentEvent::Text` to keep the UI from exploding.
//! - **Permission request bookkeeping**: when `session/request_permission`
//!   arrives, we record `tool_call_id → acp_request_id` so the adapter's
//!   `approve()`/`reject()` can send the JSON-RPC response to the correct id.

use std::collections::HashMap;
use std::time::Instant;

use panes_events::{AgentEvent, RiskLevel};
use serde_json::Value;

use super::transport::JsonRpcMessage;

/// Text chunks arriving within this gap are coalesced into a single `Text` event.
const COALESCE_GAP_MS: u128 = 50;

/// Pending tool info tracked across a `tool_call` → `tool_call_update` pair.
#[derive(Debug, Clone)]
struct PendingTool {
    tool_name: String,
    started_at: Instant,
}

/// One option offered by the backend in `session/request_permission`.
/// The adapter uses this when responding to pick an option whose `kind`
/// matches the user's decision (allow/deny) — we never hardcode option ids.
#[derive(Debug, Clone)]
pub(crate) struct PermissionOption {
    pub id: String,
    pub kind: String, // "allow" | "allow_always" | "deny" | "reject" | etc.
}

/// What we remember about a pending permission request so approve/reject
/// can respond to the correct id with a valid optionId.
#[derive(Debug, Clone)]
pub(crate) struct PendingPermission {
    pub request_id: Value,
    pub options: Vec<PermissionOption>,
}

/// Translator state for a single ACP session.
#[derive(Debug, Default)]
pub(crate) struct TranslationContext {
    pending_tools: HashMap<String, PendingTool>,
    /// Pending permission requests keyed by tool_call_id. The adapter layer
    /// reads these when the user approves/rejects.
    pending_permissions: HashMap<String, PendingPermission>,
    /// Last emitted text chunk and when. Used for coalescing.
    last_text: Option<(String, Instant)>,
    /// req_id of the currently in-flight session/prompt — Complete emits when
    /// the response for this id arrives.
    prompt_req_id: Option<u64>,
    /// When the current prompt started. Used to compute Complete.duration_ms.
    prompt_started_at: Option<Instant>,
    /// Accumulated text from this prompt. Becomes Complete.summary.
    accumulated_summary: String,
    /// Turn counter for Complete.turns.
    turns: u32,
}

impl TranslationContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that a prompt has just been sent with the given request id.
    /// The next matching response will emit Complete.
    pub fn begin_prompt(&mut self, req_id: u64) {
        self.prompt_req_id = Some(req_id);
        self.prompt_started_at = Some(Instant::now());
        self.accumulated_summary.clear();
        self.last_text = None;
        self.turns = self.turns.saturating_add(1);
    }

    /// Take the pending permission record for a tool-use id so the adapter
    /// can send a response. Returns `None` if the tool is unknown — the
    /// adapter should surface an error in that case.
    pub fn take_permission(&mut self, tool_use_id: &str) -> Option<PendingPermission> {
        self.pending_permissions.remove(tool_use_id)
    }

    /// Flush any buffered text chunks as a final `Text` event. Called by the
    /// adapter when the stream ends abnormally (EOF, transport error) so the
    /// user doesn't lose partial output.
    pub fn flush_pending_text(&mut self) -> Option<AgentEvent> {
        self.last_text
            .take()
            .map(|(text, _)| AgentEvent::Text { text })
    }

    /// Translate a single inbound JSON-RPC message to zero-or-more `AgentEvent`s.
    pub fn translate(&mut self, msg: &JsonRpcMessage) -> Vec<AgentEvent> {
        // Fast path: responses to prompt completion.
        if let Some(pid) = self.prompt_req_id {
            if msg.is_response_for(pid) {
                return self.handle_prompt_response(msg);
            }
        }

        // session/request_permission (server-initiated request with method + id)
        if msg.is_method("session/request_permission") && msg.has_id() {
            return self.handle_permission_request(msg);
        }

        // session/update notifications
        if msg.is_method("session/update") {
            return self.handle_session_update(msg);
        }

        // _kiro.dev/metadata — silently consumed
        if msg.is_method("_kiro.dev/metadata") {
            return Vec::new();
        }

        Vec::new()
    }

    fn handle_prompt_response(&mut self, msg: &JsonRpcMessage) -> Vec<AgentEvent> {
        if let Some(err) = &msg.error {
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown JSON-RPC error")
                .to_string();
            self.prompt_req_id = None;
            return vec![AgentEvent::Error {
                message,
                recoverable: true,
            }];
        }

        let result = msg.result.clone().unwrap_or(Value::Null);
        let stop_reason = result
            .get("stopReason")
            .and_then(|v| v.as_str())
            .unwrap_or("end_turn")
            .to_string();

        let mut events = Vec::new();
        // Flush any trailing buffered text before Complete.
        if let Some((text, _)) = self.last_text.take() {
            events.push(AgentEvent::Text { text });
        }

        let duration_ms = self
            .prompt_started_at
            .take()
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);

        let mut summary = std::mem::take(&mut self.accumulated_summary);
        // Surface non-end_turn reasons in the summary so the user knows the
        // turn terminated for a reason other than natural completion.
        if stop_reason != "end_turn" && stop_reason != "stop" && !stop_reason.is_empty() {
            if !summary.is_empty() {
                summary.push_str("\n\n");
            }
            summary.push_str(&format!("[stop reason: {stop_reason}]"));
        }

        self.prompt_req_id = None;
        events.push(AgentEvent::Complete {
            summary,
            total_cost_usd: 0.0,
            duration_ms,
            turns: self.turns,
        });
        events
    }

    fn handle_permission_request(&mut self, msg: &JsonRpcMessage) -> Vec<AgentEvent> {
        let params = match msg.params.as_ref() {
            Some(p) => p,
            None => return Vec::new(),
        };
        let tool_call = params.get("toolCall").unwrap_or(&Value::Null);
        let tool_call_id = tool_call
            .get("toolCallId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let title = tool_call
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let kind = tool_call
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let raw_input = tool_call
            .get("rawInput")
            .cloned()
            .unwrap_or(Value::Null);

        let risk_level = classify_acp_risk(&kind, &raw_input);

        // Parse the options array so approve()/reject() can pick a valid
        // optionId rather than hardcoding one.
        let options: Vec<PermissionOption> = params
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| {
                        let id = o.get("optionId").and_then(|v| v.as_str())?.to_string();
                        let kind = o
                            .get("kind")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        Some(PermissionOption { id, kind })
                    })
                    .collect()
            })
            .unwrap_or_default();

        // Record the ACP request id + options so approve()/reject() can respond.
        if let Some(id) = msg.id.clone() {
            self.pending_permissions.insert(
                tool_call_id.clone(),
                PendingPermission {
                    request_id: id,
                    options,
                },
            );
        }

        // Also record this as a pending tool so a subsequent tool_call_update
        // can emit a ToolResult with proper duration.
        self.pending_tools.insert(
            tool_call_id.clone(),
            PendingTool {
                tool_name: kind.clone(),
                started_at: Instant::now(),
            },
        );

        let mut events = Vec::new();
        // Flush buffered text before the gate event so ordering matches the UI.
        if let Some((text, _)) = self.last_text.take() {
            events.push(AgentEvent::Text { text });
        }
        events.push(AgentEvent::ToolRequest {
            id: tool_call_id,
            tool_name: kind,
            description: title,
            input: raw_input,
            needs_approval: true,
            risk_level,
        });
        events
    }

    fn handle_session_update(&mut self, msg: &JsonRpcMessage) -> Vec<AgentEvent> {
        let params = match msg.params.as_ref() {
            Some(p) => p,
            None => return Vec::new(),
        };
        let update = params.get("update").unwrap_or(params);
        let session_update = update
            .get("sessionUpdate")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match session_update {
            "agent_message_chunk" => self.handle_message_chunk(update, /*thinking=*/ false),
            "agent_thought_chunk" => self.handle_message_chunk(update, /*thinking=*/ true),
            "tool_call" => self.handle_tool_call(update),
            "tool_call_update" => self.handle_tool_call_update(update),
            _ => Vec::new(),
        }
    }

    fn handle_message_chunk(&mut self, update: &Value, thinking: bool) -> Vec<AgentEvent> {
        let text = extract_chunk_text(update);
        if text.is_empty() {
            return Vec::new();
        }

        // Thinking chunks are emitted immediately and never coalesced — they
        // typically arrive less frequently and callers want to render them
        // distinctly.
        if thinking {
            return vec![AgentEvent::Thinking { text }];
        }

        // Coalesce agent_message_chunk.
        self.accumulated_summary.push_str(&text);
        let now = Instant::now();
        match self.last_text.take() {
            Some((mut buf, seen_at))
                if now.duration_since(seen_at).as_millis() < COALESCE_GAP_MS =>
            {
                buf.push_str(&text);
                self.last_text = Some((buf, now));
                Vec::new()
            }
            Some((buf, _stale)) => {
                self.last_text = Some((text, now));
                vec![AgentEvent::Text { text: buf }]
            }
            None => {
                self.last_text = Some((text, now));
                Vec::new()
            }
        }
    }

    fn handle_tool_call(&mut self, update: &Value) -> Vec<AgentEvent> {
        let tool_call_id = update
            .get("toolCallId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if tool_call_id.is_empty() {
            return Vec::new();
        }
        let kind = update
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let title = update
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let raw_input = update
            .get("rawInput")
            .cloned()
            .unwrap_or(Value::Null);

        self.pending_tools.insert(
            tool_call_id.clone(),
            PendingTool {
                tool_name: kind.clone(),
                started_at: Instant::now(),
            },
        );

        let mut events = Vec::new();
        if let Some((text, _)) = self.last_text.take() {
            events.push(AgentEvent::Text { text });
        }
        events.push(AgentEvent::ToolRequest {
            id: tool_call_id,
            tool_name: kind.clone(),
            description: title,
            input: raw_input.clone(),
            needs_approval: false,
            risk_level: classify_acp_risk(&kind, &raw_input),
        });
        events
    }

    fn handle_tool_call_update(&mut self, update: &Value) -> Vec<AgentEvent> {
        let tool_call_id = update
            .get("toolCallId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if tool_call_id.is_empty() {
            return Vec::new();
        }
        let status = update
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        // Only terminal statuses produce a ToolResult — intermediate
        // "in_progress" etc. are ignored here (no corresponding event type).
        match status {
            "completed" | "failed" => {}
            _ => return Vec::new(),
        }

        let pending = match self.pending_tools.remove(&tool_call_id) {
            Some(p) => p,
            None => return Vec::new(), // orphan update — ignore
        };
        let duration_ms = pending.started_at.elapsed().as_millis() as u64;
        let output = update
            .get("output")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let raw_output = update
            .get("rawOutput")
            .and_then(|v| match v {
                Value::String(s) => Some(s.clone()),
                other => Some(other.to_string()),
            });

        let mut events = Vec::new();
        if let Some((text, _)) = self.last_text.take() {
            events.push(AgentEvent::Text { text });
        }
        events.push(AgentEvent::ToolResult {
            id: tool_call_id,
            tool_name: pending.tool_name,
            success: status == "completed",
            output,
            raw_output,
            duration_ms,
        });
        events
    }
}

/// Extract the textual payload from an `agent_message_chunk` / `agent_thought_chunk`
/// update. kiro-cli puts it at either `content.text` or directly in `content`
/// (when content is a plain string).
fn extract_chunk_text(update: &Value) -> String {
    match update.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(obj)) => obj
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

/// Classify an ACP tool call's risk. ACP kinds differ from Claude tool names
/// (`read`, `edit`, `execute`, `fetch`, `think`, `search`, `move`, `delete`,
/// `other`) — we map them to risk levels directly and delegate `execute` to
/// the bash classifier so a `rm -rf` doesn't slip through as plain Medium.
pub(crate) fn classify_acp_risk(kind: &str, raw_input: &Value) -> RiskLevel {
    // kiro-cli sometimes uses the raw tool name (e.g. "fs_write") rather than
    // the semantic kind — handle both. Lower-case for case-insensitive matching.
    let lower = kind.to_ascii_lowercase();
    match lower.as_str() {
        // Read-family
        "read" | "fs_read" | "fetch" | "search" | "think" => RiskLevel::Low,
        // Write-family (non-destructive)
        "edit" | "fs_edit" | "fs_write" | "fs_create" | "write" => RiskLevel::Medium,
        // Shell / execute — defer to bash classifier
        "execute" | "execute_bash" | "shell" | "bash" => {
            // Map the common shell field names kiro-cli uses ("command" stays
            // the same; some tools pass it as "cmd").
            let normalized = normalize_bash_input(raw_input);
            crate::claude::risk::classify_risk("Bash", &normalized)
        }
        // Deletion
        "delete" | "fs_delete" | "remove" => RiskLevel::High,
        // Move / rename — usually destructive
        "move" | "fs_move" | "rename" => RiskLevel::High,
        // Anything else — default to Medium so we err on the side of gating.
        // Log it so the user can see when a backend introduces a new kind.
        other => {
            tracing::debug!(kind = other, "classify_acp_risk: unknown kind — defaulting to Medium");
            RiskLevel::Medium
        }
    }
}

/// Ensure a `command` field exists for the bash risk classifier. Accepts
/// `command`, `cmd`, or falls through unchanged.
fn normalize_bash_input(raw_input: &Value) -> Value {
    if raw_input.get("command").is_some() {
        return raw_input.clone();
    }
    if let Some(cmd) = raw_input.get("cmd").and_then(|v| v.as_str()) {
        return serde_json::json!({ "command": cmd });
    }
    raw_input.clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use panes_events::RiskLevel;

    fn msg(raw: &str) -> JsonRpcMessage {
        serde_json::from_str(raw).expect("test fixture JSON should be valid")
    }

    fn single<T: std::fmt::Debug>(v: Vec<T>) -> T {
        let len = v.len();
        v.into_iter().next().unwrap_or_else(|| panic!("expected one event, got {len}"))
    }

    // ─── agent_message_chunk / agent_thought_chunk ────────────────────────

    #[test]
    fn agent_message_chunk_emits_text_after_gap() {
        let mut ctx = TranslationContext::new();
        let chunk = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hello "}}
            }}"#,
        );
        // First chunk is buffered (no event yet).
        assert!(ctx.translate(&chunk).is_empty());

        // Wait longer than the coalesce gap, then send another chunk + flush.
        std::thread::sleep(std::time::Duration::from_millis(COALESCE_GAP_MS as u64 + 20));
        let next = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"world"}}
            }}"#,
        );
        let events = ctx.translate(&next);
        // First call after the gap flushes the buffered "hello " and buffers "world".
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::Text { text } => assert_eq!(text, "hello "),
            other => panic!("expected Text('hello '), got {other:?}"),
        }
    }

    #[test]
    fn agent_message_chunks_within_gap_are_coalesced() {
        let mut ctx = TranslationContext::new();
        let c1 = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"foo "}}
            }}"#,
        );
        let c2 = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"bar"}}
            }}"#,
        );
        assert!(ctx.translate(&c1).is_empty());
        assert!(ctx.translate(&c2).is_empty(), "chunks within 50ms should coalesce silently");

        // Trigger a flush by emitting a tool_call (non-text event).
        let tc = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call","toolCallId":"tc-1","kind":"read","title":"read x","rawInput":{}}
            }}"#,
        );
        let events = ctx.translate(&tc);
        // Expect: flushed "foo bar", then the tool_call as ToolRequest.
        assert_eq!(events.len(), 2);
        match &events[0] {
            AgentEvent::Text { text } => assert_eq!(text, "foo bar"),
            other => panic!("expected flushed Text('foo bar'), got {other:?}"),
        }
        match &events[1] {
            AgentEvent::ToolRequest { tool_name, .. } => assert_eq!(tool_name, "read"),
            other => panic!("expected ToolRequest, got {other:?}"),
        }
    }

    #[test]
    fn agent_thought_chunk_emits_thinking_immediately() {
        let mut ctx = TranslationContext::new();
        let thought = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"let me think"}}
            }}"#,
        );
        let e = single(ctx.translate(&thought));
        match e {
            AgentEvent::Thinking { text } => assert_eq!(text, "let me think"),
            other => panic!("expected Thinking, got {other:?}"),
        }
    }

    #[test]
    fn text_chunk_with_no_text_is_ignored() {
        let mut ctx = TranslationContext::new();
        let empty = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":""}}
            }}"#,
        );
        assert!(ctx.translate(&empty).is_empty());
    }

    // ─── tool_call ────────────────────────────────────────────────────────

    #[test]
    fn tool_call_emits_tool_request_with_needs_approval_false() {
        let mut ctx = TranslationContext::new();
        let tc = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call","toolCallId":"tc-7","kind":"read",
                          "title":"read main.rs","rawInput":{"path":"/tmp/main.rs"}}
            }}"#,
        );
        let e = single(ctx.translate(&tc));
        match e {
            AgentEvent::ToolRequest { id, tool_name, description, input, needs_approval, risk_level } => {
                assert_eq!(id, "tc-7");
                assert_eq!(tool_name, "read");
                assert_eq!(description, "read main.rs");
                assert_eq!(input, serde_json::json!({"path":"/tmp/main.rs"}));
                assert!(!needs_approval);
                assert_eq!(risk_level, RiskLevel::Low);
            }
            other => panic!("expected ToolRequest, got {other:?}"),
        }
    }

    #[test]
    fn tool_call_with_no_id_is_skipped() {
        let mut ctx = TranslationContext::new();
        let tc = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call","kind":"read","title":"x"}
            }}"#,
        );
        assert!(ctx.translate(&tc).is_empty());
    }

    #[test]
    fn tool_call_update_completed_emits_success_tool_result() {
        let mut ctx = TranslationContext::new();
        let tc = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call","toolCallId":"tc-9","kind":"read","title":"x","rawInput":{}}
            }}"#,
        );
        let _ = ctx.translate(&tc);

        let upd = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call_update","toolCallId":"tc-9","status":"completed","output":"OK"}
            }}"#,
        );
        let e = single(ctx.translate(&upd));
        match e {
            AgentEvent::ToolResult { id, tool_name, success, output, .. } => {
                assert_eq!(id, "tc-9");
                assert_eq!(tool_name, "read");
                assert!(success);
                assert_eq!(output, "OK");
            }
            other => panic!("expected ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn tool_call_update_failed_emits_failure_tool_result() {
        let mut ctx = TranslationContext::new();
        let tc = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call","toolCallId":"tc-9","kind":"edit","title":"x","rawInput":{}}
            }}"#,
        );
        let _ = ctx.translate(&tc);

        let upd = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call_update","toolCallId":"tc-9","status":"failed","output":"boom"}
            }}"#,
        );
        let e = single(ctx.translate(&upd));
        match e {
            AgentEvent::ToolResult { success, output, .. } => {
                assert!(!success);
                assert_eq!(output, "boom");
            }
            other => panic!("expected failed ToolResult, got {other:?}"),
        }
    }

    #[test]
    fn orphan_tool_call_update_is_skipped() {
        let mut ctx = TranslationContext::new();
        let upd = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call_update","toolCallId":"never-seen","status":"completed"}
            }}"#,
        );
        assert!(ctx.translate(&upd).is_empty());
    }

    #[test]
    fn tool_call_update_intermediate_status_emits_nothing() {
        let mut ctx = TranslationContext::new();
        let tc = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call","toolCallId":"tc-11","kind":"read","title":"x","rawInput":{}}
            }}"#,
        );
        let _ = ctx.translate(&tc);
        let in_progress = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call_update","toolCallId":"tc-11","status":"in_progress"}
            }}"#,
        );
        assert!(ctx.translate(&in_progress).is_empty());
    }

    #[test]
    fn multiple_concurrent_tool_calls_tracked_by_id() {
        let mut ctx = TranslationContext::new();
        let tc1 = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call","toolCallId":"a","kind":"read","title":"a","rawInput":{}}
            }}"#,
        );
        let tc2 = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call","toolCallId":"b","kind":"edit","title":"b","rawInput":{}}
            }}"#,
        );
        let _ = ctx.translate(&tc1);
        let _ = ctx.translate(&tc2);

        let up_b = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call_update","toolCallId":"b","status":"completed","output":"B"}
            }}"#,
        );
        let up_a = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call_update","toolCallId":"a","status":"completed","output":"A"}
            }}"#,
        );
        let b = single(ctx.translate(&up_b));
        let a = single(ctx.translate(&up_a));
        match (a, b) {
            (
                AgentEvent::ToolResult { id: aid, tool_name: atn, output: aout, .. },
                AgentEvent::ToolResult { id: bid, tool_name: btn, output: bout, .. },
            ) => {
                assert_eq!(aid, "a");
                assert_eq!(atn, "read");
                assert_eq!(aout, "A");
                assert_eq!(bid, "b");
                assert_eq!(btn, "edit");
                assert_eq!(bout, "B");
            }
            other => panic!("expected two ToolResults, got {other:?}"),
        }
    }

    // ─── session/request_permission ───────────────────────────────────────

    #[test]
    fn request_permission_emits_gated_tool_request_and_stores_id() {
        let mut ctx = TranslationContext::new();
        let perm = msg(
            r#"{"jsonrpc":"2.0","id":"uuid-perm-1","method":"session/request_permission","params":{
                "toolCall":{"toolCallId":"tc-p","kind":"execute","title":"rm something",
                            "rawInput":{"command":"rm -rf /tmp/x"}},
                "options":[{"optionId":"allow_once","name":"Allow once","kind":"allow"}]
            }}"#,
        );
        let e = single(ctx.translate(&perm));
        match e {
            AgentEvent::ToolRequest { id, tool_name, needs_approval, risk_level, input, .. } => {
                assert_eq!(id, "tc-p");
                assert_eq!(tool_name, "execute");
                assert!(needs_approval);
                assert_eq!(risk_level, RiskLevel::Critical, "rm -rf should classify as Critical");
                assert_eq!(input, serde_json::json!({"command":"rm -rf /tmp/x"}));
            }
            other => panic!("expected gated ToolRequest, got {other:?}"),
        }

        // The ACP request id + options must be stored for approve/reject.
        let pending = ctx.take_permission("tc-p").expect("record must be stored");
        assert_eq!(pending.request_id, serde_json::json!("uuid-perm-1"));
        assert_eq!(pending.options.len(), 1);
        assert_eq!(pending.options[0].id, "allow_once");
        assert_eq!(pending.options[0].kind, "allow");
    }

    #[test]
    fn request_permission_with_deny_option_is_captured() {
        let mut ctx = TranslationContext::new();
        let perm = msg(
            r#"{"jsonrpc":"2.0","id":"uuid-2","method":"session/request_permission","params":{
                "toolCall":{"toolCallId":"tc-d","kind":"edit","title":"x","rawInput":{}},
                "options":[
                    {"optionId":"approve","name":"Approve","kind":"allow_always"},
                    {"optionId":"stop","name":"Reject","kind":"reject"}
                ]
            }}"#,
        );
        let _ = ctx.translate(&perm);
        let pending = ctx.take_permission("tc-d").expect("stored");
        assert_eq!(pending.options.len(), 2);
        assert!(pending.options.iter().any(|o| o.kind == "allow_always" && o.id == "approve"));
        assert!(pending.options.iter().any(|o| o.kind == "reject" && o.id == "stop"));
    }

    #[test]
    fn flush_pending_text_emits_buffered_chunk() {
        let mut ctx = TranslationContext::new();
        let c = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"stuck in buffer"}}
            }}"#,
        );
        // Buffer a chunk but never emit a non-text event after.
        assert!(ctx.translate(&c).is_empty());
        let flushed = ctx.flush_pending_text().expect("should flush");
        match flushed {
            AgentEvent::Text { text } => assert_eq!(text, "stuck in buffer"),
            other => panic!("expected Text, got {other:?}"),
        }
        // Second flush yields nothing.
        assert!(ctx.flush_pending_text().is_none());
    }

    #[test]
    fn request_permission_without_params_is_ignored() {
        let mut ctx = TranslationContext::new();
        let perm = msg(
            r#"{"jsonrpc":"2.0","id":"uuid-nope","method":"session/request_permission"}"#,
        );
        assert!(ctx.translate(&perm).is_empty());
    }

    // ─── prompt completion ────────────────────────────────────────────────

    #[test]
    fn prompt_response_with_end_turn_emits_complete() {
        let mut ctx = TranslationContext::new();
        ctx.begin_prompt(1);
        let done = msg(r#"{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}"#);
        let e = single(ctx.translate(&done));
        match e {
            AgentEvent::Complete { turns, .. } => {
                assert_eq!(turns, 1);
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn prompt_response_non_end_turn_annotates_summary() {
        let mut ctx = TranslationContext::new();
        ctx.begin_prompt(1);
        let chunk = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hi"}}
            }}"#,
        );
        let _ = ctx.translate(&chunk);
        let done = msg(r#"{"jsonrpc":"2.0","id":1,"result":{"stopReason":"max_tokens"}}"#);
        let events = ctx.translate(&done);
        // We expect a flushed text event, then a Complete.
        assert_eq!(events.len(), 2);
        match &events[1] {
            AgentEvent::Complete { summary, .. } => {
                assert!(summary.contains("hi"));
                assert!(summary.contains("max_tokens"));
            }
            other => panic!("expected Complete, got {other:?}"),
        }
    }

    #[test]
    fn prompt_response_with_error_emits_error_event() {
        let mut ctx = TranslationContext::new();
        ctx.begin_prompt(1);
        let done = msg(
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"context overflow"}}"#,
        );
        let e = single(ctx.translate(&done));
        match e {
            AgentEvent::Error { message, recoverable } => {
                assert!(message.contains("context overflow"));
                assert!(recoverable);
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }

    #[test]
    fn mismatched_response_id_is_not_treated_as_complete() {
        let mut ctx = TranslationContext::new();
        ctx.begin_prompt(5);
        let stray = msg(r#"{"jsonrpc":"2.0","id":99,"result":{"stopReason":"end_turn"}}"#);
        assert!(ctx.translate(&stray).is_empty());
    }

    // ─── metadata + unknown ───────────────────────────────────────────────

    #[test]
    fn metadata_notification_is_silently_consumed() {
        let mut ctx = TranslationContext::new();
        let meta = msg(r#"{"jsonrpc":"2.0","method":"_kiro.dev/metadata","params":{"contextUsagePercentage":42.0}}"#);
        assert!(ctx.translate(&meta).is_empty());
    }

    #[test]
    fn unknown_method_is_silently_consumed() {
        let mut ctx = TranslationContext::new();
        let weird = msg(r#"{"jsonrpc":"2.0","method":"some/new/thing","params":{}}"#);
        assert!(ctx.translate(&weird).is_empty());
    }

    // ─── classify_acp_risk direct tests ───────────────────────────────────

    #[test]
    fn classify_read_kinds_as_low() {
        assert_eq!(classify_acp_risk("read", &Value::Null), RiskLevel::Low);
        assert_eq!(classify_acp_risk("fs_read", &Value::Null), RiskLevel::Low);
        assert_eq!(classify_acp_risk("fetch", &Value::Null), RiskLevel::Low);
    }

    #[test]
    fn classify_edit_kinds_as_medium() {
        assert_eq!(classify_acp_risk("edit", &Value::Null), RiskLevel::Medium);
        assert_eq!(classify_acp_risk("fs_write", &Value::Null), RiskLevel::Medium);
    }

    #[test]
    fn classify_execute_defers_to_bash_classifier() {
        let safe = classify_acp_risk("execute", &serde_json::json!({"command":"ls"}));
        assert_eq!(safe, RiskLevel::Low);
        let bad = classify_acp_risk("execute", &serde_json::json!({"command":"rm -rf /"}));
        assert_eq!(bad, RiskLevel::Critical);
    }

    #[test]
    fn classify_unknown_kind_as_medium() {
        assert_eq!(classify_acp_risk("teleport", &Value::Null), RiskLevel::Medium);
    }

    /// Exhaustive sweep so a regression in any kind's mapping is caught by
    /// a single test. Keep this list in sync with `classify_acp_risk` — the
    /// explicit enumeration is the *point*: if anyone removes a mapping or
    /// flips the default arm they fail this, not just the kind-specific tests.
    #[test]
    fn classify_acp_risk_full_kind_matrix() {
        use RiskLevel::*;
        let cases: &[(&str, RiskLevel)] = &[
            // Low
            ("read", Low),
            ("Read", Low),
            ("READ", Low),
            ("fs_read", Low),
            ("fetch", Low),
            ("search", Low),
            ("think", Low),
            // Medium (edit family, unknown fallback)
            ("edit", Medium),
            ("fs_edit", Medium),
            ("fs_write", Medium),
            ("fs_create", Medium),
            ("write", Medium),
            ("teleport", Medium),           // unknown → Medium
            ("future_new_kind", Medium),    // unknown → Medium
            // High
            ("delete", High),
            ("fs_delete", High),
            ("remove", High),
            ("move", High),
            ("fs_move", High),
            ("rename", High),
        ];
        for (kind, expected) in cases {
            assert_eq!(
                classify_acp_risk(kind, &Value::Null),
                *expected,
                "kind {kind:?} should map to {expected:?}"
            );
        }
    }

    #[test]
    fn classify_execute_variants_all_defer_to_bash_classifier() {
        // The key property: execute / shell / bash / execute_bash all pass
        // through to classify_bash_risk. A malicious command must be flagged
        // as Critical regardless of which alias the backend uses.
        let rm_rf = serde_json::json!({"command": "rm -rf /"});
        for kind in ["execute", "shell", "bash", "execute_bash"] {
            assert_eq!(
                classify_acp_risk(kind, &rm_rf),
                RiskLevel::Critical,
                "kind {kind:?} with rm -rf should classify Critical"
            );
        }
        let ls = serde_json::json!({"command": "ls -la"});
        for kind in ["execute", "shell", "bash", "execute_bash"] {
            assert_eq!(
                classify_acp_risk(kind, &ls),
                RiskLevel::Low,
                "kind {kind:?} with ls should classify Low"
            );
        }
    }

    #[test]
    fn classify_execute_normalizes_cmd_field_to_command() {
        // Some ACP backends use `cmd` instead of `command`. The normalizer
        // must rewrite it before the bash classifier sees it, otherwise
        // we'd fall through to the default Medium and miss critical ops.
        let via_cmd = serde_json::json!({"cmd": "rm -rf /"});
        assert_eq!(
            classify_acp_risk("execute", &via_cmd),
            RiskLevel::Critical,
            "the cmd→command normalization must preserve risk classification"
        );
    }

    #[test]
    fn classify_delete_as_high() {
        assert_eq!(classify_acp_risk("delete", &Value::Null), RiskLevel::High);
    }

    // ─── begin_prompt semantics ───────────────────────────────────────────

    #[test]
    fn begin_prompt_increments_turns() {
        let mut ctx = TranslationContext::new();
        ctx.begin_prompt(1);
        ctx.begin_prompt(2);
        ctx.begin_prompt(3);
        let done = msg(r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}"#);
        let e = single(ctx.translate(&done));
        match e {
            AgentEvent::Complete { turns, .. } => assert_eq!(turns, 3),
            other => panic!("expected Complete, got {other:?}"),
        }
    }
}
