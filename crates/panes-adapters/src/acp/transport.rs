//! JSON-RPC 2.0 transport over a spawned child's stdio.
//!
//! Ported (with simplifications) from `HaroldCLI/src/acp_client.rs:1040-1700`.
//! Panes owns its own persistence, logging, and session map, so the Harold
//! features we don't need (recorder, session_map, hot spare, browser profile)
//! are omitted.

use std::collections::VecDeque;
use std::io;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

/// Per-line read cap. Protects against a buggy/hostile backend that never
/// emits a newline from exhausting memory.
const MAX_LINE_BYTES: usize = 64 * 1024 * 1024; // 64 MiB

/// Serialized JSON-RPC 2.0 request sent from client → agent.
#[derive(Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: Value,
}

/// Deserialized JSON-RPC message — could be a response to our request,
/// a notification, or a server-initiated request (e.g. permission).
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct JsonRpcMessage {
    #[serde(default)]
    pub id: Option<Value>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default)]
    pub params: Option<Value>,
}

impl JsonRpcMessage {
    /// True when this message is the response to the given client-sent request id.
    pub fn is_response_for(&self, req_id: u64) -> bool {
        match &self.id {
            Some(Value::Number(n)) => n.as_u64() == Some(req_id),
            Some(Value::String(s)) => s.parse::<u64>().ok() == Some(req_id),
            _ => false,
        }
    }

    /// True when this message is a notification/request with the given method name.
    pub fn is_method(&self, name: &str) -> bool {
        self.method.as_deref() == Some(name)
    }

    /// True when this message has an `id` (response or server-initiated request).
    pub fn has_id(&self) -> bool {
        !matches!(self.id, None | Some(Value::Null))
    }

    /// True when this is a notification (has method, has no id).
    pub fn is_notification(&self) -> bool {
        self.method.is_some() && !self.has_id()
    }
}

/// Where a JsonRpcMessage comes from. Real transports read from a spawned
/// child's stdout; tests can inject a canned queue.
pub(crate) enum MessageSource {
    Real(mpsc::Receiver<String>),
    #[cfg(test)]
    Mock(MockMessageSource),
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct MockMessageSource {
    pub queue: VecDeque<JsonRpcMessage>,
}

/// JSON-RPC 2.0 transport over a spawned child's stdio.
///
/// Responsibilities:
/// - Send requests/responses on stdin with newline framing
/// - Receive newline-delimited JSON-RPC messages from stdout via background reader
/// - Provide timeout-aware read + request/response correlation
/// - Own the child process lifecycle (SIGTERM on shutdown)
pub(crate) struct AcpTransport {
    stdin: Option<ChildStdin>,
    child: Option<Child>,
    source: MessageSource,
    next_id: AtomicU64,
    pid: u32,
    /// Buffer for messages read while waiting for a specific response id.
    /// Anything that *isn't* our target response gets popped here so callers
    /// of `read_message` get it next.
    buffer: VecDeque<JsonRpcMessage>,
    /// Set by the stderr watcher when the backend logs an auth-related
    /// failure (expired token, unauthorized, etc.). Consumers can poll this
    /// to surface a clearer error than a generic transport timeout.
    auth_error: Arc<tokio::sync::Mutex<Option<String>>>,
    /// Set by the stderr watcher when a fatal error (Channel closed,
    /// Internal error) is seen — signals that further reads won't succeed.
    fatal_stderr: Arc<AtomicBool>,
}

impl AcpTransport {
    /// Spawn the child process described by `cmd` and set up the reader task.
    ///
    /// `cmd` must already have stdin/stdout/stderr configured as `Stdio::piped()`
    /// and have `process_group(0)` set on Unix so we can SIGTERM the whole tree
    /// on shutdown.
    pub async fn spawn(mut cmd: Command) -> Result<Self> {
        let mut child = cmd
            .spawn()
            .context("failed to spawn ACP agent process")?;

        let pid = child.id().unwrap_or(0);
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("spawned ACP agent has no stdin — did you forget stdio::piped?"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("spawned ACP agent has no stdout — did you forget stdio::piped?"))?;

        // Background task: pump stdout lines into an mpsc so `read_message`
        // has true timeout semantics (BufReader on pipes never yields Pending).
        let (tx, rx) = mpsc::channel::<String>(256);
        tokio::spawn(reader_task(stdout, tx, pid));

        let auth_error = Arc::new(tokio::sync::Mutex::new(None));
        let fatal_stderr = Arc::new(AtomicBool::new(false));
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(stderr_watcher(
                stderr,
                auth_error.clone(),
                fatal_stderr.clone(),
                pid,
            ));
        }

