//! Per-thread git worktree lifecycle for Phase 2 concurrent threads.
//!
//! A worktree is a second checkout of a git repository at a separate path
//! sharing the same `.git` directory. Each concurrent Panes thread gets its
//! own worktree so agents can't step on each other's file edits.
//!
//! This module owns only the worktree primitives (create / remove / merge /
//! orphan cleanup). Phase 1's snapshot/revert/diff helpers in `git.rs` still
//! work inside a worktree unchanged — a worktree is just another checkout.
//!
//! All functions here are synchronous because `git2` is. Callers at the
//! `SessionManager` layer wrap invocations in `tokio::task::spawn_blocking`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::{fs, io};

use anyhow::{Context, Result, anyhow};
use git2::{
    BranchType, Repository, ResetType, WorktreeAddOptions, WorktreePruneOptions,
};
use tracing::warn;

/// Concrete handle for a worktree Panes has created. Returned by `create`
/// and stored on `ActiveThread` + persisted in the `threads.worktree_path`
/// column so it survives an app restart.
#[derive(Debug, Clone)]
pub struct WorktreeHandle {
    /// Absolute filesystem path to the worktree's working directory.
    pub path: PathBuf,
    /// Branch name git created to back the worktree. Deleted on removal.
    pub branch: String,
    /// Commit hash the branch was created from. Kept for diagnostics —
    /// snapshots still use `git::snapshot` on the worktree path itself.
    pub base_commit: String,
}

/// Outcome of merging a worktree's branch back into the main repo's HEAD.
#[derive(Debug)]
pub enum MergeOutcome {
    /// No new commits on the worktree branch relative to HEAD.
    UpToDate,
    /// HEAD could be moved forward without a merge commit.
    FastForwarded { commit: String },
    /// A real merge commit was created with two parents.
    Merged { commit: String },
    /// Merge aborted due to conflicts. The main repo's index, HEAD, and
    /// working tree are restored to their pre-merge state. Files listed
    /// are those git flagged as conflicting.
    Conflicts { files: Vec<String> },
}

/// Whole-merge conflict strategy for Option A of the Phase 2 resolution
/// UX. When a three-way merge produces conflicts, callers can retry with
/// a blanket preference for one side instead of aborting.
///
/// Option B (per-file resolution + 3-way diff viewer) is documented as
/// future work in the Phase 2 follow-up plan; it'd build on this same
/// IPC surface by adding conflict-inspection + per-file-resolve calls
/// rather than picking a whole-merge side up front.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeStrategy {
    /// Classic three-way merge. On conflict, main is restored and the
    /// outcome is `Conflicts { files }`. Caller is expected to surface
    /// the list and ask the user which side to prefer.
    Auto,
    /// On conflict, keep the worktree's version of the conflicted file
    /// (labelled "Use yours" in the UI — the worktree is the user's
    /// side because they sent the prompt).
    PreferTheirs,
    /// On conflict, keep main's version of the conflicted file
    /// (labelled "Keep main").
    PreferOurs,
}

/// A worktree directory under `worktrees_root` with no matching active
/// thread, discovered at startup. Used for crash recovery.
#[derive(Debug, Clone)]
pub struct OrphanedWorktree {
    /// Absolute path to the orphaned worktree directory.
    pub path: PathBuf,
    /// Thread id extracted from the directory name, for logging. Panes
    /// names worktree dirs by the full thread id so this is reliable.
    pub thread_id: String,
}

