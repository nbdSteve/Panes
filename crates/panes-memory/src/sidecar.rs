use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::process::{Child, Command};
use tracing::{error, info, warn};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_RESTART_ATTEMPTS: u32 = 3;

pub struct SidecarManager {
    python_path: PathBuf,
    server_script: PathBuf,
    port: u16,
    child: Option<Child>,
    restart_count: u32,
}

impl SidecarManager {
    pub fn new(python_path: impl Into<PathBuf>, server_script: impl Into<PathBuf>, port: u16) -> Self {
        Self {
            python_path: python_path.into(),
            server_script: server_script.into(),
            port,
            child: None,
            restart_count: 0,
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub async fn start(&mut self) -> Result<()> {
        info!(python = %self.python_path.display(), port = self.port, "starting mem0 server");

        let mut cmd = Command::new(&self.python_path);
        cmd.arg(&self.server_script)
            .arg("--port")
            .arg(self.port.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                nix::unistd::setsid().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                Ok(())
            });
        }

        let child = cmd.spawn().context("failed to spawn mem0 server — is mem0ai installed?")?;
        self.child = Some(child);

        let client = reqwest::Client::new();
        let probe_url = format!("{}/health", self.base_url());
        let start = std::time::Instant::now();

        loop {
            if start.elapsed() > STARTUP_TIMEOUT {
                self.stop().await;
                anyhow::bail!("mem0 server failed to start within {:?}", STARTUP_TIMEOUT);
            }

            match client
                .get(&probe_url)
                .timeout(Duration::from_secs(2))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 422 => {
                    info!("mem0 server ready on port {}", self.port);
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
                    info!(pid, "sending SIGTERM to mem0 server");
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
            warn!("mem0 server exceeded max restart attempts");
            return Ok(false);
        }

        self.restart_count += 1;
        warn!(attempt = self.restart_count, "restarting mem0 server");

        self.stop().await;
        match self.start().await {
            Ok(()) => Ok(true),
            Err(e) => {
                error!(error = %e, "failed to restart mem0 server");
                Ok(false)
            }
        }
    }

    pub fn is_running(&self) -> bool {
        self.child.is_some()
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
        let mgr = SidecarManager::new("/usr/bin/python3", "mem0_server.py", 9999);
        assert_eq!(mgr.port, 9999);
        assert_eq!(mgr.python_path, PathBuf::from("/usr/bin/python3"));
        assert!(mgr.child.is_none());
        assert_eq!(mgr.restart_count, 0);
    }

    #[test]
    fn test_base_url() {
        let mgr = SidecarManager::new("/usr/bin/python3", "mem0_server.py", 8080);
        assert_eq!(mgr.base_url(), "http://127.0.0.1:8080");
    }

    #[test]
    fn test_base_url_different_port() {
        let mgr = SidecarManager::new("/usr/bin/python3", "mem0_server.py", 3000);
        assert_eq!(mgr.base_url(), "http://127.0.0.1:3000");
    }

    #[test]
    fn test_is_running_false_initially() {
        let mgr = SidecarManager::new("/usr/bin/python3", "mem0_server.py", 8080);
        assert!(!mgr.is_running());
    }

    #[tokio::test]
    async fn test_stop_when_not_running() {
        let mut mgr = SidecarManager::new("/usr/bin/python3", "mem0_server.py", 8080);
        mgr.stop().await;
        assert!(!mgr.is_running());
    }

    #[tokio::test]
    async fn test_start_nonexistent_python() {
        let mut mgr = SidecarManager::new(
            "/nonexistent/python/that/does/not/exist",
            "mem0_server.py",
            19876,
        );
        let result = mgr.start().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("failed to spawn"));
    }

    #[tokio::test]
    async fn test_restart_exceeds_max_attempts() {
        let mut mgr = SidecarManager::new("/nonexistent", "mem0_server.py", 19877);
        mgr.restart_count = MAX_RESTART_ATTEMPTS;
        let result = mgr.restart().await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_restart_increments_count() {
        let mut mgr = SidecarManager::new("/nonexistent", "mem0_server.py", 19878);
        assert_eq!(mgr.restart_count, 0);
        let _ = mgr.restart().await;
        assert_eq!(mgr.restart_count, 1);
    }

    #[tokio::test]
    async fn test_restart_with_failed_start_returns_false() {
        let mut mgr = SidecarManager::new("/nonexistent", "mem0_server.py", 19879);
        let result = mgr.restart().await.unwrap();
        assert!(!result);
    }

    #[test]
    fn test_drop_no_panic_when_no_child() {
        let mgr = SidecarManager::new("/usr/bin/python3", "mem0_server.py", 8080);
        drop(mgr);
    }
}