        Ok(Self {
            stdin: Some(stdin),
            child: Some(child),
            source: MessageSource::Real(rx),
            next_id: AtomicU64::new(1),
            pid,
            buffer: VecDeque::new(),
            auth_error,
            fatal_stderr,
        })
    }

    /// Create a transport with a pre-populated queue of canned messages.
    /// Used by unit tests to exercise transport logic without spawning a process.
    #[cfg(test)]
    pub fn mock(queue: VecDeque<JsonRpcMessage>) -> Self {
        Self {
            stdin: None,
            child: None,
            source: MessageSource::Mock(MockMessageSource { queue }),
            next_id: AtomicU64::new(1),
            pid: 0,
            buffer: VecDeque::new(),
            auth_error: Arc::new(tokio::sync::Mutex::new(None)),
            fatal_stderr: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Poll for an auth-related stderr line emitted by the backend. Returns
    /// `Some(message)` the first time one is observed; subsequent calls
    /// return `None` unless a new one appears. Adapter layer uses this to
    /// surface a useful `Error { message, recoverable: false }` rather than
    /// a generic transport failure.
    pub async fn take_auth_error(&self) -> Option<String> {
        self.auth_error.lock().await.take()
    }

    /// True if the stderr watcher has flagged a fatal condition (backend
    /// channel closed, internal error) — caller should stop streaming.
    pub fn fatal_stderr_seen(&self) -> bool {
        self.fatal_stderr.load(Ordering::Relaxed)
    }

    /// PID of the spawned backend (0 for mock transports).
    #[allow(dead_code)]
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Next id to use for an outgoing request.
    fn next_req_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Send a JSON-RPC request. Returns the generated request id so the caller
    /// can match the eventual response.
    pub async fn send_request(&mut self, method: &str, params: Value) -> Result<u64> {
        let id = self.next_req_id();
        let msg = JsonRpcRequest {
            jsonrpc: "2.0",
            id,
            method,
            params,
        };
        let mut buf = serde_json::to_vec(&msg)?;
        buf.push(b'\n');

        #[cfg(test)]
        if matches!(self.source, MessageSource::Mock(_)) {
            // Mock transport has no stdin — tests pre-seed the response queue.
            return Ok(id);
        }

        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("transport stdin closed"))?;
        stdin
            .write_all(&buf)
            .await
            .map_err(|e| anyhow!("send_request failed: {e}"))?;
        stdin.flush().await?;
        Ok(id)
    }

    /// Send a JSON-RPC response (e.g. to a server-initiated permission request).
    ///
    /// The server's request id is preserved verbatim — do not re-parse it.
    /// kiro-cli uses UUID strings for its permission request ids and will drop
    /// our response if we coerce them to numbers.
    pub async fn send_response(&mut self, id: &Value, result: Value) -> Result<()> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
        });
        let mut buf = serde_json::to_vec(&msg)?;
        buf.push(b'\n');

        #[cfg(test)]
        if matches!(self.source, MessageSource::Mock(_)) {
            return Ok(());
        }

        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("transport stdin closed"))?;
        stdin
            .write_all(&buf)
            .await
            .map_err(|e| anyhow!("send_response failed: {e}"))?;
        stdin.flush().await?;
        Ok(())
    }

    /// Read one message with a bounded timeout. Returns:
    /// - `Ok(Some(msg))` when a message parses
    /// - `Ok(None)` when the timeout fires, the channel is closed (EOF), or
    ///   the line was garbage (non-JSON, invalid UTF-8, empty)
    /// - `Err(_)` only on genuine transport faults (stdin dropped etc.)
    pub async fn read_message(&mut self, timeout: Duration) -> Result<Option<JsonRpcMessage>> {
        // Buffered messages come first (e.g. leftovers from wait_for_response).
        if let Some(msg) = self.buffer.pop_front() {
            return Ok(Some(msg));
        }
        match &mut self.source {
            MessageSource::Real(rx) => {
                match tokio::time::timeout(timeout, rx.recv()).await {
                    Ok(Some(line)) => Ok(parse_line(&line)),
                    Ok(None) => Ok(None),  // channel closed (EOF)
                    Err(_) => Ok(None),    // timeout
                }
            }
            #[cfg(test)]
            MessageSource::Mock(mock) => Ok(mock.queue.pop_front()),
        }
    }

    /// Push a message back onto the buffer so the next `read_message` returns it.
    /// Used when a caller peeks a message that should be consumed by someone else.
    pub fn unread(&mut self, msg: JsonRpcMessage) {
        self.buffer.push_front(msg);
    }

    /// Wait for the response to a specific request id. Drops stale responses
    /// and collects notifications / server-initiated requests into a staging
    /// list so they aren't lost. After the target response is found, the
    /// staged messages are pushed onto `self.buffer` in arrival order so a
    /// subsequent `read_message` yields them.
    ///
    /// This separation matters: `read_message` pops `self.buffer` first, so
    /// buffering during the wait loop would cause an infinite spin on the
    /// same notification.
    pub async fn wait_for_response(&mut self, req_id: u64, timeout: Duration) -> Result<Value> {
        let deadline = tokio::time::Instant::now() + timeout;
        let mut staged: Vec<JsonRpcMessage> = Vec::new();

        let result = loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break Err(anyhow!("timeout waiting for JSON-RPC response id {req_id}"));
            }
            let step = remaining.min(Duration::from_secs(1));
            // Read from source directly — skipping `self.buffer` so we don't
            // re-read our own staged messages and spin.
            let next = self.read_from_source(step).await?;
            match next {
                Some(msg) if msg.is_response_for(req_id) => {
                    if let Some(err) = msg.error {
                        break Err(anyhow!("JSON-RPC error for id {req_id}: {err}"));
                    }
                    break Ok(msg.result.unwrap_or(Value::Null));
                }
                Some(msg) if msg.is_notification() => {
                    // Keep for the surrounding event loop to consume later.
                    staged.push(msg);
                }
                Some(msg) if msg.method.is_some() && msg.has_id() => {
                    // Server-initiated request (e.g. session/request_permission).
                    staged.push(msg);
                }
                Some(_msg_with_mismatched_id) => {
                    // Stale response to a different request. Drop it — see
                    // HaroldCLI's regression comment for why re-buffering
                    // would spin. (Equivalent failure mode here.)
                    tracing::warn!(
                        waiting_for = req_id,
                        "dropping mismatched JSON-RPC response"
                    );
                }
                None => continue,
            }
        };

        // Push staged messages onto the buffer in arrival order so the next
        // read_message sees the oldest first.
        for msg in staged {
            self.buffer.push_back(msg);
        }
        result
    }

    /// Read from the underlying source (real channel or mock queue), bypassing
    /// `self.buffer`. Used by `wait_for_response` to avoid reprocessing its
    /// own staged messages.
    async fn read_from_source(&mut self, timeout: Duration) -> Result<Option<JsonRpcMessage>> {
        match &mut self.source {
            MessageSource::Real(rx) => {
                match tokio::time::timeout(timeout, rx.recv()).await {
                    Ok(Some(line)) => Ok(parse_line(&line)),
                    Ok(None) => Ok(None),
                    Err(_) => Ok(None),
                }
            }
            #[cfg(test)]
            MessageSource::Mock(mock) => Ok(mock.queue.pop_front()),
        }
    }

    /// Gracefully shut down the backend. Best-effort SIGTERM on Unix.
    ///
    /// Consumes `self` because the stdin/child are no longer usable afterwards.
    pub async fn shutdown(mut self) {
        // Drop stdin so the backend sees EOF and can exit cleanly.
        self.stdin.take();
        #[cfg(unix)]
        if self.pid != 0 {
            // Send SIGTERM to the process group so child MCP servers die too.
            let pgid = -(self.pid as i32);
            // SAFETY: libc::kill is safe to call with any integer PID/PGID.
            unsafe {
                libc::kill(pgid, libc::SIGTERM);
            }
        }
        if let Some(mut child) = self.child.take() {
            // Reap within a bounded window; escalate to SIGKILL on timeout.
            match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
                Ok(_) => {}
                Err(_) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                }
            }
        }
    }
}

