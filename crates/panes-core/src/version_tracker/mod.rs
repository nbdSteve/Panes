//! Pre-edit version tracking for agent file writes.
//!
//! Provides a uniform diff/revert surface across two backends:
//! - `GitVersionTracker` — delegates to the pre-thread commit snapshot
//!   captured in `git::snapshot` at thread start.
//! - `ShadowVersionTracker` — Panes-owned content-addressed blob store
//!   under `$PANES_DATA_DIR/shadow-blobs/`, used when the workspace is
//!   not a git repo so the UI's diff / "Revert all" work uniformly.
//!
//! The tracker kind is pinned on the `threads` row at thread start, so a
//! mid-thread `git init` can't rug-pull an active thread.

use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod git;
pub mod shadow;

pub use git::GitVersionTracker;
pub use shadow::ShadowVersionTracker;

/// Tool names whose invocation implies a file write. Mirrors the frontend
/// set in `src/lib/threadHelpers.ts` and the ACP kinds in
/// `crates/panes-adapters/src/acp/events.rs::classify_acp_risk`. Extend
/// this list when a new adapter surfaces a new write tool.
///
/// Case-sensitivity matters: Claude emits capitalised tool names
/// (`"Edit"`, `"Write"`), while ACP/kiro-cli sometimes emits the
/// semantic *kind* (lowercase `"edit"`, `"write"`) rather than the raw
/// tool name. We enumerate both shapes so neither transport slips past
/// the pre-edit hook.
pub const FILE_WRITE_TOOLS: &[&str] = &[
    // Claude (capitalised tool names)
    "Write",
    "Edit",
    "MultiEdit",
    "NotebookEdit",
    // ACP / kiro-cli — raw tool names
    "fs_write",
    "fs_edit",
    "fs_create",
    // ACP / kiro-cli — semantic kinds (see classify_acp_risk)
    "edit",
    "write",
];

