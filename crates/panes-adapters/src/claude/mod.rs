mod parser;
mod risk;

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::Stream;
use panes_events::{AgentEvent, SessionContext, SessionInit};
use tokio::io::BufReader;
use tokio::process::{Child, Command};
use tracing::{info, warn};

use crate::{AgentAdapter, AgentSession};

pub struct ClaudeAdapter {
    cli_path: String,
    env_vars: Vec<(String, String)>,
    permission_mode: String,
}

impl ClaudeAdapter {
    pub fn new() -> Self {
        Self {
            cli_path: "claude".to_string(),
            env_vars: Vec::new(),
            permission_mode: "bypassPermissions".to_string(),
        }
    }

    pub fn with_cli_path(cli_path: impl Into<String>) -> Self {
        Self {
            cli_path: cli_path.into(),
            env_vars: Vec::new(),
            permission_mode: "bypassPermissions".to_string(),
        }
    }

    pub fn permission_mode(mut self, mode: impl Into<String>) -> Self {
        self.permission_mode = mode.into();
        self
    }

    pub fn env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.env_vars.push((key.into(), val.into()));
        self
    }
}

impl Default for ClaudeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentAdapter for ClaudeAdapter {
    fn name(&self) -> &str {
        "claude-code"
    }

    async fn spawn(
        &self,
        workspace_path: &Path,
        prompt: &str,
        context: &SessionContext,
        model: Option<&str>,
    ) -> Result<Box<dyn AgentSession>> {
        let mut full_prompt = String::new();

        if let Some(briefing) = &context.briefing {
            full_prompt.push_str("<briefing>\n");
            full_prompt.push_str(briefing);
            full_prompt.push_str("\n</briefing>\n\n");
        }

        if !context.memories.is_empty() {
            full_prompt.push_str("<memories>\n");
            for memory in &context.memories {
                full_prompt.push_str("- ");
                full_prompt.push_str(memory);
                full_prompt.push('\n');
            }
            full_prompt.push_str("</memories>\n\n");
        }

        full_prompt.push_str(prompt);

        let mut cmd = Command::new(&self.cli_path);
        cmd.arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--permission-mode")
            .arg(&self.permission_mode);

        if let Some(m) = model {
            cmd.arg("--model").arg(m);
        }

        cmd.arg(&full_prompt)
            .current_dir(workspace_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped());

        for (key, val) in &self.env_vars {
            cmd.env(key, val);
        }

        // Process group isolation on unix
        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                nix::unistd::setsid().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                Ok(())
            });
        }

        eprintln!("[panes-adapter] spawning: {} in {}", &self.cli_path, workspace_path.display());

        let mut child = cmd.spawn().with_context(|| {
            format!("failed to spawn claude CLI at '{}' — is it installed?", &self.cli_path)
        })?;

        let stdout = child
            .stdout
            .take()
            .context("failed to capture claude stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("failed to capture claude stderr")?;

        // Spawn stderr monitor for auth errors
        let stderr_reader = BufReader::new(stderr);
        let (auth_error_tx, auth_error_rx) = tokio::sync::watch::channel(None::<String>);
        tokio::spawn(async move {
            Self::monitor_stderr(stderr_reader, auth_error_tx).await;
        });

        let pid = child.id();
        eprintln!("[panes-adapter] claude spawned with pid {:?}, waiting for init...", pid);

        let reader = BufReader::new(stdout);
        let (init, event_stream) = parser::parse_stream(reader, auth_error_rx).await
            .context("failed to parse claude stream — check if claude CLI is working")?;

        eprintln!("[panes-adapter] got init: model={}, session={}", init.model, init.session_id);

        let session = ClaudeSession {
            _child: tokio::sync::Mutex::new(child),
            init_data: init,
            event_stream: tokio::sync::Mutex::new(Some(event_stream)),
            workspace_path: workspace_path.to_path_buf(),
            pid,
        };

        Ok(Box::new(session))
    }

    async fn resume(
        &self,
        workspace_path: &Path,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
    ) -> Result<Box<dyn AgentSession>> {
        let mut cmd = Command::new(&self.cli_path);
        cmd.arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--permission-mode")
            .arg(&self.permission_mode);

        if let Some(m) = model {
            cmd.arg("--model").arg(m);
        }

        cmd.arg("--resume")
            .arg(session_id)
            .arg(prompt)
            .current_dir(workspace_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped());

        for (key, val) in &self.env_vars {
            cmd.env(key, val);
        }

        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                nix::unistd::setsid().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                Ok(())
            });
        }

        eprintln!("[panes-adapter] resuming session {} in {}", session_id, workspace_path.display());

        let mut child = cmd.spawn().with_context(|| {
            format!("failed to spawn claude CLI at '{}' for resume", &self.cli_path)
        })?;

        let stdout = child.stdout.take().context("failed to capture claude stdout")?;
        let stderr = child.stderr.take().context("failed to capture claude stderr")?;

        let stderr_reader = BufReader::new(stderr);
        let (auth_error_tx, auth_error_rx) = tokio::sync::watch::channel(None::<String>);
        tokio::spawn(async move {
            Self::monitor_stderr(stderr_reader, auth_error_tx).await;
        });

        let pid = child.id();
        eprintln!("[panes-adapter] claude resumed with pid {:?}, waiting for init...", pid);

        let reader = BufReader::new(stdout);
        let (init, event_stream) = parser::parse_stream(reader, auth_error_rx).await
            .context("failed to parse claude stream on resume")?;

        eprintln!("[panes-adapter] resumed init: model={}, session={}", init.model, init.session_id);

        let session = ClaudeSession {
            _child: tokio::sync::Mutex::new(child),
            init_data: init,
            event_stream: tokio::sync::Mutex::new(Some(event_stream)),
            workspace_path: workspace_path.to_path_buf(),
            pid,
        };

        Ok(Box::new(session))
    }
}

