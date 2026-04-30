use std::collections::HashSet;
use std::sync::Mutex;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use reqwest::Client;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tracing::debug;
use crate::types::{Memory, MemoryType};
use crate::MemoryStore;

const GLOBAL_USER_ID: &str = "__global__";

pub struct Mem0Store {
    client: Client,
    base_url: String,
    pin_db: Mutex<Connection>,
}

impl Mem0Store {
    pub fn new(base_url: &str) -> Self {
        Self::with_pin_db(base_url, ":memory:")
    }

    pub fn with_pin_db(base_url: &str, pin_db_path: &str) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .unwrap_or_else(|_| Client::new());
        let conn = Connection::open(pin_db_path).expect("failed to open pin overlay db");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pinned_memories (
                memory_id TEXT PRIMARY KEY
            )",
        )
        .expect("failed to create pinned_memories table");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            pin_db: Mutex::new(conn),
        }
    }

    fn pinned_ids(&self) -> HashSet<String> {
        let conn = self.pin_db.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT memory_id FROM pinned_memories")
            .unwrap();
        stmt.query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    fn apply_pin_state(&self, memories: &mut [Memory]) {
        let pinned = self.pinned_ids();
        for m in memories.iter_mut() {
            m.pinned = pinned.contains(&m.id);
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
        let mut memories: Vec<Memory> = result
            .get("results")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value::<Mem0Memory>(v.clone()).ok())
                    .map(|m| mem0_to_memory(m, workspace_id))
                    .collect()
            })
            .unwrap_or_default();

        self.apply_pin_state(&mut memories);
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
        let mut memories: Vec<Memory> = result
            .get("results")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value::<Mem0Memory>(v.clone()).ok())
                    .map(|m| mem0_to_memory(m, workspace_id))
                    .collect()
            })
            .unwrap_or_default();

        self.apply_pin_state(&mut memories);
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
        let mut memories: Vec<Memory> = result
            .get("results")
            .and_then(|r| r.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value::<Mem0Memory>(v.clone()).ok())
                    .map(|m| mem0_to_memory(m, workspace_id))
                    .collect()
            })
            .unwrap_or_default();

        self.apply_pin_state(&mut memories);
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

        let conn = self.pin_db.lock().unwrap();
        let _ = conn.execute("DELETE FROM pinned_memories WHERE memory_id = ?1", rusqlite::params![id]);

        Ok(())
    }

    async fn pin(&self, id: &str, pinned: bool) -> Result<()> {
        let conn = self.pin_db.lock().unwrap();
        if pinned {
            conn.execute(
                "INSERT OR IGNORE INTO pinned_memories (memory_id) VALUES (?1)",
                rusqlite::params![id],
            )?;
        } else {
            conn.execute(
                "DELETE FROM pinned_memories WHERE memory_id = ?1",
                rusqlite::params![id],
            )?;
        }
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

    #[tokio::test]
    async fn test_pin_and_unpin() {
        let store = Mem0Store::new("http://127.0.0.1:19999");
        assert!(store.pinned_ids().is_empty());

        store.pin("mem-1", true).await.unwrap();
        store.pin("mem-2", true).await.unwrap();
        assert_eq!(store.pinned_ids().len(), 2);
        assert!(store.pinned_ids().contains("mem-1"));

        store.pin("mem-1", false).await.unwrap();
        assert_eq!(store.pinned_ids().len(), 1);
        assert!(!store.pinned_ids().contains("mem-1"));
    }

    #[tokio::test]
    async fn test_pin_idempotent() {
        let store = Mem0Store::new("http://127.0.0.1:19999");
        store.pin("mem-1", true).await.unwrap();
        store.pin("mem-1", true).await.unwrap();
        assert_eq!(store.pinned_ids().len(), 1);
    }

    #[test]
    fn test_apply_pin_state() {
        let store = Mem0Store::new("http://127.0.0.1:19999");
        {
            let conn = store.pin_db.lock().unwrap();
            conn.execute("INSERT INTO pinned_memories (memory_id) VALUES ('abc')", []).unwrap();
        }

        let mut memories = vec![
            mem0_to_memory(
                Mem0Memory { id: "abc".into(), memory: "pinned one".into(), metadata: serde_json::json!({}), created_at: None, updated_at: None },
                None,
            ),
            mem0_to_memory(
                Mem0Memory { id: "def".into(), memory: "not pinned".into(), metadata: serde_json::json!({}), created_at: None, updated_at: None },
                None,
            ),
        ];
        store.apply_pin_state(&mut memories);
        assert!(memories[0].pinned);
        assert!(!memories[1].pinned);
    }

    #[tokio::test]
    async fn test_delete_cleans_up_pin() {
        let store = Mem0Store::new("http://127.0.0.1:19999");
        store.pin("mem-1", true).await.unwrap();
        assert_eq!(store.pinned_ids().len(), 1);

        // delete will fail (no server) but pin cleanup happens before the HTTP call check
        // Actually, delete sends HTTP first, so we just test pin cleanup directly
        {
            let conn = store.pin_db.lock().unwrap();
            conn.execute("DELETE FROM pinned_memories WHERE memory_id = 'mem-1'", []).unwrap();
        }
        assert!(store.pinned_ids().is_empty());
    }

    // --- HTTP-level tests using mockito ---

    const SINGLE_MEMORY_RESPONSE: &str = r#"{"results": [{"id": "m1", "memory": "User prefers tabs", "metadata": {"thread_id": "t1"}, "created_at": "2026-01-01T00:00:00Z", "updated_at": null}]}"#;

    #[tokio::test]
    async fn test_add_memory_success() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/memories/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(SINGLE_MEMORY_RESPONSE)
            .create_async()
            .await;

        let store = Mem0Store::new(&server.url());
        let memories = store.add("transcript", Some("ws1"), "t1").await.unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].id, "m1");
        assert_eq!(memories[0].content, "User prefers tabs");
        assert_eq!(memories[0].workspace_id.as_deref(), Some("ws1"));
        assert_eq!(memories[0].source_thread_id, "t1");
        assert_eq!(memories[0].memory_type, MemoryType::Preference);
        assert!(!memories[0].pinned);
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_add_memory_api_error() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/memories/")
            .with_status(500)
            .with_body("internal server error")
            .create_async()
            .await;

        let store = Mem0Store::new(&server.url());
        let result = store.add("transcript", Some("ws1"), "t1").await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("500"), "error should mention status code: {err_msg}");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_search_returns_results() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/memories/search/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(SINGLE_MEMORY_RESPONSE)
            .create_async()
            .await;

        let store = Mem0Store::new(&server.url());
        let memories = store.search("tabs", Some("ws1"), 10).await.unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].id, "m1");
        assert_eq!(memories[0].content, "User prefers tabs");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_search_empty_results() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("POST", "/v1/memories/search/")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"results": []}"#)
            .create_async()
            .await;

        let store = Mem0Store::new(&server.url());
        let memories = store.search("nothing", None, 10).await.unwrap();
        assert!(memories.is_empty());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_get_all_memories() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/memories/")
            .match_query(mockito::Matcher::UrlEncoded("user_id".into(), "ws:ws1".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(SINGLE_MEMORY_RESPONSE)
            .create_async()
            .await;

        let store = Mem0Store::new(&server.url());
        let memories = store.get_all(Some("ws1")).await.unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].id, "m1");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_update_memory_success() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("PUT", "/v1/memories/m1/")
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let store = Mem0Store::new(&server.url());
        let result = store.update("m1", "updated content").await;
        assert!(result.is_ok());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_update_memory_failure() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("PUT", "/v1/memories/m1/")
            .with_status(404)
            .with_body("not found")
            .create_async()
            .await;

        let store = Mem0Store::new(&server.url());
        let result = store.update("m1", "updated content").await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("404"), "error should mention status code: {err_msg}");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_delete_memory_success() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("DELETE", "/v1/memories/m1/")
            .with_status(200)
            .with_body("{}")
            .create_async()
            .await;

        let store = Mem0Store::new(&server.url());
        // Pin the memory first, then delete -- verify pin is cleaned up
        store.pin("m1", true).await.unwrap();
        assert!(store.pinned_ids().contains("m1"));

        let result = store.delete("m1").await;
        assert!(result.is_ok());
        assert!(!store.pinned_ids().contains("m1"), "pin should be cleaned up after delete");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_delete_memory_failure() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("DELETE", "/v1/memories/m1/")
            .with_status(500)
            .with_body("internal server error")
            .create_async()
            .await;

        let store = Mem0Store::new(&server.url());
        let result = store.delete("m1").await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("500"), "error should mention status code: {err_msg}");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_get_all_applies_pin_state() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/v1/memories/")
            .match_query(mockito::Matcher::UrlEncoded("user_id".into(), "ws:ws1".into()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"results": [
                {"id": "m1", "memory": "User prefers tabs", "metadata": {}, "created_at": "2026-01-01T00:00:00Z", "updated_at": null},
                {"id": "m2", "memory": "Uses React hooks", "metadata": {}, "created_at": "2026-01-01T00:00:00Z", "updated_at": null}
            ]}"#)
            .create_async()
            .await;

        let store = Mem0Store::new(&server.url());
        // Pin only m1
        store.pin("m1", true).await.unwrap();

        let memories = store.get_all(Some("ws1")).await.unwrap();
        assert_eq!(memories.len(), 2);

        let m1 = memories.iter().find(|m| m.id == "m1").unwrap();
        let m2 = memories.iter().find(|m| m.id == "m2").unwrap();
        assert!(m1.pinned, "m1 should be pinned");
        assert!(!m2.pinned, "m2 should not be pinned");
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn test_health_check_success() {
        let mut server = mockito::Server::new_async().await;
        let mock = server
            .mock("GET", "/health")
            .with_status(200)
            .with_body("ok")
            .create_async()
            .await;

        let store = Mem0Store::new(&server.url());
        let healthy = store.health_check().await.unwrap();
        assert!(healthy);
        mock.assert_async().await;
    }
}
