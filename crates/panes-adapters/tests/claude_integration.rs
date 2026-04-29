use std::path::Path;

use futures::StreamExt;
use panes_adapters::claude::ClaudeAdapter;
use panes_adapters::AgentAdapter;
use panes_events::{AgentEvent, SessionContext};

#[tokio::test]
async fn test_claude_adapter_end_to_end() {
    let adapter = ClaudeAdapter::with_cli_path("/Users/goodhill/.local/bin/claude")
        .env("CLAUDE_CODE_USE_BEDROCK", "1")
        .env("AWS_PROFILE", "bedrock-beta")
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", std::env::var("HOME").unwrap_or_default());

    let workspace = Path::new("/Users/goodhill/workplace");
    let context = SessionContext {
        briefing: None,
        memories: vec![],
        budget_cap: None,
    };

    eprintln!("=== Spawning claude adapter ===");
    let mut session = adapter
        .spawn(workspace, "respond with only the word 'pong'. nothing else.", &context)
        .await
        .expect("failed to spawn session");

    eprintln!("=== Got session: model={}, session_id={} ===", session.init().model, session.init().session_id);
    assert!(!session.init().session_id.is_empty());
    assert!(!session.init().model.is_empty());

    let mut events_stream = session.events();
    let mut got_complete = false;
    let mut event_count = 0;

    while let Some(event) = events_stream.next().await {
        event_count += 1;
        eprintln!("=== Event {event_count}: {event:?}");

        match &event {
            AgentEvent::Complete { summary, total_cost_usd, .. } => {
                got_complete = true;
                eprintln!("  COMPLETE: cost=${total_cost_usd}, summary={summary}");
                break;
            }
            AgentEvent::Error { message, recoverable } => {
                if !recoverable {
                    panic!("fatal error from adapter: {message}");
                }
            }
            _ => {}
        }
    }

    eprintln!("=== Test done: {event_count} events, got_complete={got_complete} ===");
    assert!(got_complete, "should have received a Complete event");
    assert!(event_count > 1, "should have received multiple events");
}
