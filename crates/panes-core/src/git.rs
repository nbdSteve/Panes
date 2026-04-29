use std::path::Path;

use anyhow::{Context, Result};
use tokio::process::Command;
use tracing::info;

#[derive(Debug, Clone)]
pub struct SnapshotRef {
    pub commit_hash: String,
    pub had_dirty_changes: bool,
    pub stash_ref: Option<String>,
}

pub async fn is_git_repo(workspace_path: &Path) -> bool {
    workspace_path.join(".git").exists()
}

pub async fn snapshot(workspace_path: &Path, thread_id: &str) -> Result<SnapshotRef> {
    let commit_hash = run_git(workspace_path, &["rev-parse", "HEAD"])
        .await
        .context("failed to get HEAD commit hash")?;

    // Check if working tree is dirty
    let status = run_git(workspace_path, &["status", "--porcelain"]).await?;
    let had_dirty_changes = !status.trim().is_empty();

    let stash_ref = if had_dirty_changes {
        let stash_msg = format!("panes:thread:{thread_id}:pre");
        run_git(
            workspace_path,
            &["stash", "push", "-m", &stash_msg, "--include-untracked"],
        )
        .await
        .context("failed to stash dirty changes")?;

        // Get the stash ref
        let stash_list = run_git(workspace_path, &["stash", "list", "--format=%H", "-1"]).await?;
        let stash_hash = stash_list.trim().to_string();

        // Pop the stash to restore working state (we just wanted to record the ref)
        run_git(workspace_path, &["stash", "pop"]).await.ok();

        info!(
            thread_id,
            stash = %stash_hash,
            "stashed dirty changes before thread"
        );
        Some(stash_hash)
    } else {
        None
    };

    info!(
        thread_id,
        commit = %commit_hash.trim(),
        dirty = had_dirty_changes,
        "created pre-thread snapshot"
    );

    Ok(SnapshotRef {
        commit_hash: commit_hash.trim().to_string(),
        had_dirty_changes,
        stash_ref,
    })
}

pub async fn revert(workspace_path: &Path, _snapshot: &SnapshotRef) -> Result<()> {
    info!(workspace = %workspace_path.display(), "reverting all changes");

    // Discard all changes since the snapshot
    run_git(workspace_path, &["checkout", "."])
        .await
        .context("failed to checkout clean state")?;

    run_git(workspace_path, &["clean", "-fd"])
        .await
        .context("failed to clean untracked files")?;

    Ok(())
}

pub async fn commit(workspace_path: &Path, message: &str) -> Result<String> {
    run_git(workspace_path, &["add", "-A"])
        .await
        .context("failed to stage changes")?;

    run_git(workspace_path, &["commit", "-m", message])
        .await
        .context("failed to create commit")?;

    let hash = run_git(workspace_path, &["rev-parse", "HEAD"]).await?;
    info!(commit = %hash.trim(), "committed changes");

    Ok(hash.trim().to_string())
}

pub async fn get_changed_files(workspace_path: &Path) -> Result<Vec<String>> {
    let output = run_git(workspace_path, &["status", "--porcelain"]).await?;
    Ok(output
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect())
}

async fn run_git(workspace_path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_path)
        .output()
        .await
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git {} failed: {}", args.join(" "), stderr.trim());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
