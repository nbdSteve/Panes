use std::collections::HashMap;
use std::path::{Path, PathBuf};

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

pub async fn commit(workspace_path: &Path, message: &str, files: Option<&[String]>) -> Result<String> {
    info!(
        workspace = %workspace_path.display(),
        files = ?files,
        "committing changes"
    );

    if is_git_repo(workspace_path).await {
        commit_in_repo(workspace_path, message, files).await
    } else {
        let repos = find_git_repos(workspace_path).await;
        if repos.is_empty() {
            anyhow::bail!("no git repository found in {}", workspace_path.display());
        }
        let mut last_hash = String::new();
        match files {
            Some(paths) if !paths.is_empty() => {
                let grouped = group_files_by_repo(workspace_path, paths, &repos);
                for (repo_path, repo_files) in grouped {
                    let relative_files: Vec<String> = repo_files.iter().map(|f| {
                        Path::new(f)
                            .strip_prefix(repo_path.strip_prefix(workspace_path).unwrap_or(Path::new("")))
                            .unwrap_or(Path::new(f))
                            .to_string_lossy()
                            .to_string()
                    }).collect();
                    last_hash = commit_in_repo(&repo_path, message, Some(&relative_files)).await?;
                }
            }
            _ => {
                for repo in &repos {
                    let has_changes = run_git(repo, &["status", "--porcelain"]).await
                        .map(|o| !o.trim().is_empty())
                        .unwrap_or(false);
                    if has_changes {
                        last_hash = commit_in_repo(repo, message, None).await?;
                    }
                }
            }
        }
        if last_hash.is_empty() {
            anyhow::bail!("no changes to commit");
        }
        Ok(last_hash)
    }
}

async fn commit_in_repo(repo_path: &Path, message: &str, files: Option<&[String]>) -> Result<String> {
    match files {
        Some(paths) if !paths.is_empty() => {
            let mut args = vec!["add", "--"];
            let owned: Vec<&str> = paths.iter().map(|s| s.as_str()).collect();
            args.extend(owned);
            run_git(repo_path, &args)
                .await
                .with_context(|| format!("failed to stage selected files: {:?}", paths))?;
        }
        _ => {
            run_git(repo_path, &["add", "-A"])
                .await
                .with_context(|| format!("failed to stage changes in {}", repo_path.display()))?;
        }
    }

    run_git(repo_path, &["commit", "-m", message])
        .await
        .with_context(|| format!("failed to create commit in {}", repo_path.display()))?;

    let hash = run_git(repo_path, &["rev-parse", "HEAD"]).await?;
    info!(commit = %hash.trim(), repo = %repo_path.display(), "committed changes");
    Ok(hash.trim().to_string())
}

pub async fn get_changed_files(workspace_path: &Path) -> Result<Vec<String>> {
    if is_git_repo(workspace_path).await {
        let output = run_git(workspace_path, &["status", "--porcelain"]).await?;
        return Ok(output
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.to_string())
            .collect());
    }

    let repos = find_git_repos(workspace_path).await;
    let mut all_files = Vec::new();
    for repo in repos {
        let output = run_git(&repo, &["status", "--porcelain"]).await.unwrap_or_default();
        let prefix = repo.strip_prefix(workspace_path).unwrap_or(Path::new(""));
        for line in output.lines().filter(|l| !l.trim().is_empty()) {
            let status = &line[..3];
            let file_path = &line[3..];
            let full_path = prefix.join(file_path);
            all_files.push(format!("{}{}", status, full_path.to_string_lossy()));
        }
    }
    Ok(all_files)
}

const SKIP_DIRS: &[&str] = &[
    "node_modules", "target", "build", "dist", ".gradle", ".idea", ".vscode",
    "__pycache__", ".tox", "venv", ".venv", ".cache",
];

fn should_skip_dir(name: &str) -> bool {
    name.starts_with('.') && name != ".git" || SKIP_DIRS.contains(&name)
}

