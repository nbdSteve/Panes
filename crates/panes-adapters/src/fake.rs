use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use futures::Stream;
use panes_events::{AgentEvent, RiskLevel, SessionContext, SessionInit};
use uuid::Uuid;

use crate::{AgentAdapter, AgentSession};

#[derive(Debug, Clone)]
pub enum FakeScenario {
    /// Simple text response, no tools
    TextOnly {
        response: String,
    },
    /// Reads some files, responds with text
    ReadAndRespond {
        files: Vec<String>,
        response: String,
    },
    /// Edits files — triggers commit/revert buttons
    FileEdit {
        files: Vec<String>,
        response: String,
    },
    /// Hits a gate that needs approval, then completes
    GatedAction {
        tool_name: String,
        description: String,
        risk_level: RiskLevel,
        response: String,
    },
    /// Multiple turns of tool use
    MultiStep {
        steps: Vec<FakeStep>,
        response: String,
    },
    /// Immediate error
    Error {
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct FakeStep {
    pub tool_name: String,
    pub description: String,
    pub risk_level: RiskLevel,
    pub needs_approval: bool,
    pub success: bool,
    pub output: String,
}

pub struct FakeAdapter {
    scenario: FakeScenario,
    delay_ms: u64,
}

impl FakeAdapter {
    pub fn new(scenario: FakeScenario) -> Self {
        Self {
            scenario,
            delay_ms: 100,
        }
    }

    pub fn with_delay(mut self, ms: u64) -> Self {
        self.delay_ms = ms;
        self
    }
}

#[async_trait]
impl AgentAdapter for FakeAdapter {
    fn name(&self) -> &str {
        "fake"
    }

    async fn spawn(
        &self,
        _workspace_path: &Path,
        _prompt: &str,
        _context: &SessionContext,
    ) -> Result<Box<dyn AgentSession>> {
        let session_id = Uuid::new_v4().to_string();
        let init = SessionInit {
            session_id,
            model: "fake-model".to_string(),
            cwd: "/fake".to_string(),
            tools: vec![
                "Read".into(), "Write".into(), "Edit".into(),
                "Bash".into(), "WebSearch".into(),
            ],
        };

        let events = build_events(&self.scenario);

        Ok(Box::new(FakeSession {
            init_data: init,
            events: tokio::sync::Mutex::new(Some(events)),
            delay_ms: self.delay_ms,
        }))
    }

    async fn resume(
        &self,
        workspace_path: &Path,
        _session_id: &str,
        prompt: &str,
    ) -> Result<Box<dyn AgentSession>> {
        self.spawn(
            workspace_path,
            prompt,
            &SessionContext { briefing: None, memories: vec![], budget_cap: None },
        ).await
    }
}

struct FakeSession {
    init_data: SessionInit,
    events: tokio::sync::Mutex<Option<Vec<AgentEvent>>>,
    delay_ms: u64,
}

#[async_trait]
impl AgentSession for FakeSession {
    fn init(&self) -> &SessionInit {
        &self.init_data
    }

    fn events(&mut self) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send + '_>> {
        let events = self.events.get_mut().take().unwrap_or_default();
        let delay = self.delay_ms;

        Box::pin(async_stream::stream! {
            for event in events {
                tokio::time::sleep(Duration::from_millis(delay)).await;
                yield event;
            }
        })
    }

    async fn approve(&self, _tool_use_id: &str) -> Result<()> {
        Ok(())
    }

    async fn reject(&self, _tool_use_id: &str, _reason: &str) -> Result<()> {
        Ok(())
    }

    async fn cancel(&self) -> Result<()> {
        Ok(())
    }
}

fn build_events(scenario: &FakeScenario) -> Vec<AgentEvent> {
    let mut events = vec![];

    match scenario {
        FakeScenario::TextOnly { response } => {
            events.push(AgentEvent::Thinking {
                text: "Let me think about this...".to_string(),
            });
            events.push(AgentEvent::CostUpdate {
                input_tokens: 1500,
                output_tokens: 200,
                cache_read_tokens: 500,
                cache_creation_tokens: 0,
                total_usd: 0.003,
                model: "fake-model".to_string(),
            });
            events.push(AgentEvent::Text {
                text: response.clone(),
            });
            events.push(AgentEvent::Complete {
                summary: response.clone(),
                total_cost_usd: 0.003,
                duration_ms: 2500,
                turns: 1,
            });
        }

        FakeScenario::ReadAndRespond { files, response } => {
            events.push(AgentEvent::Thinking {
                text: "I'll read the relevant files first.".to_string(),
            });
            for (i, file) in files.iter().enumerate() {
                let id = format!("tool_{i}");
                events.push(AgentEvent::ToolRequest {
                    id: id.clone(),
                    tool_name: "Read".to_string(),
                    description: format!("Read file: {file}"),
                    input: serde_json::json!({"file_path": file}),
                    needs_approval: false,
                    risk_level: RiskLevel::Low,
                });
                events.push(AgentEvent::ToolResult {
                    id,
                    tool_name: "Read".to_string(),
                    success: true,
                    output: format!("(contents of {file})"),
                    raw_output: None,
                    duration_ms: 50,
                });
            }
            events.push(AgentEvent::CostUpdate {
                input_tokens: 5000,
                output_tokens: 800,
                cache_read_tokens: 2000,
                cache_creation_tokens: 0,
                total_usd: 0.012,
                model: "fake-model".to_string(),
            });
            events.push(AgentEvent::Text {
                text: response.clone(),
            });
            events.push(AgentEvent::Complete {
                summary: response.clone(),
                total_cost_usd: 0.012,
                duration_ms: 5000,
                turns: 2,
            });
        }

        FakeScenario::FileEdit { files, response } => {
            events.push(AgentEvent::Thinking {
                text: "I'll make the requested changes.".to_string(),
            });
            for (i, file) in files.iter().enumerate() {
                let id = format!("tool_{i}");
                events.push(AgentEvent::ToolRequest {
                    id: id.clone(),
                    tool_name: "Edit".to_string(),
                    description: format!("Edit file: {file}"),
                    input: serde_json::json!({"file_path": file, "old_string": "old", "new_string": "new"}),
                    needs_approval: false,
                    risk_level: RiskLevel::Medium,
                });
                events.push(AgentEvent::ToolResult {
                    id,
                    tool_name: "Edit".to_string(),
                    success: true,
                    output: "File edited successfully".to_string(),
                    raw_output: None,
                    duration_ms: 100,
                });
            }
            events.push(AgentEvent::CostUpdate {
                input_tokens: 8000,
                output_tokens: 1200,
                cache_read_tokens: 3000,
                cache_creation_tokens: 0,
                total_usd: 0.025,
                model: "fake-model".to_string(),
            });
            events.push(AgentEvent::Text {
                text: response.clone(),
            });
            events.push(AgentEvent::Complete {
                summary: response.clone(),
                total_cost_usd: 0.025,
                duration_ms: 8000,
                turns: 3,
            });
        }

        FakeScenario::GatedAction { tool_name, description, risk_level, response } => {
            events.push(AgentEvent::Thinking {
                text: "This requires a potentially risky operation.".to_string(),
            });
            let id = "gate_0".to_string();
            events.push(AgentEvent::ToolRequest {
                id: id.clone(),
                tool_name: tool_name.clone(),
                description: description.clone(),
                input: serde_json::json!({"command": description}),
                needs_approval: true,
                risk_level: *risk_level,
            });
            events.push(AgentEvent::ToolResult {
                id,
                tool_name: tool_name.clone(),
                success: true,
                output: "Command executed successfully".to_string(),
                raw_output: None,
                duration_ms: 3000,
            });
            events.push(AgentEvent::CostUpdate {
                input_tokens: 6000,
                output_tokens: 500,
                cache_read_tokens: 1000,
                cache_creation_tokens: 0,
                total_usd: 0.018,
                model: "fake-model".to_string(),
            });
            events.push(AgentEvent::Text {
                text: response.clone(),
            });
            events.push(AgentEvent::Complete {
                summary: response.clone(),
                total_cost_usd: 0.018,
                duration_ms: 12000,
                turns: 2,
            });
        }

        FakeScenario::MultiStep { steps, response } => {
            events.push(AgentEvent::Thinking {
                text: "I'll work through this step by step.".to_string(),
            });
            for (i, step) in steps.iter().enumerate() {
                let id = format!("tool_{i}");
                events.push(AgentEvent::ToolRequest {
                    id: id.clone(),
                    tool_name: step.tool_name.clone(),
                    description: step.description.clone(),
                    input: serde_json::json!({"description": step.description}),
                    needs_approval: step.needs_approval,
                    risk_level: step.risk_level,
                });
                events.push(AgentEvent::ToolResult {
                    id,
                    tool_name: step.tool_name.clone(),
                    success: step.success,
                    output: step.output.clone(),
                    raw_output: None,
                    duration_ms: 200,
                });
                events.push(AgentEvent::CostUpdate {
                    input_tokens: 2000 * (i as u64 + 1),
                    output_tokens: 300 * (i as u64 + 1),
                    cache_read_tokens: 800 * (i as u64 + 1),
                    cache_creation_tokens: 0,
                    total_usd: 0.005 * (i as f64 + 1.0),
                    model: "fake-model".to_string(),
                });
            }
            events.push(AgentEvent::Text {
                text: response.clone(),
            });
            events.push(AgentEvent::Complete {
                summary: response.clone(),
                total_cost_usd: 0.005 * steps.len() as f64,
                duration_ms: 3000 * steps.len() as u64,
                turns: steps.len() as u32 + 1,
            });
        }

        FakeScenario::Error { message } => {
            events.push(AgentEvent::Thinking {
                text: "Let me try...".to_string(),
            });
            events.push(AgentEvent::Error {
                message: message.clone(),
                recoverable: false,
            });
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    #[tokio::test]
    async fn test_fake_text_only() {
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hello!".to_string(),
        }).with_delay(0);

        let workspace = Path::new("/tmp");
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let mut session = adapter.spawn(workspace, "test", &ctx).await.unwrap();

        assert_eq!(session.init().model, "fake-model");

        let mut events: Vec<AgentEvent> = vec![];
        let mut stream = session.events();
        while let Some(ev) = stream.next().await {
            events.push(ev);
        }

        assert!(events.len() == 4); // Thinking, CostUpdate, Text, Complete
        assert!(matches!(&events[0], AgentEvent::Thinking { .. }));
        assert!(matches!(&events[2], AgentEvent::Text { text } if text == "Hello!"));
        assert!(matches!(&events[3], AgentEvent::Complete { .. }));
    }

    #[tokio::test]
    async fn test_fake_gated_action() {
        let adapter = FakeAdapter::new(FakeScenario::GatedAction {
            tool_name: "Bash".to_string(),
            description: "rm -rf /tmp/test".to_string(),
            risk_level: RiskLevel::Critical,
            response: "Done.".to_string(),
        }).with_delay(0);

        let workspace = Path::new("/tmp");
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let mut session = adapter.spawn(workspace, "test", &ctx).await.unwrap();

        let mut events: Vec<AgentEvent> = vec![];
        let mut stream = session.events();
        while let Some(ev) = stream.next().await {
            events.push(ev);
        }

        let gate = events.iter().find(|e| matches!(e, AgentEvent::ToolRequest { needs_approval: true, .. }));
        assert!(gate.is_some());
        if let Some(AgentEvent::ToolRequest { risk_level, .. }) = gate {
            assert_eq!(*risk_level, RiskLevel::Critical);
        }
    }

    #[tokio::test]
    async fn test_fake_error() {
        let adapter = FakeAdapter::new(FakeScenario::Error {
            message: "Auth failed".to_string(),
        }).with_delay(0);

        let workspace = Path::new("/tmp");
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let mut session = adapter.spawn(workspace, "test", &ctx).await.unwrap();

        let mut events: Vec<AgentEvent> = vec![];
        let mut stream = session.events();
        while let Some(ev) = stream.next().await {
            events.push(ev);
        }

        assert!(events.iter().any(|e| matches!(e, AgentEvent::Error { message, .. } if message == "Auth failed")));
    }

    #[tokio::test]
    async fn test_fake_resume() {
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Resumed!".to_string(),
        }).with_delay(0);

        let workspace = Path::new("/tmp");
        let mut session = adapter.resume(workspace, "some-session-id", "follow up").await.unwrap();
        assert!(!session.init().session_id.is_empty());

        let mut stream = session.events();
        let mut got_complete = false;
        while let Some(ev) = stream.next().await {
            if matches!(ev, AgentEvent::Complete { .. }) {
                got_complete = true;
            }
        }
        assert!(got_complete);
    }
}