/// Create a new worktree for `thread_id` rooted under `worktrees_root`.
///
/// The worktree path must not pre-exist — libgit2 creates the directory
/// and refuses if it already exists. Branch name is `panes/<id[..8]>` so
/// users running `git branch` can see which branches are Panes-managed.
pub fn create(
    repo_root: &Path,
    thread_id: &str,
    worktrees_root: &Path,
) -> Result<WorktreeHandle> {
    fs::create_dir_all(worktrees_root).with_context(|| {
        format!("failed to create worktrees root {}", worktrees_root.display())
    })?;

    let worktree_path = worktrees_root.join(thread_id);
    if worktree_path.exists() {
        return Err(anyhow!(
            "worktree path {} already exists — leftover from a prior run?",
            worktree_path.display()
        ));
    }

    let repo = Repository::open(repo_root)
        .with_context(|| format!("failed to open repo at {}", repo_root.display()))?;

    // Anchor the branch on the current HEAD commit so the worktree starts
    // from the same code the user is looking at in the main checkout.
    let head_commit = repo
        .head()
        .and_then(|h| h.peel_to_commit())
        .context("repo has no HEAD — is it empty?")?;
    let base_commit = head_commit.id().to_string();

    // git2 requires the branch to exist BEFORE the worktree references it.
    let branch_name = format!("panes/{}", short_id(thread_id));
    if let Ok(mut existing) = repo.find_branch(&branch_name, BranchType::Local) {
        // Recovery case: a previous panes/<short> branch leaked without
        // the accompanying worktree. Delete it so we can re-create.
        existing
            .delete()
            .with_context(|| format!("failed to delete stale branch {branch_name}"))?;
    }
    repo.branch(&branch_name, &head_commit, false)
        .with_context(|| format!("failed to create branch {branch_name}"))?;

    // Resolve the branch reference for the worktree options.
    let branch_ref_name = format!("refs/heads/{branch_name}");
    let reference = repo
        .find_reference(&branch_ref_name)
        .with_context(|| format!("branch reference {branch_ref_name} missing"))?;

    let mut opts = WorktreeAddOptions::new();
    opts.reference(Some(&reference));

    // libgit2 identifies the worktree in its internal registry by name.
    // Use the thread id so `git worktree list` shows something meaningful.
    repo.worktree(thread_id, &worktree_path, Some(&opts))
        .with_context(|| format!("failed to create worktree at {}", worktree_path.display()))?;

    Ok(WorktreeHandle {
        path: worktree_path,
        branch: branch_name,
        base_commit,
    })
}

/// Remove a worktree: prune libgit2's record, delete the working tree
/// directory on disk, and delete the backing branch. All three steps are
/// required — missing any leaves stale state that `git worktree list`
/// will keep reporting forever.
pub fn remove(repo_root: &Path, handle: &WorktreeHandle) -> Result<()> {
    let repo = Repository::open(repo_root)
        .with_context(|| format!("failed to open repo at {}", repo_root.display()))?;

    // Prune the worktree from git's internal registry. `valid(true)` lets
    // us prune worktrees that still have a working directory on disk —
    // without it, libgit2 refuses unless git thinks the worktree is gone.
    let mut prune_opts = WorktreePruneOptions::new();
    prune_opts.valid(true).working_tree(true);

    // Find by name (we named it after the thread id on create). Failing
    // to find it is not fatal — libgit2 may have already forgotten it if
    // the directory was removed out-of-band.
    let thread_id = handle
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| anyhow!("worktree path has no trailing component"))?;
    if let Ok(worktree) = repo.find_worktree(thread_id) {
        if let Err(e) = worktree.prune(Some(&mut prune_opts)) {
            warn!(
                thread_id = %thread_id,
                error = %e,
                "worktree prune failed — proceeding with filesystem + branch cleanup",
            );
        }
    }

    // Filesystem cleanup. Ignore not-found so repeated calls are idempotent.
    if let Err(e) = fs::remove_dir_all(&handle.path) {
        if e.kind() != io::ErrorKind::NotFound {
            return Err(anyhow!(
                "failed to remove worktree directory {}: {e}",
                handle.path.display()
            ));
        }
    }

    // Delete the backing branch. Missing branch is fine — branch may
    // have been renamed / deleted manually.
    if let Ok(mut branch) = repo.find_branch(&handle.branch, BranchType::Local) {
        branch
            .delete()
            .with_context(|| format!("failed to delete branch {}", handle.branch))?;
    }

    Ok(())
}

/// Merge the worktree's branch back into the main repo's HEAD with
/// `MergeStrategy::Auto` — backwards-compatible wrapper around
/// `merge_with_strategy` for callers that don't care about conflict
/// resolution.
pub fn merge_into_head(
    repo_root: &Path,
    handle: &WorktreeHandle,
    message: &str,
) -> Result<MergeOutcome> {
    merge_with_strategy(repo_root, handle, message, MergeStrategy::Auto)
}

