//! ACP (Agent Client Protocol) adapter.
//!
//! JSON-RPC 2.0 over stdio between Panes and an ACP-speaking agent CLI.
//! See [`AcpAdapter::kiro_cli`] for the default kiro-cli preset.

pub(crate) mod adapter;
pub(crate) mod cost;
pub(crate) mod events;
pub(crate) mod session;
pub(crate) mod transport;

pub use adapter::{AcpAdapter, AcpSession};

/// Test-only helper to replay a stream of JSON-RPC messages through the
/// translation layer. Used by the `acp_replay` integration test so fixtures
/// can be exercised without spawning a process.
///
/// Each line in `raw_lines` must be a valid JSON-RPC message. Responses to
/// the implicit prompt request id (1) are matched; tests that need to script
/// a different prompt id should call this with a custom `prompt_req_id`.
#[doc(hidden)]
pub fn replay_messages_for_tests(raw_lines: &[String], prompt_req_id: u64) -> Vec<panes_events::AgentEvent> {
    use crate::acp::events::TranslationContext;
    use crate::acp::transport::JsonRpcMessage;

    let mut ctx = TranslationContext::new();
    ctx.begin_prompt(prompt_req_id, "");
    let mut out = Vec::new();
    for line in raw_lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: JsonRpcMessage = match serde_json::from_str(trimmed) {
            Ok(m) => m,
            Err(e) => panic!("replay fixture contains invalid JSON-RPC line:\n  {trimmed}\n  error: {e}"),
        };
        out.extend(ctx.translate(&msg));
    }
    out
}
