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
    messages: Vec<Message>,
    user_id: String,
    metadata: AddMetadata,
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct AddMetadata {
    thread_id: String,
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
            messages: vec![Message {
                role: "user".to_string(),
                content: transcript.to_string(),
            }],
            user_id: user_id.clone(),
            metadata: AddMetadata {
                thread_id: thread_id.to_string(),
                workspace_id: workspace_id.map(String::from),
            },
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
            .query(&[("user_id", &user_id)])
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