/// Merge the worktree's branch back into the main repo's HEAD.
///
/// Fast-forward when possible, otherwise a two-parent merge commit. On
/// conflict, behaviour depends on `strategy`:
/// - `Auto`: the main repo is hard-reset to pre-merge HEAD and the
///   outcome is `Conflicts { files }`. Caller expected to either retry
///   with a different strategy or call `remove(handle)` to discard.
/// - `PreferTheirs`: every conflicted file is replaced with the
///   worktree-branch version and the merge commit is created normally.
/// - `PreferOurs`: every conflicted file keeps main's version; a merge
///   commit still records the worktree branch as a second parent so
///   future merges see the history as merged.
pub fn merge_with_strategy(
    repo_root: &Path,
    handle: &WorktreeHandle,
    message: &str,
    strategy: MergeStrategy,
) -> Result<MergeOutcome> {
    let repo = Repository::open(repo_root)
        .with_context(|| format!("failed to open repo at {}", repo_root.display()))?;

    let branch = repo
        .find_branch(&handle.branch, BranchType::Local)
        .with_context(|| format!("branch {} missing on merge", handle.branch))?;
    let branch_commit = branch
        .get()
        .peel_to_commit()
        .with_context(|| format!("branch {} has no commit to peel", handle.branch))?;
    let branch_annotated = repo
        .find_annotated_commit(branch_commit.id())
        .context("failed to annotate branch commit for merge analysis")?;

    // Snapshot HEAD so we can restore if the merge conflicts.
    let pre_head_commit = repo
        .head()
        .and_then(|h| h.peel_to_commit())
        .context("main HEAD missing on merge")?;
    let pre_head_oid = pre_head_commit.id();

    let (analysis, _pref) = repo
        .merge_analysis(&[&branch_annotated])
        .context("merge analysis failed")?;

    if analysis.is_up_to_date() {
        return Ok(MergeOutcome::UpToDate);
    }

    if analysis.is_fast_forward() {
        // Move HEAD forward. git2 doesn't have a single "fast-forward HEAD"
        // call, so we update the reference manually + check out the tree.
        let mut head_ref = repo
            .head()
            .context("HEAD missing during fast-forward")?;
        head_ref
            .set_target(branch_commit.id(), "panes: fast-forward merge")
            .context("failed to move HEAD for fast-forward")?;
        repo.set_head(head_ref.name().unwrap_or("HEAD"))
            .context("failed to re-set HEAD after fast-forward")?;
        // Force checkout so the main working tree matches the new HEAD.
        let mut co = git2::build::CheckoutBuilder::new();
        co.force();
        repo.checkout_head(Some(&mut co))
            .context("failed to checkout HEAD after fast-forward")?;
        return Ok(MergeOutcome::FastForwarded {
            commit: branch_commit.id().to_string(),
        });
    }

    // Normal three-way merge.
    repo.merge(&[&branch_annotated], None, None)
        .context("merge failed")?;

    let mut index = repo.index().context("failed to read index after merge")?;

    if index.has_conflicts() {
        // Collect conflicted paths + the blob oids for each side so we
        // can either (a) return them for an Auto-strategy retry, or
        // (b) resolve each in-index for PreferOurs/PreferTheirs.
        let mut conflicts: Vec<ConflictEntry> = Vec::new();
        if let Ok(iter) = index.conflicts() {
            for entry in iter.flatten() {
                let path = entry
                    .our
                    .as_ref()
                    .or(entry.their.as_ref())
                    .or(entry.ancestor.as_ref())
                    .map(|e| String::from_utf8_lossy(&e.path).into_owned());
                if let Some(p) = path {
                    if !conflicts.iter().any(|c| c.path == p) {
                        conflicts.push(ConflictEntry {
                            path: p,
                            our_oid: entry.our.as_ref().map(|e| e.id),
                            their_oid: entry.their.as_ref().map(|e| e.id),
                            our_mode: entry.our.as_ref().map(|e| e.mode),
                            their_mode: entry.their.as_ref().map(|e| e.mode),
                        });
                    }
                }
            }
        }

        match strategy {
            MergeStrategy::Auto => {
                // Restore main repo to pre-merge state so nothing's
                // half-merged — the caller is expected to either retry
                // with PreferOurs/PreferTheirs or call `remove`.
                repo.cleanup_state()
                    .context("failed to clean up merge state")?;
                let head_obj = repo
                    .find_object(pre_head_oid, None)
                    .context("failed to resolve pre-merge HEAD for reset")?;
                let mut co = git2::build::CheckoutBuilder::new();
                co.force();
                repo.reset(&head_obj, ResetType::Hard, Some(&mut co))
                    .context("failed to hard-reset after conflict")?;
                return Ok(MergeOutcome::Conflicts {
                    files: conflicts.into_iter().map(|c| c.path).collect(),
                });
            }
            MergeStrategy::PreferOurs | MergeStrategy::PreferTheirs => {
                // Resolve each conflict in the index by staging the
                // chosen side's blob. git2's `add` / `remove` API is
                // enough here because we don't need to touch the
                // working tree — `checkout_index` at the end syncs it.
                for c in &conflicts {
                    resolve_conflict_in_index(&mut index, c, strategy)
                        .with_context(|| format!("failed to resolve conflict for {}", c.path))?;
                }
                index.write().context("failed to write resolved index")?;
            }
        }
    }

    // Clean merge — write a commit.
    let tree_oid = index.write_tree_to(&repo).context("failed to write merge tree")?;
    let tree = repo
        .find_tree(tree_oid)
        .context("failed to load merge tree")?;
    let signature = repo.signature().or_else(|_| {
        git2::Signature::now("Panes", "panes@local")
    })?;
    let merge_commit = repo
        .commit(
            Some("HEAD"),
            &signature,
            &signature,
            message,
            &tree,
            &[&pre_head_commit, &branch_commit],
        )
        .context("failed to create merge commit")?;

    // Clear merge state flags + refresh working tree.
    repo.cleanup_state()
        .context("failed to clean up merge state after commit")?;
    let mut co = git2::build::CheckoutBuilder::new();
    co.force();
    repo.checkout_head(Some(&mut co))
        .context("failed to checkout merged tree")?;

    Ok(MergeOutcome::Merged {
        commit: merge_commit.to_string(),
    })
}

