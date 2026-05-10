//! End-to-end integration tests for the ACP adapter.
//!
//! Each test drives the full `AgentAdapter` + `AgentSession` contract against
//! the `fake-acp-agent` test binary. No mocks — we spawn a real child process
//! and exercise the transport, session lifecycle, event translation, and
//! approval state machine together.

use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use panes_adapters::{AcpAdapter, AgentAdapter};
use panes_events::{AgentEvent, SessionContext};

/// Absolute path to the fake-acp-agent binary Cargo built for us.
fn fake_agent_path() -> String {
    env!("CARGO_BIN_EXE_fake-acp-agent").to_string()
}

fn adapter_for(scenario: &str) -> AcpAdapter {
    AcpAdapter::new("fake-acp-agent", fake_agent_path(), Vec::new())
        .env("FAKE_ACP_SCENARIO", scenario)
}

fn workspace() -> PathBuf {
    std::env::temp_dir()
}

fn noop_context() -> SessionContext {
    SessionContext {
        briefing: None,
        memories: Vec::new(),
        budget_cap: None,
    }
}

async fn drain(
    stream: &mut (impl futures::Stream<Item = AgentEvent> + Unpin),
    timeout: Duration,
) -> Vec<AgentEvent> {
    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(ev)) => {
                let is_complete = matches!(ev, AgentEvent::Complete { .. });
                let is_error = matches!(ev, AgentEvent::Error { .. });
                events.push(ev);
                if is_complete || is_error {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    events
}

// ─── happy path ──────────────────────────────────────────────────────────

#[tokio::test]
async fn text_only_scenario_yields_complete() {
    let adapter = adapter_for("text_only");
    let mut session = adapter
        .spawn(&workspace(), "hi", &noop_context(), None, None)
        .await
        .expect("spawn");
    assert!(!session.init().session_id.is_empty());

    let mut stream = session.events();
    let events = drain(&mut stream, Duration::from_secs(10)).await;
    assert!(
        matches!(events.last(), Some(AgentEvent::Complete { .. })),
        "expected Complete as final event, got: {events:?}"
    );
}

#[tokio::test]
async fn tool_error_scenario_emits_failed_tool_result() {
    let adapter = adapter_for("tool_error");
    let mut session = adapter
        .spawn(&workspace(), "run tests", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();
    let events = drain(&mut stream, Duration::from_secs(10)).await;
    let failed = events
        .iter()
        .find(|e| matches!(e, AgentEvent::ToolResult { success: false, .. }));
    assert!(
        failed.is_some(),
        "expected a failed ToolResult somewhere, got: {events:?}"
    );
}

// ─── gated flow ──────────────────────────────────────────────────────────

#[tokio::test]
async fn gated_tool_call_approve_yields_success_and_complete() {
    let adapter = adapter_for("gated_edit");
    let mut session = adapter
        .spawn(&workspace(), "edit main.rs", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();

    // First event should be the gated ToolRequest.
    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("stream produced something")
        .expect("non-empty");
    let tool_id = match &first {
        AgentEvent::ToolRequest { id, needs_approval, .. } => {
            assert!(*needs_approval, "first event should be a gated tool request");
            id.clone()
        }
        other => panic!("expected gated ToolRequest, got {other:?}"),
    };

    // Approve the tool.
    session.approve(&tool_id).await.expect("approve");

    let rest = drain(&mut stream, Duration::from_secs(10)).await;
    let success = rest
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolResult { success: true, .. }));
    assert!(success, "expected successful ToolResult after approve, got: {rest:?}");
    assert!(
        matches!(rest.last(), Some(AgentEvent::Complete { .. })),
        "expected Complete as final event after approve, got: {rest:?}"
    );
}

#[tokio::test]
async fn gated_tool_call_reject_ends_stream_without_tool_result() {
    let adapter = adapter_for("gated_edit_reject");
    let mut session = adapter
        .spawn(&workspace(), "edit main.rs", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();

    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("stream produced")
        .expect("non-empty");
    let tool_id = match &first {
        AgentEvent::ToolRequest { id, needs_approval: true, .. } => id.clone(),
        other => panic!("expected gated ToolRequest, got {other:?}"),
    };

    session
        .reject(&tool_id, "user declined")
        .await
        .expect("reject");

    let rest = drain(&mut stream, Duration::from_secs(10)).await;
    let has_successful_tool_result = rest
        .iter()
        .any(|e| matches!(e, AgentEvent::ToolResult { success: true, .. }));
    assert!(
        !has_successful_tool_result,
        "reject must not produce a successful ToolResult, got: {rest:?}"
    );
}

#[tokio::test]
async fn approve_unknown_tool_id_returns_error() {
    let adapter = adapter_for("gated_edit");
    let session = adapter
        .spawn(&workspace(), "edit main.rs", &noop_context(), None, None)
        .await
        .expect("spawn");
    // We haven't pulled the gate event yet, so no permission is known.
    let err = session
        .approve("never-seen-this-id")
        .await
        .expect_err("approve of unknown tool must error");
    assert!(err.to_string().contains("no pending permission"));
}

// ─── cancel ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn cancel_during_slow_prompt_ends_stream_promptly() {
    let adapter = adapter_for("slow_prompt");
    let mut session = adapter
        .spawn(&workspace(), "take your time", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();

    // Pull at least one event so we know the stream is live (the fake emits
    // a ToolRequest to flush the translator's buffer).
    let _first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("first event arrives within 5s")
        .expect("non-empty");

    let start = std::time::Instant::now();
    session.cancel().await.expect("cancel");
    // Stream must terminate within a bounded window.
    while start.elapsed() < Duration::from_secs(10) {
        match tokio::time::timeout(Duration::from_secs(1), stream.next()).await {
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
    }
    assert!(
        start.elapsed() < Duration::from_secs(10),
        "stream did not end within 10s of cancel"
    );
}

// ─── resilience ──────────────────────────────────────────────────────────

#[tokio::test]
async fn crash_mid_prompt_ends_stream_without_panic() {
    let adapter = adapter_for("crash_mid_prompt");
    let mut session = adapter
        .spawn(&workspace(), "please crash", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();
    let events = drain(&mut stream, Duration::from_secs(10)).await;
    // Stream may end with nothing, a single Text, or an Error — any is
    // acceptable, so long as we don't panic and don't hang.
    // Final event must NOT be Complete (the agent crashed).
    assert!(
        !matches!(events.last(), Some(AgentEvent::Complete { .. })),
        "crashed agent must not produce Complete, got: {events:?}"
    );
}

#[tokio::test]
async fn spam_notifications_does_not_deadlock() {
    let adapter = adapter_for("spam_notifications");
    let mut session = adapter
        .spawn(&workspace(), "spam", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();
    let events = drain(&mut stream, Duration::from_secs(30)).await;
    assert!(
        matches!(events.last(), Some(AgentEvent::Complete { .. })),
        "spam_notifications must still complete, got {} events ending with: {:?}",
        events.len(),
        events.last()
    );
}

// ─── events() double-call ────────────────────────────────────────────────

#[tokio::test]
async fn events_called_twice_second_stream_is_empty() {
    let adapter = adapter_for("text_only");
    let mut session = adapter
        .spawn(&workspace(), "hi", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut first = session.events();
    let _ = drain(&mut first, Duration::from_secs(10)).await;
    let mut second = session.events();
    let more = drain(&mut second, Duration::from_millis(200)).await;
    assert!(more.is_empty(), "second events() must yield nothing");
}

// ─── resume ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn resume_with_known_session_id_succeeds() {
    let adapter = adapter_for("resume_existing");
    let mut session = adapter
        .resume(
            &workspace(),
            "sess-fake-abc",
            "continue the work",
            None,
            None,
        )
        .await
        .expect("resume");
    assert_eq!(session.init().session_id, "sess-fake-abc");
    let mut stream = session.events();
    let events = drain(&mut stream, Duration::from_secs(10)).await;
    assert!(matches!(events.last(), Some(AgentEvent::Complete { .. })));
}

#[tokio::test]
async fn resume_with_unknown_session_id_falls_back_to_new() {
    // text_only fake exits "session not found" on arbitrary load requests.
    let adapter = adapter_for("text_only");
    let mut session = adapter
        .resume(
            &workspace(),
            "not-a-real-session",
            "hi",
            None,
            None,
        )
        .await
        .expect("resume should fall back");
    // A fresh session_id must have been minted.
    assert_ne!(session.init().session_id, "not-a-real-session");
    assert!(session.init().session_id.starts_with("sess-fake-"));
    let mut stream = session.events();
    let events = drain(&mut stream, Duration::from_secs(10)).await;
    assert!(matches!(events.last(), Some(AgentEvent::Complete { .. })));
}

// ─── concurrent sessions ─────────────────────────────────────────────────

// ─── regression: non-gated tool flow (most common case) ─────────────────

#[tokio::test]
async fn non_gated_tool_call_produces_request_result_and_complete() {
    let adapter = adapter_for("non_gated_tool");
    let mut session = adapter
        .spawn(&workspace(), "read main.rs", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();
    let events = drain(&mut stream, Duration::from_secs(10)).await;

    let request = events
        .iter()
        .find(|e| matches!(e, AgentEvent::ToolRequest { needs_approval: false, .. }));
    assert!(request.is_some(), "expected non-gated ToolRequest, got: {events:?}");
    let result = events
        .iter()
        .find(|e| matches!(e, AgentEvent::ToolResult { success: true, .. }));
    assert!(result.is_some(), "expected successful ToolResult, got: {events:?}");
    assert!(matches!(events.last(), Some(AgentEvent::Complete { .. })));
}

// ─── regression: approve picks valid optionId from agent's list ──────────

#[tokio::test]
async fn approve_picks_allow_option_from_agent_offered_list() {
    // The fake offers options ["proceed_now":allow, "halt_execution":reject].
    // The adapter must pick "proceed_now" — NOT "allow_once" which is
    // kiro-cli's convention but is nowhere in this backend's options.
    let adapter = adapter_for("custom_option_ids");
    let mut session = adapter
        .spawn(&workspace(), "do it", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();

    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("stream produces")
        .expect("non-empty");
    let tool_id = match &first {
        AgentEvent::ToolRequest { id, needs_approval: true, .. } => id.clone(),
        other => panic!("expected gated ToolRequest, got {other:?}"),
    };

    // approve must succeed — the adapter must pick "proceed_now", not a
    // hardcoded "allow_once" that the fake agent would reject.
    session
        .approve(&tool_id)
        .await
        .expect("approve must succeed with backend-specific optionId");
}

// ─── regression: EOF flushes buffered text ──────────────────────────────

#[tokio::test]
async fn eof_mid_stream_flushes_buffered_text_before_closing() {
    let adapter = adapter_for("truncated_no_complete");
    let mut session = adapter
        .spawn(&workspace(), "hi", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();
    let events = drain(&mut stream, Duration::from_secs(15)).await;

    // The buffered "partial output" text must reach the user, even though
    // no Complete arrived.
    let saw_partial = events.iter().any(|e| match e {
        AgentEvent::Text { text } => text.contains("partial output"),
        _ => false,
    });
    assert!(
        saw_partial,
        "buffered text must be flushed on EOF, got: {events:?}"
    );
    assert!(
        !matches!(events.last(), Some(AgentEvent::Complete { .. })),
        "truncated stream must not produce Complete"
    );
}

// ─── regression: auth failure surfaces as typed Error ───────────────────

#[tokio::test]
async fn auth_failure_on_stderr_emits_non_recoverable_error() {
    // The fake writes "401 Unauthorized" to stderr on startup. The transport's
    // stderr watcher should flag that and the adapter should emit a
    // non-recoverable Error with the original message, not a generic timeout.
    let adapter = adapter_for("auth_failure");
    // This adapter's stdin/stdout still work — it only wrote to stderr — so
    // spawn should succeed. The auth error is observed only once we start
    // streaming.
    let spawn_result = tokio::time::timeout(
        Duration::from_secs(5),
        adapter.spawn(&workspace(), "hi", &noop_context(), None, None),
    )
    .await;
    // The fake is "text_only"-equivalent for every other scenario — spawn
    // proceeds normally. Now drain and look for the typed Error.
    let mut session = match spawn_result {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            // If spawn itself failed due to the auth error surfacing during
            // init, that's an acceptable outcome too.
            assert!(
                e.to_string().to_lowercase().contains("unauthor")
                    || e.to_string().to_lowercase().contains("auth")
                    || e.to_string().contains("401"),
                "spawn error should reference auth: {e}"
            );
            return;
        }
        Err(_) => panic!("spawn timed out — adapter should fail fast on auth"),
    };
    let mut stream = session.events();
    let events = drain(&mut stream, Duration::from_secs(10)).await;
    let saw_auth = events.iter().any(|e| match e {
        AgentEvent::Error { message, recoverable } => {
            !recoverable
                && (message.to_lowercase().contains("unauthor")
                    || message.contains("401")
                    || message.to_lowercase().contains("auth"))
        }
        _ => false,
    });
    assert!(
        saw_auth,
        "auth failure must surface as non-recoverable Error, got: {events:?}"
    );
}

// ─── regression: missing set_mode notification doesn't hang ─────────────

#[tokio::test]
async fn missing_commands_available_notification_does_not_hang_spawn() {
    // The fake swallows the readiness notification; adapter should time out
    // on set_mode gracefully and still be usable for subsequent prompts.
    let adapter = adapter_for("mode_notification_missing");
    let start = std::time::Instant::now();
    let _session = tokio::time::timeout(
        Duration::from_secs(15),
        adapter.spawn(&workspace(), "hi", &noop_context(), None, None),
    )
    .await
    .expect("spawn should not hang")
    .expect("spawn should succeed even without readiness notif");
    // In test builds SET_MODE_TIMEOUT is compressed to 200ms, so the full
    // spawn should complete well under 2s.
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "spawn must not hang waiting for a missing readiness notification"
    );
}

// ─── race conditions / idempotency ──────────────────────────────────────

#[tokio::test]
async fn approve_twice_for_same_tool_id_errors_on_second_call() {
    let adapter = adapter_for("gated_edit");
    let mut session = adapter
        .spawn(&workspace(), "edit main.rs", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();

    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("stream produces")
        .expect("non-empty");
    let tool_id = match &first {
        AgentEvent::ToolRequest { id, needs_approval: true, .. } => id.clone(),
        other => panic!("expected gated ToolRequest, got {other:?}"),
    };

    session.approve(&tool_id).await.expect("first approve");
    let err = session
        .approve(&tool_id)
        .await
        .expect_err("second approve for same id must error");
    assert!(
        err.to_string().contains("no pending permission"),
        "error should name the missing permission, got: {err}"
    );
}

#[tokio::test]
async fn reject_after_approve_errors_on_same_tool_id() {
    let adapter = adapter_for("gated_edit");
    let mut session = adapter
        .spawn(&workspace(), "edit main.rs", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();

    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("stream produces")
        .expect("non-empty");
    let tool_id = match &first {
        AgentEvent::ToolRequest { id, needs_approval: true, .. } => id.clone(),
        other => panic!("expected gated ToolRequest, got {other:?}"),
    };

    session.approve(&tool_id).await.expect("approve");
    let err = session
        .reject(&tool_id, "changed my mind")
        .await
        .expect_err("reject after approve must error");
    assert!(err.to_string().contains("no pending permission"));
}

#[tokio::test]
async fn cancel_twice_is_idempotent() {
    let adapter = adapter_for("slow_prompt");
    let session = adapter
        .spawn(&workspace(), "take your time", &noop_context(), None, None)
        .await
        .expect("spawn");
    session.cancel().await.expect("first cancel");
    // Second cancel must not panic or error — the transport is already shut
    // down but that should be treated as success.
    session.cancel().await.expect("second cancel must be idempotent");
}

#[tokio::test]
async fn approve_after_cancel_errors_without_panicking() {
    let adapter = adapter_for("gated_edit");
    let mut session = adapter
        .spawn(&workspace(), "edit main.rs", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();

    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("stream produces")
        .expect("non-empty");
    let tool_id = match &first {
        AgentEvent::ToolRequest { id, needs_approval: true, .. } => id.clone(),
        other => panic!("expected gated ToolRequest, got {other:?}"),
    };

    session.cancel().await.expect("cancel");
    // The transport is gone; approving must fail with a clear error, not
    // deadlock and not panic.
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        session.approve(&tool_id),
    )
    .await
    .expect("approve must not hang after cancel");
    assert!(result.is_err(), "approve after cancel must return Err");
}

#[tokio::test]
async fn concurrent_approve_and_cancel_do_not_deadlock() {
    // Issue approve and cancel simultaneously. Ordering is nondeterministic
    // — we just assert neither blocks forever and the session ends cleanly.
    let adapter = adapter_for("gated_edit");
    let mut session = adapter
        .spawn(&workspace(), "edit main.rs", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();

    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("stream produces")
        .expect("non-empty");
    let tool_id = match &first {
        AgentEvent::ToolRequest { id, needs_approval: true, .. } => id.clone(),
        other => panic!("expected gated ToolRequest, got {other:?}"),
    };

    let session = std::sync::Arc::new(session);
    let s1 = session.clone();
    let s2 = session.clone();
    let id1 = tool_id.clone();
    let (a, b) = tokio::join!(
        tokio::time::timeout(Duration::from_secs(5), async move { s1.approve(&id1).await }),
        tokio::time::timeout(Duration::from_secs(5), async move { s2.cancel().await }),
    );
    // Whatever the ordering, neither call hangs.
    assert!(a.is_ok(), "approve blocked longer than 5s");
    assert!(b.is_ok(), "cancel blocked longer than 5s");
}

// ─── multi-gate scenario (gap #6) ───────────────────────────────────────

#[tokio::test]
async fn two_sequential_gates_in_one_prompt_both_resolve_cleanly() {
    let adapter = adapter_for("two_gates");
    let mut session = adapter
        .spawn(&workspace(), "do two risky things", &noop_context(), None, None)
        .await
        .expect("spawn");
    let mut stream = session.events();

    // First gate.
    let g1 = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("first event")
        .expect("non-empty");
    let id1 = match &g1 {
        AgentEvent::ToolRequest { id, needs_approval: true, .. } => id.clone(),
        other => panic!("expected first gated ToolRequest, got {other:?}"),
    };
    session.approve(&id1).await.expect("approve first");

    // Consume events until the second gate arrives.
    let mut id2 = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline && id2.is_none() {
        match tokio::time::timeout(Duration::from_secs(2), stream.next()).await {
            Ok(Some(AgentEvent::ToolRequest { id, needs_approval: true, .. })) => {
                id2 = Some(id);
            }
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => break,
        }
    }
    let id2 = id2.expect("second gated ToolRequest should arrive");
    assert_ne!(id1, id2, "the two gates must have distinct tool_call_ids");

    session.approve(&id2).await.expect("approve second");

    let rest = drain(&mut stream, Duration::from_secs(10)).await;
    assert!(
        matches!(rest.last(), Some(AgentEvent::Complete { .. })),
        "expected Complete after two approvals, got: {rest:?}"
    );
}

// ─── spawn-failure paths (gap #3) ───────────────────────────────────────

#[tokio::test]
async fn spawn_on_nonexistent_binary_fails_fast_with_clear_error() {
    let adapter = panes_adapters::AcpAdapter::new(
        "ghost",
        "/nonexistent/path/to/kiro-cli-xyz",
        vec![],
    );
    let result = adapter
        .spawn(&workspace(), "hi", &noop_context(), None, None)
        .await;
    let err = match result {
        Ok(_) => panic!("spawning a ghost binary must error"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("spawn") || msg.to_lowercase().contains("not found") || msg.to_lowercase().contains("no such file"),
        "error should clearly explain spawn failure, got: {msg}"
    );
}

#[tokio::test]
async fn spawn_against_binary_that_exits_immediately_errors_during_handshake() {
    // `/bin/true` spawns fine but exits immediately with no stdio activity —
    // the initialize handshake must detect this and error rather than hang.
    let adapter = panes_adapters::AcpAdapter::new(
        "true-binary",
        "/usr/bin/true",
        vec![],
    );
    let result = tokio::time::timeout(
        Duration::from_secs(15),
        adapter.spawn(&workspace(), "hi", &noop_context(), None, None),
    )
    .await
    .expect("spawn must fail within 15s, not hang forever");
    // spawn returned — we need an Err, not an Ok(session).
    assert!(
        result.is_err(),
        "spawn against /bin/true must error — got Ok session somehow"
    );
}

#[tokio::test]
async fn two_concurrent_sessions_have_distinct_ids() {
    let adapter_a = adapter_for("text_only");
    let adapter_b = adapter_for("text_only");
    let ws = workspace();
    let ctx = noop_context();
    let fut_a = adapter_a.spawn(&ws, "hi", &ctx, None, None);
    let fut_b = adapter_b.spawn(&ws, "hi", &ctx, None, None);
    let (sa, sb) = tokio::join!(fut_a, fut_b);
    let sa = sa.expect("spawn a");
    let sb = sb.expect("spawn b");
    assert_ne!(
        sa.init().session_id,
        sb.init().session_id,
        "concurrent sessions must have distinct ids"
    );
}
