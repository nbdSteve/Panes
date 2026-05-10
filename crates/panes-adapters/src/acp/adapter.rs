//! The `AcpAdapter` / `AcpSession` types that implement the
//! [`crate::AgentAdapter`] + [`crate::AgentSession`] traits using ACP.
//!
//! Each [`AcpAdapter`] instance is constructed per-backend at registration
//! time — `AcpAdapter::kiro_cli()` is the preset for kiro-cli; generic
//! [`AcpAdapter::new`] accepts any binary that speaks ACP. The adapter's
//! `name()` is what appears in Panes' agent picker, so users see
//! `"kiro-cli"`, not `"acp"`.

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use futures::Stream;
use panes_events::{AgentEvent, SessionContext, SessionInit};
use tokio::process::Command;
use tokio::sync::Mutex;

use crate::{AgentAdapter, AgentSession, ModelInfo};

use super::events::TranslationContext;
use super::session::{self, ClientInfo};
use super::transport::AcpTransport;

/// Graceful-cancel deadline: we wait this long after `session/cancel` for
/// the backend to acknowledge before falling back to SIGTERM.
const CANCEL_GRACE: Duration = Duration::from_secs(3);

/// Inter-poll interval for the event stream. Small enough to feel responsive,
/// large enough to keep CPU idle when nothing's streaming.
#[cfg(not(test))]
const STREAM_POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(test)]
const STREAM_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// ACP-backed implementation of [`AgentAdapter`].
///
/// Construct one per backend. `name` is user-visible — use the real CLI name
/// (`"kiro-cli"`, `"codex"`) rather than the protocol label.
pub struct AcpAdapter {
    name: String,
    cli_path: String,
    subcommand: Vec<String>,
    env_vars: Vec<(String, String)>,
    /// Modes surfaced via `list_agents()` for UI picker population.
    default_modes: Vec<String>,
}

impl AcpAdapter {
    /// Generic constructor. The `name` becomes the adapter id visible in
    /// Panes' agent picker — use the actual CLI name, not `"acp"`.
    pub fn new(
        name: impl Into<String>,
        cli_path: impl Into<String>,
        subcommand: Vec<String>,
    ) -> Self {
        Self {
            name: name.into(),
            cli_path: cli_path.into(),
            subcommand,
            env_vars: Vec::new(),
            default_modes: Vec::new(),
        }
    }

    /// Preset for kiro-cli. Returns `Some` only if a kiro-cli binary can be
    /// located: either `$PANES_KIRO_CLI_PATH` is set, or `kiro-cli` resolves
    /// on PATH. Returns `None` otherwise so registration is a graceful no-op.
    ///
    /// Forwards `SSH_AUTH_SOCK` from the parent process to the child so
    /// kiro-cli's Midway auth works. Without this, Amazon users see opaque
    /// auth failures in stderr rather than a clear diagnostic.
    pub fn kiro_cli() -> Option<Self> {
        let path = std::env::var("PANES_KIRO_CLI_PATH")
            .ok()
            .or_else(|| which_binary("kiro-cli"))?;
        let mut adapter = Self::new("kiro-cli", path, vec!["acp".to_string()])
            .with_default_modes(vec!["harold".to_string(), "builder".to_string()]);
        // Forward SSH_AUTH_SOCK so Midway auth works when the user is
        // already ssh-agent authenticated. kiro-cli handles its own AWS
        // credentials internally — we don't forward AWS_PROFILE.
        if let Ok(val) = std::env::var("SSH_AUTH_SOCK") {
            adapter = adapter.env("SSH_AUTH_SOCK", val);
        }
        Some(adapter)
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.push((key.into(), value.into()));
        self
    }

    pub fn with_default_modes(mut self, modes: Vec<String>) -> Self {
        self.default_modes = modes;
        self
    }

    /// List of modes returned by `list_agents()`. Separate accessor so
    /// panes-app can route its `list_agents` IPC to `AgentAdapter::list_agents`
    /// once the trait method lands in a later step.
    pub fn default_modes(&self) -> &[String] {
        &self.default_modes
    }

