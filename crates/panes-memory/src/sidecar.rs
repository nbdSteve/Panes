use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::process::{Child, Command};
use tracing::{error, info, warn};

const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(30);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_RESTART_ATTEMPTS: u32 = 3;

pub struct SidecarManager {
    binary_path: PathBuf,
    data_dir: PathBuf,
    port: u16,
    child: Option<Child>,
    restart_count: u32,
}

impl SidecarManager {
    pub fn new(binary_path: impl Into<PathBuf>, data_dir: impl Into<PathBuf>, port: u16) -> Self {
        Self {
            binary_path: binary_path.into(),
            data_dir: data_dir.into(),
            port,
            child: None,
            restart_count: 0,
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub async fn start(&mut self) -> Result<()> {
        info!(binary = %self.binary_path.display(), port = self.port, "starting mem0 sidecar");

        std::fs::create_dir_all(&self.data_dir)
            .context("failed to create mem0 data directory")?;

        let mut cmd = Command::new(&self.binary_path);
        cmd.arg("--port")
            .arg(self.port.to_string())
            .arg("--data-dir")
            .arg(&self.data_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                nix::unistd::setsid().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                Ok(())
            });
        }

        let child = cmd.spawn().context("failed to spawn mem0 sidecar")?;
        self.child = Some(child);

        // Wait for health check to pass
        let client = reqwest::Client::new();
        let health_url = format!("{}/health", self.base_url());
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > STARTUP_TIMEOUT {
                self.stop().await;
                anyhow::bail!("mem0 sidecar failed to start within {:?}", STARTUP_TIMEOUT);
            }

            match client
                .get(&health_url)
                .timeout(Duration::from_secs(2))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    info!("mem0 sidecar healthy on port {}", self.port);
                    self.restart_count = 0;
                    return Ok(());
                }
                _ => {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }

    pub async fn stop(&mut self) {
        if let Some(ref child) = self.child {
            #[cfg(unix)]
            {
                if let Some(pid) = child.id() {
                    info!(pid, "sending SIGTERM to mem0 sidecar");
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGTERM);
                    }
                }
            }
        }

        if let Some(mut child) = self.child.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            let _ = child.kill().await;
        }
    }

    pub async fn restart(&mut self) -> Result<bool> {
        if self.restart_count >= MAX_RESTART_ATTEMPTS {
            warn!("mem0 sidecar exceeded max restart attempts");
            return Ok(false);
        }

        self.restart_count += 1;
        warn!(attempt = self.restart_count, "restarting mem0 sidecar");

        self.stop().await;
        match self.start().await {
            Ok(()) => Ok(true),
            Err(e) => {
                error!(error = %e, "failed to restart mem0 sidecar");
                Ok(false)
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
    }

    /// Spawn a background task that monitors sidecar health.
    /// Returns a watch receiver that signals when sidecar becomes unavailable.
    pub fn spawn_health_monitor(
        base_url: String,
    ) -> tokio::sync::watch::Receiver<bool> {
        let (tx, rx) = tokio::sync::watch::channel(true);

        tokio::spawn(async move {
            let client = reqwest::Client::new();
            let health_url = format!("{base_url}/health");

            loop {
                tokio::time::sleep(HEALTH_CHECK_INTERVAL).await;

                let healthy = match client
                    .get(&health_url)
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                {
                    Ok(resp) => resp.status().is_success(),
                    Err(_) => false,
                };

                let _ = tx.send(healthy);
            }
        });

        rx
    }
}

impl Drop for SidecarManager {
    fn drop(&mut self) {
        if let Some(ref child) = self.child {
            #[cfg(unix)]
            {
                if let Some(pid) = child.id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGTERM);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_creates_manager() {
        let mgr = SidecarManager::new("/usr/bin/fake", "/tmp/data", 9999);
        assert_eq!(mgr.port, 9999);
        assert_eq!(mgr.binary_path, PathBuf::from("/usr/bin/fake"));
        assert_eq!(mgr.data_dir, PathBuf::from("/tmp/data"));
        assert!(mgr.child.is_none());
        assert_eq!(mgr.restart_count, 0);
    }

    #[test]
    fn test_base_url() {
        let mgr = SidecarManager::new("/bin/x", "/tmp", 8080);
        assert_eq!(mgr.base_url(), "http://127.0.0.1:8080");
    }

    #[test]
    fn test_base_url_different_port() {
        let mgr = SidecarManager::new("/bin/x", "/tmp", 3000);
        assert_eq!(mgr.base_url(), "http://127.0.0.1:3000");
    }

    #[test]
    fn test_is_running_false_initially() {
        let mgr = SidecarManager::new("/bin/x", "/tmp", 8080);
        assert!(!mgr.is_running());
    }

    #[tokio::test]
    async fn test_stop_when_not_running() {
        let mut mgr = SidecarManager::new("/bin/x", "/tmp", 8080);
        mgr.stop().await;
        assert!(!mgr.is_running());
    }

    #[tokio::test]
    async fn test_start_nonexistent_binary() {
        let mut mgr = SidecarManager::new(
            "/nonexistent/binary/that/does/not/exist",
            "/tmp/panes-test-sidecar",
            19876,
        );
        let result = mgr.start().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("failed to spawn"));
    }

    #[tokio::test]
    async fn test_restart_exceeds_max_attempts() {
        let mut mgr = SidecarManager::new("/nonexistent", "/tmp", 19877);
        mgr.restart_count = MAX_RESTART_ATTEMPTS;
        let result = mgr.restart().await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_restart_increments_count() {
        let mut mgr = SidecarManager::new("/nonexistent", "/tmp/panes-test-sc2", 19878);
        assert_eq!(mgr.restart_count, 0);
        let _ = mgr.restart().await;
        assert_eq!(mgr.restart_count, 1);
    }

    #[tokio::test]
    async fn test_restart_with_failed_start_returns_false() {
        let mut mgr = SidecarManager::new("/nonexistent", "/tmp/panes-test-sc3", 19879);
        let result = mgr.restart().await.unwrap();
        assert!(!result);
    }

    #[test]
    fn test_drop_no_panic_when_no_child() {
        let mgr = SidecarManager::new("/bin/x", "/tmp", 8080);
        drop(mgr);
    }

    #[tokio::test]
    async fn test_health_monitor_returns_receiver() {
        let rx = SidecarManager::spawn_health_monitor("http://127.0.0.1:19999".to_string());
        assert!(*rx.borrow());
    }
}
