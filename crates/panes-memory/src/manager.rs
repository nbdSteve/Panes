use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::mem0_store::Mem0Store;
use crate::sidecar::SidecarManager;
use crate::sqlite_store::SqliteMemoryStore;
use crate::types::{Briefing, Memory};
use crate::{BriefingStore, MemoryStore};

const BACKEND_SQLITE: u8 = 0;
const BACKEND_MEM0: u8 = 1;
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(30);

pub struct MemoryConfig {
    pub memory_db_path: String,
    pub mem0_binary: Option<String>,
    pub mem0_port: u16,
    pub mem0_data_dir: PathBuf,
    pub mem0_pin_db_path: String,
}

impl MemoryConfig {
    pub fn from_env(data_dir: &Path) -> Self {
        let mem0_binary = std::env::var("PANES_MEM0_BINARY").ok();
        let mem0_port = std::env::var("PANES_MEM0_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8019);

        Self {
            memory_db_path: data_dir.join("memory.db").to_string_lossy().to_string(),
            mem0_binary,
            mem0_port,
            mem0_data_dir: data_dir.join("mem0"),
            mem0_pin_db_path: data_dir.join("mem0_pins.db").to_string_lossy().to_string(),
        }
    }

    pub fn for_test() -> Self {
        Self {
            memory_db_path: ":memory:".to_string(),
            mem0_binary: None,
            mem0_port: 0,
            mem0_data_dir: PathBuf::new(),
            mem0_pin_db_path: ":memory:".to_string(),
        }
    }
}

pub struct MemoryManager {
    sqlite: Arc<SqliteMemoryStore>,
    mem0: Option<Arc<Mem0Store>>,
    sidecar: tokio::sync::Mutex<Option<SidecarManager>>,
    active: AtomicU8,
}

impl MemoryManager {
    pub fn new(config: &MemoryConfig) -> Result<Self> {
        let sqlite = Arc::new(SqliteMemoryStore::new(&config.memory_db_path)?);

        let (mem0, sidecar) = if let Some(ref binary) = config.mem0_binary {
            let sidecar = SidecarManager::new(binary, &config.mem0_data_dir, config.mem0_port);
            let base_url = sidecar.base_url();
            let store = Arc::new(Mem0Store::with_pin_db(&base_url, &config.mem0_pin_db_path));
            (Some(store), Some(sidecar))
        } else {
            (None, None)
        };

        Ok(Self {
            sqlite,
            mem0,
            sidecar: tokio::sync::Mutex::new(sidecar),
            active: AtomicU8::new(BACKEND_SQLITE),
        })
    }

    pub async fn init(&self) {
        let mut guard = self.sidecar.lock().await;
        if let Some(ref mut sidecar) = *guard {
            match sidecar.start().await {
                Ok(()) => {
                    info!("mem0 sidecar started, using mem0 for memory extraction");
                    self.active.store(BACKEND_MEM0, Ordering::Relaxed);
                }
                Err(e) => {
                    warn!(error = %e, "mem0 sidecar failed to start — using sqlite");
                }
            }
        }
    }

    fn is_mem0_active(&self) -> bool {
        self.active.load(Ordering::Relaxed) == BACKEND_MEM0
    }