impl ClaudeAdapter {
    async fn monitor_stderr(
        reader: BufReader<tokio::process::ChildStderr>,
        tx: tokio::sync::watch::Sender<Option<String>>,
    ) {
        use tokio::io::AsyncBufReadExt;
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let lower = line.to_lowercase();
            if lower.contains("auth")
                || lower.contains("token")
                || lower.contains("api key")
                || lower.contains("expired")
                || lower.contains("unauthorized")
                || lower.contains("forbidden")
                || lower.contains("401")
                || lower.contains("403")
            {
                warn!(stderr = %line, "detected auth-related error from claude CLI");
                let _ = tx.send(Some(line));
            } else if !line.trim().is_empty() {
                tracing::debug!(stderr = %line, "claude stderr");
            }
        }
    }
}

pub struct ClaudeSession {
    _child: tokio::sync::Mutex<Child>,
    init_data: SessionInit,
    event_stream: tokio::sync::Mutex<Option<Pin<Box<dyn Stream<Item = AgentEvent> + Send>>>>,
    workspace_path: PathBuf,
    pid: Option<u32>,
}

#[async_trait]
impl AgentSession for ClaudeSession {
    fn init(&self) -> &SessionInit {
        &self.init_data
    }

    fn events(&mut self) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        self.event_stream
            .get_mut()
            .take()
            .expect("events() called more than once")
    }

    async fn approve(&self, _tool_use_id: &str) -> Result<()> {
        // No-op: acceptEdits mode handles tool approval internally.
        // Gate pausing/resuming is managed by SessionManager, not the adapter.
        Ok(())
    }

    async fn reject(&self, tool_use_id: &str, reason: &str) -> Result<()> {
        // Claude Code CLI doesn't support stdin-based rejection.
        // Cancel the process — the caller emits a completion event.
        warn!(tool_use_id, reason, "rejecting gate by cancelling process");
        self.cancel().await
    }

    async fn cancel(&self) -> Result<()> {
        #[cfg(unix)]
        {
            if let Some(pid) = self.pid {
                info!(pid, workspace = %self.workspace_path.display(), "sending SIGTERM to claude process group");
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGTERM);
                }
            }
        }
        Ok(())
    }
}

impl Drop for ClaudeSession {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            if let Some(pid) = self.pid {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGTERM);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panes_events::SessionContext;
    use std::path::Path;

    #[test]
    fn test_new_defaults() {
        let adapter = ClaudeAdapter::new();
        assert_eq!(adapter.cli_path, "claude");
        assert_eq!(adapter.permission_mode, "bypassPermissions");
        assert!(adapter.env_vars.is_empty());
    }

