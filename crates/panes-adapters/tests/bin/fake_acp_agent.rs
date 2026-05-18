//! Tiny ACP-speaking agent used by `acp_integration.rs`.
//!
//! Reads JSON-RPC 2.0 requests on stdin, writes responses/notifications on
//! stdout, and branches on the `FAKE_ACP_SCENARIO` env var to drive the
//! adapter through specific flows. Ships no real tool execution — the point
//! is to exercise Panes' transport + translation + session plumbing.
//!
//! Scenarios:
//!   text_only         — one text chunk then end_turn
//!   gated_edit        — emits a session/request_permission; follows server
//!                       response with tool_call_update + end_turn
//!   gated_edit_reject — emits a session/request_permission; exits cleanly
//!                       if client sends cancelled outcome
//!   tool_error        — tool_call + tool_call_update(failed) + end_turn
//!   slow_prompt       — sleeps indefinitely after session/prompt until
//!                       session/cancel is received
//!   crash_mid_prompt  — writes a text chunk then exits process with code 1
//!   spam_notifications — 500 session/update notifications then end_turn
//!
//! All request ids are echoed verbatim in responses; notifications have no id.

use std::io::{BufRead, Write};
use std::time::Duration;

use serde_json::{json, Value};

fn main() {
    let scenario = std::env::var("FAKE_ACP_SCENARIO").unwrap_or_else(|_| "text_only".to_string());
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    let mut current_prompt_id: Option<u64> = None;
    let mut permission_req_count = 0u32;
    // For "two_gates": remember whether we've already issued the second gate.
    let mut two_gates_second_issued = false;

    // auth_failure scenario: print the auth error to stderr immediately and
    // then keep running normally — the adapter should see the stderr line
    // and surface a non-recoverable Error event.
    if scenario == "auth_failure" {
        let _ = writeln!(stderr, "401 Unauthorized: Midway token expired");
        let _ = stderr.flush();
    }

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = msg.get("id").cloned();

        match method {
            "initialize" => {
                respond(&mut stdout, id, json!({"protocolVersion": "2025-08-22"}));
            }
            "session/new" => {
                let sid = format!("sess-fake-{}", uuid_like());
                respond(
                    &mut stdout,
                    id,
                    json!({
                        "sessionId": sid,
                        "models": {
                            "availableModels": [
                                {"modelId": "fake-model-a"},
                                {"modelId": "fake-model-b"}
                            ]
                        }
                    }),
                );
            }
            "session/load" => {
                // If the client asks to load a session whose id starts with
                // "sess-fake-", pretend we have it. Otherwise error.
                let sid = msg
                    .pointer("/params/sessionId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if sid.starts_with("sess-fake-") || scenario == "resume_existing" {
                    respond(&mut stdout, id, json!({"modes": ["default"]}));
                } else {
                    respond_error(&mut stdout, id, -32001, "session not found");
                }
            }
            "session/set_mode" => {
                // Normally: kiro-cli returns no response, just the notification.
                // In the "mode_notification_missing" scenario we send neither to
                // validate the adapter's timeout-tolerance path.
                if scenario != "mode_notification_missing" {
                    notify(&mut stdout, "_kiro.dev/commands/available", json!({}));
                }
            }
            "session/set_model" => {
                respond(&mut stdout, id, json!({}));
            }
            "session/prompt" => {
                // Extract request id for later matching.
                let rid = id
                    .as_ref()
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                current_prompt_id = Some(rid);

                match scenario.as_str() {
                    "text_only" => {
                        send_text_chunk(&mut stdout, "hello");
                        respond(&mut stdout, id, json!({"stopReason": "end_turn"}));
                    }
                    "gated_edit" | "gated_edit_reject" | "two_gates" => {
                        let perm_id = format!("perm-{}", permission_req_count);
                        permission_req_count += 1;
                        send_permission_request(
                            &mut stdout,
                            &perm_id,
                            "tc-1",
                            "delete",
                            "delete main.rs",
                            json!({"path": "/tmp/main.rs"}),
                        );
                        // Next inbound message should be the client's
                        // response to perm-id. We'll read it on the next
                        // loop iteration and react below.
                        let _ = id;
                    }
                    "custom_option_ids" => {
                        // Emit a permission request whose options use
                        // backend-specific ids (no "allow_once"). The
                        // adapter must pick the first kind:"allow" option
                        // rather than hardcoding an id.
                        let perm_id = format!("perm-{}", permission_req_count);
                        permission_req_count += 1;
                        let msg = json!({
                            "jsonrpc": "2.0",
                            "id": perm_id,
                            "method": "session/request_permission",
                            "params": {
                                "toolCall": {
                                    "toolCallId": "tc-custom",
                                    "kind": "delete",
                                    "title": "custom delete",
                                    "rawInput": {}
                                },
                                "options": [
                                    {"optionId": "proceed_now", "name": "Proceed", "kind": "allow"},
                                    {"optionId": "halt_execution", "name": "Halt", "kind": "reject"}
                                ]
                            }
                        });
                        writeln!(stdout, "{msg}").ok();
                        stdout.flush().ok();
                    }
                    "non_gated_tool" => {
                        send_tool_call(
                            &mut stdout,
                            "tc-read",
                            "read",
                            "read main.rs",
                            json!({"path": "src/main.rs"}),
                        );
                        send_tool_call_update(&mut stdout, "tc-read", "completed", "file contents");
                        send_text_chunk(&mut stdout, "done reading");
                        respond(&mut stdout, id, json!({"stopReason": "end_turn"}));
                    }
                    "truncated_no_complete" => {
                        // Send partial output then stop writing — simulates a
                        // backend that silently stops responding. Adapter should
                        // eventually flush buffered text and close.
                        send_text_chunk(&mut stdout, "partial output");
                        // Exit the process so the transport sees EOF quickly.
                        let _ = stdout.flush();
                        std::process::exit(0);
                    }
                    "tool_error" => {
                        send_tool_call(&mut stdout, "tc-err", "execute", "run tests", json!({"command": "make test"}));
                        send_tool_call_update(&mut stdout, "tc-err", "failed", "compile error");
                        respond(&mut stdout, id, json!({"stopReason": "end_turn"}));
                    }
                    "slow_prompt" => {
                        // Don't respond to session/prompt — wait for
                        // session/cancel. Send a tool_call right away so the
                        // transport has something observable (text chunks
                        // would get stuck in the translator's coalesce
                        // buffer until another non-text event flushes them).
                        send_tool_call(
                            &mut stdout,
                            "slow-op",
                            "execute",
                            "doing slow work",
                            json!({"command": "sleep 999"}),
                        );
                    }
                    "crash_mid_prompt" => {
                        send_text_chunk(&mut stdout, "about to crash");
                        // Flush and exit abnormally.
                        let _ = stdout.flush();
                        std::process::exit(1);
                    }
                    "spam_notifications" => {
                        for i in 0..500 {
                            send_text_chunk(&mut stdout, &format!("chunk {i} "));
                        }
                        respond(&mut stdout, id, json!({"stopReason": "end_turn"}));
                    }
                    "realistic_streaming" => {
                        // Emulate kiro-cli's real token streaming: 30 chunks
                        // emitted with the same inter-chunk spacing we see
                        // from the real binary (~100ms). This exercises the
                        // translator's coalesce logic + the UI's adjacent-
                        // text-merge path end-to-end.
                        let chunks = [
                            "This ", "is ", "a ", "**Brazil ",
                            "workspace** ", "containing ", "the ",
                            "**GroceryPackUWC** ", "fleet ", "— ", "a ",
                            "UWC ", "(Universal ", "Workc", "ell) ",
                            "project ", "for ", "grocery ", "automation. ",
                            "It ", "has ", "3 ", "packages:\n\n",
                            "1. ", "**GroceryPackUWCCDK** ", "— ", "CDK ",
                            "infrastructure ", "(Type", "Script).",
                        ];
                        for chunk in chunks {
                            send_text_chunk(&mut stdout, chunk);
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        respond(&mut stdout, id, json!({"stopReason": "end_turn"}));
                    }
                    "resume_existing" => {
                        send_text_chunk(&mut stdout, "resumed session here");
                        respond(&mut stdout, id, json!({"stopReason": "end_turn"}));
                    }
                    _ => {
                        // Default: same as text_only
                        send_text_chunk(&mut stdout, "default response");
                        respond(&mut stdout, id, json!({"stopReason": "end_turn"}));
                    }
                }
            }
            "session/cancel" => {
                // If we're in a slow_prompt, complete the in-flight prompt.
                if let Some(pid) = current_prompt_id.take() {
                    respond(
                        &mut stdout,
                        Some(json!(pid)),
                        json!({"stopReason": "cancelled"}),
                    );
                }
                respond(&mut stdout, id, json!({}));
                // Most backends exit on cancel; we don't, because the adapter
                // will `shutdown()` us next.
            }
            _ => {
                // Non-request method — check for responses to our permission requests.
                if id.is_some() && method.is_empty() {
                    // A response to one of our server-initiated requests.
                    let outcome = msg
                        .pointer("/result/outcome/outcome")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let pid = current_prompt_id.take();
                    match scenario.as_str() {
                        "gated_edit" if outcome == "selected" => {
                            send_tool_call_update(
                                &mut stdout,
                                "tc-1",
                                "completed",
                                "edited successfully",
                            );
                            if let Some(p) = pid {
                                respond(&mut stdout, Some(json!(p)), json!({"stopReason": "end_turn"}));
                            }
                        }
                        "gated_edit_reject" if outcome == "cancelled" => {
                            if let Some(p) = pid {
                                respond(&mut stdout, Some(json!(p)), json!({"stopReason": "cancelled"}));
                            }
                        }
                        "two_gates" if outcome == "selected" && !two_gates_second_issued => {
                            // Acknowledge first tool completion.
                            send_tool_call_update(
                                &mut stdout,
                                "tc-1",
                                "completed",
                                "first step done",
                            );
                            // Issue the second gate — keep the prompt open.
                            let perm_id = format!("perm-{}", permission_req_count);
                            permission_req_count += 1;
                            send_permission_request(
                                &mut stdout,
                                &perm_id,
                                "tc-2",
                                "delete",
                                "delete the second file",
                                json!({"path": "/tmp/step-2"}),
                            );
                            two_gates_second_issued = true;
                            // Put the prompt id back so the next approval
                            // can close it.
                            current_prompt_id = pid;
                        }
                        "two_gates" if outcome == "selected" && two_gates_second_issued => {
                            send_tool_call_update(
                                &mut stdout,
                                "tc-2",
                                "completed",
                                "second step done",
                            );
                            if let Some(p) = pid {
                                respond(&mut stdout, Some(json!(p)), json!({"stopReason": "end_turn"}));
                            }
                        }
                        _ => {
                            // Unexpected outcome — end the turn anyway.
                            if let Some(p) = pid {
                                respond(&mut stdout, Some(json!(p)), json!({"stopReason": "end_turn"}));
                            }
                        }
                    }
                }
            }
        }

        // Handle slow_prompt keepalive: periodically send a tick so the
        // transport reader has something to do. Best-effort.
        if scenario == "slow_prompt" {
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn respond(out: &mut impl Write, id: Option<Value>, result: Value) {
    let msg = json!({"jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result});
    writeln!(out, "{msg}").ok();
    out.flush().ok();
}

fn respond_error(out: &mut impl Write, id: Option<Value>, code: i64, message: &str) {
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": {"code": code, "message": message}
    });
    writeln!(out, "{msg}").ok();
    out.flush().ok();
}

fn notify(out: &mut impl Write, method: &str, params: Value) {
    let msg = json!({"jsonrpc": "2.0", "method": method, "params": params});
    writeln!(out, "{msg}").ok();
    out.flush().ok();
}

fn send_text_chunk(out: &mut impl Write, text: &str) {
    notify(
        out,
        "session/update",
        json!({
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": text}
            }
        }),
    );
}

fn send_permission_request(
    out: &mut impl Write,
    request_id: &str,
    tool_call_id: &str,
    kind: &str,
    title: &str,
    raw_input: Value,
) {
    let msg = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "session/request_permission",
        "params": {
            "toolCall": {
                "toolCallId": tool_call_id,
                "kind": kind,
                "title": title,
                "rawInput": raw_input
            },
            "options": [
                {"optionId": "allow_once", "name": "Allow once", "kind": "allow"},
                {"optionId": "cancel", "name": "Cancel", "kind": "deny"}
            ]
        }
    });
    writeln!(out, "{msg}").ok();
    out.flush().ok();
}

fn send_tool_call(out: &mut impl Write, id: &str, kind: &str, title: &str, raw_input: Value) {
    notify(
        out,
        "session/update",
        json!({
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": id,
                "kind": kind,
                "title": title,
                "rawInput": raw_input
            }
        }),
    );
}

fn send_tool_call_update(out: &mut impl Write, id: &str, status: &str, output: &str) {
    notify(
        out,
        "session/update",
        json!({
            "update": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": id,
                "status": status,
                "output": output
            }
        }),
    );
}

fn uuid_like() -> String {
    // No need to pull in uuid just for the fake agent — pid + monotonic
    // counter gives us distinct ids across sibling processes in tests.
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    let n = N.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id() as u64;
    format!("{pid:08x}{n:08x}")
}