/// In-memory view of one conflicted index entry at the moment the
/// merge engine detected conflicts. We snapshot the blob oids so the
/// resolution step can re-stage them without re-running merge analysis.
struct ConflictEntry {
    path: String,
    our_oid: Option<git2::Oid>,
    their_oid: Option<git2::Oid>,
    our_mode: Option<u32>,
    their_mode: Option<u32>,
}

/// Resolve a single conflicted path in the index by staging the blob
/// from the chosen side and clearing the conflict markers. Does not
/// touch the working tree — the caller runs `checkout_index` (via the
/// `checkout_head` at the end of the merge) to sync files on disk.
fn resolve_conflict_in_index(
    index: &mut git2::Index,
    c: &ConflictEntry,
    strategy: MergeStrategy,
) -> Result<()> {
    let (chosen_oid, chosen_mode) = match strategy {
        MergeStrategy::PreferOurs => (c.our_oid, c.our_mode),
        MergeStrategy::PreferTheirs => (c.their_oid, c.their_mode),
        MergeStrategy::Auto => {
            return Err(anyhow!("resolve_conflict_in_index called with Auto"));
        }
    };

    // Drop the three conflicting stage entries before inserting the
    // resolved one. `remove_path` removes all stage entries for this
    // path (ancestor + ours + theirs).
    let path_bytes = c.path.as_bytes();
    index
        .remove_path(Path::new(&c.path))
        .with_context(|| format!("failed to drop conflict entries for {}", c.path))?;

    match (chosen_oid, chosen_mode) {
        (Some(oid), Some(mode)) => {
            // Stage the chosen side's blob.
            let entry = git2::IndexEntry {
                ctime: git2::IndexTime::new(0, 0),
                mtime: git2::IndexTime::new(0, 0),
                dev: 0,
                ino: 0,
                mode,
                uid: 0,
                gid: 0,
                file_size: 0,
                id: oid,
                flags: 0,
                flags_extended: 0,
                path: path_bytes.to_vec(),
            };
            index
                .add(&entry)
                .with_context(|| format!("failed to stage resolved blob for {}", c.path))?;
        }
        _ => {
            // Chosen side didn't have this path (add/delete conflict).
            // The `remove_path` above already dropped it — intentional
            // "delete" outcome. Nothing more to stage.
        }
    }

    Ok(())
}