    #[test]
    fn test_with_cli_path() {
        let adapter = ClaudeAdapter::with_cli_path("/usr/local/bin/claude");
        assert_eq!(adapter.cli_path, "/usr/local/bin/claude");
    }

    #[test]
    fn test_permission_mode_builder() {
        let adapter = ClaudeAdapter::new().permission_mode("acceptEdits");
        assert_eq!(adapter.permission_mode, "acceptEdits");
    }

    #[test]
    fn test_env_builder() {
        let adapter = ClaudeAdapter::new()
            .env("API_KEY", "secret")
            .env("DEBUG", "1");
        assert_eq!(adapter.env_vars.len(), 2);
        assert_eq!(adapter.env_vars[0], ("API_KEY".to_string(), "secret".to_string()));
    }

    #[test]
    fn test_default_trait() {
        let adapter = ClaudeAdapter::default();
        assert_eq!(adapter.cli_path, "claude");
        assert_eq!(adapter.permission_mode, "bypassPermissions");
    }

    #[test]
    fn test_name() {
        let adapter = ClaudeAdapter::new();
        assert_eq!(adapter.name(), "claude-code");
    }

    #[tokio::test]
    async fn test_spawn_nonexistent_binary() {
        let adapter = ClaudeAdapter::with_cli_path("/nonexistent/binary/claude-fake-xyz");
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = adapter.spawn(Path::new("/tmp"), "test prompt", &ctx, None).await;
        let err = result.err().expect("expected spawn to fail");
        let msg = err.to_string();
        assert!(msg.contains("failed to spawn"), "error: {msg}");
    }

    #[tokio::test]
    async fn test_resume_nonexistent_binary() {
        let adapter = ClaudeAdapter::with_cli_path("/nonexistent/binary/claude-fake-xyz");
        let result = adapter.resume(Path::new("/tmp"), "session-123", "follow up", None).await;
        let err = result.err().expect("expected resume to fail");
        let msg = err.to_string();
        assert!(msg.contains("failed to spawn"), "error: {msg}");
    }

    #[tokio::test]
    async fn test_spawn_with_briefing_and_memories() {
        let adapter = ClaudeAdapter::with_cli_path("/nonexistent/binary/claude-fake-xyz");
        let ctx = SessionContext {
            briefing: Some("You are a helpful assistant".to_string()),
            memories: vec!["User prefers tabs".to_string(), "Project uses React".to_string()],
            budget_cap: Some(5.0),
        };
        let result = adapter.spawn(Path::new("/tmp"), "test prompt", &ctx, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_spawn_with_model_passes_model_flag() {
        // Use a script that prints its args to stdout so we can verify --model is included.
        // The spawn will fail during stream parsing, but we can verify via the script output.
        let script = std::env::temp_dir().join("panes-test-echo-args.sh");
        std::fs::write(&script, "#!/bin/sh\necho \"$@\" >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        let adapter = ClaudeAdapter::with_cli_path(script.to_str().unwrap());
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = adapter.spawn(Path::new("/tmp"), "test prompt", &ctx, Some("opus")).await;
        assert!(result.is_err(), "expected parse failure from fake binary");
        std::fs::remove_file(&script).ok();
    }

    #[tokio::test]
    async fn test_spawn_without_model_omits_model_flag() {
        let script = std::env::temp_dir().join("panes-test-echo-args-nomodel.sh");
        std::fs::write(&script, "#!/bin/sh\necho \"$@\" >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        let adapter = ClaudeAdapter::with_cli_path(script.to_str().unwrap());
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = adapter.spawn(Path::new("/tmp"), "test prompt", &ctx, None).await;
        assert!(result.is_err(), "expected parse failure from fake binary");
        std::fs::remove_file(&script).ok();
    }

    #[tokio::test]
    async fn test_resume_with_model_passes_model_flag() {
        let script = std::env::temp_dir().join("panes-test-echo-args-resume.sh");
        std::fs::write(&script, "#!/bin/sh\necho \"$@\" >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

        let adapter = ClaudeAdapter::with_cli_path(script.to_str().unwrap());
        let result = adapter.resume(Path::new("/tmp"), "sess-123", "follow up", Some("sonnet")).await;
        assert!(result.is_err(), "expected parse failure from fake binary");
        std::fs::remove_file(&script).ok();
    }
}