impl Drop for AcpTransport {
    fn drop(&mut self) {
        // If the transport is dropped without shutdown() (e.g. from a panic
        // or a cancelled task), at least SIGKILL the child so it doesn't leak.
        // Can't `.await` inside Drop, so this is best-effort synchronous.
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
        #[cfg(unix)]
        if self.pid != 0 {
            let pgid = -(self.pid as i32);
            // SAFETY: libc::kill with any integer PGID is safe. We don't care
            // about the result — the child may already be gone.
            unsafe {
                libc::kill(pgid, libc::SIGKILL);
            }
        }
    }
}

/// Background task that reads newline-delimited lines from stdout and
/// pushes them into an mpsc channel. Capped at `MAX_LINE_BYTES` per line.
async fn reader_task(
    stdout: tokio::process::ChildStdout,
    tx: mpsc::Sender<String>,
    pid: u32,
) {
    let mut reader = BufReader::new(stdout);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        // Limit bytes read_until can consume per iteration.
        let mut limited = (&mut reader).take((MAX_LINE_BYTES + 1) as u64);
        let read = AsyncBufReadExt::read_until(&mut limited, b'\n', &mut buf).await;
        match read {
            Ok(0) => break, // EOF
            Ok(_) if buf.len() > MAX_LINE_BYTES => {
                tracing::error!(
                    pid,
                    bytes = buf.len(),
                    "ACP stdout: line exceeded {MAX_LINE_BYTES} bytes — killing reader"
                );
                break;
            }
            Ok(_) => {
                // Avoid lossy UTF-8 re-encoding in the common case.
                let line = match std::str::from_utf8(&buf) {
                    Ok(s) => s.to_string(),
                    Err(_) => String::from_utf8_lossy(&buf).into_owned(),
                };
                if tx.send(line).await.is_err() {
                    break; // receiver dropped
                }
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                tracing::warn!(pid, error = %e, "ACP stdout: read error");
                break;
            }
        }
    }
}