/// Scan `worktrees_root` and return any directories whose name isn't in
/// `known_thread_ids`. Used at startup to clean up after crashes.
pub fn list_orphans(
    worktrees_root: &Path,
    known_thread_ids: &HashSet<String>,
) -> Vec<OrphanedWorktree> {
    let entries = match fs::read_dir(worktrees_root) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            warn!(
                error = %e,
                root = %worktrees_root.display(),
                "failed to list worktrees root — skipping orphan scan",
            );
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if known_thread_ids.contains(name) {
            continue;
        }
        out.push(OrphanedWorktree {
            path: path.clone(),
            thread_id: name.to_string(),
        });
    }
    out
}

/// Clean up a single orphaned worktree: prune libgit2's record, remove
/// the directory, and drop the `panes/<short>` branch if it still exists.
/// Best-effort — logs but doesn't propagate per-step failures because
/// orphan cleanup must never block startup.
pub fn prune_orphan(repo_root: &Path, orphan: &OrphanedWorktree) -> Result<()> {
    // Synthesize a handle: we don't know the real branch name, only the
    // thread id, so compute it from the same `short_id` formula.
    let handle = WorktreeHandle {
        path: orphan.path.clone(),
        branch: format!("panes/{}", short_id(&orphan.thread_id)),
        base_commit: String::new(),
    };
    remove(repo_root, &handle)
}

fn short_id(thread_id: &str) -> String {
    thread_id.chars().take(8).collect()
}

