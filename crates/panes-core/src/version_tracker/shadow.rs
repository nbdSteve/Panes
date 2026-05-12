//! Shadow (blob-store) version tracker for non-git workspaces.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use sha2::{Digest, Sha256};
use similar::{ChangeTag, TextDiff};
use tracing::warn;

use crate::db::DbHandle;

use super::{ChangedFile, FileAction, TrackerKind, VersionTracker};

pub struct ShadowVersionTracker {
    db: DbHandle,
    blob_root: PathBuf,
}

#[derive(Debug, Clone)]
struct ShadowRow {
    file_path: String,
    pre_existed: bool,
    content_hash: Option<String>,
    /// Raw unix mode bits captured from `Metadata::permissions().mode()`
    /// at record_pre_edit time. NULL for tombstone rows (no pre-existing
    /// file). Used to restore executable / read-only bits on revert.
    mode: Option<u32>,
}

/// Snapshot of a file's pre-edit state, computed on a blocking worker
/// before the DB write so the async event-loop never touches disk.
struct PreEditSnapshot {
    pre_existed: bool,
    content_hash: Option<String>,
    mode: Option<u32>,
}

impl ShadowVersionTracker {
    pub fn new(db: DbHandle, blob_root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&blob_root)
            .with_context(|| format!("failed to create shadow blob root: {}", blob_root.display()))?;
        Ok(Self { db, blob_root })
    }

    fn blob_path_for(blob_root: &Path, hash: &str) -> PathBuf {
        let (prefix, _) = hash.split_at(2);
        blob_root.join(prefix).join(hash)
    }

    #[cfg(test)]
    fn blob_path(&self, hash: &str) -> PathBuf {
        Self::blob_path_for(&self.blob_root, hash)
    }

    async fn load_rows(&self, thread_id: &str) -> Result<Vec<ShadowRow>> {
        let tid = thread_id.to_string();
        self.db
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT file_path, pre_existed, content_hash, mode \
                     FROM shadow_edits WHERE thread_id = ?1",
                )?;
                let rows = stmt.query_map(rusqlite::params![tid], |row| {
                    let pre_existed: i64 = row.get(1)?;
                    let mode: Option<i64> = row.get(3)?;
                    Ok(ShadowRow {
                        file_path: row.get(0)?,
                        pre_existed: pre_existed != 0,
                        content_hash: row.get(2)?,
                        mode: mode.map(|m| m as u32),
                    })
                })?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await
    }

    async fn row_exists(&self, thread_id: &str, file_path: &str) -> Result<bool> {
        let tid = thread_id.to_string();
        let fp = file_path.to_string();
        self.db
            .execute(move |conn| {
                let n: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM shadow_edits WHERE thread_id = ?1 AND file_path = ?2",
                    rusqlite::params![tid, fp],
                    |row| row.get(0),
                )?;
                Ok(n > 0)
            })
            .await
    }

    /// Delete a thread's shadow_edits rows and garbage-collect any blobs
    /// that aren't referenced by another thread. Called from
    /// `delete_thread` IPC so shadow data doesn't accumulate indefinitely.
    pub async fn delete_thread_data(&self, thread_id: &str) -> Result<()> {
        let tid = thread_id.to_string();

        // Collect the content_hashes this thread uses, then delete the rows.
        let hashes_for_thread: Vec<String> = self
            .db
            .execute({
                let tid = tid.clone();
                move |conn| {
                    let mut stmt = conn.prepare(
                        "SELECT content_hash FROM shadow_edits \
                         WHERE thread_id = ?1 AND content_hash IS NOT NULL",
                    )?;
                    let rows = stmt.query_map(rusqlite::params![tid], |row| {
                        row.get::<_, Option<String>>(0)
                    })?;
                    let mut out = Vec::new();
                    for r in rows {
                        if let Some(h) = r? {
                            out.push(h);
                        }
                    }
                    Ok(out)
                }
            })
            .await?;

        let tid_for_delete = tid.clone();
        self.db
            .execute(move |conn| {
                conn.execute(
                    "DELETE FROM shadow_edits WHERE thread_id = ?1",
                    rusqlite::params![tid_for_delete],
                )?;
                Ok(())
            })
            .await?;

        // For each hash this thread referenced, GC the blob unless another
        // thread still points at it (content-addressed dedup across threads).
        let blob_root = self.blob_root.clone();
        for hash in hashes_for_thread {
            let still_referenced: bool = self
                .db
                .execute({
                    let h = hash.clone();
                    move |conn| {
                        let n: i64 = conn.query_row(
                            "SELECT COUNT(*) FROM shadow_edits WHERE content_hash = ?1",
                            rusqlite::params![h],
                            |row| row.get(0),
                        )?;
                        Ok(n > 0)
                    }
                })
                .await?;
            if !still_referenced {
                let blob_path = Self::blob_path_for(&blob_root, &hash);
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = std::fs::remove_file(&blob_path);
                })
                .await;
            }
        }

        Ok(())
    }
}

