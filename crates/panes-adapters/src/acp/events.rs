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
//!   buffer chunks that arrive close together in time into a single
//!   `AgentEvent::Text` to keep the UI from exploding.
//! - **Permission request bookkeeping**: when `session/request_permission`
//!   arrives, we record `tool_call_id → acp_request_id` so the adapter's
//!   `approve()`/`reject()` can send the JSON-RPC response to the correct id.

use std::collections::HashMap;
use std::time::Instant;

use panes_events::{AgentEvent, RiskLevel};
use serde_json::Value;

use super::cost::CostEstimator;
use super::transport::JsonRpcMessage;

/// Text chunks arriving within this gap are coalesced into a single `Text`
/// event. kiro-cli streams tokens 50-175ms apart, so 50ms (the original
/// Claude-tuned value) splits almost every chunk into its own event. We
/// raise to 300ms: large enough that normal token streaming accumulates,
/// small enough that distinct pauses (LLM thinking mid-turn) still become
/// separate events. The UI additionally merges adjacent Text events into
/// a single card — see `groupToolEvents.ts`.
const COALESCE_GAP_MS: u128 = 300;

/// Pending tool info tracked across a `tool_call` → `tool_call_update` pair.
#[derive(Debug, Clone)]
struct PendingTool {
    tool_name: String,
    started_at: Instant,
    /// If this tool_call represents an orchestrator delegating to a
    /// sub-agent, the name/kind of that sub-agent. A corresponding
    /// `SubAgentComplete` is emitted when the tool_call_update arrives.
    sub_agent: Option<String>,
}

