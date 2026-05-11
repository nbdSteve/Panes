pub mod acp;
pub mod claude;
pub mod fake;

pub use acp::{AcpAdapter, replay_messages_for_tests};

use std::path::Path;
use std::pin::Pin;

use anyhow::Result;
use async_trait::async_trait;
use futures::Stream;
use panes_events::{AgentEvent, SessionContext, SessionInit};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    pub label: String,
    pub description: String,
}

/// One agent/mode/subagent surface an adapter exposes to the picker.
/// Distinct from `ModelInfo` — an `AgentInfo` selects the *behavior profile*
/// (claude-code subagent, kiro-cli mode), not the underlying LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub name: String,
    pub model: Option<String>,
    pub description: Option<String>,
}

#[async_trait]
pub trait AgentAdapter: Send + Sync {
    fn name(&self) -> &str;

    async fn spawn(
        &self,
        workspace_path: &Path,
        prompt: &str,
        context: &SessionContext,
        model: Option<&str>,
        agent: Option<&str>,
    ) -> Result<Box<dyn AgentSession>>;

    async fn resume(
        &self,
        workspace_path: &Path,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
        agent: Option<&str>,
    ) -> Result<Box<dyn AgentSession>>;

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        Ok(vec![])
    }

    /// Agent/mode/subagent options shown in the picker. Default empty so
    /// adapters that don't have a concept of sub-agents don't need to
    /// override. ACP adapters populate this from their discovered-metadata
    /// cache; the Claude adapter scans `~/.claude/agents`.
    async fn list_agents(&self) -> Result<Vec<AgentInfo>> {
        Ok(vec![])
    }
}

#[async_trait]
pub trait AgentSession: Send + Sync {
    fn init(&self) -> &SessionInit;

    /// Must only be called once. Behavior on second call is adapter-dependent.
    fn events(&mut self) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>>;

    async fn approve(&self, tool_use_id: &str) -> Result<()>;

    async fn reject(&self, tool_use_id: &str, reason: &str) -> Result<()>;

    async fn cancel(&self) -> Result<()>;

    /// Switch the active model on a live session without restarting. Adapters
    /// that can't do this mid-flight (e.g. Claude's stream-json has `--model`
    /// only at spawn) should return `Err` with a clear message; the session
    /// manager surfaces that to the UI so the user sees why it didn't change.
    ///
    /// Default returns "unsupported" so adapters opt in by overriding.
    async fn set_model(&self, _model: &str) -> Result<()> {
        Err(anyhow::anyhow!(
            "this adapter does not support live model switching — start a new thread to change models"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_info_serialization() {
        let info = ModelInfo {
            id: "sonnet".to_string(),
            label: "Sonnet".to_string(),
            description: "Fast & capable".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"id\""));
        assert!(json.contains("\"label\""));
        assert!(json.contains("\"description\""));
        assert!(!json.contains("_"), "should use camelCase, not snake_case");
    }

    #[test]
    fn test_model_info_deserialization() {
        let json = r#"{"id":"opus","label":"Opus","description":"Most capable"}"#;
        let info: ModelInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.id, "opus");
        assert_eq!(info.label, "Opus");
    }
}
