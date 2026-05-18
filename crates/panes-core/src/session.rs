use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::Utc;
use futures::StreamExt;
use panes_adapters::{AgentAdapter, AgentSession};
use panes_cost::{self, CostTracker};
use panes_events::{AgentEvent, SessionContext, ThreadEvent};
use tokio::sync::{mpsc, oneshot, Mutex};
use tracing::{info, warn};
use uuid::Uuid;

use crate::db::{self, DbHandle};
use crate::error::PanesError;
use crate::features::{is_feature_enabled, FEATURE_VALIDATORS};
use crate::git;
use crate::validation::{ValidationContext, ValidatorRegistry};
use crate::version_tracker::{
    extract_file_path, is_file_write_tool, GitVersionTracker, ShadowVersionTracker, TrackerKind,
    VersionTracker,
};
use crate::worktree::{self, WorktreeHandle};

/// Describes the git layout of a workspace, used to decide whether
/// worktree isolation is possible. Extensibility point for future
/// multi-repo worktree support (the `MultiRepo` variant).
#[derive(Debug, Clone)]
enum WorkspaceLayout {
    /// Workspace root is inside a single git repo (or was auto-init'd
    /// into one). Worktree isolation works normally.
    SingleRepo { repo_root: PathBuf },
    /// Workspace root is NOT a git repo, but contains nested git repos
    /// (e.g. a Brazil workspace with `src/PackageA/`, `src/PackageB/`).
    /// Worktree isolation is skipped for now — agents run directly in
    /// the workspace. Future: create per-repo worktrees and stitch
    /// them together.
    MultiRepo { nested_repos: Vec<PathBuf> },
}

#[derive(Debug)]
pub enum GateDecision {
    Continue,
    Abort,
}

#[derive(Debug)]
enum ValidatorFlow {
    Continue,
    Aborted,
}

#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub default_agent: Option<String>,
    pub budget_cap: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThreadStatus {
    Pending,
    Running,
    Gate,
    Completed,
    Error,
    Interrupted,
}

impl std::fmt::Display for ThreadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThreadStatus::Pending => write!(f, "pending"),
            ThreadStatus::Running => write!(f, "running"),
            ThreadStatus::Gate => write!(f, "gate"),
            ThreadStatus::Completed => write!(f, "completed"),
            ThreadStatus::Error => write!(f, "error"),
            ThreadStatus::Interrupted => write!(f, "interrupted"),
        }
    }
}

type GateSender = Arc<Mutex<Option<oneshot::Sender<GateDecision>>>>;

/// Drop guard that persists thread costs when `consume_events` exits.
///
/// Most callers are on a tokio runtime worker, where a blocking recv would
/// panic. We fire a detached async task via the current tokio handle when one
/// is available; if called from a non-tokio context (e.g. a Drop during test
/// teardown) we fall back to the blocking path.
struct CostFinalizer {
    thread_id: String,
    cost_tracker: Arc<CostTracker>,
    db: DbHandle,
}

impl Drop for CostFinalizer {
    fn drop(&mut self) {
        let Some(cost) = self.cost_tracker.finalize(&self.thread_id) else {
            return;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                // Fire-and-forget on the current runtime so we don't block the
                // task being torn down. `db.execute` is async and doesn't panic
                // when called from a runtime worker.
                let db = self.db.clone();
                handle.spawn(async move {
                    let _ = db
                        .execute(move |conn| {
                            panes_cost::save_cost(conn, &cost).ok();
                            Ok(())
                        })
                        .await;
                });
            }
            Err(_) => {
                // No tokio runtime active (e.g. synchronous test teardown).
                // Safe to block here because this thread is not a runtime worker.
                let _ = self.db.try_execute_blocking(move |conn| {
                    panes_cost::save_cost(conn, &cost).ok();
                    Ok(())
                });
            }
        }
    }
}

struct ActiveThread {
    workspace_id: String,
    /// Path the thread's agent actually runs in. Equal to `workspace.path`
    /// for shadow-tracked threads; equal to the worktree path for
    /// git-tracked threads in Phase 2. All downstream operations
    /// (diff, revert, get_changed_files, commit) must use this, not the
    /// logical workspace path, or they'll read/write the wrong checkout.
    effective_path: PathBuf,
    /// Handle to the per-thread worktree for git-tracked threads.
    /// None for shadow threads and for git threads created before the
    /// worktrees migration.
    worktree: Option<WorktreeHandle>,
    /// Carried for lifecycle decisions (merge/discard UI is only valid
    /// for git-tracked threads). Not every consumer reads it today but
    /// stripping it would force redundant DB lookups later.
    #[allow(dead_code)]
    tracker_kind: TrackerKind,
    session: Box<dyn AgentSession>,
    snapshot: Option<git::SnapshotRef>,
    gate_tx: GateSender,
}

pub struct SessionManager {
    active_threads: Arc<Mutex<HashMap<String, ActiveThread>>>,
    /// Workspace ids with a thread actively reserving the one-thread-per-
    /// workspace invariant. Phase 2 scopes this to shadow-tracked
    /// workspaces only — git workspaces get per-thread worktrees so
    /// concurrent threads are safe and never hit this map.
    reservations: Arc<Mutex<HashSet<String>>>,
    session_ids: Arc<Mutex<HashMap<String, String>>>,
    adapters: HashMap<String, Arc<dyn AgentAdapter>>,
    cost_tracker: Arc<CostTracker>,
    event_tx: mpsc::UnboundedSender<ThreadEvent>,
    pub(crate) db: DbHandle,
    pub validators: Arc<ValidatorRegistry>,
    git_tracker: Arc<GitVersionTracker>,
    shadow_tracker: Arc<ShadowVersionTracker>,
    /// Root directory where per-thread worktrees live on disk. Each
    /// concurrent git thread gets a subdirectory named by its thread id.
    worktrees_root: PathBuf,
}

impl SessionManager {
    pub async fn new(
        cost_tracker: Arc<CostTracker>,
        event_tx: mpsc::UnboundedSender<ThreadEvent>,
        db: DbHandle,
        shadow_blob_root: PathBuf,
        worktrees_root: PathBuf,
    ) -> Self {
        let session_ids = Self::load_session_ids(&db).await;

        let git_tracker = Arc::new(GitVersionTracker::new(db.clone()));
        let shadow_tracker = Arc::new(
            ShadowVersionTracker::new(db.clone(), shadow_blob_root)
                .expect("failed to initialise shadow version tracker"),
        );

        let mgr = Self {
            active_threads: Arc::new(Mutex::new(HashMap::new())),
            reservations: Arc::new(Mutex::new(HashSet::new())),
            session_ids: Arc::new(Mutex::new(session_ids)),
            adapters: HashMap::new(),
            cost_tracker,
            event_tx,
            db,
            validators: Arc::new(ValidatorRegistry::with_builtins()),
            git_tracker,
            shadow_tracker,
            worktrees_root,
        };

        // Best-effort crash recovery: any worktree directory under
        // `worktrees_root` that has no matching row in `threads` was
        // orphaned by a crashed prior run. `recover_stale_threads` in
        // `db::initialize` has already flipped stale rows to `interrupted`
        // — we leave those worktrees in place so the user can inspect
        // them, and only prune the truly-dangling ones.
        mgr.cleanup_orphan_worktrees().await;

        mgr
    }

    /// Scan the worktrees root at startup and drop any subdirectory whose
    /// name doesn't match a row in `threads`. Logs failures — never
    /// propagates because cleanup must not block app startup.
    async fn cleanup_orphan_worktrees(&self) {
        let worktrees_root = self.worktrees_root.clone();
        if !worktrees_root.exists() {
            return;
        }

        // Build the set of known thread ids from the DB.
        let known: HashSet<String> = match self
            .db
            .execute(|conn| {
                let mut stmt = conn.prepare("SELECT id FROM threads")?;
                let rows = stmt
                    .query_map([], |row| row.get::<_, String>(0))?
                    .filter_map(|r| r.ok())
                    .collect::<HashSet<_>>();
                Ok(rows)
            })
            .await
        {
            Ok(ids) => ids,
            Err(e) => {
                warn!(error = %e, "worktree orphan scan: failed to read thread ids");
                return;
            }
        };

        let orphans = worktree::list_orphans(&worktrees_root, &known);
        if orphans.is_empty() {
            return;
        }

        // We don't know which workspace repo each orphan belongs to, so
        // look it up from the DB. If the thread row is gone entirely
        // (the user deleted it without us cleaning up), we can't prune
        // the git branch — just drop the directory.
        for orphan in orphans {
            let repo_root = self
                .db
                .execute({
                    let tid = orphan.thread_id.clone();
                    move |conn| {
                        Ok(conn
                            .query_row(
                                "SELECT w.path FROM workspaces w JOIN threads t ON t.workspace_id = w.id WHERE t.id = ?1",
                                rusqlite::params![tid],
                                |row| row.get::<_, String>(0),
                            )
                            .ok())
                    }
                })
                .await
                .ok()
                .flatten();

            if let Some(repo) = repo_root {
                if let Err(e) = worktree::prune_orphan(&PathBuf::from(repo), &orphan) {
                    warn!(
                        path = %orphan.path.display(),
                        error = %e,
                        "failed to prune orphan worktree",
                    );
                }
            } else {
                // No workspace info — just remove the directory.
                if let Err(e) = std::fs::remove_dir_all(&orphan.path) {
                    warn!(
                        path = %orphan.path.display(),
                        error = %e,
                        "failed to remove dangling orphan directory",
                    );
                }
            }
        }
    }

