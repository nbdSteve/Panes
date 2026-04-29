use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use crate::types::{Memory, MemoryType};
use crate::MemoryStore;

const GLOBAL_USER_ID: &str = "__global__";

pub struct Mem0Store {
    client: Client,
    base_url: String,
}

impl Mem0Store {
    pub fn new(base_url: &str) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }
}

#[derive(Serialize)]
struct AddRequest {
    transcript: String,
    user_id: String,
    thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<String>,
}

#[derive(Serialize)]
struct SearchRequest {
    query: String,
    user_id: String,
    limit: usize,
}

#[derive(Deserialize)]
struct Mem0Memory {
    id: String,
    memory: String,
    #[serde(default)]
    metadata: serde_json::Value,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

#[derive(Serialize)]
struct UpdateRequest {
    data: String,
}

fn to_user_id(workspace_id: Option<&str>) -> String {
    workspace_id
        .map(|w| format!("ws:{w}"))
        .unwrap_or_else(|| GLOBAL_USER_ID.to_string())
}

fn mem0_to_memory(m: Mem0Memory, workspace_id: Option<&str>) -> Memory {
    let memory_type = guess_memory_type(&m.memory);
    Memory {
        id: m.id,
        workspace_id: workspace_id.map(String::from),
        memory_type,
        content: m.memory,
        source_thread_id: m
            .metadata
            .get("thread_id")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        created_at: m
            .created_at
            .and_then(|s| s.parse().ok())
            .unwrap_or_else(Utc::now),
        edited_at: m.updated_at.and_then(|s| s.parse().ok()),
        pinned: false,
    }
}

fn guess_memory_type(content: &str) -> MemoryType {
    let lower = content.to_lowercase();
    if lower.contains("decided") || lower.contains("chose") || lower.contains("decision") {
        MemoryType::Decision
    } else if lower.contains("prefer") || lower.contains("always use") || lower.contains("never use")
    {
        MemoryType::Preference
    } else if lower.contains("must") || lower.contains("require") || lower.contains("constraint") {
        MemoryType::Constraint
    } else {
        MemoryType::Pattern
    }
}

#[async_trait]
impl MemoryStore for Mem0Store {
    async fn add(
        &self,
        transcript: &str,
        workspace_id: Option<&str>,
        thread_id: &str,
    ) -> Result<Vec<Memory>> {
        let user_id = to_user_id(workspace_id);

        let req = AddRequest {
            transcript: transcript.to_string(),
            user_id: user_id.clone(),
            thread_id: thread_id.to_string(),
            workspace_id: workspace_id.map(String::from),
        };

        let resp = self
            .client
            .post(format!("{}/v1/memories/", self.base_url))
            .json(&req)
            .send()
            .await
            .context("failed to call Mem0 /add")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Mem0 /add failed with {status}: {body}");
        }