async fn find_git_repos(workspace_path: &Path) -> Vec<PathBuf> {
    let mut repos = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(workspace_path).await else {
        return repos;
    };
    let mut dirs_to_check = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
            let name = entry.file_name();
            if !should_skip_dir(name.to_str().unwrap_or("")) {
                dirs_to_check.push(entry.path());
            }
        }
    }
    for dir in dirs_to_check {
        if dir.join(".git").exists() {
            repos.push(dir);
        } else {
            if let Ok(mut sub) = tokio::fs::read_dir(&dir).await {
                while let Ok(Some(entry)) = sub.next_entry().await {
                    if entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false) {
                        let name = entry.file_name();
                        if !should_skip_dir(name.to_str().unwrap_or("")) {
                            let p = entry.path();
                            if p.join(".git").exists() {
                                repos.push(p);
                            }
                        }
                    }
                }
            }
        }
    }
    repos
}

fn group_files_by_repo<'a>(
    workspace_path: &Path,
    files: &'a [String],
    repos: &[PathBuf],
) -> HashMap<PathBuf, Vec<&'a String>> {
    let mut grouped: HashMap<PathBuf, Vec<&'a String>> = HashMap::new();
    let mut repo_prefixes: Vec<(&PathBuf, &str)> = repos.iter()
        .filter_map(|r| r.strip_prefix(workspace_path).ok().map(|rel| (r, rel.to_str().unwrap_or(""))))
        .collect();
    // Sort longest prefix first for correct matching
    repo_prefixes.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    for file in files {
        let matched = repo_prefixes.iter().find(|(_, prefix)| file.starts_with(prefix));
        if let Some((repo_path, _)) = matched {
            grouped.entry((*repo_path).clone()).or_default().push(file);
        }
    }
    grouped
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
        commit(dir.path(), "agent commit", None).await.unwrap();

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

        let hash = commit(dir.path(), "add test file", None).await.unwrap();
        assert!(!hash.is_empty());

        let changed_after = get_changed_files(dir.path()).await.unwrap();
        assert!(changed_after.is_empty());
    }

    async fn make_empty_git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init"]).await.unwrap();
        run_git(dir.path(), &["config", "user.email", "test@test.com"]).await.unwrap();
        run_git(dir.path(), &["config", "user.name", "Test"]).await.unwrap();
        dir
    }

    #[tokio::test]
    async fn test_snapshot_fails_on_empty_repo() {
        let dir = make_empty_git_repo().await;
        let result = snapshot(dir.path()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_is_git_repo_on_empty_repo() {
        let dir = make_empty_git_repo().await;
        assert!(is_git_repo(dir.path()).await);
    }

    #[tokio::test]
    async fn test_revert_with_invalid_hash() {
        let dir = make_git_repo().await;
        let bad_snapshot = SnapshotRef {
            commit_hash: "0000000000000000000000000000000000000000".to_string(),
        };
        let result = revert(dir.path(), &bad_snapshot).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_multi_repo_workspace() {
        let workspace = tempfile::tempdir().unwrap();

        // Create two sub-repos
        let repo_a = workspace.path().join("repo-a");
        let repo_b = workspace.path().join("repo-b");
        fs::create_dir_all(&repo_a).unwrap();
        fs::create_dir_all(&repo_b).unwrap();

        for repo in [&repo_a, &repo_b] {
            run_git(repo, &["init"]).await.unwrap();
            run_git(repo, &["config", "user.email", "test@test.com"]).await.unwrap();
            run_git(repo, &["config", "user.name", "Test"]).await.unwrap();
            fs::write(repo.join("init.txt"), "init").unwrap();
            run_git(repo, &["add", "-A"]).await.unwrap();
            run_git(repo, &["commit", "-m", "init"]).await.unwrap();
        }

        // Make changes in both repos
        fs::write(repo_a.join("a.txt"), "change").unwrap();
        fs::write(repo_b.join("b.txt"), "change").unwrap();

        // get_changed_files should find changes in both
        let changed = get_changed_files(workspace.path()).await.unwrap();
        assert_eq!(changed.len(), 2);
        assert!(changed.iter().any(|f| f.contains("a.txt")));
        assert!(changed.iter().any(|f| f.contains("b.txt")));

        // commit all should succeed
        let hash = commit(workspace.path(), "multi-repo commit", None).await.unwrap();
        assert!(!hash.is_empty());

        // No changes remaining
        let after = get_changed_files(workspace.path()).await.unwrap();
        assert!(after.is_empty());
    }
}