/// Returns true if a tool call's name/kind looks like an agent dispatch —
/// orchestrator modes (e.g. tsuki-orchestrator) invoke sub-agents as tool
/// calls. We recognise the common naming conventions seen in kiro-cli
/// traffic and in Claude's Task tool. Extend this list when new backends
/// surface new delegation tools.
///
/// Extracts the child agent's name from either:
/// - `rawInput.subagent_type` / `rawInput.agent` / `rawInput.agent_type`
///   — the pattern Claude's Task tool uses
/// - the tool name itself when it embeds the target (e.g. `dispatch:foo`)
/// Returns `None` if the call doesn't look like delegation.
fn detect_sub_agent_dispatch(kind: &str, raw_input: &Value) -> Option<String> {
    let lower = kind.to_ascii_lowercase();
    let is_dispatch = lower == "task"
        || lower == "agent"
        || lower == "dispatch_agent"
        || lower == "dispatch"
        || lower == "run_agent"
        || lower == "call_agent"
        || lower.starts_with("agent_")
        || lower.contains("_agent")
        || lower.contains("subagent");
    if !is_dispatch {
        return None;
    }
    // Try to extract the sub-agent's identity from rawInput.
    if let Some(obj) = raw_input.as_object() {
        for field in ["subagent_type", "agent", "agent_type", "name", "mode"] {
            if let Some(v) = obj.get(field).and_then(|v| v.as_str()) {
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    // Fall back to the kind itself so the UI at least shows *something*.
    Some(kind.to_string())
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

/// An auto-approve action queued by the translator for the stream loop to
/// send back to the backend without user interaction.
#[derive(Debug, Clone)]
pub(crate) struct AutoApproveAction {
    pub request_id: Value,
    pub option_id: String,
}

/// Translator state for a single ACP session.
#[derive(Debug, Default)]
pub(crate) struct TranslationContext {
    pending_tools: HashMap<String, PendingTool>,
    /// Pending permission requests keyed by tool_call_id. The adapter layer
    /// reads these when the user approves/rejects.
    pending_permissions: HashMap<String, PendingPermission>,
    /// Permission requests that were auto-approved (risk <= Medium). The
    /// stream loop drains this after each translate() call and sends the
    /// approve response directly to the backend.
    pub(crate) auto_approve_queue: Vec<AutoApproveAction>,
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
    /// Running local cost estimator. The backend doesn't report real token
    /// counts for ACP, so we tokenize everything we see ourselves. Emits
    /// CostUpdate with `estimated: true` so the UI can badge it as "est."
    pub(crate) cost: CostEstimator,
}

impl TranslationContext {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that a prompt has just been sent with the given request id.
    /// The next matching response will emit Complete. `prompt_text` is the
    /// full text (including any prepended briefing / memories) so the cost
    /// estimator can count the input tokens.
    pub fn begin_prompt(&mut self, req_id: u64, prompt_text: &str) {
        self.prompt_req_id = Some(req_id);
        self.prompt_started_at = Some(Instant::now());
        self.accumulated_summary.clear();
        self.last_text = None;
        self.turns = self.turns.saturating_add(1);
        self.cost.record_input(prompt_text);
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

        // _kiro.dev/metadata carries context-window usage as a percentage.
        // Translate to ContextUsage so the UI can render a fill bar.
        if msg.is_method("_kiro.dev/metadata") {
            if let Some(pct) = msg
                .params
                .as_ref()
                .and_then(|p| p.get("contextUsagePercentage"))
                .and_then(|v| v.as_f64())
            {
                return vec![AgentEvent::ContextUsage { percentage: pct }];
            }
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

        // Emit a CostUpdate snapshot before Complete so the UI can settle
        // final numbers. Marked estimated — real cost lives in the user's
        // Bedrock bill, our number is a rough char/4 approximation.
        let snapshot = self.cost.snapshot();
        let total_cost_usd = match &snapshot {
            AgentEvent::CostUpdate { total_usd, .. } => *total_usd,
            _ => 0.0,
        };
        events.push(snapshot);

        self.prompt_req_id = None;
        events.push(AgentEvent::Complete {
            summary,
            total_cost_usd,
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

        // Auto-approve Medium and below: queue the approve response for the
        // stream loop to send, and emit the ToolRequest with needs_approval=false
        // so it doesn't trigger the gate in SessionManager.
        let auto_approve = risk_level <= RiskLevel::Medium;

        if auto_approve {
            if let Some(id) = msg.id.clone() {
                let option_id = options
                    .iter()
                    .find(|o| matches!(o.kind.as_str(), "allow_once" | "allow" | "allow_always"))
                    .map(|o| o.id.clone())
                    .or_else(|| options.first().map(|o| o.id.clone()));
                if let Some(oid) = option_id {
                    tracing::info!(
                        tool = %kind,
                        risk = ?risk_level,
                        "auto-approving ACP permission request",
                    );
                    self.auto_approve_queue.push(AutoApproveAction {
                        request_id: id,
                        option_id: oid,
                    });
                }
            }
        } else {
            // High/Critical: store for manual approve/reject via the gate UI.
            if let Some(id) = msg.id.clone() {
                self.pending_permissions.insert(
                    tool_call_id.clone(),
                    PendingPermission {
                        request_id: id,
                        options,
                    },
                );
            }
        }

        // Record as a pending tool so tool_call_update emits a ToolResult.
        let sub_agent = detect_sub_agent_dispatch(&kind, &raw_input);
        self.pending_tools.insert(
            tool_call_id.clone(),
            PendingTool {
                tool_name: kind.clone(),
                started_at: Instant::now(),
                sub_agent: sub_agent.clone(),
            },
        );

        let mut events = Vec::new();
        if let Some((text, _)) = self.last_text.take() {
            events.push(AgentEvent::Text { text });
        }
        events.push(AgentEvent::ToolRequest {
            id: tool_call_id,
            tool_name: kind,
            description: title,
            input: raw_input,
            needs_approval: !auto_approve,
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

        // Every character the model emits counts toward output tokens. We
        // record thinking chunks too — internal reasoning is billed.
        self.cost.record_output(&text);

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

        let sub_agent = detect_sub_agent_dispatch(&kind, &raw_input);
        self.pending_tools.insert(
            tool_call_id.clone(),
            PendingTool {
                tool_name: kind.clone(),
                started_at: Instant::now(),
                sub_agent: sub_agent.clone(),
            },
        );

        let mut events = Vec::new();
        if let Some((text, _)) = self.last_text.take() {
            events.push(AgentEvent::Text { text });
        }
        events.push(AgentEvent::ToolRequest {
            id: tool_call_id.clone(),
            tool_name: kind.clone(),
            description: title.clone(),
            input: raw_input.clone(),
            needs_approval: false,
            risk_level: classify_acp_risk(&kind, &raw_input),
        });
        // Sub-agent dispatch tools get a parallel SubAgentSpawned event so
        // the timeline can render them as a collapsible nested branch.
        if sub_agent.is_some() {
            events.push(AgentEvent::SubAgentSpawned {
                parent_tool_use_id: tool_call_id,
                description: if title.is_empty() {
                    sub_agent.unwrap_or_default()
                } else {
                    title
                },
            });
        }
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

        // Tool output becomes context the model consumes on the next turn,
        // so it counts toward input tokens. Prefer rawOutput when present
        // since that's what the backend will actually re-inject.
        self.cost.record_input(raw_output.as_deref().unwrap_or(&output));

        let mut events = Vec::new();
        if let Some((text, _)) = self.last_text.take() {
            events.push(AgentEvent::Text { text });
        }
        let sub_agent_parent = tool_call_id.clone();
        let was_sub_agent = pending.sub_agent.is_some();
        let sub_agent_summary = output.clone();
        events.push(AgentEvent::ToolResult {
            id: tool_call_id,
            tool_name: pending.tool_name,
            success: status == "completed",
            output,
            raw_output,
            duration_ms,
        });
        if was_sub_agent {
            // Close the sub-agent branch the timeline opened on tool_call.
            events.push(AgentEvent::SubAgentComplete {
                parent_tool_use_id: sub_agent_parent,
                summary: sub_agent_summary,
                cost_usd: 0.0, // backend doesn't report per-sub-agent cost today
            });
        }
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
        assert!(ctx.translate(&c2).is_empty(), "chunks within the gap should coalesce silently");

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
    fn request_permission_high_risk_stores_for_manual_approval() {
        let mut ctx = TranslationContext::new();
        let perm = msg(
            r#"{"jsonrpc":"2.0","id":"uuid-2","method":"session/request_permission","params":{
                "toolCall":{"toolCallId":"tc-d","kind":"delete","title":"x","rawInput":{}},
                "options":[
                    {"optionId":"approve","name":"Approve","kind":"allow_always"},
                    {"optionId":"stop","name":"Reject","kind":"reject"}
                ]
            }}"#,
        );
        let events = ctx.translate(&perm);
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolRequest { needs_approval: true, .. })));
        assert!(ctx.auto_approve_queue.is_empty());
        let pending = ctx.take_permission("tc-d").expect("stored");
        assert_eq!(pending.options.len(), 2);
        assert!(pending.options.iter().any(|o| o.kind == "allow_always" && o.id == "approve"));
        assert!(pending.options.iter().any(|o| o.kind == "reject" && o.id == "stop"));
    }

    #[test]
    fn request_permission_medium_risk_is_auto_approved() {
        let mut ctx = TranslationContext::new();
        let perm = msg(
            r#"{"jsonrpc":"2.0","id":"uuid-3","method":"session/request_permission","params":{
                "toolCall":{"toolCallId":"tc-e","kind":"edit","title":"edit file","rawInput":{}},
                "options":[
                    {"optionId":"allow","name":"Allow","kind":"allow_once"},
                    {"optionId":"deny","name":"Deny","kind":"reject"}
                ]
            }}"#,
        );
        let events = ctx.translate(&perm);
        assert!(events.iter().any(|e| matches!(e, AgentEvent::ToolRequest { needs_approval: false, .. })));
        assert!(ctx.take_permission("tc-e").is_none(), "should not be stored for manual approval");
        assert_eq!(ctx.auto_approve_queue.len(), 1);
        assert_eq!(ctx.auto_approve_queue[0].option_id, "allow");
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
    fn prompt_response_with_end_turn_emits_cost_then_complete() {
        // end_turn now produces CostUpdate (estimated) immediately before
        // Complete so the UI can settle final numbers at the same moment.
        let mut ctx = TranslationContext::new();
        ctx.begin_prompt(1, "test prompt");
        let done = msg(r#"{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}"#);
        let events = ctx.translate(&done);
        assert_eq!(events.len(), 2, "expected CostUpdate + Complete, got: {events:?}");
        match &events[0] {
            AgentEvent::CostUpdate { estimated, .. } => {
                assert!(*estimated, "ACP cost snapshots must be marked estimated");
            }
            other => panic!("expected CostUpdate first, got {other:?}"),
        }
        match &events[1] {
            AgentEvent::Complete { turns, .. } => assert_eq!(*turns, 1),
            other => panic!("expected Complete second, got {other:?}"),
        }
    }

    #[test]
    fn prompt_response_non_end_turn_annotates_summary() {
        let mut ctx = TranslationContext::new();
        ctx.begin_prompt(1, "test prompt");
        let chunk = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hi"}}
            }}"#,
        );
        let _ = ctx.translate(&chunk);
        let done = msg(r#"{"jsonrpc":"2.0","id":1,"result":{"stopReason":"max_tokens"}}"#);
        let events = ctx.translate(&done);
        // Flushed Text, then CostUpdate, then Complete.
        assert_eq!(events.len(), 3, "got: {events:?}");
        assert!(matches!(events[0], AgentEvent::Text { .. }));
        assert!(matches!(events[1], AgentEvent::CostUpdate { .. }));
        match &events[2] {
            AgentEvent::Complete { summary, .. } => {
                assert!(summary.contains("hi"));
                assert!(summary.contains("max_tokens"));
            }
            other => panic!("expected Complete last, got {other:?}"),
        }
    }

    #[test]
    fn prompt_response_with_error_emits_error_event() {
        let mut ctx = TranslationContext::new();
        ctx.begin_prompt(1, "test prompt");
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
        ctx.begin_prompt(5, "test prompt");
        let stray = msg(r#"{"jsonrpc":"2.0","id":99,"result":{"stopReason":"end_turn"}}"#);
        assert!(ctx.translate(&stray).is_empty());
    }

    // ─── metadata + unknown ───────────────────────────────────────────────

    #[test]
    fn metadata_with_context_usage_emits_context_usage_event() {
        let mut ctx = TranslationContext::new();
        let meta = msg(r#"{"jsonrpc":"2.0","method":"_kiro.dev/metadata","params":{"contextUsagePercentage":42.0}}"#);
        let e = single(ctx.translate(&meta));
        match e {
            AgentEvent::ContextUsage { percentage } => assert!((percentage - 42.0).abs() < f64::EPSILON),
            other => panic!("expected ContextUsage, got {other:?}"),
        }
    }

    #[test]
    fn metadata_without_context_usage_is_silently_consumed() {
        let mut ctx = TranslationContext::new();
        let meta = msg(r#"{"jsonrpc":"2.0","method":"_kiro.dev/metadata","params":{}}"#);
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

    // ─── sub-agent dispatch detection ─────────────────────────────────────

    #[test]
    fn detect_sub_agent_catches_task_tool_with_subagent_type() {
        // Claude's Task tool uses subagent_type in rawInput.
        let raw = serde_json::json!({"subagent_type": "codebase-analyzer", "prompt": "..."});
        assert_eq!(
            detect_sub_agent_dispatch("task", &raw).as_deref(),
            Some("codebase-analyzer")
        );
    }

    #[test]
    fn detect_sub_agent_catches_dispatch_agent_variants() {
        let raw = serde_json::json!({"agent": "tsuki-executor"});
        assert_eq!(
            detect_sub_agent_dispatch("dispatch_agent", &raw).as_deref(),
            Some("tsuki-executor")
        );
    }

    #[test]
    fn detect_sub_agent_falls_back_to_kind_when_input_lacks_name() {
        let raw = serde_json::json!({});
        assert_eq!(
            detect_sub_agent_dispatch("run_agent", &raw).as_deref(),
            Some("run_agent")
        );
    }

    #[test]
    fn detect_sub_agent_none_for_regular_tools() {
        assert!(detect_sub_agent_dispatch("read", &Value::Null).is_none());
        assert!(detect_sub_agent_dispatch("edit", &Value::Null).is_none());
        assert!(detect_sub_agent_dispatch("execute", &Value::Null).is_none());
        // Word-boundary-like: "agent_mode_tool" is a real concern but we
        // currently accept `_agent` as evidence of delegation. That's OK
        // unless a backend names a non-dispatch tool like "set_agent_hat"
        // — if that happens, tighten the heuristic.
    }

    #[test]
    fn tool_call_with_sub_agent_dispatch_emits_spawn_event() {
        let mut ctx = TranslationContext::new();
        let tc = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call","toolCallId":"tc-9",
                          "kind":"task","title":"Research the API",
                          "rawInput":{"subagent_type":"codebase-analyzer","prompt":"how does X work?"}}
            }}"#,
        );
        let events = ctx.translate(&tc);
        assert_eq!(events.len(), 2, "expected ToolRequest + SubAgentSpawned");
        match &events[0] {
            AgentEvent::ToolRequest { tool_name, .. } => assert_eq!(tool_name, "task"),
            other => panic!("expected ToolRequest first, got {other:?}"),
        }
        match &events[1] {
            AgentEvent::SubAgentSpawned { parent_tool_use_id, description } => {
                assert_eq!(parent_tool_use_id, "tc-9");
                assert!(description.contains("Research the API"));
            }
            other => panic!("expected SubAgentSpawned second, got {other:?}"),
        }
    }

    #[test]
    fn sub_agent_dispatch_emits_complete_event_on_tool_result() {
        let mut ctx = TranslationContext::new();
        let tc = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call","toolCallId":"tc-9","kind":"task",
                          "title":"research","rawInput":{"subagent_type":"analyzer"}}
            }}"#,
        );
        let _ = ctx.translate(&tc);
        let done = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call_update","toolCallId":"tc-9",
                          "status":"completed","output":"Analyzer findings go here"}
            }}"#,
        );
        let events = ctx.translate(&done);
        assert_eq!(events.len(), 2, "expected ToolResult + SubAgentComplete");
        match &events[1] {
            AgentEvent::SubAgentComplete { parent_tool_use_id, summary, .. } => {
                assert_eq!(parent_tool_use_id, "tc-9");
                assert!(summary.contains("Analyzer findings"));
            }
            other => panic!("expected SubAgentComplete, got {other:?}"),
        }
    }

    #[test]
    fn non_dispatch_tool_call_does_not_emit_sub_agent_events() {
        let mut ctx = TranslationContext::new();
        let tc = msg(
            r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"tool_call","toolCallId":"tc-read","kind":"read",
                          "title":"read main.rs","rawInput":{"path":"main.rs"}}
            }}"#,
        );
        let events = ctx.translate(&tc);
        assert_eq!(events.len(), 1, "regular read tool is single ToolRequest");
        assert!(matches!(events[0], AgentEvent::ToolRequest { .. }));
    }

    // ─── begin_prompt semantics ───────────────────────────────────────────

    #[test]
    fn begin_prompt_increments_turns() {
        let mut ctx = TranslationContext::new();
        ctx.begin_prompt(1, "test prompt");
        ctx.begin_prompt(2, "test prompt");
        ctx.begin_prompt(3, "test prompt");
        let done = msg(r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"end_turn"}}"#);
        // Now emits CostUpdate + Complete.
        let events = ctx.translate(&done);
        let e = events
            .into_iter()
            .find(|ev| matches!(ev, AgentEvent::Complete { .. }))
            .expect("should emit Complete");
        match e {
            AgentEvent::Complete { turns, .. } => assert_eq!(turns, 3),
            other => panic!("expected Complete, got {other:?}"),
        }
    }
}