// ─── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// Init a git repo at `path` with one committed file so HEAD exists.
    /// libgit2 operations require HEAD so this is the minimum viable repo.
    fn init_repo(path: &Path) {
        let ok = |out: std::process::Output| {
            assert!(
                out.status.success(),
                "git command failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        ok(Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(path)
            .output()
            .unwrap());
        ok(Command::new("git")
            .args(["config", "user.email", "test@panes.local"])
            .current_dir(path)
            .output()
            .unwrap());
        ok(Command::new("git")
            .args(["config", "user.name", "Panes Test"])
            .current_dir(path)
            .output()
            .unwrap());
        fs::write(path.join("README.md"), "hello\n").unwrap();
        ok(Command::new("git")
            .args(["add", "README.md"])
            .current_dir(path)
            .output()
            .unwrap());
        ok(Command::new("git")
            .args(["commit", "-q", "-m", "initial"])
            .current_dir(path)
            .output()
            .unwrap());
    }

    #[test]
    fn create_produces_working_tree_with_content() {
        let repo = TempDir::new().unwrap();
        init_repo(repo.path());
        let roots = TempDir::new().unwrap();

        let h = create(repo.path(), "abcdef123456789", roots.path()).unwrap();
        assert!(h.path.exists(), "worktree path missing");
        assert!(h.path.join("README.md").exists(), "worktree content missing");
        assert!(h.branch.starts_with("panes/"));
        assert_eq!(h.branch.len(), "panes/".len() + 8);
    }

    #[test]
    fn create_refuses_if_path_already_exists() {
        let repo = TempDir::new().unwrap();
        init_repo(repo.path());
        let roots = TempDir::new().unwrap();
        // Pre-create the directory so create() must bail.
        fs::create_dir_all(roots.path().join("t1")).unwrap();
        let err = create(repo.path(), "t1", roots.path()).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn remove_cleans_directory_and_branch() {
        let repo = TempDir::new().unwrap();
        init_repo(repo.path());
        let roots = TempDir::new().unwrap();

        let h = create(repo.path(), "abcdef123456789", roots.path()).unwrap();
        assert!(h.path.exists());

        remove(repo.path(), &h).unwrap();
        assert!(!h.path.exists(), "worktree dir should be gone");

        let git_repo = Repository::open(repo.path()).unwrap();
        assert!(
            git_repo.find_branch(&h.branch, BranchType::Local).is_err(),
            "branch should be deleted"
        );
    }

    #[test]
    fn remove_is_idempotent_on_missing_directory() {
        let repo = TempDir::new().unwrap();
        init_repo(repo.path());
        let roots = TempDir::new().unwrap();

        let h = create(repo.path(), "abcdef123456789", roots.path()).unwrap();
        // First removal: real work.
        remove(repo.path(), &h).unwrap();
        // Second removal: should be a no-op, not an error. Callers may
        // invoke this from cleanup paths that don't know the state.
        remove(repo.path(), &h).unwrap();
    }

    #[test]
    fn merge_fast_forwards_when_main_has_no_new_commits() {
        let repo = TempDir::new().unwrap();
        init_repo(repo.path());
        let roots = TempDir::new().unwrap();

        let h = create(repo.path(), "abcdef123456789", roots.path()).unwrap();

        // Commit inside the worktree.
        fs::write(h.path.join("new.txt"), "from worktree\n").unwrap();
        Command::new("git")
            .args(["add", "new.txt"])
            .current_dir(&h.path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["-c", "user.email=t@x.com", "-c", "user.name=T", "commit", "-q", "-m", "wt change"])
            .current_dir(&h.path)
            .output()
            .unwrap();

        let outcome = merge_into_head(repo.path(), &h, "merge test").unwrap();
        match outcome {
            MergeOutcome::FastForwarded { .. } => {}
            other => panic!("expected FastForwarded, got {other:?}"),
        }
        assert!(
            repo.path().join("new.txt").exists(),
            "main checkout should have the merged file"
        );
    }

    #[test]
    fn merge_returns_conflicts_without_touching_main_head() {
        let repo = TempDir::new().unwrap();
        init_repo(repo.path());
        let roots = TempDir::new().unwrap();

        let h = create(repo.path(), "abcdef123456789", roots.path()).unwrap();

        // Worktree modifies README.
        fs::write(h.path.join("README.md"), "worktree version\n").unwrap();
        Command::new("git")
            .args(["-c", "user.email=t@x.com", "-c", "user.name=T", "commit", "-q", "-am", "wt"])
            .current_dir(&h.path)
            .output()
            .unwrap();

        // Main also modifies README differently — this creates the conflict.
        fs::write(repo.path().join("README.md"), "main version\n").unwrap();
        Command::new("git")
            .args(["-c", "user.email=t@x.com", "-c", "user.name=T", "commit", "-q", "-am", "main"])
            .current_dir(repo.path())
            .output()
            .unwrap();

        let pre_head = {
            let r = Repository::open(repo.path()).unwrap();
            r.head().unwrap().peel_to_commit().unwrap().id()
        };

        let outcome = merge_into_head(repo.path(), &h, "merge test").unwrap();
        match &outcome {
            MergeOutcome::Conflicts { files } => {
                assert!(
                    files.iter().any(|f| f == "README.md"),
                    "expected README.md in conflicts, got {files:?}"
                );
            }
            other => panic!("expected Conflicts, got {other:?}"),
        }

        // Main HEAD must be unchanged — the merge attempt cleaned up.
        let post_head = {
            let r = Repository::open(repo.path()).unwrap();
            r.head().unwrap().peel_to_commit().unwrap().id()
        };
        assert_eq!(pre_head, post_head, "main HEAD must not move on conflict");
        assert_eq!(
            fs::read_to_string(repo.path().join("README.md")).unwrap(),
            "main version\n",
            "main working tree must be restored",
        );
    }

    /// Set up a conflict scenario: repo with a worktree branch and main
    /// branch both editing the same file with different content. Returns
    /// the repo + worktree handle ready to merge.
    fn setup_conflict(repo: &TempDir, roots: &TempDir) -> WorktreeHandle {
        init_repo(repo.path());
        let h = create(repo.path(), "conflict-thread-id", roots.path()).unwrap();

        fs::write(h.path.join("README.md"), "worktree version\n").unwrap();
        Command::new("git")
            .args(["-c", "user.email=t@x.com", "-c", "user.name=T", "commit", "-q", "-am", "wt"])
            .current_dir(&h.path)
            .output()
            .unwrap();

        fs::write(repo.path().join("README.md"), "main version\n").unwrap();
        Command::new("git")
            .args(["-c", "user.email=t@x.com", "-c", "user.name=T", "commit", "-q", "-am", "main"])
            .current_dir(repo.path())
            .output()
            .unwrap();

        h
    }

    #[test]
    fn merge_with_strategy_prefer_theirs_takes_worktree_version() {
        // Option A: user picks "Use yours" in the conflict UI. The
        // engine stages the worktree (theirs) blob and commits the
        // merge — main's README ends up with the worktree content.
        let repo = TempDir::new().unwrap();
        let roots = TempDir::new().unwrap();
        let h = setup_conflict(&repo, &roots);

        let outcome =
            merge_with_strategy(repo.path(), &h, "merge: prefer yours", MergeStrategy::PreferTheirs)
                .unwrap();
        match outcome {
            MergeOutcome::Merged { .. } => {}
            other => panic!("expected Merged, got {other:?}"),
        }

        assert_eq!(
            fs::read_to_string(repo.path().join("README.md")).unwrap(),
            "worktree version\n",
            "main should now hold the worktree's content",
        );

        // Main HEAD advanced — there's a new merge commit with two parents.
        let r = Repository::open(repo.path()).unwrap();
        let head_commit = r.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head_commit.parent_count(), 2, "merge commit should have 2 parents");
    }

    #[test]
    fn merge_with_strategy_prefer_ours_keeps_main_version() {
        // Option A: user picks "Keep main" in the conflict UI. The
        // engine stages main's blob and commits the merge — main's
        // README keeps its content but the history records the merge.
        let repo = TempDir::new().unwrap();
        let roots = TempDir::new().unwrap();
        let h = setup_conflict(&repo, &roots);

        let outcome =
            merge_with_strategy(repo.path(), &h, "merge: keep main", MergeStrategy::PreferOurs)
                .unwrap();
        match outcome {
            MergeOutcome::Merged { .. } => {}
            other => panic!("expected Merged, got {other:?}"),
        }

        assert_eq!(
            fs::read_to_string(repo.path().join("README.md")).unwrap(),
            "main version\n",
            "main should keep its own content",
        );

        let r = Repository::open(repo.path()).unwrap();
        let head_commit = r.head().unwrap().peel_to_commit().unwrap();
        assert_eq!(head_commit.parent_count(), 2, "merge commit should have 2 parents");
    }

    #[test]
    fn merge_into_head_default_is_auto_and_still_conflicts() {
        // Regression: the old `merge_into_head` wrapper must still
        // behave identically — callers that haven't adopted the
        // strategy API get Auto semantics (bail on conflict).
        let repo = TempDir::new().unwrap();
        let roots = TempDir::new().unwrap();
        let h = setup_conflict(&repo, &roots);

        let outcome = merge_into_head(repo.path(), &h, "auto").unwrap();
        assert!(matches!(outcome, MergeOutcome::Conflicts { .. }));
        assert_eq!(
            fs::read_to_string(repo.path().join("README.md")).unwrap(),
            "main version\n",
            "Auto must restore main working tree on conflict",
        );
    }

    #[test]
    fn list_orphans_finds_unknown_directories() {
        let repo = TempDir::new().unwrap();
        init_repo(repo.path());
        let roots = TempDir::new().unwrap();

        let active = create(repo.path(), "aaaaaaaa-active-thread", roots.path()).unwrap();
        let _dead = create(repo.path(), "bbbbbbbb-dead-thread", roots.path()).unwrap();

        let mut known = HashSet::new();
        known.insert("aaaaaaaa-active-thread".to_string());

        let orphans = list_orphans(roots.path(), &known);
        assert_eq!(orphans.len(), 1, "exactly one orphan expected");
        assert_eq!(orphans[0].thread_id, "bbbbbbbb-dead-thread");
        assert!(active.path.exists(), "active worktree untouched");
    }

    #[test]
    fn prune_orphan_cleans_up_directory_and_branch() {
        let repo = TempDir::new().unwrap();
        init_repo(repo.path());
        let roots = TempDir::new().unwrap();

        let orphan_handle = create(repo.path(), "bbbbbbbb-orphan", roots.path()).unwrap();

        let orphans = list_orphans(roots.path(), &HashSet::new());
        assert_eq!(orphans.len(), 1);

        prune_orphan(repo.path(), &orphans[0]).unwrap();
        assert!(!orphan_handle.path.exists(), "orphan dir should be gone");

        let git_repo = Repository::open(repo.path()).unwrap();
        assert!(
            git_repo
                .find_branch(&orphan_handle.branch, BranchType::Local)
                .is_err(),
            "orphan branch should be gone",
        );
    }

    #[test]
    fn list_orphans_tolerates_missing_root() {
        let roots = TempDir::new().unwrap();
        let missing = roots.path().join("does-not-exist");
        let orphans = list_orphans(&missing, &HashSet::new());
        assert!(orphans.is_empty());
    }
}