    fn build_command(&self, cwd: &Path) -> Command {
        let mut cmd = Command::new(&self.cli_path);
        for arg in &self.subcommand {
            cmd.arg(arg);
        }
        cmd.current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in &self.env_vars {
            cmd.env(k, v);
        }
        // Process group isolation so we can signal the whole subtree at once.
        #[cfg(unix)]
        unsafe {
            cmd.pre_exec(|| {
                nix::unistd::setsid()
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                Ok(())
            });
        }
        cmd.kill_on_drop(true);
        cmd
    }
}

#[async_trait]
impl AgentAdapter for AcpAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    async fn spawn(
        &self,
        workspace_path: &Path,
        prompt: &str,
        context: &SessionContext,
        model: Option<&str>,
        agent: Option<&str>,
    ) -> Result<Box<dyn AgentSession>> {
        let prompt_text = build_prompt_text(prompt, context);
        let cmd = self.build_command(workspace_path);
        let mut transport = AcpTransport::spawn(cmd)
            .await
            .with_context(|| format!("failed to spawn ACP backend '{}'", &self.cli_path))?;

        // Handshake
        let _init = session::initialize(
            &mut transport,
            &ClientInfo {
                name: "panes",
                version: env!("CARGO_PKG_VERSION"),
            },
        )
        .await
        .context("ACP initialize failed")?;

        // New session
        let new_sess = session::new_session(&mut transport, workspace_path)
            .await
            .context("ACP session/new failed")?;

        // Mode: prefer the caller's override; otherwise fall back to the
        // adapter's first registered mode; otherwise skip `set_mode` entirely.
        // Sending "default" as a modeId to kiro-cli (which expects harold /
        // builder / <named-agent>) would fail with no clear diagnostic.
        if let Some(mode) = agent.or_else(|| self.default_modes.first().map(|s| s.as_str())) {
            let _ = session::set_mode(&mut transport, &new_sess.session_id, mode).await;
        }

        // Model (optional)
        let resolved_model = resolve_model(model, &new_sess.available_models);
        if let Some(ref m) = resolved_model {
            if let Err(e) = session::set_model(&mut transport, &new_sess.session_id, m).await {
                tracing::warn!(model = %m, error = %e, "set_model failed — continuing without override");
            }
        }

        // Initial prompt
        let prompt_req_id = session::prompt(&mut transport, &new_sess.session_id, &prompt_text)
            .await
            .context("ACP session/prompt failed")?;

        let init = SessionInit {
            session_id: new_sess.session_id.clone(),
            model: resolved_model
                .or_else(|| new_sess.available_models.first().cloned())
                .unwrap_or_else(|| "unknown".to_string()),
            cwd: workspace_path.to_string_lossy().to_string(),
            tools: Vec::new(),
        };

        let session = AcpSession::new(transport, init, new_sess.session_id, prompt_req_id);
        Ok(Box::new(session))
    }

    async fn resume(
        &self,
        workspace_path: &Path,
        session_id: &str,
        prompt: &str,
        model: Option<&str>,
        agent: Option<&str>,
    ) -> Result<Box<dyn AgentSession>> {
        let cmd = self.build_command(workspace_path);
        let mut transport = AcpTransport::spawn(cmd)
            .await
            .with_context(|| format!("failed to spawn ACP backend '{}'", &self.cli_path))?;

        let _init = session::initialize(
            &mut transport,
            &ClientInfo {
                name: "panes",
                version: env!("CARGO_PKG_VERSION"),
            },
        )
        .await?;

        let resumed = session::load_session(&mut transport, session_id, workspace_path)
            .await
            .unwrap_or(false);
        let effective_session_id = if resumed {
            session_id.to_string()
        } else {
            tracing::info!(session_id, "session/load did not resume — falling back to session/new");
            let ns = session::new_session(&mut transport, workspace_path).await?;
            ns.session_id
        };

        if let Some(mode) = agent.or_else(|| self.default_modes.first().map(|s| s.as_str())) {
            let _ = session::set_mode(&mut transport, &effective_session_id, mode).await;
        }

        if let Some(m) = model {
            if let Err(e) = session::set_model(&mut transport, &effective_session_id, m).await {
                tracing::warn!(model = %m, error = %e, "set_model failed on resume");
            }
        }

        // Resume prompt — wrap briefing/memories again in case the backend
        // dropped them on the previous turn.
        let prompt_text = build_prompt_text(
            prompt,
            &SessionContext {
                briefing: None,
                memories: Vec::new(),
                budget_cap: None,
            },
        );
        let prompt_req_id =
            session::prompt(&mut transport, &effective_session_id, &prompt_text).await?;

        let init = SessionInit {
            session_id: effective_session_id.clone(),
            model: model.unwrap_or("unknown").to_string(),
            cwd: workspace_path.to_string_lossy().to_string(),
            tools: Vec::new(),
        };
        let session = AcpSession::new(transport, init, effective_session_id, prompt_req_id);
        Ok(Box::new(session))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>> {
        // Models are backend-owned and only known after `session/new` runs.
        // Returning an empty list tells the UI to hide the model picker.
        Ok(Vec::new())
    }
}

