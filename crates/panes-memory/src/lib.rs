pub mod mem0_store;
pub mod sidecar;
pub mod sqlite_store;
pub mod types;

use anyhow::Result;
use async_trait::async_trait;
use types::{Briefing, InjectedContext, Memory};

#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Extract and store memories from a thread transcript.
    /// The implementation handles extraction logic (Mem0 does it natively, SQLite uses LLM).
    async fn add(
        &self,
        transcript: &str,
        workspace_id: Option<&str>,
        thread_id: &str,
    ) -> Result<Vec<Memory>>;

    /// Search for relevant memories given a query.
    async fn search(
        &self,
        query: &str,
        workspace_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Memory>>;

    /// Get all memories for a workspace (or global if None).
    async fn get_all(&self, workspace_id: Option<&str>) -> Result<Vec<Memory>>;

    /// Update a memory's content.
    async fn update(&self, id: &str, content: &str) -> Result<()>;

    /// Delete a memory.
    async fn delete(&self, id: &str) -> Result<()>;

    /// Pin or unpin a memory.
    async fn pin(&self, id: &str, pinned: bool) -> Result<()>;

    /// Health check — is this store operational?
    async fn health_check(&self) -> Result<bool>;
}

/// Briefing store is always SQLite, regardless of memory backend.
#[async_trait]
pub trait BriefingStore: Send + Sync {
    async fn get_briefing(&self, workspace_id: &str) -> Result<Option<Briefing>>;
    async fn set_briefing(&self, workspace_id: &str, content: &str) -> Result<()>;
    async fn delete_briefing(&self, workspace_id: &str) -> Result<()>;
}

/// Build the injected context for a new thread.
pub async fn build_context(
    memory_store: &dyn MemoryStore,
    briefing_store: &dyn BriefingStore,
    prompt: &str,
    workspace_id: &str,
    token_budget: usize,
) -> Result<InjectedContext> {
    let briefing = briefing_store.get_briefing(workspace_id).await?;

    // Search workspace-scoped memories
    let ws_memories = memory_store
        .search(prompt, Some(workspace_id), 10)
        .await
        .unwrap_or_default();

    // Search global memories
    let global_memories = memory_store
        .search(prompt, None, 5)
        .await
        .unwrap_or_default();

    // Merge, dedup by content
    let mut seen = std::collections::HashSet::new();
    let mut memories = Vec::new();

    // Pinned memories always included first
    for m in ws_memories.iter().chain(global_memories.iter()) {
        if m.pinned && seen.insert(m.id.clone()) {
            memories.push(m.clone());
        }
    }

    // Then ranked memories within budget
    let mut token_count = memories
        .iter()
        .map(|m| m.content.len() / 4)
        .sum::<usize>();

    for m in ws_memories.iter().chain(global_memories.iter()) {
        if !m.pinned && seen.insert(m.id.clone()) {
            let est_tokens = m.content.len() / 4;
            if token_count + est_tokens <= token_budget {
                token_count += est_tokens;
                memories.push(m.clone());
            }
        }
    }

    Ok(InjectedContext {
        briefing: briefing.map(|b| b.content),
        memories,
        token_estimate: token_count,
    })
}
