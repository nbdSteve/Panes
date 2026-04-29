use std::pin::Pin;

use anyhow::{Context, Result};
use futures::Stream;
use panes_events::{AgentEvent, SessionInit};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::watch;
use tracing::{debug, warn};

use super::risk::classify_risk;

/// Parse the Claude Code stream-json output into AgentEvents.
///
/// First waits for the `system/init` event, then returns a stream of AgentEvents.
/// Forward-compatible: unknown event types are logged and skipped.
pub async fn parse_stream<R: tokio::io::AsyncRead + Unpin + Send + 'static>(
    reader: BufReader<R>,
    auth_error_rx: watch::Receiver<Option<String>>,
) -> Result<(SessionInit, Pin<Box<dyn Stream<Item = AgentEvent> + Send>>)> {
    let mut lines = reader.lines();

    // Read lines until we get the system/init event
    let init = loop {
        let line = lines
            .next_line()
            .await
            .context("failed to read from claude stdout")?
            .context("claude stdout closed before init event")?;

        let v: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                debug!(line = %line, error = %e, "skipping non-JSON line from claude stdout");
                continue;
            }
        };

        if v.get("type").and_then(|t| t.as_str()) == Some("system")
            && v.get("subtype").and_then(|s| s.as_str()) == Some("init")
        {
            let session_id = v
                .get("session_id")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown")
                .to_string();
            let model = v
                .get("model")
                .and_then(|s| s.as_str())
                .unwrap_or("unknown")
                .to_string();
            let cwd = v
                .get("cwd")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let tools = v
                .get("tools")
                .and_then(|t| t.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default();

            break SessionInit {
                session_id,
                model,
                cwd,
                tools,
            };
        }

        debug!(event_type = ?v.get("type"), "skipping pre-init event");
    };

    let stream = async_stream::stream! {
        let mut auth_rx = auth_error_rx;

        loop {
            tokio::select! {
                line_result = lines.next_line() => {
                    match line_result {
                        Ok(Some(line)) => {
                            let events = parse_line(&line);
                            for event in events {
                                yield event;
                            }
                        }
                        Ok(None) => break, // stdout closed
                        Err(e) => {
                            warn!(error = %e, "error reading claude stdout");
                            yield AgentEvent::Error {
                                message: format!("Failed to read claude output: {e}"),
                                recoverable: false,
                            };
                            break;
                        }
                    }
                }
                _ = auth_rx.changed() => {
                    let err_msg = auth_rx.borrow_and_update().clone();
                    if let Some(err) = err_msg {
                        yield AgentEvent::Error {
                            message: format!("Authentication error: {err}. Run `claude auth` to re-authenticate."),
                            recoverable: false,
                        };
                        break;
                    }
                }
            }
        }
    };

    Ok((init, Box::pin(stream)))
}

fn parse_line(line: &str) -> Vec<AgentEvent> {
    let v: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return vec![],
    };

    let event_type = match v.get("type").and_then(|t| t.as_str()) {
        Some(t) => t,
        None => return vec![],
    };

    let parent_tool_use_id = v
        .get("parent_tool_use_id")
        .and_then(|p| p.as_str())
        .map(String::from);

    match event_type {
        "assistant" => parse_assistant_event(&v, parent_tool_use_id.as_deref()),
        "user" => parse_user_event(&v),
        "result" => parse_result_event(&v),
        "system" => {
            // system events after init (hooks, etc.) — skip for now
            debug!(subtype = ?v.get("subtype"), "skipping system event");
            vec![]
        }
        other => {
            debug!(event_type = other, "skipping unknown event type");
            vec![]
        }
    }
}