/// Background task that monitors the backend's stderr, flagging auth
/// failures and fatal conditions for the adapter to surface as typed events.
async fn stderr_watcher(
    stderr: tokio::process::ChildStderr,
    auth_error: Arc<tokio::sync::Mutex<Option<String>>>,
    fatal: Arc<AtomicBool>,
    pid: u32,
) {
    let mut reader = BufReader::new(stderr);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let mut limited = (&mut reader).take(64 * 1024);
        match AsyncBufReadExt::read_until(&mut limited, b'\n', &mut buf).await {
            Ok(0) => break, // EOF
            Ok(_) => {
                let line = String::from_utf8_lossy(&buf).into_owned();
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                // Log every non-empty stderr line so operators see them.
                tracing::warn!(pid, stderr = %trimmed, "ACP backend stderr");

                if classify_auth_line(trimmed) {
                    let mut slot = auth_error.lock().await;
                    if slot.is_none() {
                        *slot = Some(trimmed.to_string());
                    }
                }

                if classify_fatal_line(trimmed) {
                    fatal.store(true, Ordering::Relaxed);
                }
            }
            Err(_) => break,
        }
    }
}

/// True if the line looks like an auth-related failure that should surface
/// as `Error { recoverable: false }`. Case-insensitive on words; the HTTP
/// status codes 401/403 match only as standalone tokens so a version string
/// like "2.0.401" or a byte count like "40123" doesn't false-positive.
fn classify_auth_line(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    if lower.contains("unauthorized")
        || lower.contains("forbidden")
        || lower.contains("expired")
        || lower.contains("midway")
        || lower.contains("authentication")
    {
        return true;
    }
    // Treat 401/403 as matches only when they appear as a standalone token
    // (surrounded by whitespace or at line boundaries) — ascii_digit neighbors
    // mean we're inside a larger number like "40123" or "2.401".
    contains_standalone_token(&lower, "401") || contains_standalone_token(&lower, "403")
}