#[async_trait]
impl VersionTracker for ShadowVersionTracker {
    fn kind(&self) -> TrackerKind {
        TrackerKind::Shadow
    }

    async fn record_pre_edit(
        &self,
        thread_id: &str,
        _workspace_path: &Path,
        file_path: &Path,
    ) -> Result<()> {
        let file_path_s = file_path.to_string_lossy().to_string();

        // First-write-wins: idempotent per (thread, file).
        if self.row_exists(thread_id, &file_path_s).await? {
            return Ok(());
        }

        // Move the potentially-large file read and blob write off the
        // tokio worker. A 50MB agent-edited file would stall
        // SessionManager::consume_events for tens of ms otherwise.
        let blob_root = self.blob_root.clone();
        let file_path_for_worker = file_path.to_path_buf();
        let snapshot: PreEditSnapshot = tokio::task::spawn_blocking(move || -> Result<PreEditSnapshot> {
            if !file_path_for_worker.exists() {
                return Ok(PreEditSnapshot {
                    pre_existed: false,
                    content_hash: None,
                    mode: None,
                });
            }
            let bytes = std::fs::read(&file_path_for_worker).with_context(|| {
                format!("failed to read pre-edit file {}", file_path_for_worker.display())
            })?;
            let mode = std::fs::metadata(&file_path_for_worker)
                .ok()
                .map(|m| {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        m.permissions().mode()
                    }
                    #[cfg(not(unix))]
                    {
                        let _ = m;
                        0
                    }
                });

            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let hash = hex::encode(hasher.finalize());
            let path = ShadowVersionTracker::blob_path_for(&blob_root, &hash);
            if !path.exists() {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).ok();
                }
                std::fs::write(&path, &bytes).with_context(|| {
                    format!("failed to write shadow blob {}", path.display())
                })?;
            }
            Ok(PreEditSnapshot {
                pre_existed: true,
                content_hash: Some(hash),
                mode,
            })
        })
        .await
        .context("shadow blob task panicked")??;

        let now = Utc::now().to_rfc3339();
        let tid = thread_id.to_string();
        self.db
            .execute(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO shadow_edits \
                     (thread_id, file_path, pre_existed, content_hash, mode, recorded_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        tid,
                        file_path_s,
                        snapshot.pre_existed as i64,
                        snapshot.content_hash,
                        snapshot.mode.map(|m| m as i64),
                        now
                    ],
                )?;
                Ok(())
            })
            .await
            .context("failed to insert shadow_edits row")?;
        Ok(())
    }

    async fn diff(
        &self,
        thread_id: &str,
        workspace_path: &Path,
        file_paths: Option<&[PathBuf]>,
    ) -> Result<String> {
        let rows = self.load_rows(thread_id).await?;
        let filter: Option<std::collections::HashSet<String>> = file_paths.map(|paths| {
            paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect()
        });

        let blob_root = self.blob_root.clone();
        let workspace_path_buf = workspace_path.to_path_buf();
        tokio::task::spawn_blocking(move || -> Result<String> {
            let mut out = String::new();
            for row in rows {
                if let Some(f) = &filter {
                    if !f.contains(&row.file_path) {
                        continue;
                    }
                }

                let path = PathBuf::from(&row.file_path);
                let rel = path
                    .strip_prefix(&workspace_path_buf)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| row.file_path.clone());

                let current = if path.exists() {
                    std::fs::read(&path).unwrap_or_default()
                } else {
                    Vec::new()
                };

                let pre = if let Some(h) = &row.content_hash {
                    let blob_path = ShadowVersionTracker::blob_path_for(&blob_root, h);
                    std::fs::read(&blob_path).unwrap_or_default()
                } else {
                    Vec::new()
                };

                if pre == current {
                    continue;
                }

                let pre_label = if row.pre_existed {
                    format!("a/{rel}")
                } else {
                    "/dev/null".to_string()
                };
                let cur_label = if path.exists() {
                    format!("b/{rel}")
                } else {
                    "/dev/null".to_string()
                };

                out.push_str(&format!("diff --git a/{rel} b/{rel}\n"));

                // Binary detection: emit a git-style one-liner if either
                // side contains a NUL byte. Line-based unified diffs on
                // binary content would produce lossy garbage (replacement
                // chars from from_utf8_lossy + meaningless hunks).
                if is_binary(&pre) || is_binary(&current) {
                    out.push_str(&format!("Binary files {pre_label} and {cur_label} differ\n"));
                    continue;
                }

                out.push_str(&format!("--- {pre_label}\n"));
                out.push_str(&format!("+++ {cur_label}\n"));

                let pre_s = String::from_utf8_lossy(&pre);
                let cur_s = String::from_utf8_lossy(&current);
                let diff = TextDiff::from_lines(pre_s.as_ref(), cur_s.as_ref());
                for group in diff.grouped_ops(3) {
                    let (old_start, old_len, new_start, new_len) = hunk_ranges(&group);
                    out.push_str(&format!(
                        "@@ -{old_start},{old_len} +{new_start},{new_len} @@\n"
                    ));
                    for op in group {
                        for change in diff.iter_changes(&op) {
                            let sign = match change.tag() {
                                ChangeTag::Delete => '-',
                                ChangeTag::Insert => '+',
                                ChangeTag::Equal => ' ',
                            };
                            let value = change.value();
                            out.push(sign);
                            out.push_str(value);
                            if !value.ends_with('\n') {
                                out.push('\n');
                            }
                        }
                    }
                }
            }
            Ok(out)
        })
        .await
        .context("shadow diff task panicked")?
    }

    async fn revert(&self, thread_id: &str, _workspace_path: &Path) -> Result<()> {
        let rows = self.load_rows(thread_id).await?;
        let blob_root = self.blob_root.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            for row in rows {
                let path = PathBuf::from(&row.file_path);
                if row.pre_existed {
                    if let Some(hash) = &row.content_hash {
                        let blob_path = ShadowVersionTracker::blob_path_for(&blob_root, hash);
                        match std::fs::read(&blob_path) {
                            Ok(bytes) => {
                                if let Some(parent) = path.parent() {
                                    if !parent.exists() {
                                        std::fs::create_dir_all(parent).ok();
                                    }
                                }
                                if let Err(e) = std::fs::write(&path, &bytes) {
                                    warn!(error = %e, path = %path.display(), "failed to restore file");
                                    continue;
                                }
                                // Restore original mode so executable /
                                // read-only bits survive revert.
                                #[cfg(unix)]
                                if let Some(mode) = row.mode {
                                    use std::os::unix::fs::PermissionsExt;
                                    let perms = std::fs::Permissions::from_mode(mode);
                                    if let Err(e) = std::fs::set_permissions(&path, perms) {
                                        warn!(
                                            error = %e,
                                            path = %path.display(),
                                            "failed to restore file mode"
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, hash = %hash, "failed to read shadow blob during revert");
                            }
                        }
                    }
                } else if path.exists() {
                    if let Err(e) = std::fs::remove_file(&path) {
                        warn!(error = %e, path = %path.display(), "failed to delete tombstoned file");
                    }
                }
            }
            Ok(())
        })
        .await
        .context("shadow revert task panicked")?
    }

    async fn list_changed_files(
        &self,
        thread_id: &str,
        workspace_path: &Path,
    ) -> Result<Vec<ChangedFile>> {
        let rows = self.load_rows(thread_id).await?;
        let blob_root = self.blob_root.clone();
        let workspace_path_buf = workspace_path.to_path_buf();
        tokio::task::spawn_blocking(move || -> Result<Vec<ChangedFile>> {
            let mut out = Vec::new();
            for row in rows {
                let path = PathBuf::from(&row.file_path);
                let currently_exists = path.exists();
                let current_bytes = if currently_exists {
                    std::fs::read(&path).unwrap_or_default()
                } else {
                    Vec::new()
                };
                let pre_bytes = if let Some(h) = &row.content_hash {
                    let blob_path = ShadowVersionTracker::blob_path_for(&blob_root, h);
                    std::fs::read(&blob_path).unwrap_or_default()
                } else {
                    Vec::new()
                };

                if pre_bytes == current_bytes {
                    continue;
                }

                let action = match (row.pre_existed, currently_exists) {
                    (false, true) => FileAction::Created,
                    (true, false) => FileAction::Deleted,
                    _ => FileAction::Modified,
                };

                let rel = path
                    .strip_prefix(&workspace_path_buf)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| row.file_path.clone());

                out.push(ChangedFile {
                    absolute_path: path,
                    relative_path: rel,
                    action,
                });
            }
            Ok(out)
        })
        .await
        .context("shadow list_changed_files task panicked")?
    }
}