        let result: serde_json::Value = resp.json().await?;
        let memories: Vec<Memory> = result
            .get("results")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value::<Mem0Memory>(v.clone()).ok())
                    .map(|m| mem0_to_memory(m, workspace_id))
                    .collect()
            })
            .unwrap_or_default();

        debug!(count = memories.len(), "mem0 extracted memories");
        Ok(memories)
    }

    async fn search(
        &self,
        query: &str,
        workspace_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Memory>> {
        let user_id = to_user_id(workspace_id);

        let req = SearchRequest {
            query: query.to_string(),
            user_id,
            limit,
        };

        let resp = self
            .client
            .post(format!("{}/v1/memories/search/", self.base_url))
            .json(&req)
            .send()
            .await
            .context("failed to call Mem0 /search")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Mem0 /search failed with {status}: {body}");
        }

        let result: serde_json::Value = resp.json().await?;
        let memories = result
            .get("results")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value::<Mem0Memory>(v.clone()).ok())
                    .map(|m| mem0_to_memory(m, workspace_id))
                    .collect()
            })
            .unwrap_or_default();

        Ok(memories)
    }

    async fn get_all(&self, workspace_id: Option<&str>) -> Result<Vec<Memory>> {
        let user_id = to_user_id(workspace_id);

        let resp = self
            .client
            .get(format!("{}/v1/memories/", self.base_url))
            .query(&[("user_id", user_id.as_str())])
            .send()
            .await
            .context("failed to call Mem0 /memories")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Mem0 /memories failed with {status}: {body}");
        }

        let result: serde_json::Value = resp.json().await?;
        let memories = result
            .get("results")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value::<Mem0Memory>(v.clone()).ok())
                    .map(|m| mem0_to_memory(m, workspace_id))
                    .collect()
            })
            .unwrap_or_default();

        Ok(memories)
    }

    async fn update(&self, id: &str, content: &str) -> Result<()> {
        let req = UpdateRequest {
            data: content.to_string(),
        };

        let resp = self
            .client
            .put(format!("{}/v1/memories/{id}/", self.base_url))
            .json(&req)
            .send()
            .await
            .context("failed to call Mem0 update")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Mem0 update failed with {status}: {body}");
        }

        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let resp = self
            .client
            .delete(format!("{}/v1/memories/{id}/", self.base_url))
            .send()
            .await
            .context("failed to call Mem0 delete")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("Mem0 delete failed with {status}: {body}");
        }

        Ok(())
    }

    async fn pin(&self, id: &str, pinned: bool) -> Result<()> {
        // Mem0 doesn't have native pinning — store pin state in local SQLite
        // TODO: implement pin tracking in a local overlay table
        warn!(id, pinned, "mem0 pinning not yet implemented");
        Ok(())
    }

    async fn health_check(&self) -> Result<bool> {
        let resp = self
            .client
            .get(format!("{}/health", self.base_url))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await;

        match resp {
            Ok(r) => Ok(r.status().is_success()),
            Err(e) => {
                debug!(error = %e, "mem0 health check failed");
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_user_id_workspace() {
        assert_eq!(to_user_id(Some("my-workspace")), "ws:my-workspace");
    }

    #[test]
    fn test_to_user_id_global() {
        assert_eq!(to_user_id(None), "__global__");
    }

    #[test]
    fn test_guess_memory_type_decision() {
        assert_eq!(guess_memory_type("We decided to use React"), MemoryType::Decision);
        assert_eq!(guess_memory_type("Team chose TypeScript"), MemoryType::Decision);
    }

    #[test]
    fn test_guess_memory_type_preference() {
        assert_eq!(guess_memory_type("User prefers tabs"), MemoryType::Preference);
        assert_eq!(guess_memory_type("Always use pnpm"), MemoryType::Preference);
        assert_eq!(guess_memory_type("Never use npm"), MemoryType::Preference);
    }

    #[test]
    fn test_guess_memory_type_constraint() {
        assert_eq!(guess_memory_type("API must be backwards compatible"), MemoryType::Constraint);
        assert_eq!(guess_memory_type("Require strict mode"), MemoryType::Constraint);
    }

    #[test]
    fn test_guess_memory_type_pattern_default() {
        assert_eq!(guess_memory_type("Uses React with hooks"), MemoryType::Pattern);
    }

    #[test]
    fn test_mem0_to_memory_basic() {
        let m = Mem0Memory {
            id: "abc".to_string(),
            memory: "User prefers pnpm".to_string(),
            metadata: serde_json::json!({"thread_id": "t1"}),
            created_at: Some("2026-01-01T00:00:00Z".to_string()),
            updated_at: None,
        };
        let result = mem0_to_memory(m, Some("ws1"));
        assert_eq!(result.id, "abc");
        assert_eq!(result.content, "User prefers pnpm");
        assert_eq!(result.workspace_id.as_deref(), Some("ws1"));
        assert_eq!(result.source_thread_id, "t1");
        assert_eq!(result.memory_type, MemoryType::Preference);
        assert!(!result.pinned);
    }

    #[test]
    fn test_mem0_to_memory_global() {
        let m = Mem0Memory {
            id: "def".to_string(),
            memory: "Some pattern".to_string(),
            metadata: serde_json::json!({}),
            created_at: None,
            updated_at: None,
        };
        let result = mem0_to_memory(m, None);
        assert!(result.workspace_id.is_none());
        assert_eq!(result.source_thread_id, "");
    }

    #[tokio::test]
    async fn test_health_check_no_server() {
        let store = Mem0Store::new("http://127.0.0.1:19999");
        let healthy = store.health_check().await.unwrap();
        assert!(!healthy, "should be unhealthy when no server running");
    }
}