    /// Resolve the tracker to use for a given thread by reading the
    /// `tracker_kind` column persisted at thread start. Falls back to the
    /// Git tracker when the column is missing (legacy threads predating
    /// the shadow store).
    pub async fn tracker_for_thread(
        &self,
        thread_id: &str,
    ) -> Result<Arc<dyn VersionTracker>, PanesError> {
        let tid = thread_id.to_string();
        let kind_str: String = self
            .db
            .execute(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT tracker_kind FROM threads WHERE id = ?1",
                        rusqlite::params![tid],
                        |row| row.get::<_, Option<String>>(0),
                    )?
                    .unwrap_or_else(|| "git".to_string()))
            })
            .await
            .map_err(|e| PanesError::GitError {
                message: format!("tracker lookup failed: {e}"),
            })?;
        Ok(match TrackerKind::parse(&kind_str) {
            TrackerKind::Git => self.git_tracker.clone() as Arc<dyn VersionTracker>,
            TrackerKind::Shadow => self.shadow_tracker.clone() as Arc<dyn VersionTracker>,
        })
    }

    /// Direct access to the git tracker for commit/branch commands that
    /// are meaningful only in git-backed workspaces.
    pub fn git_tracker(&self) -> Arc<GitVersionTracker> {
        self.git_tracker.clone()
    }

    /// Direct handle to the shadow tracker so callers outside the tracker
    /// abstraction (e.g. `delete_thread`, which garbage-collects shadow
    /// state when a thread is removed) can invoke shadow-only methods.
    pub fn shadow_tracker(&self) -> Arc<ShadowVersionTracker> {
        self.shadow_tracker.clone()
    }

    async fn load_session_ids(db: &DbHandle) -> HashMap<String, String> {
        let map = db
            .execute(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, session_id FROM threads WHERE session_id IS NOT NULL",
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .filter_map(|r| r.ok())
                    .collect::<HashMap<String, String>>();
                Ok(rows)
            })
            .await
            .unwrap_or_default();
        if !map.is_empty() {
            info!(count = map.len(), "restored session_ids from database");
        }
        map
    }

    pub fn register_adapter(&mut self, adapter: Arc<dyn AgentAdapter>) {
        self.adapters.insert(adapter.name().to_string(), adapter);
    }

    pub async fn start_thread(
        &self,
        workspace: &Workspace,
        prompt: &str,
        agent_name: &str,
        context: SessionContext,
        model: Option<&str>,
    ) -> Result<String, PanesError> {
        let adapter = self
            .adapters
            .get(agent_name)
            .or_else(|| self.adapters.get("claude-code"))
            .ok_or_else(|| PanesError::AdapterNotFound {
                adapter: agent_name.to_string(),
                message: format!("unknown agent: {agent_name}"),
            })?
            .clone();

        let cli_agent = if self.adapters.contains_key(agent_name) {
            None
        } else {
            Some(agent_name.to_string())
        };

        let layout = match git::find_repo_root(&workspace.path).await {
            Some(root) => WorkspaceLayout::SingleRepo { repo_root: root },
            None => {
                let nested = git::find_git_repos(&workspace.path).await;
                if nested.is_empty() {
                    // No git at all — auto-init so worktree isolation works.
                    let root = git::init_repo(&workspace.path)
                        .await
                        .map_err(|e| PanesError::Internal {
                            message: format!("failed to auto-init git for worktree isolation: {e}"),
                        })?;
                    WorkspaceLayout::SingleRepo { repo_root: root }
                } else {
                    WorkspaceLayout::MultiRepo { nested_repos: nested }
                }
            }
        };

        let result = match &layout {
            WorkspaceLayout::SingleRepo { repo_root } => {
                self.start_thread_inner(workspace, prompt, agent_name, adapter, context, model, cli_agent.as_deref(), Some(repo_root))
                    .await
                    .map_err(PanesError::from)
            }
            WorkspaceLayout::MultiRepo { nested_repos } => {
                info!(
                    workspace = %workspace.path.display(),
                    repos = nested_repos.len(),
                    "multi-repo workspace — skipping worktree isolation",
                );
                self.start_thread_inner(workspace, prompt, agent_name, adapter, context, model, cli_agent.as_deref(), None)
                    .await
                    .map_err(PanesError::from)
            }
        };

        result
    }

    /// `repo_root` is `Some` for single-repo workspaces (worktree isolation
    /// enabled) and `None` for multi-repo workspaces (agents run directly
    /// in the workspace — no isolation yet).
    async fn start_thread_inner(
        &self,
        workspace: &Workspace,
        prompt: &str,
        agent_name: &str,
        adapter: Arc<dyn AgentAdapter>,
        context: SessionContext,
        model: Option<&str>,
        cli_agent: Option<&str>,
        repo_root: Option<&Path>,
    ) -> Result<String> {
        let tracker_kind = TrackerKind::Git;
        let thread_id = Uuid::new_v4().to_string();

        let (worktree_handle, effective_path) = if let Some(repo_root) = repo_root {
            let handle = {
                let repo_root = repo_root.to_path_buf();
                let wt_root = self.worktrees_root.clone();
                let tid = thread_id.clone();
                tokio::task::spawn_blocking(move || {
                    worktree::create(&repo_root, &tid, &wt_root)
                })
                .await
                .context("worktree creation task panicked")?
                .context("failed to create worktree")?
            };
            let epath = match workspace.path.strip_prefix(repo_root) {
                Ok(rel) if !rel.as_os_str().is_empty() => handle.path.join(rel),
                _ => handle.path.clone(),
            };
            (Some(handle), epath)
        } else {
            (None, workspace.path.clone())
        };

        let snapshot = match git::snapshot(&effective_path).await {
            Ok(s) => Some(s),
            Err(e) => {
                warn!(error = %e, "failed to create git snapshot — continuing without rollback");
                None
            }
        };

        let tracker: Arc<dyn VersionTracker> = self.git_tracker.clone() as Arc<dyn VersionTracker>;

        let session = adapter
            .spawn(&effective_path, prompt, &context, model, cli_agent)
            .await
            .context("failed to spawn agent session")?;

        let session_id = session.init().session_id.clone();

        {
            let mut sids = self.session_ids.lock().await;
            sids.insert(thread_id.clone(), session_id.clone());
        }

        {
            let now = Utc::now().to_rfc3339();
            let snapshot_hash = snapshot.as_ref().map(|s| s.commit_hash.clone());
            let tid = thread_id.clone();
            let wid = workspace.id.clone();
            let agent = agent_name.to_string();
            let p = prompt.to_string();
            let sid = session_id.clone();
            let kind = tracker_kind.as_str().to_string();
            let wt_path = worktree_handle.as_ref().map(|h| h.path.to_string_lossy().into_owned());
            let wt_branch = worktree_handle.as_ref().map(|h| h.branch.clone());
            let _ = self.db.execute(move |conn| {
                conn.execute(
                    "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, session_id, snapshot_ref, tracker_kind, worktree_path, worktree_branch, started_at, created_at)
                     VALUES (?1, ?2, ?3, 'running', ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                    rusqlite::params![tid, wid, agent, p, sid, snapshot_hash, kind, wt_path, wt_branch, now],
                )?;
                Ok(())
            }).await;
        }

        self.cost_tracker
            .start_tracking(&thread_id, &workspace.id);

        let gate_tx: GateSender = Arc::new(Mutex::new(None));

        let active_thread = ActiveThread {
            workspace_id: workspace.id.clone(),
            effective_path: effective_path.clone(),
            worktree: worktree_handle,
            tracker_kind,
            session,
            snapshot,
            gate_tx: gate_tx.clone(),
        };

        {
            let mut active = self.active_threads.lock().await;
            active.insert(thread_id.clone(), active_thread);
        }

        let event_stream = {
            let mut active = self.active_threads.lock().await;
            let thread = active.get_mut(&thread_id).expect("just inserted");
            thread.session.events()
        };

        let thread_id_clone = thread_id.clone();
        let event_tx = self.event_tx.clone();
        let cost_tracker = self.cost_tracker.clone();
        let active_threads = self.active_threads.clone();
        let budget_cap = workspace.budget_cap;
        let db = self.db.clone();
        let validators = self.validators.clone();
        let workspace_id_owned = workspace.id.clone();
        // Downstream consumers (validators, file-path extractors) operate
        // against the worktree path, not the logical workspace path, so
        // they see the same files the agent is editing.
        let workspace_path_owned = effective_path.clone();
        let tracker_for_task = tracker.clone();

        tokio::spawn(async move {
            Self::consume_events(
                thread_id_clone,
                event_tx,
                cost_tracker,
                active_threads,
                budget_cap,
                event_stream,
                db,
                gate_tx,
                validators,
                workspace_id_owned,
                workspace_path_owned,
                tracker_for_task,
            )
            .await;
        });

        info!(thread_id = %thread_id, "thread started");
        Ok(thread_id)
    }

    pub async fn resume_thread(
        &self,
        thread_id: &str,
        workspace: &Workspace,
        prompt: &str,
        agent_name: &str,
        model: Option<&str>,
    ) -> Result<(), PanesError> {
        let adapter = self
            .adapters
            .get(agent_name)
            .or_else(|| self.adapters.get("claude-code"))
            .ok_or_else(|| PanesError::AdapterNotFound {
                adapter: agent_name.to_string(),
                message: format!("unknown agent: {agent_name}"),
            })?
            .clone();

        let cli_agent = if self.adapters.contains_key(agent_name) {
            None
        } else {
            Some(agent_name.to_string())
        };

        // Read persisted tracker kind so we know whether to apply the
        // shadow-only guard. Threads created before Phase 2 have the
        // git tracker but no worktree_path; they still get per-thread
        // worktree isolation on resume is not retrofitted — they
        // continue in the main checkout. That's fine: they were already
        // running there pre-upgrade.
        let persisted_kind = self.tracker_kind_for_thread(thread_id).await;

        {
            let active = self.active_threads.lock().await;
            let mut reserved = self.reservations.lock().await;
            if active.contains_key(thread_id) {
                return Err(PanesError::WorkspaceOccupied {
                    workspace_id: workspace.id.clone(),
                    message: format!("thread {thread_id} is still active. Wait for it to complete first."),
                });
            }
            // Only enforce the one-thread guard when the resumed thread
            // is shadow-tracked. Git-tracked threads resume into their
            // own worktree and can coexist with other threads.
            if persisted_kind == TrackerKind::Shadow {
                if active.iter().any(|(id, t)| t.workspace_id == workspace.id && id != thread_id)
                    || reserved.contains(&workspace.id)
                {
                    return Err(PanesError::WorkspaceOccupied {
                        workspace_id: workspace.id.clone(),
                        message: "A thread is already running in this workspace. Wait for it to complete or cancel it first.".to_string(),
                    });
                }
                reserved.insert(workspace.id.clone());
            }
        }

        let result = self
            .resume_thread_inner(thread_id, workspace, prompt, adapter, model, cli_agent.as_deref(), persisted_kind)
            .await
            .map_err(PanesError::from);

        if result.is_err() && persisted_kind == TrackerKind::Shadow {
            self.reservations.lock().await.remove(&workspace.id);
        }

        result
    }

    /// Resolve the effective (cwd) path for resuming a thread + the
    /// worktree handle if one is persisted. Used by `resume_thread_inner`
    /// to rehydrate the ActiveThread with the same worktree the thread
    /// was previously running in.
    async fn resolve_worktree_for_resume(
        &self,
        thread_id: &str,
        workspace: &Workspace,
    ) -> (PathBuf, Option<WorktreeHandle>) {
        let tid = thread_id.to_string();
        let row: Option<(Option<String>, Option<String>)> = self
            .db
            .execute(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT worktree_path, worktree_branch FROM threads WHERE id = ?1",
                        rusqlite::params![tid],
                        |r| {
                            Ok((
                                r.get::<_, Option<String>>(0)?,
                                r.get::<_, Option<String>>(1)?,
                            ))
                        },
                    )
                    .ok())
            })
            .await
            .ok()
            .flatten();

        match row {
            Some((Some(path), Some(branch))) => {
                let wt_root = PathBuf::from(&path);
                if !wt_root.exists() {
                    warn!(
                        path = %wt_root.display(),
                        "persisted worktree path missing on resume — falling back to main checkout",
                    );
                    return (workspace.path.clone(), None);
                }
                let handle = WorktreeHandle {
                    path: wt_root.clone(),
                    branch,
                    base_commit: String::new(),
                };
                // Map workspace subdirectory into the worktree, same as
                // start_thread_inner. The repo root is the worktree's
                // parent repo — resolve via find_repo_root on workspace.
                let effective = match git::find_repo_root(&workspace.path).await {
                    Some(repo_root) => match workspace.path.strip_prefix(&repo_root) {
                        Ok(rel) if !rel.as_os_str().is_empty() => wt_root.join(rel),
                        _ => wt_root,
                    },
                    None => wt_root,
                };
                (effective, Some(handle))
            }
            _ => (workspace.path.clone(), None),
        }
    }

    /// Read the workspace path for a thread straight from its workspaces
    /// row. Used by the merge/discard path where we need the *repo root*
    /// (the main checkout) not the per-thread worktree path.
    pub async fn repo_root_for_thread(&self, thread_id: &str) -> Option<PathBuf> {
        let tid = thread_id.to_string();
        self.db
            .execute(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT w.path FROM workspaces w JOIN threads t ON t.workspace_id = w.id WHERE t.id = ?1",
                        rusqlite::params![tid],
                        |r| r.get::<_, String>(0),
                    )
                    .ok())
            })
            .await
            .ok()
            .flatten()
            .map(PathBuf::from)
    }

    /// Return the stored worktree handle for a given thread, reading the
    /// active map first and falling back to the DB for completed threads.
    pub async fn worktree_handle_for_thread(&self, thread_id: &str) -> Option<WorktreeHandle> {
        {
            let active = self.active_threads.lock().await;
            if let Some(t) = active.get(thread_id) {
                if let Some(h) = &t.worktree {
                    return Some(h.clone());
                }
            }
        }
        let tid = thread_id.to_string();
        let row: Option<(Option<String>, Option<String>)> = self
            .db
            .execute(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT worktree_path, worktree_branch FROM threads WHERE id = ?1",
                        rusqlite::params![tid],
                        |r| {
                            Ok((
                                r.get::<_, Option<String>>(0)?,
                                r.get::<_, Option<String>>(1)?,
                            ))
                        },
                    )
                    .ok())
            })
            .await
            .ok()
            .flatten();
        match row {
            Some((Some(path), Some(branch))) => Some(WorktreeHandle {
                path: PathBuf::from(path),
                branch,
                base_commit: String::new(),
            }),
            _ => None,
        }
    }

    /// Clear the worktree bookkeeping for a thread after a successful
    /// merge or discard. Nulls out the `threads.worktree_path` / branch
    /// columns and removes the in-memory handle if any. Doesn't touch
    /// the filesystem — `worktree::remove` already does that.
    pub async fn clear_worktree_for_thread(&self, thread_id: &str) {
        {
            let mut active = self.active_threads.lock().await;
            if let Some(t) = active.get_mut(thread_id) {
                t.worktree = None;
            }
        }
        let tid = thread_id.to_string();
        let _ = self
            .db
            .execute(move |conn| {
                conn.execute(
                    "UPDATE threads SET worktree_path = NULL, worktree_branch = NULL WHERE id = ?1",
                    rusqlite::params![tid],
                )?;
                Ok(())
            })
            .await;
    }

    /// Resolve the filesystem path a given thread's IPC operations should
    /// target. For git threads with a live worktree, returns the worktree
    /// path; otherwise returns `default_path` (typically the logical
    /// workspace path the frontend sent).
    ///
    /// Called by the IPC layer for commit / revert / diff / changed-files
    /// so the backend silently substitutes the right checkout — the
    /// frontend keeps sending `workspace.path` and nothing else has to
    /// know worktrees exist.
    pub async fn workspace_path_for_thread(
        &self,
        thread_id: &str,
        default_path: &Path,
    ) -> PathBuf {
        // First check the active map — cheapest hit, and the most common
        // case (UI actions happen while a thread is still loaded).
        {
            let active = self.active_threads.lock().await;
            if let Some(t) = active.get(thread_id) {
                return t.effective_path.clone();
            }
        }
        // Fall back to the DB — covers closed threads the user is still
        // interacting with via Commit / Revert / Diff on the completion
        // card.
        let tid = thread_id.to_string();
        let persisted: Option<String> = self
            .db
            .execute(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT worktree_path FROM threads WHERE id = ?1",
                        rusqlite::params![tid],
                        |r| r.get::<_, Option<String>>(0),
                    )
                    .ok()
                    .flatten())
            })
            .await
            .ok()
            .flatten();

        match persisted {
            Some(p) => {
                let buf = PathBuf::from(p);
                if buf.exists() { buf } else { default_path.to_path_buf() }
            }
            None => default_path.to_path_buf(),
        }
    }

    /// Read the persisted tracker kind for a thread. Falls back to Git
    /// for legacy rows written before the `tracker_kind` column existed
    /// — the column has a `NOT NULL DEFAULT 'git'` so this should be
    /// rare but the fallback keeps us robust.
    async fn tracker_kind_for_thread(&self, thread_id: &str) -> TrackerKind {
        let tid = thread_id.to_string();
        let kind_str: Option<String> = self
            .db
            .execute(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT tracker_kind FROM threads WHERE id = ?1",
                        rusqlite::params![tid],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .ok()
                    .flatten())
            })
            .await
            .ok()
            .flatten();
        match kind_str.as_deref() {
            Some("shadow") => TrackerKind::Shadow,
            _ => TrackerKind::Git,
        }
    }

    async fn resume_thread_inner(
        &self,
        thread_id: &str,
        workspace: &Workspace,
        prompt: &str,
        adapter: Arc<dyn AgentAdapter>,
        model: Option<&str>,
        cli_agent: Option<&str>,
        tracker_kind: TrackerKind,
    ) -> Result<()> {
        let tracker = self
            .tracker_for_thread(thread_id)
            .await
            .unwrap_or_else(|_| self.git_tracker.clone() as Arc<dyn VersionTracker>);

        let claude_session_id = {
            let sids = self.session_ids.lock().await;
            sids.get(thread_id)
                .cloned()
                .with_context(|| format!("no session_id for thread {thread_id}"))?
        };

        // For git-tracked threads resume into their persisted worktree
        // path. Legacy rows without a worktree fall back to the main
        // checkout (they were running there pre-Phase-2).
        let (effective_path, worktree_handle) =
            self.resolve_worktree_for_resume(thread_id, workspace).await;

        let session = adapter
            .resume(&effective_path, &claude_session_id, prompt, model, cli_agent)
            .await
            .context("failed to resume agent session")?;

        // Update stored session_id in case it changed
        let new_session_id = session.init().session_id.clone();
        {
            let mut sids = self.session_ids.lock().await;
            sids.insert(thread_id.to_string(), new_session_id.clone());
        }
        {
            let sid = new_session_id.clone();
            let tid = thread_id.to_string();
            let _ = self.db.execute(move |conn| {
                conn.execute(
                    "UPDATE threads SET session_id = ?1, status = 'running' WHERE id = ?2",
                    rusqlite::params![sid, tid],
                )?;
                Ok(())
            }).await;
        }

        self.cost_tracker
            .start_tracking(thread_id, &workspace.id);

        let gate_tx: GateSender = Arc::new(Mutex::new(None));

        let active_thread = ActiveThread {
            workspace_id: workspace.id.clone(),
            effective_path: effective_path.clone(),
            worktree: worktree_handle,
            tracker_kind,
            session,
            snapshot: None,
            gate_tx: gate_tx.clone(),
        };

        {
            let mut active = self.active_threads.lock().await;
            active.insert(thread_id.to_string(), active_thread);
            if tracker_kind == TrackerKind::Shadow {
                self.reservations.lock().await.remove(&workspace.id);
            }
        }

        let event_stream = {
            let mut active = self.active_threads.lock().await;
            let thread = active.get_mut(thread_id).expect("just inserted");
            thread.session.events()
        };

        let thread_id_clone = thread_id.to_string();
        let event_tx = self.event_tx.clone();
        let cost_tracker = self.cost_tracker.clone();
        let active_threads = self.active_threads.clone();
        let budget_cap = workspace.budget_cap;
        let db = self.db.clone();
        let validators = self.validators.clone();
        let workspace_id_owned = workspace.id.clone();
        let workspace_path_owned = effective_path.clone();
        let tracker_for_task = tracker.clone();

        tokio::spawn(async move {
            Self::consume_events(
                thread_id_clone,
                event_tx,
                cost_tracker,
                active_threads,
                budget_cap,
                event_stream,
                db,
                gate_tx,
                validators,
                workspace_id_owned,
                workspace_path_owned,
                tracker_for_task,
            )
            .await;
        });

        info!(thread_id = %thread_id, "thread resumed");
        Ok(())
    }

    async fn consume_events(
        thread_id: String,
        event_tx: mpsc::UnboundedSender<ThreadEvent>,
        cost_tracker: Arc<CostTracker>,
        active_threads: Arc<Mutex<HashMap<String, ActiveThread>>>,
        budget_cap: Option<f64>,
        mut events_stream: std::pin::Pin<Box<dyn futures::Stream<Item = AgentEvent> + Send>>,
        db: DbHandle,
        gate_tx: GateSender,
        validators: Arc<ValidatorRegistry>,
        workspace_id: String,
        workspace_path: PathBuf,
        version_tracker: Arc<dyn VersionTracker>,
    ) {
        let _cost_guard = CostFinalizer {
            thread_id: thread_id.clone(),
            cost_tracker: cost_tracker.clone(),
            db: db.clone(),
        };

        let mut final_status = "completed";

        while let Some(event) = events_stream.next().await {
            cost_tracker.process_event(&thread_id, &event);

            if let Some(cap) = budget_cap {
                if cost_tracker.check_budget(&thread_id, cap) {
                    warn!(thread_id = %thread_id, cap, "budget cap exceeded — killing session");
                    {
                        let active = active_threads.lock().await;
                        if let Some(thread) = active.get(&thread_id) {
                            let _ = thread.session.cancel().await;
                        }
                    }
                    let error_event = AgentEvent::Error {
                        message: format!("Budget cap of ${cap:.2} exceeded. Session terminated."),
                        recoverable: false,
                    };
                    Self::persist_event(&db, &thread_id, &error_event).await;
                    let _ = event_tx.send(ThreadEvent {
                        thread_id: thread_id.clone(),
                        timestamp: Utc::now(),
                        event: error_event,
                        parent_tool_use_id: None,
                    });
                    final_status = "error";
                    break;
                }
            }

            Self::persist_event(&db, &thread_id, &event).await;

            // Record pre-edit state for file-write tools before the agent
            // executes them. Claude stream-json guarantees tool_use arrives
            // before tool_result, and ACP delivers tool_call before any
            // tool_call_update with the content written — so this runs
            // before the file actually hits disk in either transport.
            if let AgentEvent::ToolRequest {
                tool_name, input, ..
            } = &event
            {
                if is_file_write_tool(tool_name) {
                    if let Some(fp) = extract_file_path(tool_name, input, &workspace_path) {
                        if let Err(e) = version_tracker
                            .record_pre_edit(&thread_id, &workspace_path, &fp)
                            .await
                        {
                            warn!(
                                error = %e,
                                file = %fp.display(),
                                "version tracker failed to record pre-edit — continuing"
                            );
                        }
                    }
                }
            }

            let gate_tool_id = match &event {
                AgentEvent::ToolRequest { id, needs_approval: true, .. } => Some(id.clone()),
                _ => None,
            };

            if gate_tool_id.is_some() {
                let tid = thread_id.clone();
                let _ = db.execute(move |conn| {
                    conn.execute(
                        "UPDATE threads SET status = 'gate' WHERE id = ?1",
                        rusqlite::params![tid],
                    )?;
                    Ok(())
                }).await;
            }

            // Set up gate oneshot BEFORE sending the event so approve/reject
            // can find it immediately after the frontend receives the event.
            let gate_rx = if gate_tool_id.is_some() {
                let (tx, rx) = oneshot::channel();
                {
                    let mut slot = gate_tx.lock().await;
                    *slot = Some(tx);
                }
                Some(rx)
            } else {
                None
            };

            // Defer forwarding Complete until after validators have run so that a
            // validator-rejected completion never surfaces to the frontend as a
            // successful completion.
            let is_complete = matches!(event, AgentEvent::Complete { .. });
            if !is_complete {
                let thread_event = ThreadEvent {
                    thread_id: thread_id.clone(),
                    timestamp: Utc::now(),
                    event: event.clone(),
                    parent_tool_use_id: None,
                };
                if event_tx.send(thread_event).is_err() {
                    break;
                }
            }

            // Gate pausing: wait for user decision before consuming more events
            if let Some(rx) = gate_rx {
                let tool_id = gate_tool_id.unwrap();
                info!(thread_id = %thread_id, "gate paused — waiting for user decision");

                match rx.await {
                    Ok(GateDecision::Continue) => {
                        info!(thread_id = %thread_id, "gate continued — resuming event stream");
                        {
                            let active = active_threads.lock().await;
                            if let Some(thread) = active.get(&thread_id) {
                                thread.session.approve(&tool_id).await.ok();
                            }
                        }
                        let tid = thread_id.clone();
                        let _ = db.execute(move |conn| {
                            conn.execute(
                                "UPDATE threads SET status = 'running' WHERE id = ?1",
                                rusqlite::params![tid],
                            )?;
                            Ok(())
                        }).await;
                    }
                    Ok(GateDecision::Abort) | Err(_) => {
                        info!(thread_id = %thread_id, "gate aborted — killing session");
                        {
                            let active = active_threads.lock().await;
                            if let Some(thread) = active.get(&thread_id) {
                                thread.session.reject(&tool_id, "rejected by user").await.ok();
                                let _ = thread.session.cancel().await;
                            }
                        }
                        let abort_event = AgentEvent::Error {
                            message: "Gate rejected by user".to_string(),
                            recoverable: false,
                        };
                        Self::persist_event(&db, &thread_id, &abort_event).await;
                        let _ = event_tx.send(ThreadEvent {
                            thread_id: thread_id.clone(),
                            timestamp: Utc::now(),
                            event: abort_event,
                            parent_tool_use_id: None,
                        });
                        final_status = "interrupted";
                        break;
                    }
                }
            }

            // Output validators: only run on Complete events in v1.
            if is_complete {
                let decision = Self::maybe_run_validators(
                    &event,
                    &validators,
                    &workspace_id,
                    &workspace_path,
                    &thread_id,
                    &db,
                    &event_tx,
                    &gate_tx,
                )
                .await;
                match decision {
                    ValidatorFlow::Continue => {
                        // Forward the previously-deferred Complete event now.
                        let thread_event = ThreadEvent {
                            thread_id: thread_id.clone(),
                            timestamp: Utc::now(),
                            event: event.clone(),
                            parent_tool_use_id: None,
                        };
                        if event_tx.send(thread_event).is_err() {
                            break;
                        }
                    }
                    ValidatorFlow::Aborted => {
                        {
                            let active = active_threads.lock().await;
                            if let Some(thread) = active.get(&thread_id) {
                                let _ = thread.session.cancel().await;
                            }
                        }
                        let abort_event = AgentEvent::Error {
                            message: "Validator findings rejected by user".to_string(),
                            recoverable: false,
                        };
                        Self::persist_event(&db, &thread_id, &abort_event).await;
                        let _ = event_tx.send(ThreadEvent {
                            thread_id: thread_id.clone(),
                            timestamp: Utc::now(),
                            event: abort_event,
                            parent_tool_use_id: None,
                        });
                        final_status = "interrupted";
                        break;
                    }
                }
            }

            // threads.cost_usd comes from the Complete event (authoritative per-run
            // cost from the agent). The costs table (CostFinalizer/CostTracker) is an
            // independent audit log — slight divergence is expected and by design.
            match &event {
                AgentEvent::Complete { summary, total_cost_usd, duration_ms, .. } => {
                    let now = Utc::now().to_rfc3339();
                    let s = summary.clone();
                    let cost = *total_cost_usd;
                    let dur = *duration_ms as i64;
                    let tid = thread_id.clone();
                    let _ = db.execute(move |conn| {
                        conn.execute(
                            "UPDATE threads SET status = 'completed', summary = ?1, cost_usd = cost_usd + ?2, duration_ms = ?3, completed_at = ?4 WHERE id = ?5",
                            rusqlite::params![s, cost, dur, now, tid],
                        )?;
                        Ok(())
                    }).await;
                    final_status = "completed";
                    break;
                }
                AgentEvent::Error { recoverable: false, .. } => {
                    final_status = "error";
                    break;
                }
                _ => {}
            }
        }

        if final_status == "error" || final_status == "interrupted" {
            let status = final_status.to_string();
            let tid = thread_id.clone();
            let _ = db.execute(move |conn| {
                conn.execute(
                    "UPDATE threads SET status = ?1 WHERE id = ?2",
                    rusqlite::params![status, tid],
                )?;
                Ok(())
            }).await;
        }

        let mut active = active_threads.lock().await;
        active.remove(&thread_id);
    }

    async fn persist_event(db: &DbHandle, thread_id: &str, event: &AgentEvent) {
        let event_type = match event {
            AgentEvent::Thinking { .. } => "thinking",
            AgentEvent::Text { .. } => "text",
            AgentEvent::ToolRequest { .. } => "tool_request",
            AgentEvent::ToolResult { .. } => "tool_result",
            AgentEvent::CostUpdate { .. } => "cost_update",
            AgentEvent::Error { .. } => "error",
            AgentEvent::SubAgentSpawned { .. } => "sub_agent_spawned",
            AgentEvent::SubAgentComplete { .. } => "sub_agent_complete",
            AgentEvent::Complete { .. } => "complete",
            AgentEvent::ValidationResult { .. } => "validation_result",
            AgentEvent::ContextUsage { .. } => "context_usage",
        }
        .to_string();
        let data = serde_json::to_string(event).unwrap_or_default();
        let now = Utc::now().to_rfc3339();
        let tid = thread_id.to_string();
        let _ = db.execute(move |conn| {
            conn.execute(
                "INSERT INTO events (thread_id, event_type, timestamp, data) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![tid, event_type, now, data],
            )?;
            Ok(())
        }).await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn maybe_run_validators(
        event: &AgentEvent,
        validators: &ValidatorRegistry,
        workspace_id: &str,
        workspace_path: &std::path::Path,
        thread_id: &str,
        db: &DbHandle,
        event_tx: &mpsc::UnboundedSender<ThreadEvent>,
        gate_tx: &GateSender,
    ) -> ValidatorFlow {
        // Feature-flag check: load feature flag + configured validators in one DB hop.
        let wid = workspace_id.to_string();
        let loaded = db
            .execute(move |conn| {
                let enabled = is_feature_enabled(conn, FEATURE_VALIDATORS).unwrap_or(false);
                if !enabled {
                    return Ok::<_, anyhow::Error>((false, Vec::new()));
                }
                let rows = db::list_enabled_validators(conn, &wid).unwrap_or_default();
                Ok((true, rows))
            })
            .await;

        let (feature_enabled, configured) = match loaded {
            Ok(v) => v,
            Err(_) => return ValidatorFlow::Continue,
        };
        if !feature_enabled || configured.is_empty() {
            return ValidatorFlow::Continue;
        }

        let mut any_failed = false;
        let mut target_index = 0u64;
        if let AgentEvent::Complete { turns, .. } = event {
            target_index = *turns as u64;
        }

        for row in configured {
            let Some(validator) = validators.get(&row.validator_type) else {
                continue;
            };
            if !validator.wants(event) {
                continue;
            }
            let config_value: serde_json::Value =
                serde_json::from_str(&row.config_json).unwrap_or(serde_json::Value::Null);
            let ctx = ValidationContext {
                thread_id: thread_id.to_string(),
                workspace_path: workspace_path.to_path_buf(),
                config: config_value,
                recent_text: Vec::new(),
            };

            let start = std::time::Instant::now();
            let report = validator.validate(event, &ctx).await;
            let duration_ms = start.elapsed().as_millis() as u64;

            if report.outcome == panes_events::ValidationOutcome::Fail {
                any_failed = true;
            }

            let result_event = AgentEvent::ValidationResult {
                validator: row.validator_type.clone(),
                target_event_index: target_index,
                outcome: report.outcome,
                findings: report.findings,
                duration_ms,
            };
            Self::persist_event(db, thread_id, &result_event).await;
            let _ = event_tx.send(ThreadEvent {
                thread_id: thread_id.to_string(),
                timestamp: Utc::now(),
                event: result_event,
                parent_tool_use_id: None,
            });
        }

        if !any_failed {
            return ValidatorFlow::Continue;
        }

        // At least one validator failed — install a gate oneshot and pause.
        let (tx, rx) = oneshot::channel::<GateDecision>();
        {
            let mut slot = gate_tx.lock().await;
            *slot = Some(tx);
        }
        {
            let tid = thread_id.to_string();
            let _ = db
                .execute(move |conn| {
                    conn.execute(
                        "UPDATE threads SET status = 'gate' WHERE id = ?1",
                        rusqlite::params![tid],
                    )?;
                    Ok(())
                })
                .await;
        }
        info!(thread_id = %thread_id, "validator gate paused — waiting for user decision");

        match rx.await {
            Ok(GateDecision::Continue) | Err(_) => {
                let tid = thread_id.to_string();
                let _ = db
                    .execute(move |conn| {
                        conn.execute(
                            "UPDATE threads SET status = 'running' WHERE id = ?1",
                            rusqlite::params![tid],
                        )?;
                        Ok(())
                    })
                    .await;
                ValidatorFlow::Continue
            }
            Ok(GateDecision::Abort) => ValidatorFlow::Aborted,
        }
    }

    pub async fn approve(&self, thread_id: &str, _tool_use_id: &str) -> Result<(), PanesError> {
        let active = self.active_threads.lock().await;
        let thread = active
            .get(thread_id)
            .ok_or_else(|| PanesError::ThreadNotFound {
                thread_id: thread_id.to_string(),
                message: "thread not found".to_string(),
            })?;
        let mut slot = thread.gate_tx.lock().await;
        match slot.take() {
            Some(tx) => { let _ = tx.send(GateDecision::Continue); Ok(()) }
            None => Err(PanesError::NoGatePending {
                thread_id: thread_id.to_string(),
                message: format!("no gate pending for thread {thread_id}"),
            }),
        }
    }

    pub async fn reject(&self, thread_id: &str, _tool_use_id: &str, _reason: &str) -> Result<(), PanesError> {
        let active = self.active_threads.lock().await;
        let thread = active
            .get(thread_id)
            .ok_or_else(|| PanesError::ThreadNotFound {
                thread_id: thread_id.to_string(),
                message: "thread not found".to_string(),
            })?;
        let mut slot = thread.gate_tx.lock().await;
        match slot.take() {
            Some(tx) => { let _ = tx.send(GateDecision::Abort); Ok(()) }
            None => Err(PanesError::NoGatePending {
                thread_id: thread_id.to_string(),
                message: format!("no gate pending for thread {thread_id}"),
            }),
        }
    }

    pub async fn cancel(&self, thread_id: &str) -> Result<(), PanesError> {
        let active = self.active_threads.lock().await;
        if let Some(thread) = active.get(thread_id) {
            thread.session.cancel().await
                .map_err(|e| PanesError::Internal { message: e.to_string() })?;
        }
        Ok(())
    }

    /// Switch the active model on a live thread. Unsupported by adapters
    /// whose backend can't change mid-session (Claude stream-json) — those
    /// return a clear error the UI surfaces to the user.
    pub async fn set_thread_model(&self, thread_id: &str, model: &str) -> Result<(), PanesError> {
        let active = self.active_threads.lock().await;
        let thread = active
            .get(thread_id)
            .ok_or_else(|| PanesError::ThreadNotFound {
                thread_id: thread_id.to_string(),
                message: "thread not found".to_string(),
            })?;
        thread
            .session
            .set_model(model)
            .await
            .map_err(|e| PanesError::Internal { message: e.to_string() })
    }

    pub async fn get_snapshot(&self, thread_id: &str) -> Option<git::SnapshotRef> {
        let active = self.active_threads.lock().await;
        active
            .get(thread_id)
            .and_then(|t| t.snapshot.clone())
    }

    pub async fn remove_thread(&self, thread_id: &str) {
        let mut active = self.active_threads.lock().await;
        active.remove(thread_id);
    }

    pub fn list_adapters(&self) -> Vec<String> {
        self.adapters.keys().cloned().collect()
    }

    pub async fn list_models(&self, adapter_name: &str) -> Result<Vec<panes_adapters::ModelInfo>, PanesError> {
        let adapter = self
            .adapters
            .get(adapter_name)
            .ok_or_else(|| PanesError::AdapterNotFound {
                adapter: adapter_name.to_string(),
                message: format!("unknown adapter: {adapter_name}"),
            })?;
        adapter.list_models().await
            .map_err(|e| PanesError::Internal { message: e.to_string() })
    }

    /// Borrow the Arc for a named adapter so callers can reach its
    /// `list_agents` / `list_models` trait methods. Returns `None` if the
    /// adapter isn't registered — matches what `list_adapters` would have
    /// shown. Marked async-safe via Arc cloning rather than returning a
    /// reference tied to the SessionManager lock lifetime.
    pub fn adapter(&self, adapter_name: &str) -> Option<Arc<dyn AgentAdapter>> {
        self.adapters.get(adapter_name).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use panes_adapters::fake::{FakeAdapter, FakeScenario};

    async fn setup_session_manager() -> (SessionManager, mpsc::UnboundedReceiver<ThreadEvent>) {
        let (mgr, _db, rx) = crate::test_support::test_session_manager().await;
        (mgr, rx)
    }

    fn make_workspace() -> (tempfile::TempDir, Workspace) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace {
            id: "ws-test".to_string(),
            path: tmp.path().to_path_buf(),
            name: "test-workspace".to_string(),
            default_agent: None,
            budget_cap: None,
        };
        (tmp, ws)
    }

    async fn wait_for_thread_cleanup(mgr: &SessionManager, thread_id: &str, timeout_ms: u64) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            if !mgr.active_threads.lock().await.contains_key(thread_id) {
                return;
            }
            if tokio::time::Instant::now() > deadline {
                panic!("thread {} not cleaned up within {}ms", thread_id, timeout_ms);
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    async fn wait_for_db_status(mgr: &SessionManager, thread_id: &str, expected: &str, timeout_ms: u64) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let tid = thread_id.to_string();
            if let Ok(status) = mgr.db.execute(move |conn| {
                Ok(conn.query_row(
                    "SELECT status FROM threads WHERE id = ?1",
                    rusqlite::params![tid],
                    |row| row.get::<_, String>(0),
                )?)
            }).await {
                if status == expected {
                    return;
                }
            }
            if tokio::time::Instant::now() > deadline {
                panic!("thread {} did not reach status '{}' within {}ms", thread_id, expected, timeout_ms);
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn test_start_thread_unknown_agent() {
        let (mgr, _rx) = setup_session_manager().await;
        let (_tmp, ws) = make_workspace();
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = mgr.start_thread(&ws, "hello", "nonexistent-agent", ctx, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown agent"));
    }

    #[tokio::test]
    async fn test_start_thread_empty_agent_name_rejected() {
        let (mgr, _rx) = setup_session_manager().await;
        let (_tmp, ws) = make_workspace();
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = mgr.start_thread(&ws, "hello", "", ctx, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown agent"));
    }

    #[tokio::test]
    async fn test_resume_thread_empty_agent_name_rejected() {
        let (mgr, _rx) = setup_session_manager().await;
        let (_tmp, ws) = make_workspace();
        let result = mgr.resume_thread("t1", &ws, "hello", "", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown agent"));
    }

    #[tokio::test]
    async fn test_approve_nonexistent_thread() {
        let (mgr, _rx) = setup_session_manager().await;
        let result = mgr.approve("no-such-thread", "tool1").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("thread not found"));
    }

    #[tokio::test]
    async fn test_reject_nonexistent_thread() {
        let (mgr, _rx) = setup_session_manager().await;
        let result = mgr.reject("no-such-thread", "tool1", "reason").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("thread not found"));
    }

    #[tokio::test]
    async fn test_cancel_nonexistent_thread_is_ok() {
        let (mgr, _rx) = setup_session_manager().await;
        let result = mgr.cancel("no-such-thread").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_start_and_complete_with_fake() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hello!".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        assert!(!thread_id.is_empty());

        let mut got_complete = false;
        while let Some(te) = rx.recv().await {
            if matches!(te.event, AgentEvent::Complete { .. }) {
                got_complete = true;
                break;
            }
        }
        assert!(got_complete);
    }

    #[tokio::test]
    async fn test_custom_agent_falls_through_to_claude_code_adapter() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hello from agent!".to_string(),
        }).with_delay(0);
        // Register as "claude-code" — the default fallback
        struct NamedFake(FakeAdapter);
        #[async_trait::async_trait]
        impl AgentAdapter for NamedFake {
            fn name(&self) -> &str { "claude-code" }
            async fn spawn(&self, wp: &Path, p: &str, c: &SessionContext, m: Option<&str>, _a: Option<&str>) -> Result<Box<dyn AgentSession>> {
                self.0.spawn(wp, p, c, m, _a).await
            }
            async fn resume(&self, wp: &Path, sid: &str, p: &str, m: Option<&str>, _a: Option<&str>) -> Result<Box<dyn AgentSession>> {
                self.0.resume(wp, sid, p, m, _a).await
            }
        }
        mgr.register_adapter(Arc::new(NamedFake(adapter)));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        // Use a custom agent name that doesn't match any registered adapter
        let thread_id = mgr.start_thread(&ws, "hello", "my-custom-agent", ctx, None).await.unwrap();
        assert!(!thread_id.is_empty());

        let mut got_complete = false;
        while let Some(te) = rx.recv().await {
            if matches!(te.event, AgentEvent::Complete { .. }) {
                got_complete = true;
                break;
            }
        }
        assert!(got_complete);
    }

    // ---------------------------------------------------------------
    // Helper: insert workspace row into the DB (needed for FK)
    // ---------------------------------------------------------------
    async fn insert_workspace_row(mgr: &SessionManager, ws: &Workspace) {
        let id = ws.id.clone();
        let path = ws.path.to_string_lossy().to_string();
        let name = ws.name.clone();
        mgr.db
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO workspaces (id, path, name, created_at) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![id, path, name, "2024-01-01"],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    fn make_workspace_with_budget(cap: Option<f64>) -> (tempfile::TempDir, Workspace) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace {
            id: "ws-test".to_string(),
            path: tmp.path().to_path_buf(),
            name: "test-workspace".to_string(),
            default_agent: None,
            budget_cap: cap,
        };
        (tmp, ws)
    }

    async fn query_thread_status(mgr: &SessionManager, thread_id: &str) -> String {
        let tid = thread_id.to_string();
        mgr.db
            .execute(move |conn| {
                Ok(conn.query_row(
                    "SELECT status FROM threads WHERE id = ?1",
                    rusqlite::params![tid],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap()
    }

    /// Collect all events until Complete, Error, or timeout.
    async fn collect_events_until_done(
        rx: &mut mpsc::UnboundedReceiver<ThreadEvent>,
    ) -> Vec<ThreadEvent> {
        let mut events = vec![];
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(te)) => {
                    let done = matches!(
                        &te.event,
                        AgentEvent::Complete { .. } | AgentEvent::Error { recoverable: false, .. }
                    );
                    events.push(te);
                    if done {
                        break;
                    }
                }
                _ => break,
            }
        }
        events
    }

    /// Wait for a gated ToolRequest and return its tool_use_id.
    async fn wait_for_gate_event(
        rx: &mut mpsc::UnboundedReceiver<ThreadEvent>,
    ) -> String {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(te)) => {
                    if let AgentEvent::ToolRequest {
                        id,
                        needs_approval: true,
                        ..
                    } = &te.event
                    {
                        return id.clone();
                    }
                }
                _ => panic!("timed out waiting for gate event"),
            }
        }
    }

    // ---------------------------------------------------------------
    // A gate-compatible test adapter whose event stream does NOT
    // have its own internal pausing — it freely yields all events.
    // This lets us test SessionManager's gate logic in isolation
    // without fighting FakeSession's gate_notify mechanism.
    // ---------------------------------------------------------------
    mod gate_test_adapter {
        use std::path::Path;
        use std::pin::Pin;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        use anyhow::Result;
        use async_trait::async_trait;
        use futures::Stream;
        use panes_events::{AgentEvent, RiskLevel, SessionContext, SessionInit};
        use tokio::sync::Notify;
        use futures::stream::unfold;

        use panes_adapters::{AgentAdapter, AgentSession};

        /// An adapter that emits a gated ToolRequest. After yielding the
        /// gate event the underlying stream pauses on a Notify, which the
        /// session's approve/reject/cancel methods signal. This lets
        /// SessionManager's own oneshot-based gate logic interleave
        /// correctly with the stream.
        pub struct GateTestAdapter;

        #[async_trait]
        impl AgentAdapter for GateTestAdapter {
            fn name(&self) -> &str { "gate-test" }

            async fn spawn(
                &self,
                _workspace_path: &Path,
                _prompt: &str,
                _context: &SessionContext,
                _model: Option<&str>,
                _agent: Option<&str>,
            ) -> Result<Box<dyn AgentSession>> {
                let cancelled = Arc::new(AtomicBool::new(false));
                let resume_notify = Arc::new(Notify::new());

                // Build a channel-based stream. A background task sends
                // events into the channel, pausing at the gate until
                // resume_notify is signalled.
                let (tx, rx) = tokio::sync::mpsc::channel::<AgentEvent>(16);
                let c = cancelled.clone();
                let n = resume_notify.clone();
                tokio::spawn(async move {
                    let _ = tx.send(AgentEvent::Thinking {
                        text: "Thinking about risky operation...".to_string(),
                    }).await;

                    let _ = tx.send(AgentEvent::ToolRequest {
                        id: "gate_0".to_string(),
                        tool_name: "Bash".to_string(),
                        description: "rm -rf /important".to_string(),
                        input: serde_json::json!({"command": "rm -rf /important"}),
                        needs_approval: true,
                        risk_level: RiskLevel::Critical,
                    }).await;

                    // Pause here until approve/reject/cancel signals us.
                    n.notified().await;

                    if c.load(Ordering::Relaxed) {
                        return; // stream ends — no more events
                    }

                    let _ = tx.send(AgentEvent::ToolResult {
                        id: "gate_0".to_string(),
                        tool_name: "Bash".to_string(),
                        success: true,
                        output: "Executed successfully".to_string(),
                        raw_output: None,
                        duration_ms: 500,
                    }).await;

                    let _ = tx.send(AgentEvent::Complete {
                        summary: "Risky operation completed".to_string(),
                        total_cost_usd: 0.01,
                        duration_ms: 3000,
                        turns: 2,
                    }).await;
                });

                Ok(Box::new(GateTestSession {
                    init_data: SessionInit {
                        session_id: uuid::Uuid::new_v4().to_string(),
                        model: "gate-test-model".to_string(),
                        cwd: "/tmp".to_string(),
                        tools: vec!["Bash".into()],
                    },
                    cancelled,
                    resume_notify,
                    rx: tokio::sync::Mutex::new(Some(rx)),
                }))
            }

            async fn resume(
                &self,
                workspace_path: &Path,
                _session_id: &str,
                prompt: &str,
                model: Option<&str>,
                agent: Option<&str>,
            ) -> Result<Box<dyn AgentSession>> {
                self.spawn(
                    workspace_path,
                    prompt,
                    &SessionContext { briefing: None, memories: vec![], budget_cap: None },
                    model,
                    agent,
                ).await
            }
        }

        struct GateTestSession {
            init_data: SessionInit,
            cancelled: Arc<AtomicBool>,
            resume_notify: Arc<Notify>,
            rx: tokio::sync::Mutex<Option<tokio::sync::mpsc::Receiver<AgentEvent>>>,
        }

        #[async_trait]
        impl AgentSession for GateTestSession {
            fn init(&self) -> &SessionInit { &self.init_data }

            fn events(&mut self) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
                let rx = self.rx.get_mut().take().expect("events() called twice");
                Box::pin(unfold(rx, |mut rx| async move {
                    rx.recv().await.map(|event| (event, rx))
                }))
            }

            async fn approve(&self, _tool_use_id: &str) -> Result<()> {
                self.resume_notify.notify_one();
                Ok(())
            }

            async fn reject(&self, _tool_use_id: &str, _reason: &str) -> Result<()> {
                self.cancelled.store(true, Ordering::Relaxed);
                self.resume_notify.notify_one();
                Ok(())
            }

            async fn cancel(&self) -> Result<()> {
                self.cancelled.store(true, Ordering::Relaxed);
                self.resume_notify.notify_one();
                Ok(())
            }
        }

        /// Emits a single gated Write ToolRequest with a well-formed
        /// `file_path` input so SessionManager's version-tracker hook
        /// runs against it. On approve, yields a ToolResult + Complete;
        /// on reject the stream ends immediately (no ToolResult, no
        /// file write — consistent with how a real rejected agent
        /// behaves).
        pub struct GatedWriteAdapter {
            pub file_path: String,
        }

        #[async_trait]
        impl AgentAdapter for GatedWriteAdapter {
            fn name(&self) -> &str { "gated-write-test" }

            async fn spawn(
                &self,
                _workspace_path: &Path,
                _prompt: &str,
                _context: &SessionContext,
                _model: Option<&str>,
                _agent: Option<&str>,
            ) -> Result<Box<dyn AgentSession>> {
                let cancelled = Arc::new(AtomicBool::new(false));
                let resume_notify = Arc::new(Notify::new());

                let (tx, rx) = tokio::sync::mpsc::channel::<AgentEvent>(16);
                let c = cancelled.clone();
                let n = resume_notify.clone();
                let fp = self.file_path.clone();
                tokio::spawn(async move {
                    let _ = tx.send(AgentEvent::ToolRequest {
                        id: "gw_0".to_string(),
                        tool_name: "Write".to_string(),
                        description: format!("Write file: {fp}"),
                        input: serde_json::json!({
                            "file_path": fp,
                            "content": "potentially-dangerous content",
                        }),
                        needs_approval: true,
                        risk_level: RiskLevel::High,
                    }).await;

                    n.notified().await;

                    if c.load(Ordering::Relaxed) {
                        return; // rejected — stream ends without any write
                    }

                    let _ = tx.send(AgentEvent::ToolResult {
                        id: "gw_0".to_string(),
                        tool_name: "Write".to_string(),
                        success: true,
                        output: "File written".to_string(),
                        raw_output: None,
                        duration_ms: 50,
                    }).await;

                    let _ = tx.send(AgentEvent::Complete {
                        summary: "done".to_string(),
                        total_cost_usd: 0.01,
                        duration_ms: 1000,
                        turns: 1,
                    }).await;
                });

                Ok(Box::new(GatedWriteSession {
                    init_data: SessionInit {
                        session_id: uuid::Uuid::new_v4().to_string(),
                        model: "gated-write-test-model".to_string(),
                        cwd: "/tmp".to_string(),
                        tools: vec!["Write".into()],
                    },
                    cancelled,
                    resume_notify,
                    rx: tokio::sync::Mutex::new(Some(rx)),
                }))
            }

            async fn resume(
                &self,
                workspace_path: &Path,
                _session_id: &str,
                prompt: &str,
                model: Option<&str>,
                agent: Option<&str>,
            ) -> Result<Box<dyn AgentSession>> {
                self.spawn(
                    workspace_path,
                    prompt,
                    &SessionContext { briefing: None, memories: vec![], budget_cap: None },
                    model,
                    agent,
                ).await
            }
        }

        struct GatedWriteSession {
            init_data: SessionInit,
            cancelled: Arc<AtomicBool>,
            resume_notify: Arc<Notify>,
            rx: tokio::sync::Mutex<Option<tokio::sync::mpsc::Receiver<AgentEvent>>>,
        }

        #[async_trait]
        impl AgentSession for GatedWriteSession {
            fn init(&self) -> &SessionInit { &self.init_data }

            fn events(&mut self) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
                let rx = self.rx.get_mut().take().expect("events() called twice");
                Box::pin(unfold(rx, |mut rx| async move {
                    rx.recv().await.map(|event| (event, rx))
                }))
            }

            async fn approve(&self, _tool_use_id: &str) -> Result<()> {
                self.resume_notify.notify_one();
                Ok(())
            }

            async fn reject(&self, _tool_use_id: &str, _reason: &str) -> Result<()> {
                self.cancelled.store(true, Ordering::Relaxed);
                self.resume_notify.notify_one();
                Ok(())
            }

            async fn cancel(&self) -> Result<()> {
                self.cancelled.store(true, Ordering::Relaxed);
                self.resume_notify.notify_one();
                Ok(())
            }
        }
    }

    // ---------------------------------------------------------------
    // resume_thread tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_resume_thread_success() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "First reply".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        // Start a thread first so we have a stored session_id
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        // Drain all events from the first run
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        // Now resume the same thread
        mgr.resume_thread(&thread_id, &ws, "follow up", "fake", None)
            .await
            .unwrap();

        let events = collect_events_until_done(&mut rx).await;
        assert!(
            events.iter().any(|te| matches!(&te.event, AgentEvent::Complete { .. })),
            "resumed thread should complete"
        );
    }

    #[tokio::test]
    async fn test_resume_thread_nonexistent() {
        let (mut mgr, _rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "x".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace();
        let result = mgr.resume_thread("no-such-thread", &ws, "prompt", "fake", None).await;
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("no session_id"),
            "should fail because no session_id was stored"
        );
    }

    #[tokio::test]
    async fn test_resume_thread_unknown_agent() {
        let (mgr, _rx) = setup_session_manager().await;
        let (_tmp, ws) = make_workspace();
        let result = mgr.resume_thread("t1", &ws, "prompt", "nonexistent-agent", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("unknown agent"));
    }

    // ---------------------------------------------------------------
    // Budget cap tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_budget_cap_exceeded() {
        // FakeScenario::TextOnly emits CostUpdate with total_usd: 0.003
        // Set budget_cap to 0.001 — well below the cost
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Expensive answer".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace_with_budget(Some(0.001));
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let _thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let events = collect_events_until_done(&mut rx).await;

        let has_budget_error = events.iter().any(|te| {
            matches!(&te.event, AgentEvent::Error { message, .. } if message.contains("Budget cap"))
        });
        assert!(has_budget_error, "should have a budget cap error event");

        // Should NOT have a Complete event — session was killed
        let has_complete = events
            .iter()
            .any(|te| matches!(&te.event, AgentEvent::Complete { .. }));
        assert!(!has_complete, "session should be terminated before completion");
    }

    #[tokio::test]
    async fn test_budget_cap_not_exceeded() {
        // Set budget_cap to 10.0 — well above the 0.003 cost
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Cheap answer".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace_with_budget(Some(10.0));
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let _thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let events = collect_events_until_done(&mut rx).await;

        let has_complete = events
            .iter()
            .any(|te| matches!(&te.event, AgentEvent::Complete { .. }));
        assert!(has_complete, "thread should complete normally when under budget");

        let has_budget_error = events.iter().any(|te| {
            matches!(&te.event, AgentEvent::Error { message, .. } if message.contains("Budget cap"))
        });
        assert!(!has_budget_error, "no budget error expected");
    }

    // ---------------------------------------------------------------
    // Gate approve / reject tests (using GateTestAdapter)
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_gate_approve_completes_thread() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr
            .start_thread(&ws, "do something risky", "gate-test", ctx, None)
            .await
            .unwrap();

        // Receive events until we see the gated ToolRequest
        let tool_use_id = wait_for_gate_event(&mut rx).await;

        // Approve the gate — this unblocks both consume_events and the underlying session
        mgr.approve(&thread_id, &tool_use_id).await.unwrap();

        // Collect remaining events — should see ToolResult + Complete
        let events = collect_events_until_done(&mut rx).await;
        let has_tool_result = events
            .iter()
            .any(|te| matches!(&te.event, AgentEvent::ToolResult { .. }));
        let has_complete = events
            .iter()
            .any(|te| matches!(&te.event, AgentEvent::Complete { .. }));
        assert!(has_tool_result, "approved gate should produce ToolResult");
        assert!(has_complete, "approved gate should lead to Complete");
    }

    #[tokio::test]
    async fn test_gate_reject_interrupts_thread() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr
            .start_thread(&ws, "do something risky", "gate-test", ctx, None)
            .await
            .unwrap();

        // Receive events until we see the gated ToolRequest
        let tool_use_id = wait_for_gate_event(&mut rx).await;

        // Reject the gate
        mgr.reject(&thread_id, &tool_use_id, "too dangerous")
            .await
            .unwrap();

        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        // Drain any remaining events — should NOT have a Complete
        let mut remaining = vec![];
        while let Ok(Some(te)) =
            tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await
        {
            remaining.push(te);
        }

        let has_complete = remaining
            .iter()
            .any(|te| matches!(&te.event, AgentEvent::Complete { .. }));
        assert!(!has_complete, "rejected gate should NOT lead to Complete");

        // Verify DB status is interrupted
        let status = query_thread_status(&mgr, &thread_id).await;
        assert_eq!(status, "interrupted");
    }

    // ---------------------------------------------------------------
    // get_snapshot tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_get_snapshot_nonexistent() {
        let (mgr, _rx) = setup_session_manager().await;
        let result = mgr.get_snapshot("no-such-thread").await;
        assert!(result.is_none());
    }

    // ---------------------------------------------------------------
    // remove_thread tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_remove_thread() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hi".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        // Wait for completion — consume_events removes thread from active map itself
        let _ = collect_events_until_done(&mut rx).await;

        // Even after auto-removal, calling remove_thread should be a no-op (not panic)
        mgr.remove_thread(&thread_id).await;

        // Verify it's definitely gone
        let active = mgr.active_threads.lock().await;
        assert!(
            !active.contains_key(&thread_id),
            "thread should be removed from active map"
        );
    }

    // ---------------------------------------------------------------
    // list_adapters tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_list_adapters() {
        let (mut mgr, _rx) = setup_session_manager().await;
        assert!(mgr.list_adapters().is_empty(), "no adapters registered yet");

        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "x".to_string(),
        });
        mgr.register_adapter(Arc::new(adapter));

        let names = mgr.list_adapters();
        assert_eq!(names.len(), 1);
        assert!(names.contains(&"fake".to_string()));
    }

    #[tokio::test]
    async fn test_list_adapters_multiple() {
        let (mut mgr, _rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(
            FakeAdapter::new(FakeScenario::TextOnly { response: "a".into() }),
        ));
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let mut names = mgr.list_adapters();
        names.sort();
        assert_eq!(names, vec!["fake", "gate-test"]);
    }

    // ---------------------------------------------------------------
    // list_models tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_list_models_unknown_adapter() {
        let (mgr, _rx) = setup_session_manager().await;
        let result = mgr.list_models("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_list_models_fake_adapter() {
        let (mut mgr, _rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(FakeAdapter::new(FakeScenario::TextOnly {
            response: "x".into(),
        })));
        let models = mgr.list_models("fake").await.unwrap();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id == "sonnet"));
        assert!(models.iter().any(|m| m.id == "opus"));
        assert!(models.iter().any(|m| m.id == "haiku"));
    }

    // ---------------------------------------------------------------
    // ThreadStatus Display impl
    // ---------------------------------------------------------------

    #[test]
    fn test_thread_status_display() {
        assert_eq!(format!("{}", ThreadStatus::Pending), "pending");
        assert_eq!(format!("{}", ThreadStatus::Running), "running");
        assert_eq!(format!("{}", ThreadStatus::Gate), "gate");
        assert_eq!(format!("{}", ThreadStatus::Completed), "completed");
        assert_eq!(format!("{}", ThreadStatus::Error), "error");
        assert_eq!(format!("{}", ThreadStatus::Interrupted), "interrupted");
    }

    // ---------------------------------------------------------------
    // persist_event — verify events are stored in SQLite
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_events_persisted_to_db() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Stored!".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        // Wait for thread to complete
        let _ = collect_events_until_done(&mut rx).await;

        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        // Query the events table
        let tid = thread_id.clone();
        let (event_count, types) = mgr.db
            .execute(move |conn| {
                let count: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM events WHERE thread_id = ?1",
                    rusqlite::params![tid],
                    |row| row.get(0),
                )?;
                let mut stmt = conn.prepare("SELECT event_type FROM events WHERE thread_id = ?1 ORDER BY id")?;
                let types: Vec<String> = stmt
                    .query_map(rusqlite::params![tid], |row| row.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok((count, types))
            })
            .await
            .unwrap();

        // TextOnly emits: Thinking, CostUpdate, Text, Complete = 4 events
        assert_eq!(event_count, 4, "all events should be persisted to DB");
        assert_eq!(types, vec!["thinking", "cost_update", "text", "complete"]);
    }

    // ---------------------------------------------------------------
    // DB status updates — verify thread status transitions
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_thread_status_completed_in_db() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Done!".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        let status = query_thread_status(&mgr, &thread_id).await;
        assert_eq!(status, "completed");

        // Verify summary and cost_usd were set
        let tid = thread_id.clone();
        let summary: String = mgr.db
            .execute(move |conn| {
                Ok(conn.query_row(
                    "SELECT summary FROM threads WHERE id = ?1",
                    rusqlite::params![tid],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(summary, "Done!");
    }

    #[tokio::test]
    async fn test_thread_status_error_in_db() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::Error {
            message: "Something went wrong".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        let status = query_thread_status(&mgr, &thread_id).await;
        assert_eq!(status, "error");
    }

    #[tokio::test]
    async fn test_budget_cap_sets_error_status_in_db() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Expensive".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace_with_budget(Some(0.001));
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        let status = query_thread_status(&mgr, &thread_id).await;
        assert_eq!(status, "error");
    }

    // ---------------------------------------------------------------
    // session_id persistence via start_thread + load_session_ids
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_session_id_stored_and_loadable() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "x".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;

        // Verify session_id is in the in-memory map
        {
            let sids = mgr.session_ids.lock().await;
            assert!(sids.contains_key(&thread_id));
        }

        // Verify session_id is in the DB
        let tid = thread_id.clone();
        let stored_sid: String = mgr.db
            .execute(move |conn| {
                Ok(conn.query_row(
                    "SELECT session_id FROM threads WHERE id = ?1",
                    rusqlite::params![tid],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        assert!(!stored_sid.is_empty());

        // Verify load_session_ids can reconstruct the map from DB
        let loaded = SessionManager::load_session_ids(&mgr.db).await;
        assert_eq!(loaded.get(&thread_id).unwrap(), &stored_sid);
    }

    // ---------------------------------------------------------------
    // Gate pausing sets gate status in DB
    // ---------------------------------------------------------------

    // ---------------------------------------------------------------
    // Model selection tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_start_thread_with_model() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hi".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, Some("opus")).await.unwrap();
        assert!(!thread_id.is_empty());

        let events = collect_events_until_done(&mut rx).await;
        assert!(events.iter().any(|te| matches!(&te.event, AgentEvent::Complete { .. })));
    }

    #[tokio::test]
    async fn test_start_thread_model_none_uses_default() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Hi".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let _thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();

        let events = collect_events_until_done(&mut rx).await;
        assert!(events.iter().any(|te| matches!(&te.event, AgentEvent::Complete { .. })));
    }

    #[tokio::test]
    async fn test_resume_thread_with_model() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "First".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        mgr.resume_thread(&thread_id, &ws, "follow up", "fake", Some("sonnet"))
            .await
            .unwrap();

        let events = collect_events_until_done(&mut rx).await;
        assert!(events.iter().any(|te| matches!(&te.event, AgentEvent::Complete { .. })));
    }

    #[tokio::test]
    async fn test_gate_status_set_in_db() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr
            .start_thread(&ws, "risky op", "gate-test", ctx, None)
            .await
            .unwrap();

        // Wait for the gate event
        let _tool_use_id = wait_for_gate_event(&mut rx).await;

        wait_for_db_status(&mgr, &thread_id, "gate", 2000).await;

        let status = query_thread_status(&mgr, &thread_id).await;
        assert_eq!(status, "gate", "DB should show gate status while paused");

        // Clean up — reject so the background task stops
        mgr.reject(&thread_id, "gate_0", "test cleanup").await.unwrap();
    }

    // ---------------------------------------------------------------
    // One-thread-per-workspace guard tests
    // ---------------------------------------------------------------

    fn make_workspace_with_id(id: &str) -> (tempfile::TempDir, Workspace) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace {
            id: id.to_string(),
            path: tmp.path().to_path_buf(),
            name: format!("test-{id}"),
            default_agent: None,
            budget_cap: None,
        };
        (tmp, ws)
    }

    #[tokio::test]
    async fn test_start_auto_inits_git_and_allows_concurrent() {
        // Non-git workspaces now get auto-initialized with git so that
        // concurrent threads can use worktree isolation.
        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace {
            id: "ws-auto-init".to_string(),
            path: tmp.path().to_path_buf(),
            name: "auto-init".to_string(),
            default_agent: None,
            budget_cap: None,
        };
        insert_workspace_row(&mgr, &ws).await;

        // Directory exists but is NOT a git repo yet.
        assert!(!tmp.path().join(".git").exists());

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid_a = mgr.start_thread(&ws, "first", "gate-test", ctx, None).await.unwrap();
        let gate_a = wait_for_gate_event(&mut rx).await;

        // After first start, git should have been auto-initialized.
        assert!(tmp.path().join(".git").exists());

        // Second concurrent start must succeed (worktree isolation).
        let ctx2 = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = mgr.start_thread(&ws, "second", "gate-test", ctx2, None).await;
        assert!(result.is_ok(), "auto-init workspace must allow concurrent threads: {:?}",
            result.as_ref().err().map(|e| e.to_string()));
        let tid_b = result.unwrap();
        let gate_b = wait_for_gate_event(&mut rx).await;

        mgr.reject(&tid_a, &gate_a, "cleanup").await.unwrap();
        mgr.reject(&tid_b, &gate_b, "cleanup").await.unwrap();
    }

    #[tokio::test]
    async fn test_start_different_workspace_concurrent_ok() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        let ws1 = Workspace {
            id: "ws-guard-2a".to_string(),
            path: tmp1.path().to_path_buf(),
            name: "test-2a".to_string(),
            default_agent: None,
            budget_cap: None,
        };
        let ws2 = Workspace {
            id: "ws-guard-2b".to_string(),
            path: tmp2.path().to_path_buf(),
            name: "test-2b".to_string(),
            default_agent: None,
            budget_cap: None,
        };
        insert_workspace_row(&mgr, &ws1).await;
        insert_workspace_row(&mgr, &ws2).await;

        let ctx1 = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid_a = mgr.start_thread(&ws1, "first", "gate-test", ctx1, None).await.unwrap();
        let gate_a = wait_for_gate_event(&mut rx).await;

        let ctx2 = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = mgr.start_thread(&ws2, "second", "gate-test", ctx2, None).await;
        assert!(result.is_ok(), "different workspace should succeed");

        let tid_b = result.unwrap();
        let gate_b = wait_for_gate_event(&mut rx).await;

        mgr.reject(&tid_a, &gate_a, "cleanup").await.unwrap();
        mgr.reject(&tid_b, &gate_b, "cleanup").await.unwrap();
    }

    #[tokio::test]
    async fn test_resume_allowed_while_other_thread_active() {
        // With auto-init, all workspaces are git-tracked and get worktree
        // isolation. Resuming thread A while thread C is active should work.
        let (mut mgr, mut rx) = setup_session_manager().await;

        let fake = FakeAdapter::new(FakeScenario::TextOnly {
            response: "done".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(fake));
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace {
            id: "ws-guard-3".to_string(),
            path: tmp.path().to_path_buf(),
            name: "test-guard-3".to_string(),
            default_agent: None,
            budget_cap: None,
        };
        insert_workspace_row(&mgr, &ws).await;

        // Start and complete thread A
        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid_a = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &tid_a, 2000).await;

        // Start thread C (gate-test) in same workspace — it will be active at the gate
        let ctx2 = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid_c = mgr.start_thread(&ws, "risky", "gate-test", ctx2, None).await.unwrap();
        let gate_id = wait_for_gate_event(&mut rx).await;

        // Resume thread A while C is active — should succeed with worktree isolation
        let result = mgr.resume_thread(&tid_a, &ws, "follow up", "fake", None).await;
        assert!(result.is_ok(), "git-tracked resume should succeed with worktree isolation: {:?}",
            result.as_ref().err().map(|e| e.to_string()));

        let _ = collect_events_until_done(&mut rx).await;
        mgr.reject(&tid_c, &gate_id, "cleanup").await.unwrap();
    }

    #[tokio::test]
    async fn test_git_workspace_allows_concurrent_threads() {
        // Phase 2: for git-backed workspaces the one-thread-per-workspace
        // guard is lifted because each thread gets its own worktree. Two
        // concurrent threads in the same workspace must both start and
        // each must have a distinct effective_path (the worktree path).
        use tokio::process::Command;

        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let tmp = tempfile::tempdir().unwrap();
        let ws_path = tmp.path().to_path_buf();
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&ws_path)
            .status()
            .await
            .unwrap();
        Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t", "commit", "--allow-empty", "-q", "-m", "init"])
            .current_dir(&ws_path)
            .status()
            .await
            .unwrap();

        let ws = Workspace {
            id: "ws-git-concurrent".to_string(),
            path: ws_path.clone(),
            name: "git-concurrent".to_string(),
            default_agent: None,
            budget_cap: None,
        };
        insert_workspace_row(&mgr, &ws).await;

        let ctx1 = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid_a = mgr
            .start_thread(&ws, "first", "gate-test", ctx1, None)
            .await
            .unwrap();
        let gate_a = wait_for_gate_event(&mut rx).await;

        // Second concurrent thread in the SAME workspace must succeed.
        let ctx2 = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = mgr
            .start_thread(&ws, "second", "gate-test", ctx2, None)
            .await;
        assert!(
            result.is_ok(),
            "git workspace must allow concurrent threads: {:?}",
            result.as_ref().err().map(|e| e.to_string())
        );
        let tid_b = result.unwrap();
        let gate_b = wait_for_gate_event(&mut rx).await;

        // Both threads should have distinct worktree paths.
        let (path_a, path_b) = {
            let active = mgr.active_threads.lock().await;
            let a = active.get(&tid_a).unwrap().effective_path.clone();
            let b = active.get(&tid_b).unwrap().effective_path.clone();
            (a, b)
        };
        assert_ne!(path_a, path_b, "concurrent threads must have distinct worktrees");
        assert_ne!(path_a, ws_path, "thread A must not run in the main checkout");
        assert_ne!(path_b, ws_path, "thread B must not run in the main checkout");

        mgr.reject(&tid_a, &gate_a, "cleanup").await.unwrap();
        mgr.reject(&tid_b, &gate_b, "cleanup").await.unwrap();
    }

    #[tokio::test]
    async fn test_formerly_non_git_workspace_allows_concurrent_via_auto_init() {
        // Workspaces without git are auto-initialized on first thread
        // start, so a second concurrent start succeeds via worktree isolation.
        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace {
            id: "ws-auto-concurrent".to_string(),
            path: tmp.path().to_path_buf(),
            name: "auto-concurrent".to_string(),
            default_agent: None,
            budget_cap: None,
        };
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid_a = mgr
            .start_thread(&ws, "first", "gate-test", ctx, None)
            .await
            .unwrap();
        let gate_a = wait_for_gate_event(&mut rx).await;

        let ctx2 = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let result = mgr
            .start_thread(&ws, "second", "gate-test", ctx2, None)
            .await;
        assert!(result.is_ok(), "auto-init workspace must allow concurrent threads: {:?}",
            result.as_ref().err().map(|e| e.to_string()));
        let tid_b = result.unwrap();
        let gate_b = wait_for_gate_event(&mut rx).await;

        mgr.reject(&tid_a, &gate_a, "cleanup").await.unwrap();
        mgr.reject(&tid_b, &gate_b, "cleanup").await.unwrap();
    }

    #[tokio::test]
    async fn test_resume_succeeds_same_thread_only() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "reply".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let tmp = tempfile::tempdir().unwrap();
        let ws = Workspace {
            id: "ws-guard-4".to_string(),
            path: tmp.path().to_path_buf(),
            name: "test-guard-4".to_string(),
            default_agent: None,
            budget_cap: None,
        };
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &tid, 2000).await;

        // Resume same thread in same workspace — no other active thread
        let result = mgr.resume_thread(&tid, &ws, "follow up", "fake", None).await;
        assert!(result.is_ok(), "resume of own thread should succeed");
        let _ = collect_events_until_done(&mut rx).await;
    }

    // ---------------------------------------------------------------
    // Fix #1: Gate approve/reject robustness
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_approve_no_pending_gate() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "done".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &tid, 2000).await;

        let result = mgr.approve(&tid, "fake-tool-id").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("thread not found"));
    }

    #[tokio::test]
    async fn test_double_approve_returns_error() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        mgr.register_adapter(Arc::new(gate_test_adapter::GateTestAdapter));

        let (_tmp, ws) = make_workspace_with_id("ws-double-approve");
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid = mgr.start_thread(&ws, "gate me", "gate-test", ctx, None).await.unwrap();
        let gate_id = wait_for_gate_event(&mut rx).await;

        mgr.approve(&tid, &gate_id).await.unwrap();

        let result = mgr.approve(&tid, &gate_id).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("no gate pending") || err.contains("thread not found"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn test_reject_no_pending_gate() {
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "done".to_string(),
        }).with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid = mgr.start_thread(&ws, "hello", "fake", ctx, None).await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &tid, 2000).await;

        let result = mgr.reject(&tid, "fake-tool-id", "test").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("thread not found"));
    }

    // ---------------------------------------------------------------
    // Validator gate tests
    // ---------------------------------------------------------------

    fn validator_workspace(path: std::path::PathBuf) -> Workspace {
        Workspace {
            id: "ws-val".to_string(),
            path,
            name: "validator-ws".to_string(),
            default_agent: None,
            budget_cap: None,
        }
    }

    async fn insert_workspace_at(mgr: &SessionManager, ws: &Workspace) {
        let id = ws.id.clone();
        let path = ws.path.to_string_lossy().to_string();
        let name = ws.name.clone();
        mgr.db
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO workspaces (id, path, name, created_at) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![id, path, name, "2024-01-01"],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn enable_validators_feature(mgr: &SessionManager) {
        mgr.db
            .execute(|conn| {
                crate::features::set_feature_enabled(
                    conn,
                    crate::features::FEATURE_VALIDATORS,
                    true,
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn add_citation_validator(mgr: &SessionManager, workspace_id: &str) {
        let wid = workspace_id.to_string();
        mgr.db
            .execute(move |conn| {
                crate::db::insert_validator(conn, &wid, "citation", "{}")?;
                Ok(())
            })
            .await
            .unwrap();
    }

    async fn wait_for_validation_result(
        rx: &mut mpsc::UnboundedReceiver<ThreadEvent>,
    ) -> (String, panes_events::ValidationOutcome) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(te)) => {
                    if let AgentEvent::ValidationResult {
                        validator, outcome, ..
                    } = &te.event
                    {
                        return (validator.clone(), *outcome);
                    }
                }
                _ => panic!("timed out waiting for ValidationResult event"),
            }
        }
    }

    #[tokio::test]
    async fn test_validator_feature_disabled_skips_entirely() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "See src/missing.rs for details.".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = validator_workspace(tmp.path().to_path_buf());
        insert_workspace_at(&mgr, &ws).await;
        // Feature is off by default — even with a configured validator, nothing should gate.
        add_citation_validator(&mgr, &ws.id).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let _tid = mgr.start_thread(&ws, "hi", "fake", ctx, None).await.unwrap();

        let events = collect_events_until_done(&mut rx).await;
        assert!(
            events
                .iter()
                .any(|te| matches!(&te.event, AgentEvent::Complete { .. })),
            "thread should complete without any validator activity"
        );
        assert!(
            !events
                .iter()
                .any(|te| matches!(&te.event, AgentEvent::ValidationResult { .. })),
            "no ValidationResult events should fire when feature is disabled"
        );
    }

    #[tokio::test]
    async fn test_validator_passes_on_real_path() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/real.rs"), "ok\n").unwrap();

        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "See src/real.rs for details.".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = validator_workspace(tmp.path().to_path_buf());
        insert_workspace_at(&mgr, &ws).await;
        enable_validators_feature(&mgr).await;
        add_citation_validator(&mgr, &ws.id).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let _tid = mgr.start_thread(&ws, "hi", "fake", ctx, None).await.unwrap();

        let (who, outcome) = wait_for_validation_result(&mut rx).await;
        assert_eq!(who, "citation");
        assert_eq!(outcome, panes_events::ValidationOutcome::Pass);

        let events = collect_events_until_done(&mut rx).await;
        assert!(
            events
                .iter()
                .any(|te| matches!(&te.event, AgentEvent::Complete { .. })),
            "thread should complete after passing validation"
        );
    }

    #[tokio::test]
    async fn test_validator_failure_creates_gate_and_approve_resumes() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "Bug is in src/missing.rs line 1.".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = validator_workspace(tmp.path().to_path_buf());
        insert_workspace_at(&mgr, &ws).await;
        enable_validators_feature(&mgr).await;
        add_citation_validator(&mgr, &ws.id).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid = mgr.start_thread(&ws, "hi", "fake", ctx, None).await.unwrap();

        let (_who, outcome) = wait_for_validation_result(&mut rx).await;
        assert_eq!(outcome, panes_events::ValidationOutcome::Fail);

        // DB status should flip to 'gate' while paused.
        wait_for_db_status(&mgr, &tid, "gate", 2000).await;

        // Approve via the existing gate API — tool_use_id is ignored for validator gates.
        mgr.approve(&tid, "").await.unwrap();

        let events = collect_events_until_done(&mut rx).await;
        assert!(
            events
                .iter()
                .any(|te| matches!(&te.event, AgentEvent::Complete { .. })),
            "thread should complete after validator-gate approval"
        );
    }

    #[tokio::test]
    async fn test_validator_failure_reject_interrupts() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "See src/gone.rs for the problem.".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = validator_workspace(tmp.path().to_path_buf());
        insert_workspace_at(&mgr, &ws).await;
        enable_validators_feature(&mgr).await;
        add_citation_validator(&mgr, &ws.id).await;

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let tid = mgr.start_thread(&ws, "hi", "fake", ctx, None).await.unwrap();

        let (_who, outcome) = wait_for_validation_result(&mut rx).await;
        assert_eq!(outcome, panes_events::ValidationOutcome::Fail);

        wait_for_db_status(&mgr, &tid, "gate", 2000).await;
        mgr.reject(&tid, "", "bad citation").await.unwrap();

        let events = collect_events_until_done(&mut rx).await;
        let had_error = events.iter().any(|te| {
            matches!(&te.event, AgentEvent::Error { message, .. } if message.contains("Validator"))
        });
        assert!(had_error, "expected a validator-rejected error event");

        wait_for_db_status(&mgr, &tid, "interrupted", 2000).await;
    }

    #[tokio::test]
    async fn test_disabled_validator_row_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let (mut mgr, mut rx) = setup_session_manager().await;
        let adapter = FakeAdapter::new(FakeScenario::TextOnly {
            response: "The bug is in src/missing.rs.".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ws = validator_workspace(tmp.path().to_path_buf());
        insert_workspace_at(&mgr, &ws).await;
        enable_validators_feature(&mgr).await;

        // Insert citation validator then disable it.
        let wid = ws.id.clone();
        mgr.db
            .execute(move |conn| {
                let v = crate::db::insert_validator(conn, &wid, "citation", "{}")?;
                crate::db::update_validator(conn, &v.id, Some(false), None)?;
                Ok(())
            })
            .await
            .unwrap();

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let _tid = mgr.start_thread(&ws, "hi", "fake", ctx, None).await.unwrap();

        let events = collect_events_until_done(&mut rx).await;
        assert!(
            !events
                .iter()
                .any(|te| matches!(&te.event, AgentEvent::ValidationResult { .. })),
            "disabled validators should not emit results"
        );
        assert!(
            events
                .iter()
                .any(|te| matches!(&te.event, AgentEvent::Complete { .. })),
            "thread should complete normally"
        );
    }

    // ---------------------------------------------------------------------
    // version_tracker integration — pre-edit recording in non-git workspaces
    // ---------------------------------------------------------------------

    async fn shadow_edit_count(mgr: &SessionManager, thread_id: &str) -> i64 {
        let tid = thread_id.to_string();
        mgr.db
            .execute(move |conn| {
                Ok(conn.query_row(
                    "SELECT COUNT(*) FROM shadow_edits WHERE thread_id = ?1",
                    rusqlite::params![tid],
                    |row| row.get::<_, i64>(0),
                )?)
            })
            .await
            .unwrap()
    }

    async fn thread_tracker_kind(mgr: &SessionManager, thread_id: &str) -> String {
        let tid = thread_id.to_string();
        mgr.db
            .execute(move |conn| {
                Ok(conn.query_row(
                    "SELECT tracker_kind FROM threads WHERE id = ?1",
                    rusqlite::params![tid],
                    |row| row.get::<_, String>(0),
                )?)
            })
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn thread_auto_inits_git_for_non_git_workspace() {
        // Previously non-git workspaces used a shadow tracker. Now they
        // get auto-initialized with git so all threads are git-tracked.
        let (mut mgr, mut rx) = setup_session_manager().await;

        let tmp = tempfile::tempdir().unwrap();
        let ws_path = tmp.path().to_path_buf();
        std::fs::write(ws_path.join("existing.txt"), b"original").unwrap();

        let ws = Workspace {
            id: "ws-nongit".to_string(),
            path: ws_path.clone(),
            name: "nongit".to_string(),
            default_agent: None,
            budget_cap: None,
        };
        insert_workspace_row(&mgr, &ws).await;

        let edited = ws_path.join("existing.txt").to_string_lossy().to_string();
        let created = ws_path.join("fresh.txt").to_string_lossy().to_string();
        let adapter = FakeAdapter::new(FakeScenario::FileEdit {
            files: vec![edited.clone(), created.clone()],
            response: "done".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ctx = SessionContext {
            briefing: None,
            memories: vec![],
            budget_cap: None,
        };
        let thread_id = mgr
            .start_thread(&ws, "edit", "fake", ctx, None)
            .await
            .unwrap();

        while let Some(te) = rx.recv().await {
            if matches!(te.event, AgentEvent::Complete { .. }) {
                break;
            }
        }

        assert_eq!(thread_tracker_kind(&mgr, &thread_id).await, "git");
        assert!(ws_path.join(".git").exists(), "git should have been auto-initialized");
    }

    #[tokio::test]
    async fn thread_in_git_workspace_skips_shadow_recording() {
        use tokio::process::Command;
        let (mut mgr, mut rx) = setup_session_manager().await;

        let tmp = tempfile::tempdir().unwrap();
        let ws_path = tmp.path().to_path_buf();
        // Minimal git repo so find_repo_root succeeds.
        Command::new("git")
            .args(["init", "-q"])
            .current_dir(&ws_path)
            .status()
            .await
            .unwrap();
        Command::new("git")
            .args(["-c", "user.email=t@t", "-c", "user.name=t", "commit", "--allow-empty", "-q", "-m", "init"])
            .current_dir(&ws_path)
            .status()
            .await
            .unwrap();
        std::fs::write(ws_path.join("existing.txt"), b"original").unwrap();

        let ws = Workspace {
            id: "ws-git".to_string(),
            path: ws_path.clone(),
            name: "git".to_string(),
            default_agent: None,
            budget_cap: None,
        };
        insert_workspace_row(&mgr, &ws).await;

        let edited = ws_path.join("existing.txt").to_string_lossy().to_string();
        let adapter = FakeAdapter::new(FakeScenario::FileEdit {
            files: vec![edited],
            response: "done".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        let ctx = SessionContext {
            briefing: None,
            memories: vec![],
            budget_cap: None,
        };
        let thread_id = mgr
            .start_thread(&ws, "edit", "fake", ctx, None)
            .await
            .unwrap();

        while let Some(te) = rx.recv().await {
            if matches!(te.event, AgentEvent::Complete { .. }) {
                break;
            }
        }

        assert_eq!(thread_tracker_kind(&mgr, &thread_id).await, "git");
        assert_eq!(
            shadow_edit_count(&mgr, &thread_id).await,
            0,
            "git-backed threads must not populate shadow_edits"
        );
    }

    #[tokio::test]
    async fn legacy_threads_default_to_git_tracker() {
        // Simulates a thread row that pre-dates the `tracker_kind`
        // migration: inserted without supplying the column so sqlite's
        // NOT NULL DEFAULT 'git' kicks in. Proves we continue to route
        // such threads through the git tracker (pre-existing snapshot
        // flow) rather than silently falling back to a shadow tracker
        // that has no recorded state for them.
        let (mgr, _rx) = setup_session_manager().await;
        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;
        mgr.db
            .execute(|conn| {
                conn.execute(
                    "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, created_at) \
                     VALUES ('legacy-t1', 'ws-test', 'claude-code', 'completed', 'p', '2024-01-01')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        // Column picks up the default.
        let kind: String = mgr
            .db
            .execute(|conn| {
                Ok(conn.query_row(
                    "SELECT tracker_kind FROM threads WHERE id = 'legacy-t1'",
                    [],
                    |row| row.get(0),
                )?)
            })
            .await
            .unwrap();
        assert_eq!(kind, "git");

        let tracker = mgr.tracker_for_thread("legacy-t1").await.unwrap();
        assert_eq!(tracker.kind(), TrackerKind::Git);
    }

    #[tokio::test]
    async fn tracker_for_thread_defaults_to_git_for_unknown_value() {
        // Defence-in-depth: if somehow a junk value gets into the column
        // (hand-edited DB, future migration bug), we still land on git
        // rather than panicking or returning the wrong tracker.
        let (mgr, _rx) = setup_session_manager().await;
        let (_tmp, ws) = make_workspace();
        insert_workspace_row(&mgr, &ws).await;
        mgr.db
            .execute(|conn| {
                conn.execute(
                    "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, tracker_kind, created_at) \
                     VALUES ('junk-t1', 'ws-test', 'claude-code', 'completed', 'p', 'gibberish', '2024-01-01')",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        let tracker = mgr.tracker_for_thread("junk-t1").await.unwrap();
        assert_eq!(tracker.kind(), TrackerKind::Git);
    }

    #[tokio::test]
    async fn tracker_for_thread_errors_for_unknown_thread() {
        // Unknown thread id is a caller bug — we return an error rather
        // than silently picking a tracker. The IPC layer surfaces this
        // to the UI.
        let (mgr, _rx) = setup_session_manager().await;
        let err = mgr.tracker_for_thread("does-not-exist").await;
        assert!(err.is_err(), "expected error for unknown thread");
    }

    #[tokio::test]
    async fn resumed_legacy_shadow_thread_continues_shadow_recording() {
        // Legacy threads (created before auto-init) have tracker_kind =
        // 'shadow'. Resuming them must still use the shadow tracker and
        // record pre-edits. We simulate this by inserting the thread row
        // directly with tracker_kind = 'shadow' and a fake session_id,
        // then resuming.
        let (mut mgr, mut rx) = setup_session_manager().await;

        let tmp = tempfile::tempdir().unwrap();
        let ws_path = tmp.path().to_path_buf();
        let ws = Workspace {
            id: "ws-resume".to_string(),
            path: ws_path.clone(),
            name: "resume".to_string(),
            default_agent: None,
            budget_cap: None,
        };
        insert_workspace_row(&mgr, &ws).await;

        let thread_id = "legacy-shadow-t1".to_string();
        let session_id = "fake-session-1".to_string();

        // Insert a legacy shadow thread row directly.
        {
            let tid = thread_id.clone();
            let sid = session_id.clone();
            mgr.db
                .execute(move |conn| {
                    conn.execute(
                        "INSERT INTO threads (id, workspace_id, agent_type, status, prompt, session_id, tracker_kind, created_at) \
                         VALUES (?1, 'ws-resume', 'fake', 'completed', 'legacy', ?2, 'shadow', '2024-01-01')",
                        rusqlite::params![tid, sid],
                    )?;
                    Ok(())
                })
                .await
                .unwrap();
        }

        // Pre-seed the session_id mapping so resume_thread can find it.
        {
            let mut sids = mgr.session_ids.lock().await;
            sids.insert(thread_id.clone(), session_id.clone());
        }

        let resume_files = vec![
            ws_path.join("resumed.txt").to_string_lossy().to_string(),
        ];
        let adapter = FakeAdapter::new(FakeScenario::FileEdit {
            files: resume_files.clone(),
            response: "resumed".to_string(),
        })
        .with_delay(0);
        mgr.register_adapter(Arc::new(adapter));

        mgr.resume_thread(&thread_id, &ws, "edit resumed", "fake", None)
            .await
            .unwrap();
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        assert_eq!(
            shadow_edit_count(&mgr, &thread_id).await,
            1,
            "resumed shadow thread should record pre-edit"
        );

        let tracker = mgr.tracker_for_thread(&thread_id).await.unwrap();
        assert_eq!(tracker.kind(), TrackerKind::Shadow);
    }

    #[tokio::test]
    async fn rejected_gated_write_does_not_appear_as_changed_file() {
        // A rejected gated write never touches disk, so the file must not
        // appear in the changed-files list. With git-backed worktrees
        // (the default since auto-init), rejected writes are isolated in
        // the worktree and discarded on cleanup.
        let (mut mgr, mut rx) = setup_session_manager().await;

        let tmp = tempfile::tempdir().unwrap();
        let ws_path = tmp.path().to_path_buf();
        let target_file = ws_path.join("doomed.txt");

        let ws = Workspace {
            id: "ws-gated".to_string(),
            path: ws_path.clone(),
            name: "gated".to_string(),
            default_agent: None,
            budget_cap: None,
        };
        insert_workspace_row(&mgr, &ws).await;

        mgr.register_adapter(Arc::new(gate_test_adapter::GatedWriteAdapter {
            file_path: target_file.to_string_lossy().to_string(),
        }));

        let ctx = SessionContext { briefing: None, memories: vec![], budget_cap: None };
        let thread_id = mgr
            .start_thread(&ws, "write dangerous", "gated-write-test", ctx, None)
            .await
            .unwrap();

        // Wait for the gate event, then reject it.
        let tool_id = wait_for_gate_event(&mut rx).await;
        mgr.reject(&thread_id, &tool_id, "not today").await.unwrap();
        let _ = collect_events_until_done(&mut rx).await;
        wait_for_thread_cleanup(&mgr, &thread_id, 2000).await;

        // File was never actually written (we rejected).
        assert!(!target_file.exists(), "rejected write should not touch disk");

        // Git tracker: rejected write never lands in the worktree's
        // committed state, so changed_files is empty.
        let tracker = mgr.tracker_for_thread(&thread_id).await.unwrap();
        let changed = tracker.list_changed_files(&thread_id, &ws_path).await.unwrap();
        assert!(
            changed.is_empty(),
            "rejected gated write must not appear in list_changed_files, got {changed:?}"
        );
    }
}
