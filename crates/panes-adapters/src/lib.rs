pub mod claude;
pub mod fake;

use std::path::Path;
use std::pin::Pin;

use anyhow::Result;
use async_trait::async_trait;
use futures::Stream;
use panes_events::{AgentEvent, SessionContext, SessionInit};

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
