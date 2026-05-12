//! Git-backed version tracker. Delegates to the pre-thread commit
//! snapshot captured in `git::snapshot` at thread start.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;

use crate::db::DbHandle;
use crate::git;

use super::{ChangedFile, FileAction, TrackerKind, VersionTracker};

pub struct GitVersionTracker {
    db: DbHandle,
}

impl GitVersionTracker {
    pub fn new(db: DbHandle) -> Self {
        Self { db }
    }

    async fn snapshot_hash(&self, thread_id: &str) -> Result<Option<String>> {
        let tid = thread_id.to_string();
        self.db
            .execute(move |conn| {
                let hash: Option<String> = conn
                    .query_row(
                        "SELECT snapshot_ref FROM threads WHERE id = ?1",
                        rusqlite::params![tid],
                        |row| row.get(0),
                    )
                    .ok();
                Ok(hash)
            })
            .await
    }
}

#[async_trait]
impl VersionTracker for GitVersionTracker {
    fn kind(&self) -> TrackerKind {
        TrackerKind::Git
    }

    async fn record_pre_edit(
        &self,
        _thread_id: &str,
        _workspace_path: &Path,
        _file_path: &Path,
    ) -> Result<()> {
        // The pre-thread commit snapshot already captures the entire
        // workspace state — nothing to record per edit.
        Ok(())
    }

    async fn diff(
        &self,
        _thread_id: &str,
        workspace_path: &Path,
        file_paths: Option<&[PathBuf]>,
    ) -> Result<String> {
        let files: Option<Vec<String>> = file_paths.map(|paths| {
            paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect()
        });
        git::get_workspace_diff(workspace_path, files.as_deref()).await
    }

    async fn revert(&self, thread_id: &str, workspace_path: &Path) -> Result<()> {
        let hash = self
            .snapshot_hash(thread_id)
            .await?
            .context("no pre-thread snapshot recorded for this thread")?;
        git::revert(workspace_path, &git::SnapshotRef { commit_hash: hash }).await
    }

    async fn list_changed_files(
        &self,
        _thread_id: &str,
        workspace_path: &Path,
    ) -> Result<Vec<ChangedFile>> {
        // `get_changed_files` returns porcelain-derived lines. For a
        // single-repo workspace each line is `"XY path"` (2 chars status
        // + space + path). For multi-repo workspaces the helper already
        // prefixes nested-repo paths with the subdir name before passing
        // the line back, so workspace-relative path resolution is a simple
        // `workspace_path.join(rel)` in both cases. Rename lines have the
        // form `"R  old -> new"` — we pick the post-rename path.
        let lines = git::get_changed_files(workspace_path).await?;
        let mut out = Vec::new();
        for line in lines {
            if line.len() < 3 {
                continue;
            }
            let (status, rel_raw) = (&line[..2], line[3..].trim());
            let rel = if let Some((_, new)) = rel_raw.split_once(" -> ") {
                new.trim().to_string()
            } else {
                rel_raw.to_string()
            };
            if rel.is_empty() {
                continue;
            }
            let action = if status.trim() == "??" || status.contains('A') {
                FileAction::Created
            } else if status.contains('D') {
                FileAction::Deleted
            } else {
                FileAction::Modified
            };
            let abs = workspace_path.join(&rel);
            out.push(ChangedFile {
                absolute_path: abs,
                relative_path: rel,
                action,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbHandle;
    use crate::test_support;

    fn db_handle() -> DbHandle {
        test_support::in_memory_db()
    }

    #[tokio::test]
    async fn record_pre_edit_is_noop_and_never_errors() {
        let tracker = GitVersionTracker::new(db_handle());
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("nonexistent.txt");
        tracker
            .record_pre_edit("t1", tmp.path(), &file)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn kind_returns_git() {
        let tracker = GitVersionTracker::new(db_handle());
        assert_eq!(tracker.kind(), TrackerKind::Git);
    }

    #[tokio::test]
    async fn revert_without_snapshot_errors_cleanly() {
        let tracker = GitVersionTracker::new(db_handle());
        let tmp = tempfile::tempdir().unwrap();
        let err = tracker.revert("unknown-thread", tmp.path()).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn list_changed_files_picks_up_nested_repo_paths() {
        use tokio::process::Command;
        let tracker = GitVersionTracker::new(db_handle());
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        let repo_a = ws.join("a");
        let repo_b = ws.join("b");
        std::fs::create_dir_all(&repo_a).unwrap();
        std::fs::create_dir_all(&repo_b).unwrap();
        for r in [&repo_a, &repo_b] {
            Command::new("git").args(["init", "-q"]).current_dir(r).status().await.unwrap();
            Command::new("git")
                .args([
                    "-c",
                    "user.email=t@t",
                    "-c",
                    "user.name=t",
                    "commit",
                    "--allow-empty",
                    "-q",
                    "-m",
                    "init",
                ])
                .current_dir(r)
                .status()
                .await
                .unwrap();
        }
        std::fs::write(repo_a.join("changed.txt"), b"new").unwrap();
        std::fs::write(repo_b.join("added.txt"), b"new").unwrap();

        let changed = tracker.list_changed_files("t1", ws).await.unwrap();
        let paths: Vec<String> = changed.iter().map(|c| c.relative_path.clone()).collect();
        // Each nested repo contributes its own untracked file, path is
        // prefixed with the repo's subdir so the workspace-level revert
        // model and UI see a single flat list.
        assert!(paths.iter().any(|p| p == "a/changed.txt"), "got: {paths:?}");
        assert!(paths.iter().any(|p| p == "b/added.txt"), "got: {paths:?}");
    }

    #[tokio::test]
    async fn list_changed_files_handles_rename_lines() {
        let tracker = GitVersionTracker::new(db_handle());
        // We don't have easy way to produce a real rename via porcelain
        // from a unit test, so synthesize the parser's input via a
        // single-repo git workspace in tempdir.
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path();
        tokio::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(ws)
            .status()
            .await
            .unwrap();
        std::fs::write(ws.join("old.txt"), b"content").unwrap();
        tokio::process::Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t", "add", "-A"])
            .current_dir(ws)
            .status()
            .await
            .unwrap();
        tokio::process::Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-q", "-m", "init"])
            .current_dir(ws)
            .status()
            .await
            .unwrap();
        // Rename via git mv so porcelain surfaces `R  old -> new`.
        tokio::process::Command::new("git")
            .args(["mv", "old.txt", "new.txt"])
            .current_dir(ws)
            .status()
            .await
            .unwrap();

        let changed = tracker.list_changed_files("t1", ws).await.unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].relative_path, "new.txt");
    }
}