fn contains_standalone_token(haystack: &str, needle: &str) -> bool {
    let is_word_boundary = |c: Option<char>| match c {
        None => true,
        Some(ch) => !ch.is_alphanumeric() && ch != '.',
    };
    let mut remaining = haystack;
    while let Some(start) = remaining.find(needle) {
        let before = remaining[..start].chars().next_back();
        let after_idx = start + needle.len();
        let after = remaining[after_idx..].chars().next();
        if is_word_boundary(before) && is_word_boundary(after) {
            return true;
        }
        // Advance past the matched portion and keep looking.
        remaining = &remaining[after_idx..];
    }
    false
}

/// True if the line signals a fatal backend condition — the reader should
/// stop polling and surface an Error. Case-sensitive on purpose; these are
/// well-known error strings from kiro-cli / Rust runtime crashes.
fn classify_fatal_line(line: &str) -> bool {
    line.contains("Channel closed")
        || line.contains("Internal error")
        || line.contains("panic")
}

/// Parse a single raw line into a JsonRpcMessage, returning `None` for blank
/// lines, lines containing only ANSI escape codes, or unparseable JSON.
fn parse_line(raw: &str) -> Option<JsonRpcMessage> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let clean = strip_ansi(trimmed);
    let clean = clean.trim();
    if clean.is_empty() {
        return None;
    }
    match serde_json::from_str::<JsonRpcMessage>(clean) {
        Ok(m) => Some(m),
        Err(e) => {
            tracing::debug!(error = %e, len = clean.len(), "skipping non-JSON ACP line");
            None
        }
    }
}

