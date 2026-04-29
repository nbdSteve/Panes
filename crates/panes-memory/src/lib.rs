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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlite_store::SqliteMemoryStore;

    #[tokio::test]
    async fn test_build_context_with_briefing_and_memories() {
        let store = SqliteMemoryStore::new(":memory:").unwrap();
        store.set_briefing("ws1", "Always use TypeScript").await.unwrap();
        store.add("Use pnpm for packages", Some("ws1"), "t1").await.unwrap();

        let ctx = build_context(&store, &store, "pnpm packages", "ws1", 2000)
            .await
            .unwrap();

        assert_eq!(ctx.briefing.as_deref(), Some("Always use TypeScript"));
        assert!(!ctx.memories.is_empty());
    }

    #[tokio::test]
    async fn test_build_context_no_briefing() {
        let store = SqliteMemoryStore::new(":memory:").unwrap();
        store.add("some memory", Some("ws1"), "t1").await.unwrap();

        let ctx = build_context(&store, &store, "query", "ws1", 2000)
            .await
            .unwrap();

        assert!(ctx.briefing.is_none());
    }

    #[tokio::test]
    async fn test_build_context_respects_token_budget() {
        let store = SqliteMemoryStore::new(":memory:").unwrap();
        // Each memory is ~50 chars → ~12 tokens
        for i in 0..20 {
            store.add(&format!("memory number {i} with some padding text here"), Some("ws1"), &format!("t{i}")).await.unwrap();
        }

        let ctx = build_context(&store, &store, "memory", "ws1", 50)
            .await
            .unwrap();

        assert!(ctx.token_estimate <= 50, "should respect budget, got {}", ctx.token_estimate);
    }

    #[tokio::test]
    async fn test_build_context_deduplicates() {
        let store = SqliteMemoryStore::new(":memory:").unwrap();
        store.add("unique memory content", Some("ws1"), "t1").await.unwrap();

        let ctx = build_context(&store, &store, "unique memory", "ws1", 2000)
            .await
            .unwrap();

        let ids: Vec<_> = ctx.memories.iter().map(|m| &m.id).collect();
        let unique_ids: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique_ids.len(), "no duplicate memories");
    }

    #[tokio::test]
    async fn test_build_context_pinned_always_included() {
        let store = SqliteMemoryStore::new(":memory:").unwrap();
        let mems = store.add("pinned important fact", Some("ws1"), "t1").await.unwrap();
        store.pin(&mems[0].id, true).await.unwrap();

        let ctx = build_context(&store, &store, "unrelated query", "ws1", 2000)
            .await
            .unwrap();

        let _ = ctx;
    }
}
