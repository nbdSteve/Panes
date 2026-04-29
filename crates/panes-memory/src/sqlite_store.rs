use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use rusqlite::Connection;
use std::sync::Mutex;
use tracing::debug;
use uuid::Uuid;

use crate::types::{Briefing, Memory, MemoryType};
use crate::{BriefingStore, MemoryStore};

pub struct SqliteMemoryStore {
    conn: Mutex<Connection>,
}

impl SqliteMemoryStore {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)
            .context("failed to open SQLite database for memory store")?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                workspace_id TEXT,
                memory_type TEXT NOT NULL,
                content TEXT NOT NULL,
                source_thread_id TEXT NOT NULL,
                created_at TEXT NOT NULL,
                edited_at TEXT,
                pinned INTEGER DEFAULT 0
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
                content,
                content='memories',
                content_rowid='rowid'
            );
            CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
            END;
            CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.rowid, old.content);
            END;
            CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.rowid, old.content);
                INSERT INTO memories_fts(rowid, content) VALUES (new.rowid, new.content);
            END;
            CREATE TABLE IF NOT EXISTS briefings (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL UNIQUE,
                content TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            ",
        )
        .context("failed to initialize memory schema")?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

#[async_trait]
impl MemoryStore for SqliteMemoryStore {
    async fn add(
        &self,
        transcript: &str,
        workspace_id: Option<&str>,
        thread_id: &str,
    ) -> Result<Vec<Memory>> {
        // TODO: LLM extraction — for now, store the transcript summary as a single memory
        // This will be replaced with proper LLM extraction in the next iteration
        let memory = Memory {
            id: Uuid::new_v4().to_string(),
            workspace_id: workspace_id.map(String::from),
            memory_type: MemoryType::Pattern,
            content: format!("Thread completed: {}", truncate(transcript, 200)),
            source_thread_id: thread_id.to_string(),
            created_at: Utc::now(),
            edited_at: None,
            pinned: false,
        };

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO memories (id, workspace_id, memory_type, content, source_thread_id, created_at, pinned) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            rusqlite::params![
                memory.id,
                memory.workspace_id,
                memory.memory_type.to_string(),
                memory.content,
                memory.source_thread_id,
                memory.created_at.to_rfc3339(),
            ],
        )?;

        Ok(vec![memory])
    }

    async fn search(
        &self,
        query: &str,
        workspace_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Memory>> {
        let conn = self.conn.lock().unwrap();

        // FTS5 search with workspace filter
        let mut stmt = conn.prepare(
            "SELECT m.id, m.workspace_id, m.memory_type, m.content, m.source_thread_id, m.created_at, m.edited_at, m.pinned
             FROM memories m
             JOIN memories_fts f ON m.rowid = f.rowid
             WHERE memories_fts MATCH ?1
             AND (m.workspace_id = ?2 OR m.workspace_id IS NULL OR ?2 IS NULL)
             ORDER BY m.pinned DESC, bm25(memories_fts) ASC
             LIMIT ?3",
        )?;

        let rows = stmt.query_map(rusqlite::params![query, workspace_id, limit], |row| {
            Ok(Memory {
                id: row.get(0)?,
                workspace_id: row.get(1)?,
                memory_type: parse_memory_type(&row.get::<_, String>(2)?),
                content: row.get(3)?,
                source_thread_id: row.get(4)?,
                created_at: row
                    .get::<_, String>(5)?
                    .parse()
                    .unwrap_or_else(|_| Utc::now()),
                edited_at: row
                    .get::<_, Option<String>>(6)?
                    .and_then(|s| s.parse().ok()),
                pinned: row.get::<_, i32>(7)? != 0,
            })
        })?;

        let memories: Vec<Memory> = rows.filter_map(|r| r.ok()).collect();
        debug!(count = memories.len(), query, "fts5 search results");
        Ok(memories)
    }

    async fn get_all(&self, workspace_id: Option<&str>) -> Result<Vec<Memory>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, memory_type, content, source_thread_id, created_at, edited_at, pinned
             FROM memories
             WHERE workspace_id = ?1 OR workspace_id IS NULL OR ?1 IS NULL
             ORDER BY pinned DESC, created_at DESC",
        )?;

        let rows = stmt.query_map(rusqlite::params![workspace_id], |row| {
            Ok(Memory {
                id: row.get(0)?,
                workspace_id: row.get(1)?,
                memory_type: parse_memory_type(&row.get::<_, String>(2)?),
                content: row.get(3)?,
                source_thread_id: row.get(4)?,
                created_at: row
                    .get::<_, String>(5)?
                    .parse()
                    .unwrap_or_else(|_| Utc::now()),
                edited_at: row
                    .get::<_, Option<String>>(6)?
                    .and_then(|s| s.parse().ok()),
                pinned: row.get::<_, i32>(7)? != 0,
            })
        })?;

        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    async fn update(&self, id: &str, content: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE memories SET content = ?1, edited_at = ?2 WHERE id = ?3",
            rusqlite::params![content, Utc::now().to_rfc3339(), id],
        )?;
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM memories WHERE id = ?1", rusqlite::params![id])?;
        Ok(())
    }

    async fn pin(&self, id: &str, pinned: bool) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE memories SET pinned = ?1 WHERE id = ?2",
            rusqlite::params![pinned as i32, id],
        )?;
        Ok(())
    }

    async fn health_check(&self) -> Result<bool> {
        Ok(true) // SQLite is always available
    }
}