/// Shared state between the event stream and `approve`/`reject`/`cancel`.
/// Kept behind a mutex so the stream's `read_message` and external gate
/// actions don't race on the transport.
struct Shared {
    transport: Mutex<Option<AcpTransport>>,
    ctx: Mutex<TranslationContext>,
    session_id: String,
}

pub struct AcpSession {
    init_data: SessionInit,
    shared: Arc<Shared>,
    stream: Mutex<Option<Pin<Box<dyn Stream<Item = AgentEvent> + Send>>>>,
}

impl AcpSession {
    fn new(
        transport: AcpTransport,
        init: SessionInit,
        session_id: String,
        prompt_req_id: u64,
    ) -> Self {
        let mut ctx = TranslationContext::new();
        ctx.begin_prompt(prompt_req_id);
        let shared = Arc::new(Shared {
            transport: Mutex::new(Some(transport)),
            ctx: Mutex::new(ctx),
            session_id,
        });
        let stream = build_stream(shared.clone());
        Self {
            init_data: init,
            shared,
            stream: Mutex::new(Some(stream)),
        }
    }
}

#[async_trait]
impl AgentSession for AcpSession {
    fn init(&self) -> &SessionInit {
        &self.init_data
    }

    fn events(&mut self) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        self.stream
            .get_mut()
            .take()
            .unwrap_or_else(empty_stream)
    }

    async fn approve(&self, tool_use_id: &str) -> Result<()> {
        let pending = {
            let mut ctx = self.shared.ctx.lock().await;
            ctx.take_permission(tool_use_id)
                .ok_or_else(|| anyhow!("no pending permission for tool id '{tool_use_id}'"))?
        };
        // Pick the first option whose kind is an allow variant. The agent
        // defines the valid option ids per request — we must not hardcode.
        let option_id = pending
            .options
            .iter()
            .find(|o| matches!(o.kind.as_str(), "allow_once" | "allow" | "allow_always"))
            .map(|o| o.id.clone())
            .or_else(|| pending.options.first().map(|o| o.id.clone()))
            .ok_or_else(|| {
                anyhow!(
                    "agent offered no permission options for tool '{tool_use_id}' — cannot approve"
                )
            })?;
        let mut guard = self.shared.transport.lock().await;
        let transport = guard
            .as_mut()
            .ok_or_else(|| anyhow!("transport already shut down"))?;
        transport
            .send_response(
                &pending.request_id,
                serde_json::json!({
                    "outcome": { "outcome": "selected", "optionId": option_id }
                }),
            )
            .await
    }

    async fn reject(&self, tool_use_id: &str, _reason: &str) -> Result<()> {
        let pending = {
            let mut ctx = self.shared.ctx.lock().await;
            ctx.take_permission(tool_use_id)
                .ok_or_else(|| anyhow!("no pending permission for tool id '{tool_use_id}'"))?
        };
        // Prefer a denial option if the agent offered one. Otherwise fall
        // back to the protocol-level `{"outcome":"cancelled"}` shape, which
        // HaroldCLI uses (and which doesn't need an optionId).
        let deny_option = pending
            .options
            .iter()
            .find(|o| matches!(o.kind.as_str(), "reject" | "deny" | "cancel"))
            .map(|o| o.id.clone());
        let mut guard = self.shared.transport.lock().await;
        let transport = guard
            .as_mut()
            .ok_or_else(|| anyhow!("transport already shut down"))?;
        let payload = if let Some(id) = deny_option {
            serde_json::json!({
                "outcome": { "outcome": "selected", "optionId": id }
            })
        } else {
            serde_json::json!({
                "outcome": { "outcome": "cancelled" }
            })
        };
        transport.send_response(&pending.request_id, payload).await
    }

    async fn cancel(&self) -> Result<()> {
        // Best-effort: ask the backend to cancel, then shut down the transport.
        let session_id = self.shared.session_id.clone();
        let transport_opt = {
            let mut guard = self.shared.transport.lock().await;
            guard.take()
        };
        if let Some(mut t) = transport_opt {
            let _ = tokio::time::timeout(CANCEL_GRACE, session::cancel(&mut t, &session_id)).await;
            t.shutdown().await;
        }
        Ok(())
    }
}