fn parse_assistant_event(v: &Value, parent_tool_use_id: Option<&str>) -> Vec<AgentEvent> {
    let mut events = vec![];

    // Extract cost from usage field on every assistant message
    if let Some(usage) = v.pointer("/message/usage") {
        let input = usage.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
        let output = usage.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
        let cache_read = usage
            .get("cache_read_input_tokens")
            .and_then(|t| t.as_u64())
            .unwrap_or(0);
        let cache_creation = usage
            .get("cache_creation_input_tokens")
            .and_then(|t| t.as_u64())
            .unwrap_or(0);
        let model = v
            .pointer("/message/model")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown")
            .to_string();

        // Rough cost estimation based on Claude Opus pricing
        // These will be superseded by the result event's total_cost_usd
        let estimated_cost = estimate_cost(input, output, cache_read, cache_creation);

        events.push(AgentEvent::CostUpdate {
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: cache_read,
            cache_creation_tokens: cache_creation,
            total_usd: estimated_cost,
            model,
        });
    }

    // Parse content array
    let content = match v.pointer("/message/content").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => return events,
    };

    for item in content {
        let content_type = match item.get("type").and_then(|t| t.as_str()) {
            Some(t) => t,
            None => continue,
        };

        match content_type {
            "thinking" => {
                if let Some(text) = item.get("thinking").and_then(|t| t.as_str()) {
                    events.push(AgentEvent::Thinking {
                        text: text.to_string(),
                    });
                }
            }
            "text" => {
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    events.push(AgentEvent::Text {
                        text: text.to_string(),
                    });
                }
            }
            "tool_use" => {
                let id = item
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or("")
                    .to_string();
                let tool_name = item
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                let input = item.get("input").cloned().unwrap_or(Value::Null);

                let risk_level = classify_risk(&tool_name, &input);
                let needs_approval = risk_level >= panes_events::RiskLevel::High;
                let description = build_tool_description(&tool_name, &input);

                // Detect sub-agent spawning
                if tool_name == "Task" || tool_name == "Agent" {
                    if let Some(parent_id) = parent_tool_use_id {
                        events.push(AgentEvent::SubAgentSpawned {
                            parent_tool_use_id: parent_id.to_string(),
                            description: description.clone(),
                        });
                    }
                }

                events.push(AgentEvent::ToolRequest {
                    id,
                    tool_name,
                    description,
                    input,
                    needs_approval,
                    risk_level,
                });
            }
            other => {
                debug!(content_type = other, "skipping unknown content type in assistant message");
            }
        }
    }

    events
}

fn parse_user_event(v: &Value) -> Vec<AgentEvent> {
    let mut events = vec![];

    let content = match v.pointer("/message/content").and_then(|c| c.as_array()) {
        Some(c) => c,
        None => return events,
    };

    for item in content {
        if item.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
            let id = item
                .get("tool_use_id")
                .and_then(|i| i.as_str())
                .unwrap_or("")
                .to_string();
            let is_error = item.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
            let output = item
                .get("content")
                .and_then(|c| {
                    if let Some(s) = c.as_str() {
                        Some(s.to_string())
                    } else if let Some(arr) = c.as_array() {
                        // Content can be an array of content blocks
                        Some(
                            arr.iter()
                                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("\n"),
                        )
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            events.push(AgentEvent::ToolResult {
                id,
                tool_name: String::new(), // not available in tool_result events
                success: !is_error,
                output: truncate_for_display(&output, 500),
                raw_output: Some(output),
                duration_ms: 0, // not available in stream-json
            });
        }
    }

    events
}

fn parse_result_event(v: &Value) -> Vec<AgentEvent> {
    let total_cost = v
        .get("total_cost_usd")
        .and_then(|c| c.as_f64())
        .unwrap_or(0.0);
    let duration = v
        .get("duration_ms")
        .and_then(|d| d.as_u64())
        .unwrap_or(0);
    let turns = v
        .get("num_turns")
        .and_then(|t| t.as_u64())
        .unwrap_or(0) as u32;
    let summary = v
        .get("result")
        .and_then(|r| r.as_str())
        .unwrap_or("")
        .to_string();

    vec![AgentEvent::Complete {
        summary,
        total_cost_usd: total_cost,
        duration_ms: duration,
        turns,
    }]
}

fn build_tool_description(tool_name: &str, input: &Value) -> String {
    match tool_name {
        "Bash" => {
            let cmd = input
                .get("command")
                .and_then(|c| c.as_str())
                .unwrap_or("(unknown command)");
            let desc = input.get("description").and_then(|d| d.as_str());
            match desc {
                Some(d) => format!("Run command: {d}"),
                None => format!("Run command: {}", truncate_for_display(cmd, 100)),
            }
        }
        "Read" => {
            let path = input
                .get("file_path")
                .and_then(|p| p.as_str())
                .unwrap_or("(unknown)");
            format!("Read file: {path}")
        }
        "Write" => {
            let path = input
                .get("file_path")
                .and_then(|p| p.as_str())
                .unwrap_or("(unknown)");
            format!("Create file: {path}")
        }
        "Edit" => {
            let path = input
                .get("file_path")
                .and_then(|p| p.as_str())
                .unwrap_or("(unknown)");
            format!("Edit file: {path}")
        }
        "Task" | "Agent" => {
            let desc = input
                .get("description")
                .or_else(|| input.get("prompt"))
                .and_then(|d| d.as_str())
                .unwrap_or("(sub-agent task)");
            format!("Delegate: {}", truncate_for_display(desc, 100))
        }
        "WebSearch" => {
            let query = input
                .get("query")
                .and_then(|q| q.as_str())
                .unwrap_or("(unknown)");
            format!("Web search: {query}")
        }
        "WebFetch" => {
            let url = input
                .get("url")
                .and_then(|u| u.as_str())
                .unwrap_or("(unknown)");
            format!("Fetch URL: {}", truncate_for_display(url, 80))
        }
        _ => format!("Tool: {tool_name}"),
    }
}

fn truncate_for_display(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let boundary = s.char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= max_len)
            .last()
            .unwrap_or(0);
        format!("{}...", &s[..boundary])
    }
}