pub fn is_file_write_tool(tool_name: &str) -> bool {
    FILE_WRITE_TOOLS.contains(&tool_name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackerKind {
    Git,
    Shadow,
}

impl TrackerKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TrackerKind::Git => "git",
            TrackerKind::Shadow => "shadow",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "shadow" => TrackerKind::Shadow,
            _ => TrackerKind::Git,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileAction {
    Created,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub absolute_path: PathBuf,
    pub relative_path: String,
    pub action: FileAction,
}

#[async_trait]
pub trait VersionTracker: Send + Sync {
    fn kind(&self) -> TrackerKind;

    /// Record the pre-edit state of `file_path`. Must be idempotent per
    /// `(thread_id, file_path)` — the first call wins, so repeated edits
    /// to the same file during a thread preserve the original pre-thread
    /// content (and thus produce a correct revert).
    ///
    /// If `file_path` does not exist on disk, the tracker records a
    /// tombstone so that `revert` deletes a newly created file.
    ///
    /// The Git tracker treats this as a no-op because the pre-thread
    /// commit snapshot already captures the workspace's prior state.
    async fn record_pre_edit(
        &self,
        thread_id: &str,
        workspace_path: &Path,
        file_path: &Path,
    ) -> Result<()>;

    /// Return a unified-diff text showing the difference between the
    /// pre-edit state and the current on-disk state, scoped to this
    /// thread. When `file_paths` is `Some`, only those files are included.
    async fn diff(
        &self,
        thread_id: &str,
        workspace_path: &Path,
        file_paths: Option<&[PathBuf]>,
    ) -> Result<String>;

    /// Restore every file touched by this thread to its pre-edit state.
    /// Tombstoned files are deleted. Idempotent.
    async fn revert(&self, thread_id: &str, workspace_path: &Path) -> Result<()>;

    /// List files whose current on-disk state differs from their
    /// pre-edit state. Unchanged files (e.g. gated writes the user
    /// rejected) are filtered out.
    async fn list_changed_files(
        &self,
        thread_id: &str,
        workspace_path: &Path,
    ) -> Result<Vec<ChangedFile>>;
}

/// Extract the absolute, workspace-rooted file path from a tool-request
/// `input` payload. Returns `None` when the input shape doesn't name a
/// file or when the resolved path falls outside `workspace_path`.
///
/// Paths outside the workspace are explicitly refused — the tracker's
/// revert is a destructive filesystem operation, and we never want it
/// touching anything the user didn't explicitly put under Panes's
/// management. An agent writing to `/etc/foo` is the agent's problem;
/// Panes simply won't track (and therefore won't try to "revert") it.
///
/// Handles both Claude's `file_path` / `notebook_path` and ACP's
/// `path`, and resolves relative paths under `workspace_path`.
pub fn extract_file_path(
    _tool_name: &str,
    input: &Value,
    workspace_path: &Path,
) -> Option<PathBuf> {
    let raw = input
        .get("file_path")
        .or_else(|| input.get("notebook_path"))
        .or_else(|| input.get("path"))
        .and_then(|v| v.as_str())?;

    if raw.is_empty() {
        return None;
    }

    let candidate = PathBuf::from(raw);
    let abs = if candidate.is_absolute() {
        candidate
    } else {
        workspace_path.join(candidate)
    };

    let normalized = normalize_path(&abs);
    let ws_normalized = normalize_path(workspace_path);

    if !normalized.starts_with(&ws_normalized) {
        return None;
    }

    Some(normalized)
}

/// Like `Path::canonicalize` but doesn't require the file to exist —
/// strips `.` and resolves `..` components in-place. Used for
/// workspace-rooted paths that may point at files about to be created.
fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ws() -> PathBuf {
        PathBuf::from("/tmp/panes-ws")
    }

    #[test]
    fn is_file_write_tool_recognises_claude_tools() {
        for t in ["Write", "Edit", "MultiEdit", "NotebookEdit"] {
            assert!(is_file_write_tool(t), "{t}");
        }
    }

    #[test]
    fn is_file_write_tool_recognises_acp_tools() {
        for t in ["fs_write", "fs_edit", "fs_create"] {
            assert!(is_file_write_tool(t), "{t}");
        }
    }

    #[test]
    fn is_file_write_tool_recognises_acp_semantic_kinds() {
        // kiro-cli sometimes surfaces the lowercase semantic kind rather
        // than the raw tool name — both must be caught.
        for t in ["edit", "write"] {
            assert!(is_file_write_tool(t), "{t}");
        }
    }

    #[test]
    fn is_file_write_tool_rejects_unknowns() {
        for t in ["Read", "Bash", "fs_read", ""] {
            assert!(!is_file_write_tool(t), "{t}");
        }
    }

    #[test]
    fn extract_file_path_handles_claude_write() {
        let input = json!({"file_path": "/tmp/panes-ws/src/main.ts"});
        let got = extract_file_path("Write", &input, &ws()).unwrap();
        assert_eq!(got, PathBuf::from("/tmp/panes-ws/src/main.ts"));
    }

    #[test]
    fn extract_file_path_handles_notebook_edit() {
        let input = json!({"notebook_path": "/tmp/panes-ws/nb.ipynb"});
        let got = extract_file_path("NotebookEdit", &input, &ws()).unwrap();
        assert_eq!(got, PathBuf::from("/tmp/panes-ws/nb.ipynb"));
    }

    #[test]
    fn extract_file_path_handles_acp_fs_write() {
        let input = json!({"path": "/tmp/panes-ws/hello.txt"});
        let got = extract_file_path("fs_write", &input, &ws()).unwrap();
        assert_eq!(got, PathBuf::from("/tmp/panes-ws/hello.txt"));
    }

    #[test]
    fn extract_file_path_resolves_relative_against_workspace() {
        let input = json!({"file_path": "src/lib.rs"});
        let got = extract_file_path("Edit", &input, &ws()).unwrap();
        assert_eq!(got, PathBuf::from("/tmp/panes-ws/src/lib.rs"));
    }

    #[test]
    fn extract_file_path_rejects_traversal_escape() {
        let input = json!({"file_path": "../../../etc/passwd"});
        assert!(extract_file_path("Write", &input, &ws()).is_none());
    }

    #[test]
    fn extract_file_path_rejects_absolute_path_outside_workspace() {
        let input = json!({"file_path": "/etc/passwd"});
        assert!(extract_file_path("Write", &input, &ws()).is_none());
    }

    #[test]
    fn extract_file_path_rejects_sibling_of_workspace() {
        let input = json!({"file_path": "/tmp/panes-other/file.txt"});
        assert!(extract_file_path("Write", &input, &ws()).is_none());
    }

    #[test]
    fn extract_file_path_returns_none_when_missing() {
        let input = json!({"command": "ls"});
        assert!(extract_file_path("Bash", &input, &ws()).is_none());
    }

    #[test]
    fn extract_file_path_returns_none_for_empty_string() {
        let input = json!({"file_path": ""});
        assert!(extract_file_path("Write", &input, &ws()).is_none());
    }

    #[test]
    fn tracker_kind_roundtrips_via_as_str() {
        assert_eq!(TrackerKind::parse("git"), TrackerKind::Git);
        assert_eq!(TrackerKind::parse("shadow"), TrackerKind::Shadow);
        assert_eq!(TrackerKind::parse("nonsense"), TrackerKind::Git);
        assert_eq!(TrackerKind::Git.as_str(), "git");
        assert_eq!(TrackerKind::Shadow.as_str(), "shadow");
    }
}
