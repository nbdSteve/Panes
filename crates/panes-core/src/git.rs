use std::path::Path;

use anyhow::{Context, Result};
use tokio::process::Command;
use tracing::info;

#[derive(Debug, Clone)]
pub struct SnapshotRef {
    pub commit_hash: String,
}

pub async fn is_git_repo(workspace_path: &Path) -> bool {
    workspace_path.join(".git").exists()
}

pub async fn snapshot(workspace_path: &Path) -> Result<SnapshotRef> {
    let commit_hash = run_git(workspace_path, &["rev-parse", "HEAD"])
        .await
        .context("failed to get HEAD commit hash")?;

    info!(commit = %commit_hash.trim(), "created pre-thread snapshot");

    Ok(SnapshotRef {
        commit_hash: commit_hash.trim().to_string(),
    })
}

pub async fn revert(workspace_path: &Path, snapshot: &SnapshotRef) -> Result<()> {
    info!(
        workspace = %workspace_path.display(),
        commit = %snapshot.commit_hash,
        "reverting to snapshot"
    );

    run_git(workspace_path, &["reset", "--hard", &snapshot.commit_hash])
        .await
        .context("failed to reset to snapshot")?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    async fn make_git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init"]).await.unwrap();
        run_git(dir.path(), &["config", "user.email", "test@test.com"]).await.unwrap();
        run_git(dir.path(), &["config", "user.name", "Test"]).await.unwrap();

        fs::write(dir.path().join("initial.txt"), "hello").unwrap();
        run_git(dir.path(), &["add", "-A"]).await.unwrap();
        run_git(dir.path(), &["commit", "-m", "initial"]).await.unwrap();
        dir
    }

    #[tokio::test]
    async fn test_snapshot_records_head() {
        let dir = make_git_repo().await;
        let head = run_git(dir.path(), &["rev-parse", "HEAD"]).await.unwrap();
        let snap = snapshot(dir.path()).await.unwrap();
        assert_eq!(snap.commit_hash, head.trim());
    }

    #[tokio::test]
    async fn test_revert_restores_to_snapshot() {
        let dir = make_git_repo().await;
        let snap = snapshot(dir.path()).await.unwrap();

        // Agent makes changes and commits
        fs::write(dir.path().join("new_file.txt"), "agent wrote this").unwrap();
        commit(dir.path(), "agent commit").await.unwrap();

        assert!(dir.path().join("new_file.txt").exists());

        // Revert to snapshot
        revert(dir.path(), &snap).await.unwrap();

        assert!(!dir.path().join("new_file.txt").exists());
        let head = run_git(dir.path(), &["rev-parse", "HEAD"]).await.unwrap();
        assert_eq!(head.trim(), snap.commit_hash);
    }

    #[tokio::test]
    async fn test_revert_cleans_untracked_files() {
        let dir = make_git_repo().await;
        let snap = snapshot(dir.path()).await.unwrap();

        // Create untracked file (not committed)
        fs::write(dir.path().join("untracked.txt"), "junk").unwrap();

        revert(dir.path(), &snap).await.unwrap();

        assert!(!dir.path().join("untracked.txt").exists());
    }

    #[tokio::test]
    async fn test_revert_noop_when_no_changes() {
        let dir = make_git_repo().await;
        let snap = snapshot(dir.path()).await.unwrap();

        // No changes made — revert should be a no-op
        revert(dir.path(), &snap).await.unwrap();

        let head = run_git(dir.path(), &["rev-parse", "HEAD"]).await.unwrap();
        assert_eq!(head.trim(), snap.commit_hash);
    }

    #[tokio::test]
    async fn test_is_git_repo() {
        let dir = make_git_repo().await;
        assert!(is_git_repo(dir.path()).await);

        let non_git = tempfile::tempdir().unwrap();
        assert!(!is_git_repo(non_git.path()).await);
    }

    #[tokio::test]
    async fn test_commit_and_get_changed_files() {
        let dir = make_git_repo().await;

        fs::write(dir.path().join("test.txt"), "content").unwrap();
        let changed = get_changed_files(dir.path()).await.unwrap();
        assert!(!changed.is_empty());

        let hash = commit(dir.path(), "add test file").await.unwrap();
        assert!(!hash.is_empty());

        let changed_after = get_changed_files(dir.path()).await.unwrap();
        assert!(changed_after.is_empty());
    }
}