fn estimate_cost(input: u64, output: u64, cache_read: u64, cache_creation: u64) -> f64 {
    // Claude Opus 4 pricing (approximate, per million tokens)
    const INPUT_PER_M: f64 = 15.0;
    const OUTPUT_PER_M: f64 = 75.0;
    const CACHE_READ_PER_M: f64 = 1.5;
    const CACHE_CREATION_PER_M: f64 = 18.75;

    let cost = (input as f64 * INPUT_PER_M
        + output as f64 * OUTPUT_PER_M
        + cache_read as f64 * CACHE_READ_PER_M
        + cache_creation as f64 * CACHE_CREATION_PER_M)
        / 1_000_000.0;
    (cost * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_assistant_thinking() {
        let line = r#"{"type":"assistant","message":{"model":"claude-opus-4-6","id":"msg_1","content":[{"type":"thinking","thinking":"Let me think...","signature":"sig"}],"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100,"cache_creation_input_tokens":0}},"parent_tool_use_id":null,"session_id":"sess_1"}"#;
        let events = parse_line(line);
        assert!(events.len() >= 2); // CostUpdate + Thinking
        assert!(matches!(&events[1], AgentEvent::Thinking { text } if text == "Let me think..."));
    }

    #[test]
    fn test_parse_tool_use() {
        let line = r#"{"type":"assistant","message":{"model":"claude-opus-4-6","id":"msg_2","content":[{"type":"tool_use","id":"tool_1","name":"Bash","input":{"command":"ls","description":"List files"}}],"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"parent_tool_use_id":null,"session_id":"sess_1"}"#;
        let events = parse_line(line);
        let tool_req = events.iter().find(|e| matches!(e, AgentEvent::ToolRequest { .. }));
        assert!(tool_req.is_some());
        if let Some(AgentEvent::ToolRequest { tool_name, risk_level, needs_approval, .. }) = tool_req {
            assert_eq!(tool_name, "Bash");
            assert_eq!(*risk_level, panes_events::RiskLevel::Low);
            assert!(!needs_approval);
        }
    }

    #[test]
    fn test_parse_result() {
        let line = r#"{"type":"result","subtype":"success","total_cost_usd":0.057,"duration_ms":17799,"num_turns":2,"result":"Done."}"#;
        let events = parse_line(line);
        assert_eq!(events.len(), 1);
        if let AgentEvent::Complete { total_cost_usd, duration_ms, turns, summary } = &events[0] {
            assert!((*total_cost_usd - 0.057).abs() < 0.001);
            assert_eq!(*duration_ms, 17799);
            assert_eq!(*turns, 2);
            assert_eq!(summary, "Done.");
        } else {
            panic!("expected Complete event");
        }
    }

    #[test]
    fn test_unknown_event_type_ignored() {
        let line = r#"{"type":"new_future_type","data":"something"}"#;
        let events = parse_line(line);
        assert!(events.is_empty());
    }

    #[test]
    fn test_non_json_ignored() {
        let events = parse_line("not json at all");
        assert!(events.is_empty());
    }

    #[test]
    fn test_build_tool_description_bash() {
        let input: Value = serde_json::json!({"command": "npm test", "description": "Run tests"});
        let desc = build_tool_description("Bash", &input);
        assert_eq!(desc, "Run command: Run tests");
    }

    #[test]
    fn test_build_tool_description_write() {
        let input: Value = serde_json::json!({"file_path": "/tmp/hello.ts", "content": "..."});
        let desc = build_tool_description("Write", &input);
        assert_eq!(desc, "Create file: /tmp/hello.ts");
    }

    #[test]
    fn test_truncate_ascii() {
        assert_eq!(truncate_for_display("hello", 10), "hello");
        assert_eq!(truncate_for_display("hello world", 5), "hello...");
    }

    #[test]
    fn test_truncate_utf8_no_panic() {
        // 'é' is 2 bytes, '日' is 3 bytes, '🦀' is 4 bytes
        let s = "café日本語🦀";
        for max in 0..s.len() + 2 {
            let _ = truncate_for_display(s, max); // must not panic
        }
    }

    #[test]
    fn test_truncate_utf8_boundary() {
        let s = "ab日c"; // 'ab' = 2 bytes, '日' = 3 bytes at position 2-4, 'c' = 1 byte at position 5
        let result = truncate_for_display(s, 3);
        // max_len=3 falls inside '日' (bytes 2,3,4), so we truncate before it
        assert_eq!(result, "ab...");
    }

    #[test]
    fn test_parse_user_tool_result_string() {
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool_1","content":"file contents here","is_error":false}]}}"#;
        let events = parse_line(line);
        assert_eq!(events.len(), 1);
        if let AgentEvent::ToolResult { id, success, output, .. } = &events[0] {
            assert_eq!(id, "tool_1");
            assert!(success);
            assert_eq!(output, "file contents here");
        } else {
            panic!("expected ToolResult");
        }
    }

    #[test]
    fn test_parse_user_tool_result_array_content() {
        let line = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool_2","content":[{"type":"text","text":"line1"},{"type":"text","text":"line2"}],"is_error":true}]}}"#;
        let events = parse_line(line);
        assert_eq!(events.len(), 1);
        if let AgentEvent::ToolResult { id, success, output, .. } = &events[0] {
            assert_eq!(id, "tool_2");
            assert!(!success);
            assert_eq!(output, "line1\nline2");
        } else {
            panic!("expected ToolResult");
        }
    }

    #[test]
    fn test_estimate_cost_basic() {
        let cost = estimate_cost(1_000_000, 0, 0, 0);
        assert!((cost - 15.0).abs() < 0.001); // 1M input tokens * $15/M

        let cost = estimate_cost(0, 1_000_000, 0, 0);
        assert!((cost - 75.0).abs() < 0.001); // 1M output tokens * $75/M
    }

    #[test]
    fn test_estimate_cost_with_cache() {
        let cost = estimate_cost(0, 0, 1_000_000, 0);
        assert!((cost - 1.5).abs() < 0.001); // cache read at $1.50/M

        let cost = estimate_cost(0, 0, 0, 1_000_000);
        assert!((cost - 18.75).abs() < 0.001); // cache creation at $18.75/M
    }
}