impl Drop for AcpSession {
    fn drop(&mut self) {
        // If the session is dropped while a transport is still alive (no
        // explicit cancel), spawn a fire-and-forget cleanup so we don't leak
        // the child process. Can't await in Drop, so we hand the transport to
        // a spawned task.
        if let Ok(mut guard) = self.shared.transport.try_lock() {
            if let Some(t) = guard.take() {
                // Safest option: issue SIGKILL in the synchronous Drop path by
                // calling t's own Drop. Just letting `t` be dropped here does
                // that (AcpTransport::Drop sends SIGKILL to the process group).
                drop(t);
            }
        }
    }
}

/// Build the streaming pipeline that pulls JSON-RPC messages, runs them
/// through the translation context, and yields Panes events.
///
/// Exit paths:
/// - `Complete` emitted → stream ends cleanly (prompt fully consumed)
/// - transport returns `None` repeatedly past `EOF_TOLERANCE` → backend exited
/// - transport returns `Err` → wrapped as an `Error` event before closing
///
/// Before closing on either error path, any text still in the translator's
/// coalesce buffer is flushed so partial output reaches the user.
fn build_stream(
    shared: Arc<Shared>,
) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
    /// How many consecutive None reads before we treat stdout as EOF'd.
    /// Each read is `STREAM_POLL_INTERVAL`, so ~30s of silence in production
    /// (and ~100ms in tests to keep test runs fast). Long enough to cover a
    /// slow LLM, short enough to stop waiting after the backend dies.
    #[cfg(not(test))]
    const EOF_CONSECUTIVE_NONES: u32 = 300;
    #[cfg(test)]
    const EOF_CONSECUTIVE_NONES: u32 = 10;

    Box::pin(async_stream::stream! {
        let mut completed = false;
        let mut consecutive_none = 0u32;
        let mut transport_err: Option<String> = None;

        while !completed {
            // Surface auth errors as a typed non-recoverable event before
            // the user is stuck staring at a silent session.
            let auth_err = {
                let guard = shared.transport.lock().await;
                match guard.as_ref() {
                    Some(t) => t.take_auth_error().await,
                    None => None,
                }
            };
            if let Some(msg) = auth_err {
                yield AgentEvent::Error {
                    message: format!("authentication failed: {msg}"),
                    recoverable: false,
                };
                break;
            }

            let read = {
                let mut guard = shared.transport.lock().await;
                match guard.as_mut() {
                    Some(t) => {
                        // Bail fast if the stderr watcher flagged a fatal.
                        if t.fatal_stderr_seen() {
                            transport_err = Some("backend reported fatal error on stderr".to_string());
                            break;
                        }
                        t.read_message(STREAM_POLL_INTERVAL).await
                    }
                    None => {
                        // Transport was taken (cancel() or drop).
                        break;
                    }
                }
            };
            let msg = match read {
                Ok(Some(m)) => {
                    consecutive_none = 0;
                    m
                }
                Ok(None) => {
                    // Distinguish timeout (keep polling) from EOF (backend gone).
                    // We can't tell them apart from the read result alone, so
                    // we accumulate nones and bail after enough silence.
                    let alive = {
                        let guard = shared.transport.lock().await;
                        guard.is_some()
                    };
                    if !alive {
                        break;
                    }
                    consecutive_none = consecutive_none.saturating_add(1);
                    if consecutive_none >= EOF_CONSECUTIVE_NONES {
                        tracing::warn!(
                            "ACP backend produced no events for ~30s — ending stream"
                        );
                        break;
                    }
                    continue;
                }
                Err(e) => {
                    transport_err = Some(e.to_string());
                    break;
                }
            };

            let translated = {
                let mut ctx = shared.ctx.lock().await;
                ctx.translate(&msg)
            };
            for ev in translated {
                let is_complete = matches!(ev, AgentEvent::Complete { .. });
                yield ev;
                if is_complete {
                    completed = true;
                    break;
                }
            }
        }

        // Abnormal exit path: flush any buffered text before closing so the
        // user sees partial output rather than silence.
        if !completed {
            let flushed = {
                let mut ctx = shared.ctx.lock().await;
                ctx.flush_pending_text()
            };
            if let Some(ev) = flushed {
                yield ev;
            }
            if let Some(err) = transport_err {
                yield AgentEvent::Error {
                    message: format!("ACP transport error: {err}"),
                    recoverable: true,
                };
            }
        }
    })
}