fn is_binary(bytes: &[u8]) -> bool {
    // Git's heuristic: a NUL byte in the first 8KB means binary. Cheap,
    // correct for the cases that matter (images, compiled artefacts).
    bytes.iter().take(8192).any(|&b| b == 0)
}

/// Compute unified-diff hunk range from a group of operations. Indices
/// are 1-based as per unified-diff convention.
fn hunk_ranges(
    group: &[similar::DiffOp],
) -> (usize, usize, usize, usize) {
    let mut old_start = usize::MAX;
    let mut new_start = usize::MAX;
    let mut old_len = 0usize;
    let mut new_len = 0usize;
    for op in group {
        let (os, _oe, ns, _ne) = match *op {
            similar::DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => (old_index, old_index + len, new_index, new_index + len),
            similar::DiffOp::Delete {
                old_index,
                old_len: ol,
                new_index,
            } => (old_index, old_index + ol, new_index, new_index),
            similar::DiffOp::Insert {
                old_index,
                new_index,
                new_len: nl,
            } => (old_index, old_index, new_index, new_index + nl),
            similar::DiffOp::Replace {
                old_index,
                old_len: ol,
                new_index,
                new_len: nl,
            } => (old_index, old_index + ol, new_index, new_index + nl),
        };
        if os < old_start {
            old_start = os;
        }
        if ns < new_start {
            new_start = ns;
        }
        let (old_consumed, new_consumed) = match *op {
            similar::DiffOp::Equal { len, .. } => (len, len),
            similar::DiffOp::Delete { old_len: ol, .. } => (ol, 0),
            similar::DiffOp::Insert { new_len: nl, .. } => (0, nl),
            similar::DiffOp::Replace {
                old_len: ol,
                new_len: nl,
                ..
            } => (ol, nl),
        };
        old_len += old_consumed;
        new_len += new_consumed;
    }
    // Unified diff uses 1-based line numbers; empty ranges use start 0.
    let old_start_display = if old_len == 0 { 0 } else { old_start + 1 };
    let new_start_display = if new_len == 0 { 0 } else { new_start + 1 };
    (old_start_display, old_len, new_start_display, new_len)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    struct Harness {
        _tmp: TempDir,
        workspace: PathBuf,
        blob_root: PathBuf,
        tracker: ShadowVersionTracker,
    }

    fn make_harness() -> Harness {
        let tmp = tempfile::tempdir().unwrap();
        let workspace = tmp.path().join("workspace");
        let blob_root = tmp.path().join("shadow-blobs");
        fs::create_dir_all(&workspace).unwrap();

        let db = crate::test_support::in_memory_db();
        let tracker = ShadowVersionTracker::new(db, blob_root.clone()).unwrap();
        Harness {
            _tmp: tmp,
            workspace,
            blob_root,
            tracker,
        }
    }

    fn read_blob_for_test(h: &Harness, hash: &str) -> Vec<u8> {
        let path = h.tracker.blob_path(hash);
        fs::read(&path).unwrap()
    }

    #[tokio::test]
    async fn record_pre_edit_stores_existing_file_content() {
        let h = make_harness();
        let file = h.workspace.join("a.txt");
        fs::write(&file, b"original").unwrap();
        h.tracker
            .record_pre_edit("t1", &h.workspace, &file)
            .await
            .unwrap();
        // Blob must exist under blob_root.
        let count = walkdir_count(&h.blob_root);
        assert_eq!(count, 1, "exactly one blob should be stored");
    }

    #[tokio::test]
    async fn record_pre_edit_is_idempotent() {
        let h = make_harness();
        let file = h.workspace.join("a.txt");
        fs::write(&file, b"original").unwrap();
        h.tracker
            .record_pre_edit("t1", &h.workspace, &file)
            .await
            .unwrap();
        // Mutate between calls — the original should still be what's captured.
        fs::write(&file, b"modified").unwrap();
        h.tracker
            .record_pre_edit("t1", &h.workspace, &file)
            .await
            .unwrap();

        let rows = h.tracker.load_rows("t1").await.unwrap();
        assert_eq!(rows.len(), 1);
        let hash = rows[0].content_hash.as_ref().unwrap();
        assert_eq!(read_blob_for_test(&h, hash), b"original");
    }

    #[tokio::test]
    async fn record_pre_edit_tombstones_missing_file() {
        let h = make_harness();
        let file = h.workspace.join("new.txt");
        h.tracker
            .record_pre_edit("t1", &h.workspace, &file)
            .await
            .unwrap();
        let rows = h.tracker.load_rows("t1").await.unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].pre_existed);
        assert!(rows[0].content_hash.is_none());
        assert!(rows[0].mode.is_none());
    }

    #[tokio::test]
    async fn record_pre_edit_captures_mode() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let h = make_harness();
            let file = h.workspace.join("script.sh");
            fs::write(&file, b"#!/bin/bash\necho hi").unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
            h.tracker
                .record_pre_edit("t1", &h.workspace, &file)
                .await
                .unwrap();
            let rows = h.tracker.load_rows("t1").await.unwrap();
            assert_eq!(rows.len(), 1);
            // mode() on a file returns a value whose low 9 bits are the
            // permission bits — compare those rather than asserting the
            // full u32 (which carries file-type bits).
            let stored = rows[0].mode.expect("mode should be recorded");
            assert_eq!(stored & 0o777, 0o755);
        }
    }

    #[tokio::test]
    async fn diff_unchanged_file_returns_empty() {
        let h = make_harness();
        let file = h.workspace.join("a.txt");
        fs::write(&file, b"hello\n").unwrap();
        h.tracker
            .record_pre_edit("t1", &h.workspace, &file)
            .await
            .unwrap();
        let diff = h.tracker.diff("t1", &h.workspace, None).await.unwrap();
        assert!(diff.is_empty(), "got: {diff}");
    }

    #[tokio::test]
    async fn diff_modified_file_produces_unified_diff() {
        let h = make_harness();
        let file = h.workspace.join("a.txt");
        fs::write(&file, "line1\nline2\n").unwrap();
        h.tracker
            .record_pre_edit("t1", &h.workspace, &file)
            .await
            .unwrap();
        fs::write(&file, "line1\nCHANGED\n").unwrap();
        let diff = h.tracker.diff("t1", &h.workspace, None).await.unwrap();
        assert!(diff.contains("--- a/a.txt"), "missing old header: {diff}");
        assert!(diff.contains("+++ b/a.txt"), "missing new header: {diff}");
        assert!(diff.contains("-line2"), "missing delete line: {diff}");
        assert!(diff.contains("+CHANGED"), "missing insert line: {diff}");
    }

    #[tokio::test]
    async fn diff_tombstoned_then_created_shows_full_add() {
        let h = make_harness();
        let file = h.workspace.join("new.txt");
        h.tracker
            .record_pre_edit("t1", &h.workspace, &file)
            .await
            .unwrap();
        fs::write(&file, "fresh\n").unwrap();
        let diff = h.tracker.diff("t1", &h.workspace, None).await.unwrap();
        assert!(diff.contains("--- /dev/null"), "missing /dev/null: {diff}");
        assert!(diff.contains("+++ b/new.txt"), "missing new header: {diff}");
        assert!(diff.contains("+fresh"), "missing add line: {diff}");
    }

    #[tokio::test]
    async fn diff_binary_content_emits_binary_marker() {
        let h = make_harness();
        let file = h.workspace.join("blob.bin");
        let original: Vec<u8> = (0..256u16).map(|n| n as u8).collect();
        fs::write(&file, &original).unwrap();
        h.tracker
            .record_pre_edit("t1", &h.workspace, &file)
            .await
            .unwrap();
        // Modify the binary content.
        let mut modified = original.clone();
        modified[0] = 0xff;
        fs::write(&file, &modified).unwrap();
        let diff = h.tracker.diff("t1", &h.workspace, None).await.unwrap();
        assert!(diff.contains("diff --git a/blob.bin b/blob.bin"), "got: {diff}");
        assert!(diff.contains("Binary files"), "got: {diff}");
        // Must NOT have emitted a line-based diff for binary content.
        assert!(!diff.contains("---"), "binary diff must not produce --- header: {diff}");
    }

    #[tokio::test]
    async fn revert_restores_modified_content() {
        let h = make_harness();
        let file = h.workspace.join("a.txt");
        fs::write(&file, b"original").unwrap();
        h.tracker
            .record_pre_edit("t1", &h.workspace, &file)
            .await
            .unwrap();
        fs::write(&file, b"modified").unwrap();
        h.tracker.revert("t1", &h.workspace).await.unwrap();
        let restored = fs::read(&file).unwrap();
        assert_eq!(restored, b"original");
    }

    #[tokio::test]
    async fn revert_restores_file_mode() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let h = make_harness();
            let file = h.workspace.join("script.sh");
            fs::write(&file, b"#!/bin/bash\necho before").unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
            h.tracker
                .record_pre_edit("t1", &h.workspace, &file)
                .await
                .unwrap();
            fs::write(&file, b"#!/bin/bash\necho after").unwrap();
            fs::set_permissions(&file, fs::Permissions::from_mode(0o644)).unwrap();
            h.tracker.revert("t1", &h.workspace).await.unwrap();

            let mode = fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755, "mode should be restored to 755");
        }
    }

    #[tokio::test]
    async fn revert_deletes_tombstoned_file() {
        let h = make_harness();
        let file = h.workspace.join("new.txt");
        h.tracker
            .record_pre_edit("t1", &h.workspace, &file)
            .await
            .unwrap();
        fs::write(&file, b"fresh").unwrap();
        h.tracker.revert("t1", &h.workspace).await.unwrap();
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn revert_is_idempotent() {
        let h = make_harness();
        let file = h.workspace.join("new.txt");
        h.tracker
            .record_pre_edit("t1", &h.workspace, &file)
            .await
            .unwrap();
        h.tracker.revert("t1", &h.workspace).await.unwrap();
        // Second revert shouldn't error.
        h.tracker.revert("t1", &h.workspace).await.unwrap();
    }

    #[tokio::test]
    async fn list_changed_files_filters_unchanged() {
        let h = make_harness();
        let a = h.workspace.join("a.txt");
        let b = h.workspace.join("b.txt");
        fs::write(&a, b"same").unwrap();
        fs::write(&b, b"orig").unwrap();
        h.tracker
            .record_pre_edit("t1", &h.workspace, &a)
            .await
            .unwrap();
        h.tracker
            .record_pre_edit("t1", &h.workspace, &b)
            .await
            .unwrap();
        fs::write(&b, b"changed").unwrap();

        let changed = h
            .tracker
            .list_changed_files("t1", &h.workspace)
            .await
            .unwrap();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].relative_path, "b.txt");
        assert_eq!(changed[0].action, FileAction::Modified);
    }

    #[tokio::test]
    async fn list_changed_files_reports_created_and_deleted() {
        let h = make_harness();
        let created = h.workspace.join("new.txt");
        let deleted = h.workspace.join("doomed.txt");
        fs::write(&deleted, b"bye").unwrap();

        h.tracker
            .record_pre_edit("t1", &h.workspace, &created)
            .await
            .unwrap();
        h.tracker
            .record_pre_edit("t1", &h.workspace, &deleted)
            .await
            .unwrap();

        fs::write(&created, b"hi").unwrap();
        fs::remove_file(&deleted).unwrap();

        let mut changed = h
            .tracker
            .list_changed_files("t1", &h.workspace)
            .await
            .unwrap();
        changed.sort_by_key(|c| c.relative_path.clone());
        assert_eq!(changed.len(), 2);
        assert_eq!(changed[0].relative_path, "doomed.txt");
        assert_eq!(changed[0].action, FileAction::Deleted);
        assert_eq!(changed[1].relative_path, "new.txt");
        assert_eq!(changed[1].action, FileAction::Created);
    }

    #[tokio::test]
    async fn delete_thread_data_removes_rows_and_gcs_blobs() {
        let h = make_harness();
        let file = h.workspace.join("a.txt");
        fs::write(&file, b"original").unwrap();
        h.tracker
            .record_pre_edit("t1", &h.workspace, &file)
            .await
            .unwrap();
        assert_eq!(walkdir_count(&h.blob_root), 1);

        h.tracker.delete_thread_data("t1").await.unwrap();

        let rows = h.tracker.load_rows("t1").await.unwrap();
        assert!(rows.is_empty(), "shadow_edits rows should be deleted");
        assert_eq!(
            walkdir_count(&h.blob_root),
            0,
            "blob file should have been GC'd"
        );
    }

    #[tokio::test]
    async fn delete_thread_data_keeps_blobs_referenced_by_other_threads() {
        let h = make_harness();
        // Same bytes tracked by two threads dedup to a single blob via
        // content-addressing. Deleting only one thread must keep the blob.
        let file_a = h.workspace.join("a.txt");
        let file_b = h.workspace.join("b.txt");
        fs::write(&file_a, b"shared").unwrap();
        fs::write(&file_b, b"shared").unwrap();
        h.tracker
            .record_pre_edit("t1", &h.workspace, &file_a)
            .await
            .unwrap();
        h.tracker
            .record_pre_edit("t2", &h.workspace, &file_b)
            .await
            .unwrap();
        assert_eq!(walkdir_count(&h.blob_root), 1, "content-address dedup");

        h.tracker.delete_thread_data("t1").await.unwrap();
        assert_eq!(
            walkdir_count(&h.blob_root),
            1,
            "blob still referenced by t2"
        );
    }

    fn walkdir_count(root: &Path) -> usize {
        let mut count = 0;
        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    count += walkdir_count(&entry.path());
                } else if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
                    count += 1;
                }
            }
        }
        count
    }
}
