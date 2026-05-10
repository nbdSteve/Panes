//! Replay canned ACP session fixtures through the translation layer and
//! assert the resulting `AgentEvent` stream matches expectations.
//!
//! Each fixture is a pair:
//!   tests/fixtures/acp/<name>.jsonl       — one JSON-RPC message per line
//!   tests/fixtures/acp/<name>.expected.json — array of expected events
//!
//! The expected event schema is deliberately tolerant so tests stay readable:
//!
//!   { "type": "Text", "text_contains": "..." }
//!   { "type": "Thinking", "text_contains": "..." }
//!   { "type": "ToolRequest", "tool_name": "...", "needs_approval": true/false, "risk_level": "low|medium|high|critical", "id": "..." }
//!   { "type": "ToolResult", "success": true/false, "output_contains": "...", "id": "..." }
//!   { "type": "Complete", "summary_contains": "...", "turns": 1 }
//!   { "type": "Error", "message_contains": "...", "recoverable": true/false }
//!
//! Fields are optional — only the ones you write are checked.

use std::path::PathBuf;

use panes_adapters::replay_messages_for_tests;
use panes_events::{AgentEvent, RiskLevel};
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/acp")
}

fn load_jsonl(name: &str) -> Vec<String> {
    let path = fixtures_dir().join(format!("{name}.jsonl"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read fixture {path:?}: {e}"));
    text.lines().map(|l| l.to_string()).collect()
}

fn load_expected(name: &str) -> Vec<Value> {
    let path = fixtures_dir().join(format!("{name}.expected.json"));
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read expected {path:?}: {e}"));
    let v: Value = serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("expected fixture {path:?} is not JSON: {e}"));
    v.as_array()
        .unwrap_or_else(|| panic!("expected fixture {path:?} must be a JSON array"))
        .clone()
}

fn risk_from_str(s: &str) -> RiskLevel {
    match s {
        "low" => RiskLevel::Low,
        "medium" => RiskLevel::Medium,
        "high" => RiskLevel::High,
        "critical" => RiskLevel::Critical,
        other => panic!("unknown risk level in fixture: {other}"),
    }
}

/// Check one expected-event spec against one actual event.
fn check(spec: &Value, actual: &AgentEvent, context: &str) {
    let ty = spec
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("{context}: expected spec missing 'type': {spec}"));

    let variant_name = match actual {
        AgentEvent::Text { .. } => "Text",
        AgentEvent::Thinking { .. } => "Thinking",
        AgentEvent::ToolRequest { .. } => "ToolRequest",
        AgentEvent::ToolResult { .. } => "ToolResult",
        AgentEvent::Complete { .. } => "Complete",
        AgentEvent::Error { .. } => "Error",
        AgentEvent::CostUpdate { .. } => "CostUpdate",
        AgentEvent::SubAgentSpawned { .. } => "SubAgentSpawned",
        AgentEvent::SubAgentComplete { .. } => "SubAgentComplete",
        AgentEvent::ValidationResult { .. } => "ValidationResult",
    };
    assert_eq!(
        ty, variant_name,
        "{context}: expected variant {ty}, got {variant_name}\nspec: {spec}\nactual: {actual:?}"
    );

    match actual {
        AgentEvent::Text { text } | AgentEvent::Thinking { text } => {
            if let Some(needle) = spec.get("text_contains").and_then(|v| v.as_str()) {
                assert!(
                    text.contains(needle),
                    "{context}: text {text:?} does not contain {needle:?}"
                );
            }
        }
        AgentEvent::ToolRequest {
            id,
            tool_name,
            needs_approval,
            risk_level,
            ..
        } => {
            if let Some(want) = spec.get("id").and_then(|v| v.as_str()) {
                assert_eq!(id, want, "{context}: ToolRequest id mismatch");
            }
            if let Some(want) = spec.get("tool_name").and_then(|v| v.as_str()) {
                assert_eq!(tool_name, want, "{context}: tool_name mismatch");
            }
            if let Some(want) = spec.get("needs_approval").and_then(|v| v.as_bool()) {
                assert_eq!(*needs_approval, want, "{context}: needs_approval mismatch");
            }
            if let Some(want) = spec.get("risk_level").and_then(|v| v.as_str()) {
                assert_eq!(*risk_level, risk_from_str(want), "{context}: risk mismatch");
            }
        }
        AgentEvent::ToolResult {
            id,
            success,
            output,
            ..
        } => {
            if let Some(want) = spec.get("id").and_then(|v| v.as_str()) {
                assert_eq!(id, want, "{context}: ToolResult id mismatch");
            }
            if let Some(want) = spec.get("success").and_then(|v| v.as_bool()) {
                assert_eq!(*success, want, "{context}: success mismatch");
            }
            if let Some(needle) = spec.get("output_contains").and_then(|v| v.as_str()) {
                assert!(
                    output.contains(needle),
                    "{context}: output {output:?} does not contain {needle:?}"
                );
            }
        }
        AgentEvent::Complete {
            summary, turns, ..
        } => {
            if let Some(needle) = spec.get("summary_contains").and_then(|v| v.as_str()) {
                assert!(
                    summary.contains(needle),
                    "{context}: summary {summary:?} does not contain {needle:?}"
                );
            }
            if let Some(want) = spec.get("turns").and_then(|v| v.as_u64()) {
                assert_eq!(*turns as u64, want, "{context}: turns mismatch");
            }
        }
        AgentEvent::Error {
            message,
            recoverable,
        } => {
            if let Some(needle) = spec.get("message_contains").and_then(|v| v.as_str()) {
                assert!(
                    message.contains(needle),
                    "{context}: message {message:?} does not contain {needle:?}"
                );
            }
            if let Some(want) = spec.get("recoverable").and_then(|v| v.as_bool()) {
                assert_eq!(*recoverable, want, "{context}: recoverable mismatch");
            }
        }
        _ => {}
    }
}

fn replay(name: &str) {
    let raw = load_jsonl(name);
    let expected = load_expected(name);
    let actual = replay_messages_for_tests(&raw, 1);

    assert_eq!(
        actual.len(),
        expected.len(),
        "fixture {name}: expected {} events, got {} — actual: {actual:#?}",
        expected.len(),
        actual.len()
    );
    for (i, (spec, event)) in expected.iter().zip(actual.iter()).enumerate() {
        check(spec, event, &format!("{name}[{i}]"));
    }
}

#[test]
fn replay_simple_text() {
    replay("simple_text");
}

#[test]
fn replay_file_edit_with_permission() {
    replay("file_edit_with_permission");
}

#[test]
fn replay_dangerous_bash() {
    replay("dangerous_bash");
}

#[test]
fn replay_error_context_overflow() {
    replay("error_context_overflow");
}

#[test]
fn replay_multi_tool_interleaved() {
    replay("multi_tool_interleaved");
}

#[test]
fn replay_stale_stop_reason() {
    replay("stale_stop_reason");
}

#[test]
fn replay_thought_then_text() {
    replay("thought_then_text");
}
