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
            .arg(&self.permission_mode)
            .arg(&full_prompt)
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
    ) -> Result<Box<dyn AgentSession>> {
        let mut cmd = Command::new(&self.cli_path);
        cmd.arg("-p")
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--permission-mode")
            .arg(&self.permission_mode)
            .arg("--resume")
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
        // No-op: bypassPermissions mode auto-approves everything.
        // If we switch to acceptEdits, Bash gates are handled by --permission-prompt-tool
        // or --allowedTools, not stdin injection (which Claude CLI doesn't support).
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