    pub fn spawn_health_monitor(self: &Arc<Self>) -> JoinHandle<()> {
        let mgr = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(HEALTH_CHECK_INTERVAL).await;
                if mgr.mem0.is_none() {
                    continue;
                }

                if mgr.is_mem0_active() {
                    let healthy = mgr
                        .mem0
                        .as_ref()
                        .unwrap()
                        .health_check()
                        .await
                        .unwrap_or(false);
                    if !healthy {
                        warn!("mem0 unhealthy, falling back to sqlite");
                        mgr.active.store(BACKEND_SQLITE, Ordering::Relaxed);
                        mgr.try_restart_sidecar().await;
                    }
                } else {
                    mgr.try_restart_sidecar().await;
                }
            }
        })
    }

    async fn try_restart_sidecar(&self) {
        let mut guard = self.sidecar.lock().await;
        if let Some(ref mut sidecar) = *guard {
            match sidecar.restart().await {
                Ok(true) => {
                    info!("mem0 sidecar restarted, switching back");
                    self.active.store(BACKEND_MEM0, Ordering::Relaxed);
                }
                _ => warn!("mem0 restart failed, staying on sqlite"),
            }
        }
    }

    pub fn as_memory_store(&self) -> &dyn MemoryStore {
        self
    }

    pub fn as_briefing_store(&self) -> &dyn BriefingStore {
        self
    }

    pub fn get_active_backend(&self) -> &str {
        if self.is_mem0_active() { "mem0" } else { "sqlite" }
    }

    pub fn is_mem0_configured(&self) -> bool {
        self.mem0.is_some()
    }

    pub fn set_active_backend(&self, backend: &str) -> Result<(), String> {
        match backend {
            "sqlite" => {
                self.active.store(BACKEND_SQLITE, Ordering::Relaxed);
                Ok(())
            }
            "mem0" => {
                if self.mem0.is_none() {
                    return Err("Mem0 is not configured".to_string());
                }
                self.active.store(BACKEND_MEM0, Ordering::Relaxed);
                Ok(())
            }
            _ => Err(format!("Unknown backend: {backend}")),
        }
    }
}

#[async_trait]
impl MemoryStore for MemoryManager {
    async fn add(
        &self,
        transcript: &str,
        workspace_id: Option<&str>,
        thread_id: &str,
    ) -> Result<Vec<Memory>> {
        if self.is_mem0_active() {
            match self
                .mem0
                .as_ref()
                .unwrap()
                .add(transcript, workspace_id, thread_id)
                .await
            {
                Ok(v) => return Ok(v),
                Err(e) => {
                    warn!(error = %e, "mem0 add failed, falling back to sqlite");
                    self.active.store(BACKEND_SQLITE, Ordering::Relaxed);
                }
            }
        }
        self.sqlite.add(transcript, workspace_id, thread_id).await
    }

    async fn search(
        &self,
        query: &str,
        workspace_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Memory>> {
        if self.is_mem0_active() {
            match self
                .mem0
                .as_ref()
                .unwrap()
                .search(query, workspace_id, limit)
                .await
            {
                Ok(v) => return Ok(v),
                Err(e) => {
                    warn!(error = %e, "mem0 search failed, falling back to sqlite");
                    self.active.store(BACKEND_SQLITE, Ordering::Relaxed);
                }
            }
        }
        self.sqlite.search(query, workspace_id, limit).await
    }

    async fn get_all(&self, workspace_id: Option<&str>) -> Result<Vec<Memory>> {
        if self.is_mem0_active() {
            match self
                .mem0
                .as_ref()
                .unwrap()
                .get_all(workspace_id)
                .await
            {
                Ok(v) => return Ok(v),
                Err(e) => {
                    warn!(error = %e, "mem0 get_all failed, falling back to sqlite");
                    self.active.store(BACKEND_SQLITE, Ordering::Relaxed);
                }
            }
        }
        self.sqlite.get_all(workspace_id).await
    }

    async fn update(&self, id: &str, content: &str) -> Result<()> {
        if self.is_mem0_active() {
            match self.mem0.as_ref().unwrap().update(id, content).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    warn!(error = %e, "mem0 update failed, falling back to sqlite");
                    self.active.store(BACKEND_SQLITE, Ordering::Relaxed);
                }
            }
        }
        self.sqlite.update(id, content).await
    }

    async fn delete(&self, id: &str) -> Result<()> {
        if self.is_mem0_active() {
            match self.mem0.as_ref().unwrap().delete(id).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    warn!(error = %e, "mem0 delete failed, falling back to sqlite");
                    self.active.store(BACKEND_SQLITE, Ordering::Relaxed);
                }
            }
        }
        self.sqlite.delete(id).await
    }

    async fn pin(&self, id: &str, pinned: bool) -> Result<()> {
        if self.is_mem0_active() {
            match self.mem0.as_ref().unwrap().pin(id, pinned).await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    warn!(error = %e, "mem0 pin failed, falling back to sqlite");
                    self.active.store(BACKEND_SQLITE, Ordering::Relaxed);
                }
            }
        }
        self.sqlite.pin(id, pinned).await
    }

    async fn health_check(&self) -> Result<bool> {
        if self.is_mem0_active() {
            if let Some(ref mem0) = self.mem0 {
                return mem0.health_check().await;
            }
        }
        self.sqlite.health_check().await
    }
}

