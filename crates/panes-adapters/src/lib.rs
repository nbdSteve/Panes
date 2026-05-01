pub mod claude;
pub mod fake;

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

#[async_trait]
pub trait AgentAdapter: Send + Sync {
    fn name(&self) -> &str;

    async fn spawn(
        &self,
        workspace_path: &Path,
        prompt: &str,
        context: &SessionContext,
        model: Option<&str>,
    ) -> Result<Box<dyn AgentSession>>;

    async fn resume(
        &self,
        workspace_path: &Path,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
    ) -> Result<Box<dyn AgentSession>>;

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
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