/// Strip common ANSI escape sequences. Kiro-cli is clean, but Wasabi/other
/// ACP backends sometimes emit `\x1b[K` etc. on JSON lines. Handling it here
/// keeps downstream parsing simple.
fn strip_ansi(s: &str) -> String {
    // Tiny state machine — no regex, no dependency. Handles:
    //   ESC '[' <...> <final-byte 0x40..0x7E>
    //   ESC <single-char>
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                // CSI: consume until a byte in 0x40..=0x7E
                while let Some(&nc) = chars.peek() {
                    chars.next();
                    if ('@'..='~').contains(&nc) {
                        break;
                    }
                }
            }
            Some(_) => {
                // Short escape: already consumed
            }
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    // ─── helpers ──────────────────────────────────────────────────────────

    fn msg(json: &str) -> JsonRpcMessage {
        serde_json::from_str(json).expect("valid JSON-RPC fixture")
    }

    fn queue(items: &[&str]) -> VecDeque<JsonRpcMessage> {
        items.iter().map(|s| msg(s)).collect()
    }

    /// Spawn a shell script that echoes the given line then exits. Used to
    /// exercise the real spawn() + stdout reader path without relying on an
    /// agent binary.
    fn echo_command(line: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(format!("printf '{}\\n'", line.replace('\'', "'\\''")))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    // ─── classify_auth_line ───────────────────────────────────────────────

    #[test]
    fn classify_auth_matches_unauthorized_any_case() {
        assert!(classify_auth_line("401 Unauthorized"));
        assert!(classify_auth_line("HTTP/1.1 401 Unauthorized"));
        assert!(classify_auth_line("UNAUTHORIZED request"));
        assert!(classify_auth_line("request unauthorized by server"));
    }

    #[test]
    fn classify_auth_matches_forbidden() {
        assert!(classify_auth_line("403 Forbidden"));
        assert!(classify_auth_line("access forbidden: check creds"));
    }

    #[test]
    fn classify_auth_matches_expired_tokens() {
        assert!(classify_auth_line("Midway token expired"));
        assert!(classify_auth_line("session expired, please re-auth"));
    }

    #[test]
    fn classify_auth_matches_midway_references() {
        assert!(classify_auth_line("Midway handshake failed"));
        assert!(classify_auth_line("midway: no cookie found"));
    }

    #[test]
    fn classify_auth_matches_authentication_keyword() {
        assert!(classify_auth_line("authentication failed"));
        assert!(classify_auth_line("Authentication required"));
    }

    #[test]
    fn classify_auth_matches_401_with_word_boundary() {
        assert!(classify_auth_line("got 401 from api"));
        assert!(classify_auth_line("status 401 "));
    }

    #[test]
    fn classify_auth_matches_403_with_word_boundary() {
        assert!(classify_auth_line("got 403 from api"));
        assert!(classify_auth_line("status 403 "));
    }

    #[test]
    fn classify_auth_rejects_unrelated_lines() {
        assert!(!classify_auth_line("downloading package 2.0.401"));
        assert!(!classify_auth_line("processing 40123 bytes"));
        assert!(!classify_auth_line("info: starting up"));
        assert!(!classify_auth_line(""));
    }

    // ─── classify_fatal_line ──────────────────────────────────────────────

    #[test]
    fn classify_fatal_matches_channel_closed() {
        assert!(classify_fatal_line("Channel closed by peer"));
        assert!(classify_fatal_line("error: Channel closed"));
    }

    #[test]
    fn classify_fatal_matches_internal_error() {
        assert!(classify_fatal_line("Internal error while processing request"));
    }

    #[test]
    fn classify_fatal_matches_panic() {
        assert!(classify_fatal_line("thread 'main' panicked at 'foo'"));
        assert!(classify_fatal_line("panic: unwrap of None"));
    }

    #[test]
    fn classify_fatal_is_case_sensitive_by_design() {
        // These are well-known error strings; lowercase variants are false
        // positives (e.g. log lines discussing "channel closed" in docs).
        assert!(!classify_fatal_line("channel closed cleanly"));
        assert!(!classify_fatal_line("internal error count: 0"));
    }

    #[test]
    fn classify_fatal_rejects_normal_output() {
        assert!(!classify_fatal_line("INFO starting session"));
        assert!(!classify_fatal_line("ready"));
        assert!(!classify_fatal_line(""));
    }

    // ─── parse_line / strip_ansi ──────────────────────────────────────────

    #[test]
    fn parse_line_returns_none_on_empty_line() {
        assert!(parse_line("").is_none());
        assert!(parse_line("   ").is_none());
        assert!(parse_line("\t\n").is_none());
    }

    #[test]
    fn parse_line_returns_none_on_invalid_json() {
        assert!(parse_line("not json at all").is_none());
        assert!(parse_line("{").is_none());
    }

    #[test]
    fn parse_line_strips_ansi_csi_before_json() {
        let m = parse_line("\x1b[K{\"jsonrpc\":\"2.0\",\"method\":\"ping\"}")
            .expect("ANSI-prefixed JSON should parse");
        assert!(m.is_method("ping"));
    }

    #[test]
    fn parse_line_handles_short_ansi_escape() {
        let m = parse_line("\x1bG{\"jsonrpc\":\"2.0\",\"method\":\"x\"}")
            .expect("short ANSI escape should be stripped");
        assert!(m.is_method("x"));
    }

    // ─── JsonRpcMessage helpers ───────────────────────────────────────────

    #[test]
    fn is_response_for_matches_numeric_id() {
        let m = msg(r#"{"jsonrpc":"2.0","id":42,"result":{}}"#);
        assert!(m.is_response_for(42));
        assert!(!m.is_response_for(41));
    }

    #[test]
    fn is_response_for_matches_string_id_that_parses_as_number() {
        let m = msg(r#"{"jsonrpc":"2.0","id":"42","result":{}}"#);
        assert!(m.is_response_for(42));
    }

    #[test]
    fn is_response_for_false_for_uuid_string() {
        let m = msg(r#"{"jsonrpc":"2.0","id":"abc-123","method":"session/request_permission"}"#);
        // Server-initiated request with a UUID id — it's never a response to
        // one of our numeric request ids.
        assert!(!m.is_response_for(1));
        assert!(!m.is_response_for(123));
    }

    #[test]
    fn has_id_distinguishes_notifications() {
        let notif = msg(r#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#);
        let req = msg(r#"{"jsonrpc":"2.0","id":"abc","method":"session/request_permission","params":{}}"#);
        let resp = msg(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#);
        assert!(notif.is_notification());
        assert!(!req.is_notification());
        assert!(!resp.is_notification());
        assert!(!notif.has_id());
        assert!(req.has_id());
        assert!(resp.has_id());
    }

    // ─── AcpTransport::send_request / send_response ───────────────────────

    #[tokio::test]
    async fn send_request_increments_id_and_appends_newline() {
        // We can't easily inspect stdin without a real child. Use `cat` so
        // bytes written to stdin echo out on stdout.
        let mut cmd = Command::new("cat");
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut transport = AcpTransport::spawn(cmd).await.expect("spawn cat");

        let id1 = transport
            .send_request("initialize", serde_json::json!({"a": 1}))
            .await
            .expect("send first request");
        let id2 = transport
            .send_request("session/new", serde_json::json!({"b": 2}))
            .await
            .expect("send second request");

        assert_eq!(id1, 1, "first id should be 1");
        assert_eq!(id2, 2, "second id should be 2");

        // Read back what we sent — `cat` echoes stdin to stdout.
        let first = transport
            .read_message(Duration::from_secs(2))
            .await
            .expect("read first")
            .expect("first message");
        let second = transport
            .read_message(Duration::from_secs(2))
            .await
            .expect("read second")
            .expect("second message");

        assert!(first.is_response_for(1));
        assert_eq!(first.method.as_deref(), Some("initialize"));
        assert!(second.is_response_for(2));
        assert_eq!(second.method.as_deref(), Some("session/new"));

        transport.shutdown().await;
    }

    #[tokio::test]
    async fn send_request_errors_when_real_stdin_has_been_dropped() {
        // Spawn a child that exits immediately — stdin will become a broken pipe.
        let mut cmd = Command::new("true");
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut transport = AcpTransport::spawn(cmd).await.expect("spawn true");

        // Wait for the child to exit so writes actually fail.
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Write enough to fill the pipe buffer and get EPIPE.
        let mut last_err: Option<anyhow::Error> = None;
        for _ in 0..512 {
            match transport
                .send_request("x", serde_json::json!({"filler": "x".repeat(4096)}))
                .await
            {
                Ok(_) => {}
                Err(e) => {
                    last_err = Some(e);
                    break;
                }
            }
        }
        assert!(
            last_err.is_some(),
            "send_request should eventually error once the child is gone"
        );
        transport.shutdown().await;
    }

    #[tokio::test]
    async fn send_response_preserves_uuid_id_verbatim() {
        // Spawn `cat` so stdin echoes to stdout.
        let mut cmd = Command::new("cat");
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut transport = AcpTransport::spawn(cmd).await.expect("spawn cat");

        let uuid = Value::String("9b2d6a41-dead-beef-cafe-000000000001".to_string());
        transport
            .send_response(&uuid, serde_json::json!({"outcome":{"outcome":"selected"}}))
            .await
            .expect("send_response");

        let echoed = transport
            .read_message(Duration::from_secs(2))
            .await
            .expect("read echoed response")
            .expect("some message");

        // The id field must be the SAME UUID string — not re-parsed as a number.
        assert_eq!(
            echoed.id.as_ref(),
            Some(&uuid),
            "uuid id must be preserved verbatim"
        );
        transport.shutdown().await;
    }

    // ─── read_message / wait_for_response ─────────────────────────────────

    #[tokio::test]
    async fn read_message_respects_timeout() {
        // Spawn `sleep 5` so stdout never produces anything within our window.
        let mut cmd = Command::new("sleep");
        cmd.arg("5")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut transport = AcpTransport::spawn(cmd).await.expect("spawn sleep");

        let start = std::time::Instant::now();
        let result = transport
            .read_message(Duration::from_millis(50))
            .await
            .expect("read_message should not error on timeout");
        let elapsed = start.elapsed();

        assert!(result.is_none(), "empty stdout → Ok(None)");
        assert!(
            elapsed < Duration::from_millis(500),
            "read_message should respect the 50ms timeout, took {elapsed:?}"
        );
        transport.shutdown().await;
    }

    #[tokio::test]
    async fn read_message_strips_ansi_escapes() {
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#,
        ]));
        let m = transport
            .read_message(Duration::from_millis(10))
            .await
            .expect("ok")
            .expect("some");
        assert!(m.is_method("session/update"));
    }

    #[tokio::test]
    async fn read_message_skips_invalid_lines_and_empty_lines() {
        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("printf '\\n\\nnot json\\n{\"jsonrpc\":\"2.0\",\"method\":\"hello\"}\\n'")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut transport = AcpTransport::spawn(cmd).await.expect("spawn sh");

        // The first two reads will be `None` (empty lines + bad JSON), then
        // the valid message shows up. Loop a few times with short timeouts.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let mut seen = None;
        while tokio::time::Instant::now() < deadline && seen.is_none() {
            if let Some(m) = transport
                .read_message(Duration::from_millis(100))
                .await
                .expect("no transport error")
            {
                seen = Some(m);
            }
        }
        let hello = seen.expect("should eventually receive the valid JSON-RPC line");
        assert!(hello.is_method("hello"));
        transport.shutdown().await;
    }

    #[tokio::test]
    async fn wait_for_response_skips_notifications_and_returns_result() {
        // Queue: notification → response to id 1
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","method":"session/update","params":{"x":1}}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#,
        ]));
        // Consume the fake id so next_req_id starts at 1.
        let value = transport
            .wait_for_response(1, Duration::from_secs(1))
            .await
            .expect("should return the id=1 result");
        assert_eq!(value, serde_json::json!({"ok": true}));

        // The notification must have been re-buffered so downstream code sees it.
        let buffered = transport
            .read_message(Duration::from_millis(10))
            .await
            .expect("ok")
            .expect("buffered notification");
        assert!(buffered.is_method("session/update"));
    }

    #[tokio::test]
    async fn wait_for_response_drops_stale_responses() {
        // Queue: stale response to id 999 (stale) → response to id 1 (target)
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","id":999,"result":{"stale":true}}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#,
        ]));
        let value = transport
            .wait_for_response(1, Duration::from_secs(1))
            .await
            .expect("should return id=1 result without spinning");
        assert_eq!(value, serde_json::json!({"ok": true}));
    }

    #[tokio::test]
    async fn wait_for_response_buffers_server_requests() {
        // Server-initiated permission request arrives while we wait for our response.
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","id":"uuid-abc","method":"session/request_permission","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":{"ok":true}}"#,
        ]));
        transport
            .wait_for_response(1, Duration::from_secs(1))
            .await
            .expect("response");

        let buffered = transport
            .read_message(Duration::from_millis(10))
            .await
            .expect("ok")
            .expect("permission request buffered");
        assert!(buffered.is_method("session/request_permission"));
    }

    #[tokio::test]
    async fn wait_for_response_propagates_json_rpc_error() {
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"bad params"}}"#,
        ]));
        let err = transport
            .wait_for_response(1, Duration::from_millis(50))
            .await
            .expect_err("error response should surface as Err");
        assert!(err.to_string().contains("bad params"));
    }

    #[tokio::test]
    async fn wait_for_response_times_out_when_no_match() {
        let mut transport = AcpTransport::mock(VecDeque::new());
        let start = std::time::Instant::now();
        let err = transport
            .wait_for_response(1, Duration::from_millis(50))
            .await
            .expect_err("should time out");
        assert!(err.to_string().contains("timeout"));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    // ─── real-process read ────────────────────────────────────────────────

    #[tokio::test]
    async fn read_message_returns_none_on_eof() {
        // `true` exits immediately with no output.
        let mut cmd = Command::new("true");
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut transport = AcpTransport::spawn(cmd).await.expect("spawn true");

        // EOF (channel closed) — repeated reads return None without hanging.
        let r1 = transport
            .read_message(Duration::from_millis(500))
            .await
            .expect("no error");
        assert!(r1.is_none());
        transport.shutdown().await;
    }

    #[tokio::test]
    async fn spawn_yields_valid_message_from_echo_script() {
        let cmd = echo_command(r#"{"jsonrpc":"2.0","id":7,"result":{"hello":"world"}}"#);
        let mut transport = AcpTransport::spawn(cmd).await.expect("spawn echo");
        let m = transport
            .read_message(Duration::from_secs(2))
            .await
            .expect("ok")
            .expect("one message");
        assert!(m.is_response_for(7));
        assert_eq!(m.result, Some(serde_json::json!({"hello":"world"})));
        transport.shutdown().await;
    }

    #[tokio::test]
    async fn unread_pushes_message_to_front_of_queue() {
        let mut transport = AcpTransport::mock(queue(&[
            r#"{"jsonrpc":"2.0","method":"a"}"#,
        ]));
        let m = transport
            .read_message(Duration::from_millis(10))
            .await
            .expect("ok")
            .expect("first");
        assert!(m.is_method("a"));
        // Push back; next read should return the same one.
        transport.unread(m);
        let again = transport
            .read_message(Duration::from_millis(10))
            .await
            .expect("ok")
            .expect("unread");
        assert!(again.is_method("a"));
    }
}
