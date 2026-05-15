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
        commit_in_repo(workspace_path, message, files, false).await
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
                    last_hash = commit_in_repo(&repo_path, message, Some(&relative_files), false).await?;
                }
            }
            _ => {
                for repo in &repos {
                    let has_changes = run_git(repo, &["status", "--porcelain"]).await
                        .map(|o| !o.trim().is_empty())
                        .unwrap_or(false);
                    if has_changes {
                        last_hash = commit_in_repo(repo, message, None, false).await?;
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

async fn commit_in_repo(repo_path: &Path, message: &str, files: Option<&[String]>, amend: bool) -> Result<String> {
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

    let commit_args = if amend {
        vec!["commit", "--amend", "-m", message]
    } else {
        vec!["commit", "-m", message]
    };
    run_git(repo_path, &commit_args)
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

pub async fn get_file_diff(workspace_path: &Path, file_path: &str) -> Result<String> {
    let path = Path::new(file_path);

    // If absolute path, find its repo directly
    if path.is_absolute() {
        let dir = if path.is_file() || !path.exists() {
            path.parent().unwrap_or(path)
        } else {
            path
        };
        if let Some(repo_root) = find_repo_root(dir).await {
            let relative = path.strip_prefix(&repo_root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| file_path.to_string());
            return get_diff_in_repo(&repo_root, &relative).await;
        }
        anyhow::bail!("file {} is not in a git repository", file_path);
    }

    // Relative path: resolve against workspace
    if is_git_repo(workspace_path).await {
        return get_diff_in_repo(workspace_path, file_path).await;
    }

    let repos = find_git_repos(workspace_path).await;
    for repo in &repos {
        let rel_prefix = repo.strip_prefix(workspace_path).unwrap_or(Path::new(""));
        if path.starts_with(rel_prefix) {
            let repo_relative = path.strip_prefix(rel_prefix).unwrap_or(path);
            return get_diff_in_repo(repo, repo_relative.to_str().unwrap_or(file_path)).await;
        }
    }

    anyhow::bail!("file {} not found in any git repository under {}", file_path, workspace_path.display())
}

async fn get_diff_in_repo(repo_path: &Path, file_path: &str) -> Result<String> {
    let full_path = repo_path.join(file_path);
    let is_tracked = run_git(repo_path, &["ls-files", file_path])
        .await
        .map(|o| !o.trim().is_empty())
        .unwrap_or(false);

    if is_tracked {
        run_git(repo_path, &["diff", "HEAD", "--", file_path]).await
    } else if full_path.exists() {
        let content = tokio::fs::read_to_string(&full_path).await
            .with_context(|| format!("failed to read untracked file {}", full_path.display()))?;
        let lines: Vec<&str> = content.lines().collect();
        let count = lines.len();
        let mut diff = format!("diff --git a/{f} b/{f}\nnew file mode 100644\n--- /dev/null\n+++ b/{f}\n@@ -0,0 +1,{count} @@\n", f = file_path);
        for line in &lines {
            diff.push('+');
            diff.push_str(line);
            diff.push('\n');
        }
        Ok(diff)
    } else {
        Ok(String::new())
    }
}

pub async fn get_workspace_diff(workspace_path: &Path, files: Option<&[String]>) -> Result<String> {
    if is_git_repo(workspace_path).await {
        let mut args = vec!["diff", "HEAD"];
        if let Some(file_list) = files {
            args.push("--");
            let owned: Vec<&str> = file_list.iter().map(|s| s.as_str()).collect();
            args.extend(owned);
        }
        return run_git(workspace_path, &args).await;
    }

    let repos = find_git_repos(workspace_path).await;
    let mut combined = String::new();
    for repo in &repos {
        let prefix = repo.strip_prefix(workspace_path).unwrap_or(Path::new(""));
        let mut args = vec!["diff", "HEAD"];
        if let Some(file_list) = files {
            let repo_files: Vec<&str> = file_list.iter()
                .filter_map(|f| Path::new(f.as_str()).strip_prefix(prefix).ok())
                .map(|p| p.to_str().unwrap_or(""))
                .filter(|s| !s.is_empty())
                .collect();
            if repo_files.is_empty() { continue; }
            args.push("--");
            args.extend(repo_files);
        }
        if let Ok(diff) = run_git(repo, &args).await {
            if !diff.trim().is_empty() {
                combined.push_str(&diff);
                if !combined.ends_with('\n') {
                    combined.push('\n');
                }
            }
        }
    }
    Ok(combined)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileGitStatus {
    pub absolute_path: String,
    pub relative_path: String,
    pub status: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoFileStatus {
    pub repo_path: String,
    pub repo_name: String,
    pub files: Vec<FileGitStatus>,
}

pub async fn get_files_git_status(file_paths: &[String]) -> Result<Vec<RepoFileStatus>> {
    let mut repo_cache: HashMap<PathBuf, Vec<(String, PathBuf)>> = HashMap::new();

    for file_path in file_paths {
        let path = Path::new(file_path);
        let dir = if path.is_file() {
            path.parent().unwrap_or(path)
        } else if path.exists() {
            path
        } else {
            path.parent().unwrap_or(path)
        };

        let repo_root = find_repo_root(dir).await;
        if let Some(root) = repo_root {
            repo_cache.entry(root).or_default().push((file_path.clone(), path.to_path_buf()));
        }
    }

    let mut results = Vec::new();
    for (repo_root, files) in repo_cache {
        let repo_name = repo_root.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| repo_root.to_string_lossy().to_string());

        let mut file_statuses = Vec::new();
        for (abs_path, full_path) in &files {
            let relative = full_path.strip_prefix(&repo_root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| abs_path.clone());

            let status = run_git(&repo_root, &["status", "--porcelain", "--", &relative])
                .await
                .unwrap_or_default();
            let git_status = if status.trim().is_empty() {
                String::new()
            } else {
                status.trim().chars().take(2).collect::<String>().trim().to_string()
            };

            if !git_status.is_empty() {
                file_statuses.push(FileGitStatus {
                    absolute_path: abs_path.clone(),
                    relative_path: relative,
                    status: git_status,
                });
            }
        }

        if !file_statuses.is_empty() {
            results.push(RepoFileStatus {
                repo_path: repo_root.to_string_lossy().to_string(),
                repo_name,
                files: file_statuses,
            });
        }
    }

    Ok(results)
}

pub(crate) async fn find_repo_root(start: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(start)
        .output()
        .await
        .ok()?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(PathBuf::from(path))
    } else {
        None
    }
}

/// Initialize a new git repo with all existing files committed.
/// Returns the repo root path on success.
pub(crate) async fn init_repo(dir: &Path) -> Result<PathBuf> {
    run_git(dir, &["init", "-q"]).await.context("git init failed")?;
    // Stage everything so worktrees see the same files as the main
    // checkout. Respects any existing .gitignore in the directory.
    run_git(dir, &["add", "."]).await.context("git add failed")?;
    run_git(
        dir,
        &[
            "-c", "user.email=panes@local",
            "-c", "user.name=Panes",
            "commit", "-q", "--allow-empty", "-m", "panes: auto-init for worktree isolation",
        ],
    )
    .await
    .context("initial commit failed")?;
    info!(path = %dir.display(), "auto-initialized git repo for worktree isolation");
    Ok(dir.to_path_buf())
}

pub async fn list_git_repos(workspace_path: &Path) -> Result<Vec<String>> {
    if is_git_repo(workspace_path).await {
        return Ok(vec![String::new()]);
    }

    let repos = find_git_repos(workspace_path).await;
    Ok(repos.iter()
        .filter_map(|r| r.strip_prefix(workspace_path).ok())
        .map(|p| p.to_string_lossy().to_string())
        .collect())
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoCommitParams {
    pub repo_path: String,
    pub message: String,
    pub files: Vec<String>,
    #[serde(default)]
    pub amend: bool,
}

pub async fn commit_repos(commits: &[RepoCommitParams]) -> Result<Vec<String>> {
    let mut hashes = Vec::new();
    for params in commits {
        let repo_path = Path::new(&params.repo_path);
        if !repo_path.join(".git").exists() && !repo_path.join(".git").is_file() {
            let root = find_repo_root(repo_path).await;
            if root.is_none() {
                anyhow::bail!("no git repository at {}", params.repo_path);
            }
        }
        let files: Vec<String> = params.files.clone();
        let file_refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();

        let hash = if file_refs.is_empty() {
            commit_in_repo(repo_path, &params.message, None, params.amend).await?
        } else {
            let borrowed: Vec<String> = file_refs.iter().map(|s| s.to_string()).collect();
            commit_in_repo(repo_path, &params.message, Some(&borrowed), params.amend).await?
        };
        hashes.push(hash);
    }
    Ok(hashes)
}

pub async fn generate_commit_message(_workspace_path: &str, diff: &str) -> Result<String> {
    let claude_path = std::env::var("PANES_CLAUDE_PATH").unwrap_or_else(|_| "claude".to_string());
    let prompt = format!(
        "Generate a concise git commit message (subject line + optional body) for the following diff. \
         Use conventional commit format (feat/fix/refactor/docs/chore). Subject line max 72 chars. \
         Body should explain WHY, not WHAT. Output ONLY the commit message, nothing else.\n\n```diff\n{}\n```",
        if diff.len() > 8000 { &diff[..8000] } else { diff }
    );

    let output = Command::new(&claude_path)
        .args(["--model", "haiku", "-p", &prompt, "--output-format", "text"])
        .output()
        .await
        .with_context(|| "failed to run claude for commit message generation")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("commit message generation failed: {}", stderr.trim());
    }

    let msg = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(msg)
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

    #[tokio::test]
    async fn test_get_file_diff_modified() {
        let dir = make_git_repo().await;
        fs::write(dir.path().join("initial.txt"), "modified content").unwrap();

        let diff = get_file_diff(dir.path(), "initial.txt").await.unwrap();
        assert!(diff.contains("-hello"));
        assert!(diff.contains("+modified content"));
    }

    #[tokio::test]
    async fn test_get_file_diff_untracked() {
        let dir = make_git_repo().await;
        fs::write(dir.path().join("new.txt"), "line 1\nline 2\n").unwrap();

        let diff = get_file_diff(dir.path(), "new.txt").await.unwrap();
        assert!(diff.contains("new file"));
        assert!(diff.contains("+line 1"));
        assert!(diff.contains("+line 2"));
    }

    #[tokio::test]
    async fn test_get_workspace_diff_single_repo() {
        let dir = make_git_repo().await;
        fs::write(dir.path().join("initial.txt"), "changed").unwrap();

        let diff = get_workspace_diff(dir.path(), None).await.unwrap();
        assert!(diff.contains("-hello"));
        assert!(diff.contains("+changed"));
    }

    #[tokio::test]
    async fn test_get_files_git_status_groups_by_repo() {
        let dir = make_git_repo().await;
        fs::write(dir.path().join("initial.txt"), "modified").unwrap();
        fs::write(dir.path().join("new.txt"), "new file").unwrap();

        let paths = vec![
            dir.path().join("initial.txt").to_string_lossy().to_string(),
            dir.path().join("new.txt").to_string_lossy().to_string(),
        ];
        let status = get_files_git_status(&paths).await.unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].files.len(), 2);
    }

    #[tokio::test]
    async fn test_get_files_git_status_no_changes() {
        let dir = make_git_repo().await;
        let paths = vec![dir.path().join("initial.txt").to_string_lossy().to_string()];
        let status = get_files_git_status(&paths).await.unwrap();
        assert!(status.is_empty());
    }

    #[tokio::test]
    async fn test_list_git_repos_single() {
        let dir = make_git_repo().await;
        let repos = list_git_repos(dir.path()).await.unwrap();
        assert_eq!(repos, vec![""]);
    }

    #[tokio::test]
    async fn test_list_git_repos_multi() {
        let workspace = tempfile::tempdir().unwrap();
        let repo_a = workspace.path().join("repo-a");
        let repo_b = workspace.path().join("repo-b");
        fs::create_dir_all(&repo_a).unwrap();
        fs::create_dir_all(&repo_b).unwrap();

        for repo in [&repo_a, &repo_b] {
            run_git(repo, &["init"]).await.unwrap();
        }

        let mut repos = list_git_repos(workspace.path()).await.unwrap();
        repos.sort();
        assert_eq!(repos, vec!["repo-a", "repo-b"]);
    }

    #[tokio::test]
    async fn test_commit_repos_independent_messages() {
        let workspace = tempfile::tempdir().unwrap();
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

        fs::write(repo_a.join("a.txt"), "change a").unwrap();
        fs::write(repo_b.join("b.txt"), "change b").unwrap();

        let commits = vec![
            RepoCommitParams {
                repo_path: repo_a.to_string_lossy().to_string(),
                message: "commit for repo-a".to_string(),
                files: vec!["a.txt".to_string()],
                amend: false,
            },
            RepoCommitParams {
                repo_path: repo_b.to_string_lossy().to_string(),
                message: "commit for repo-b".to_string(),
                files: vec!["b.txt".to_string()],
                amend: false,
            },
        ];

        let hashes = commit_repos(&commits).await.unwrap();
        assert_eq!(hashes.len(), 2);

        let log_a = run_git(&repo_a, &["log", "--oneline", "-1"]).await.unwrap();
        assert!(log_a.contains("commit for repo-a"));

        let log_b = run_git(&repo_b, &["log", "--oneline", "-1"]).await.unwrap();
        assert!(log_b.contains("commit for repo-b"));
    }
}