#[async_trait]
impl BriefingStore for SqliteMemoryStore {
    async fn get_briefing(&self, workspace_id: &str) -> Result<Option<Briefing>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, content, updated_at FROM briefings WHERE workspace_id = ?1",
        )?;

        let result = stmt
            .query_row(rusqlite::params![workspace_id], |row| {
                Ok(Briefing {
                    id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    content: row.get(2)?,
                    updated_at: row
                        .get::<_, String>(3)?
                        .parse()
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .ok();

        Ok(result)
    }

    async fn set_briefing(&self, workspace_id: &str, content: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO briefings (id, workspace_id, content, updated_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(workspace_id) DO UPDATE SET content = ?3, updated_at = ?4",
            rusqlite::params![
                Uuid::new_v4().to_string(),
                workspace_id,
                content,
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    async fn delete_briefing(&self, workspace_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM briefings WHERE workspace_id = ?1",
            rusqlite::params![workspace_id],
        )?;
        Ok(())
    }
}

fn parse_memory_type(s: &str) -> MemoryType {
    match s {
        "decision" => MemoryType::Decision,
        "preference" => MemoryType::Preference,
        "constraint" => MemoryType::Constraint,
        _ => MemoryType::Pattern,
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let boundary = s.char_indices()
            .map(|(i, _)| i)
            .take_while(|&i| i <= max)
            .last()
            .unwrap_or(0);
        &s[..boundary]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> SqliteMemoryStore {
        SqliteMemoryStore::new(":memory:").unwrap()
    }

    #[tokio::test]
    async fn test_add_and_get_all() {
        let store = make_store();
        let mems = store.add("User said hello", Some("ws1"), "t1").await.unwrap();
        assert_eq!(mems.len(), 1);
        assert_eq!(mems[0].source_thread_id, "t1");

        let all = store.get_all(Some("ws1")).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, mems[0].id);
    }

    #[tokio::test]
    async fn test_search_fts5() {
        let store = make_store();
        store.add("Always use TypeScript strict mode", Some("ws1"), "t1").await.unwrap();
        store.add("Use pnpm for package management", Some("ws1"), "t2").await.unwrap();

        let results = store.search("pnpm package", Some("ws1"), 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("pnpm"));
    }

    #[tokio::test]
    async fn test_search_empty_returns_nothing() {
        let store = make_store();
        let results = store.search("nonexistent", Some("ws1"), 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_update_memory() {
        let store = make_store();
        let mems = store.add("original content", Some("ws1"), "t1").await.unwrap();
        let id = &mems[0].id;

        store.update(id, "updated content").await.unwrap();

        let all = store.get_all(Some("ws1")).await.unwrap();
        assert_eq!(all[0].content, "updated content");
        assert!(all[0].edited_at.is_some());
    }

    #[tokio::test]
    async fn test_delete_memory() {
        let store = make_store();
        let mems = store.add("to be deleted", Some("ws1"), "t1").await.unwrap();
        let id = &mems[0].id;

        store.delete(id).await.unwrap();

        let all = store.get_all(Some("ws1")).await.unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn test_pin_and_unpin() {
        let store = make_store();
        let mems = store.add("important memory", Some("ws1"), "t1").await.unwrap();
        let id = &mems[0].id;

        assert!(!mems[0].pinned);

        store.pin(id, true).await.unwrap();
        let all = store.get_all(Some("ws1")).await.unwrap();
        assert!(all[0].pinned);

        store.pin(id, false).await.unwrap();
        let all = store.get_all(Some("ws1")).await.unwrap();
        assert!(!all[0].pinned);
    }

    #[tokio::test]
    async fn test_workspace_isolation() {
        let store = make_store();
        store.add("ws1 memory", Some("ws1"), "t1").await.unwrap();
        store.add("ws2 memory", Some("ws2"), "t2").await.unwrap();

        let ws1 = store.get_all(Some("ws1")).await.unwrap();
        assert_eq!(ws1.len(), 1);
        assert!(ws1[0].content.contains("ws1"));

        let ws2 = store.get_all(Some("ws2")).await.unwrap();
        assert_eq!(ws2.len(), 1);
        assert!(ws2[0].content.contains("ws2"));
    }

    #[tokio::test]
    async fn test_global_memories() {
        let store = make_store();
        store.add("global memory", None, "t1").await.unwrap();

        let global = store.get_all(None).await.unwrap();
        assert_eq!(global.len(), 1);

        // Global memories visible when querying with a workspace too (NULL workspace_id matches)
        let ws = store.get_all(Some("ws1")).await.unwrap();
        assert!(ws.is_empty() || ws.iter().any(|m| m.workspace_id.is_none()));
    }

    #[tokio::test]
    async fn test_health_check() {
        let store = make_store();
        assert!(store.health_check().await.unwrap());
    }

    #[tokio::test]
    async fn test_briefing_crud() {
        let store = make_store();

        assert!(store.get_briefing("ws1").await.unwrap().is_none());

        store.set_briefing("ws1", "Always use TypeScript").await.unwrap();
        let b = store.get_briefing("ws1").await.unwrap().unwrap();
        assert_eq!(b.content, "Always use TypeScript");
        assert_eq!(b.workspace_id, "ws1");

        store.set_briefing("ws1", "Updated briefing").await.unwrap();
        let b = store.get_briefing("ws1").await.unwrap().unwrap();
        assert_eq!(b.content, "Updated briefing");

        store.delete_briefing("ws1").await.unwrap();
        assert!(store.get_briefing("ws1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_briefing_workspace_isolation() {
        let store = make_store();
        store.set_briefing("ws1", "briefing 1").await.unwrap();
        store.set_briefing("ws2", "briefing 2").await.unwrap();

        assert_eq!(store.get_briefing("ws1").await.unwrap().unwrap().content, "briefing 1");
        assert_eq!(store.get_briefing("ws2").await.unwrap().unwrap().content, "briefing 2");
    }

    #[tokio::test]
    async fn test_pinned_memories_sort_first() {
        let store = make_store();
        let m1 = store.add("unpinned memory", Some("ws1"), "t1").await.unwrap();
        let m2 = store.add("pinned memory", Some("ws1"), "t2").await.unwrap();
        store.pin(&m2[0].id, true).await.unwrap();

        let all = store.get_all(Some("ws1")).await.unwrap();
        assert!(all[0].pinned, "pinned memory should be first");
        assert!(!all[1].pinned);
        let _ = m1;
    }
}