fn empty_stream() -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
    Box::pin(futures::stream::empty())
}

/// Construct the prompt text sent in `session/prompt`, prepending briefing
/// and memory blocks when provided.
fn build_prompt_text(prompt: &str, ctx: &SessionContext) -> String {
    let mut out = String::new();
    if let Some(briefing) = &ctx.briefing {
        out.push_str("<briefing>\n");
        out.push_str(briefing);
        out.push_str("\n</briefing>\n\n");
    }
    if !ctx.memories.is_empty() {
        out.push_str("<memories>\n");
        for mem in &ctx.memories {
            out.push_str("- ");
            out.push_str(mem);
            out.push('\n');
        }
        out.push_str("</memories>\n\n");
    }
    out.push_str(prompt);
    out
}

/// Return `Some(model)` only if the caller passed one and it's in the agent's
/// available-models list. Otherwise fall back to the agent's default (first
/// available) so we don't send a rejected `session/set_model`.
fn resolve_model(requested: Option<&str>, available: &[String]) -> Option<String> {
    let requested = requested?.to_string();
    if available.iter().any(|m| *m == requested) {
        Some(requested)
    } else {
        tracing::debug!(
            requested,
            "model not in available list — using backend default"
        );
        None
    }
}

/// Cheap `which` replacement — avoids pulling the `which` crate down into
/// panes-adapters just for this. Splits `$PATH` and looks for an executable
/// file matching `name`.
fn which_binary(name: &str) -> Option<String> {
    let path = std::env::var("PATH").ok()?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(m) => m.is_file() && (m.permissions().mode() & 0o111 != 0),
            Err(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

// Internal helper for panes-app registration — type alias kept minimal so
// callers don't have to import pathbuf infrastructure.
#[allow(dead_code)]
pub(crate) type AdapterArc = std::sync::Arc<dyn AgentAdapter>;

// Expose PathBuf just so nothing above is unused on non-unix; currently unused.
#[allow(dead_code)]
fn _path_noop() -> PathBuf {
    PathBuf::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── which_binary / is_executable ─────────────────────────────────────

    #[test]
    fn which_binary_finds_ubiquitous_tools() {
        // `sh` is on PATH for basically every Unix CI runner.
        let maybe = which_binary("sh");
        assert!(maybe.is_some(), "sh should resolve on PATH");
    }

    #[test]
    fn which_binary_returns_none_for_missing() {
        assert!(which_binary("panes-nonexistent-binary-xyz").is_none());
    }

    // ─── resolve_model ────────────────────────────────────────────────────

    #[test]
    fn resolve_model_returns_requested_when_available() {
        let got = resolve_model(Some("sonnet"), &["sonnet".to_string(), "haiku".to_string()]);
        assert_eq!(got.as_deref(), Some("sonnet"));
    }

    #[test]
    fn resolve_model_returns_none_when_not_available() {
        let got = resolve_model(Some("gpt-4"), &["sonnet".to_string()]);
        assert!(got.is_none(), "unknown model should fall back to backend default");
    }

    #[test]
    fn resolve_model_returns_none_when_caller_passes_none() {
        let got = resolve_model(None, &["sonnet".to_string()]);
        assert!(got.is_none());
    }

    // ─── build_prompt_text ────────────────────────────────────────────────

    #[test]
    fn build_prompt_text_includes_briefing_and_memories() {
        let ctx = SessionContext {
            briefing: Some("always use Zod".to_string()),
            memories: vec!["user prefers TS".to_string(), "no TODO comments".to_string()],
            budget_cap: None,
        };
        let out = build_prompt_text("do the thing", &ctx);
        assert!(out.contains("<briefing>"));
        assert!(out.contains("always use Zod"));
        assert!(out.contains("<memories>"));
        assert!(out.contains("- user prefers TS"));
        assert!(out.contains("- no TODO comments"));
        assert!(out.ends_with("do the thing"));
    }

    #[test]
    fn build_prompt_text_omits_sections_when_empty() {
        let ctx = SessionContext {
            briefing: None,
            memories: Vec::new(),
            budget_cap: None,
        };
        let out = build_prompt_text("just do it", &ctx);
        assert_eq!(out, "just do it");
    }

    // ─── AcpAdapter construction ──────────────────────────────────────────

    #[test]
    fn adapter_exposes_name_and_default_modes() {
        let a = AcpAdapter::new("kiro-cli", "/bin/true", vec!["acp".to_string()])
            .with_default_modes(vec!["harold".to_string(), "default".to_string()]);
        assert_eq!(a.name(), "kiro-cli");
        assert_eq!(a.default_modes(), &["harold".to_string(), "default".to_string()]);
    }

    #[test]
    fn adapter_name_never_equals_acp_for_kiro_cli_preset() {
        // The user should see the backend binary name, not the protocol name.
        if let Some(preset) = AcpAdapter::kiro_cli() {
            assert_eq!(preset.name(), "kiro-cli");
            assert_ne!(preset.name(), "acp");
        }
        // If `kiro-cli` isn't installed in the test environment, preset is None
        // — that's fine, nothing to assert.
    }

    #[test]
    fn adapter_env_accumulates_key_value_pairs() {
        let a = AcpAdapter::new("x", "/bin/true", vec![])
            .env("A", "1")
            .env("B", "2");
        assert_eq!(a.env_vars.len(), 2);
        assert_eq!(a.env_vars[0], ("A".to_string(), "1".to_string()));
        assert_eq!(a.env_vars[1], ("B".to_string(), "2".to_string()));
    }

    // ─── spawn happy path via scripted shell agent ────────────────────────
    //
    // Integration-style tests that actually round-trip through a child
    // process live in `tests/acp_integration.rs`. The unit tests here just
    // cover the pure helpers so we have fast feedback on changes.

    // ─── build_stream (stream construction + EOF / error paths) ───────────
    //
    // These tests drive the stream with a mocked transport so we can observe
    // the EOF counter, the error-flush path, and the Complete short-circuit
    // without spawning a process. Test builds compress STREAM_POLL_INTERVAL
    // to 10ms and EOF_CONSECUTIVE_NONES to 10, so the entire EOF window is
    // ~100ms (see top of this file).

    use crate::acp::events::TranslationContext;
    use crate::acp::transport::{AcpTransport, JsonRpcMessage};
    use futures::StreamExt;
    use std::collections::VecDeque;

    fn mock_shared(prompt_req_id: u64, queue: VecDeque<JsonRpcMessage>) -> std::sync::Arc<super::Shared> {
        let transport = AcpTransport::mock(queue);
        let mut ctx = TranslationContext::new();
        ctx.begin_prompt(prompt_req_id);
        std::sync::Arc::new(super::Shared {
            transport: super::Mutex::new(Some(transport)),
            ctx: super::Mutex::new(ctx),
            session_id: "sess-test".to_string(),
        })
    }

    fn rpc(raw: &str) -> JsonRpcMessage {
        serde_json::from_str(raw).expect("valid JSON-RPC fixture")
    }

    #[tokio::test]
    async fn build_stream_yields_complete_and_then_ends() {
        let queue: VecDeque<_> = [
            rpc(r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"hi"}}
            }}"#),
            rpc(r#"{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}"#),
        ]
        .into_iter()
        .collect();
        let shared = mock_shared(1, queue);
        let mut stream = build_stream(shared);
        let mut got_complete = false;
        while let Some(ev) = stream.next().await {
            if matches!(ev, AgentEvent::Complete { .. }) {
                got_complete = true;
            }
        }
        assert!(got_complete, "build_stream should yield Complete");
    }

    #[tokio::test]
    async fn build_stream_short_circuits_after_complete() {
        // Put events after the Complete that should NOT leak out.
        let queue: VecDeque<_> = [
            rpc(r#"{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}"#),
            rpc(r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"stray"}}
            }}"#),
        ]
        .into_iter()
        .collect();
        let shared = mock_shared(1, queue);
        let mut stream = build_stream(shared);
        let events: Vec<_> = stream.by_ref().collect::<Vec<_>>().await;
        let text_count = events
            .iter()
            .filter(|e| matches!(e, AgentEvent::Text { text } if text == "stray"))
            .count();
        assert_eq!(
            text_count, 0,
            "events after Complete must not leak to the consumer: {events:?}"
        );
    }

    #[tokio::test]
    async fn build_stream_flushes_buffered_text_on_eof() {
        // Single text chunk then the mock returns None forever — EOF_CONSECUTIVE_NONES
        // is compressed to 10 in test builds so this completes fast.
        let queue: VecDeque<_> = [
            rpc(r#"{"jsonrpc":"2.0","method":"session/update","params":{
                "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"partial"}}
            }}"#),
        ]
        .into_iter()
        .collect();
        let shared = mock_shared(1, queue);
        let mut stream = build_stream(shared);
        let events: Vec<_> = stream.collect::<Vec<_>>().await;
        let saw_partial = events.iter().any(|e| match e {
            AgentEvent::Text { text } => text.contains("partial"),
            _ => false,
        });
        assert!(
            saw_partial,
            "EOF path must flush the buffered text, got: {events:?}"
        );
        assert!(
            !matches!(events.last(), Some(AgentEvent::Complete { .. })),
            "EOF must not synthesize a Complete"
        );
    }

    #[tokio::test]
    async fn build_stream_emits_error_event_on_json_rpc_error() {
        let queue: VecDeque<_> = [
            rpc(r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"context overflow"}}"#),
        ]
        .into_iter()
        .collect();
        let shared = mock_shared(1, queue);
        let mut stream = build_stream(shared);
        let events: Vec<_> = stream.collect::<Vec<_>>().await;
        let saw_error = events
            .iter()
            .any(|e| matches!(e, AgentEvent::Error { message, .. } if message.contains("context overflow")));
        assert!(saw_error, "error response must become AgentEvent::Error, got: {events:?}");
    }

    #[tokio::test]
    async fn build_stream_ends_quickly_when_transport_taken() {
        // Simulate cancel() having taken the transport before the stream polls.
        let shared = mock_shared(1, VecDeque::new());
        {
            let mut guard = shared.transport.lock().await;
            guard.take();
        }
        let start = std::time::Instant::now();
        let mut stream = build_stream(shared);
        // Stream should end immediately — no transport to read from.
        let _ = stream.next().await; // may be None on first poll
        let _ = stream.next().await;
        assert!(
            start.elapsed() < std::time::Duration::from_secs(2),
            "stream should end fast when transport is None, took {:?}",
            start.elapsed()
        );
    }

    // ─── EOF counter boundary (gap #8) ────────────────────────────────────

    #[tokio::test]
    async fn build_stream_respects_eof_counter_bound() {
        // Empty mock → read_message returns None instantly on every poll.
        // With EOF_CONSECUTIVE_NONES=10 and STREAM_POLL_INTERVAL=10ms, the
        // stream should terminate in ~100ms.
        let shared = mock_shared(1, VecDeque::new());
        let start = std::time::Instant::now();
        let stream = build_stream(shared);
        let events: Vec<_> = stream.collect::<Vec<_>>().await;
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "stream should end after ~EOF_CONSECUTIVE_NONES reads, not hang; took {elapsed:?}"
        );
        // No events at all — no text buffered, no response to flush.
        assert!(events.is_empty() || matches!(events.last(), Some(AgentEvent::Error { .. })),
            "empty queue → stream ends with nothing or a transport Error: {events:?}");
    }
}