#[async_trait]
impl BriefingStore for MemoryManager {
    async fn get_briefing(&self, workspace_id: &str) -> Result<Option<Briefing>> {
        self.sqlite.get_briefing(workspace_id).await
    }

    async fn set_briefing(&self, workspace_id: &str, content: &str) -> Result<()> {
        self.sqlite.set_briefing(workspace_id, content).await
    }

    async fn delete_briefing(&self, workspace_id: &str) -> Result<()> {
        self.sqlite.delete_briefing(workspace_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sqlite_config() -> MemoryConfig {
        MemoryConfig::for_test()
    }

    #[test]
    fn test_new_sqlite_only() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        assert!(!mgr.is_mem0_active());
        assert!(mgr.mem0.is_none());
    }

    #[tokio::test]
    async fn test_sqlite_only_add() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        let mems = mgr.add("test transcript", Some("ws1"), "t1").await.unwrap();
        assert_eq!(mems.len(), 1);
        assert!(mems[0].content.contains("test transcript"));
    }

    #[tokio::test]
    async fn test_sqlite_only_search() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        mgr.add("pnpm package management", Some("ws1"), "t1")
            .await
            .unwrap();
        let results = mgr.search("pnpm", Some("ws1"), 10).await.unwrap();
        assert!(!results.is_empty());
    }

    #[tokio::test]
    async fn test_sqlite_only_get_all() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        mgr.add("memory one", Some("ws1"), "t1").await.unwrap();
        mgr.add("memory two", Some("ws1"), "t2").await.unwrap();
        let all = mgr.get_all(Some("ws1")).await.unwrap();
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn test_sqlite_only_update_and_delete() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        let mems = mgr.add("original", Some("ws1"), "t1").await.unwrap();
        let id = &mems[0].id;

        mgr.update(id, "updated content").await.unwrap();
        let all = mgr.get_all(Some("ws1")).await.unwrap();
        assert_eq!(all[0].content, "updated content");

        mgr.delete(id).await.unwrap();
        let all = mgr.get_all(Some("ws1")).await.unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn test_sqlite_only_pin() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        let mems = mgr.add("pinnable", Some("ws1"), "t1").await.unwrap();
        mgr.pin(&mems[0].id, true).await.unwrap();
        let all = mgr.get_all(Some("ws1")).await.unwrap();
        assert!(all[0].pinned);
    }

    #[tokio::test]
    async fn test_sqlite_only_briefing() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();

        assert!(mgr.get_briefing("ws1").await.unwrap().is_none());

        mgr.set_briefing("ws1", "Always use TypeScript")
            .await
            .unwrap();
        let b = mgr.get_briefing("ws1").await.unwrap().unwrap();
        assert_eq!(b.content, "Always use TypeScript");

        mgr.delete_briefing("ws1").await.unwrap();
        assert!(mgr.get_briefing("ws1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_sqlite_only_health_check() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        assert!(mgr.health_check().await.unwrap());
    }

    #[test]
    fn test_config_from_env() {
        let dir = PathBuf::from("/tmp/panes-test");
        let config = MemoryConfig::from_env(&dir);
        assert_eq!(config.memory_db_path, "/tmp/panes-test/memory.db");
        assert_eq!(config.mem0_data_dir, PathBuf::from("/tmp/panes-test/mem0"));
        assert_eq!(config.mem0_pin_db_path, "/tmp/panes-test/mem0_pins.db");
        assert_eq!(config.mem0_port, 8019);
    }

    #[test]
    fn test_config_for_test() {
        let config = MemoryConfig::for_test();
        assert_eq!(config.memory_db_path, ":memory:");
        assert!(config.mem0_binary.is_none());
    }

    #[test]
    fn test_accessor_methods() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        let _ms: &dyn MemoryStore = mgr.as_memory_store();
        let _bs: &dyn BriefingStore = mgr.as_briefing_store();
    }

    #[tokio::test]
    async fn test_fallback_on_mem0_failure() {
        let config = MemoryConfig {
            memory_db_path: ":memory:".to_string(),
            mem0_binary: Some("/nonexistent".to_string()),
            mem0_port: 19999,
            mem0_data_dir: PathBuf::from("/tmp/panes-test-mem0"),
            mem0_pin_db_path: ":memory:".to_string(),
        };
        let mgr = MemoryManager::new(&config).unwrap();
        // Mem0 is configured but sidecar not started, so active stays SQLITE
        assert!(!mgr.is_mem0_active());
        assert!(mgr.mem0.is_some());

        // Force active to MEM0 to test fallback
        mgr.active.store(BACKEND_MEM0, Ordering::Relaxed);
        assert!(mgr.is_mem0_active());

        // add() should fail on mem0 (no server), fall back to sqlite, and flip active
        let mems = mgr.add("test fallback", Some("ws1"), "t1").await.unwrap();
        assert_eq!(mems.len(), 1);
        assert!(!mgr.is_mem0_active(), "should have fallen back to sqlite");
    }

    #[tokio::test]
    async fn test_fallback_preserves_briefing() {
        let config = MemoryConfig {
            memory_db_path: ":memory:".to_string(),
            mem0_binary: Some("/nonexistent".to_string()),
            mem0_port: 19998,
            mem0_data_dir: PathBuf::from("/tmp/panes-test-mem0-b"),
            mem0_pin_db_path: ":memory:".to_string(),
        };
        let mgr = MemoryManager::new(&config).unwrap();

        // Briefings always go through SQLite regardless of active backend
        mgr.set_briefing("ws1", "test briefing").await.unwrap();
        mgr.active.store(BACKEND_MEM0, Ordering::Relaxed);
        let b = mgr.get_briefing("ws1").await.unwrap().unwrap();
        assert_eq!(b.content, "test briefing");
    }

    #[test]
    fn test_get_active_backend_default_sqlite() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        assert_eq!(mgr.get_active_backend(), "sqlite");
    }

    #[test]
    fn test_get_active_backend_reflects_atomic() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        mgr.active.store(BACKEND_MEM0, Ordering::Relaxed);
        assert_eq!(mgr.get_active_backend(), "mem0");
    }

    #[test]
    fn test_is_mem0_configured_false_without_binary() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        assert!(!mgr.is_mem0_configured());
    }

    #[test]
    fn test_is_mem0_configured_true_with_binary() {
        let config = MemoryConfig {
            memory_db_path: ":memory:".to_string(),
            mem0_binary: Some("/nonexistent".to_string()),
            mem0_port: 19997,
            mem0_data_dir: PathBuf::from("/tmp/panes-test-configured"),
            mem0_pin_db_path: ":memory:".to_string(),
        };
        let mgr = MemoryManager::new(&config).unwrap();
        assert!(mgr.is_mem0_configured());
    }

    #[test]
    fn test_set_active_backend_to_sqlite() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        mgr.active.store(BACKEND_MEM0, Ordering::Relaxed);
        assert!(mgr.set_active_backend("sqlite").is_ok());
        assert_eq!(mgr.get_active_backend(), "sqlite");
    }

    #[test]
    fn test_set_active_backend_to_mem0_rejects_unconfigured() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        let err = mgr.set_active_backend("mem0").unwrap_err();
        assert!(err.contains("not configured"));
    }

    #[test]
    fn test_set_active_backend_to_mem0_when_configured() {
        let config = MemoryConfig {
            memory_db_path: ":memory:".to_string(),
            mem0_binary: Some("/nonexistent".to_string()),
            mem0_port: 19996,
            mem0_data_dir: PathBuf::from("/tmp/panes-test-toggle"),
            mem0_pin_db_path: ":memory:".to_string(),
        };
        let mgr = MemoryManager::new(&config).unwrap();
        assert!(mgr.set_active_backend("mem0").is_ok());
        assert_eq!(mgr.get_active_backend(), "mem0");
    }

    #[test]
    fn test_set_active_backend_unknown_rejects() {
        let mgr = MemoryManager::new(&sqlite_config()).unwrap();
        let err = mgr.set_active_backend("redis").unwrap_err();
        assert!(err.contains("Unknown backend"));
    }
}
