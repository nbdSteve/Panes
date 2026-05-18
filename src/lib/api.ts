import { invoke } from "@tauri-apps/api/core";
import { parsePanesError, type PanesError } from "../types/errors";
import type { FeatureInfo, RoutineInfo, RoutineExecution, RepoFileStatus, RepoCommitParams, WorkspaceValidator, ValidatorTypeInfo } from "../types";

export interface StartThreadParams {
  workspaceId: string;
  workspacePath: string;
  workspaceName: string;
  prompt: string;
  /**
   * Adapter name — `"claude-code"`, `"kiro-cli"`, etc. Selects which
   * `AgentAdapter` implementation spawns the backend. If omitted, the
   * workspace's default adapter is used.
   */
  adapter?: string;
  /**
   * Per-adapter sub-agent or mode name (e.g. `"codebase-analyzer"` for
   * claude-code, a kiro-cli mode id for kiro-cli). Passed to the adapter via its
   * `--agent`/`set_mode` mechanism. Independent of `adapter`.
   */
  agent?: string;
  model?: string;
}

export interface StartThreadResult {
  threadId: string;
  injectedMemories: MemoryInfo[];
  briefingPreview: string | null;
  /**
   * "isolated" when the backend created a per-thread git worktree for
   * the freshly-started thread. Lets the frontend show Merge/Discard
   * UI on completion without a `listThreads` refresh. Absent for
   * shadow-tracked threads and for git threads where worktree creation
   * failed (backend falls back to the main checkout).
   */
  worktreeStatus?: "isolated" | "main";
  /**
   * Absolute path the backend actually ran the agent in. For worktree
   * threads this is the isolated checkout; otherwise the workspace
   * path. Frontend uses this to resolve relative tool-use file paths
   * before asking the backend for git status / diffs.
   */
  effectivePath?: string;
}

export interface ResumeThreadParams {
  threadId: string;
  workspaceId: string;
  workspacePath: string;
  workspaceName: string;
  prompt: string;
  adapter?: string;
  agent?: string;
  model?: string;
}

export interface MemoryInfo {
  id: string;
  workspaceId: string | null;
  memoryType: string;
  content: string;
  sourceThreadId: string;
  pinned: boolean;
  createdAt: string;
}

export interface BriefingInfo {
  workspaceId: string;
  content: string;
}

export interface WorkspaceInfo {
  id: string;
  path: string;
  name: string;
  /**
   * Adapter name (claude-code, kiro-cli, ...). The backend serializes this
   * under `defaultAgent` / `default_agent` for historical reasons; we
   * translate at the IPC boundary in listWorkspaces / addWorkspace so the
   * rest of the frontend can use the accurate name.
   */
  defaultAdapter?: string;
  budgetCap?: number | null;
}

// What the backend actually sends over IPC. Translated into WorkspaceInfo
// (renaming defaultAgent → defaultAdapter) so no code past this boundary
// has to deal with the misleading "agent" wording.
interface WorkspaceInfoWire {
  id: string;
  path: string;
  name: string;
  defaultAgent?: string;
  budgetCap?: number | null;
}

function decodeWorkspace(w: WorkspaceInfoWire): WorkspaceInfo {
  const { defaultAgent, ...rest } = w;
  return { ...rest, defaultAdapter: defaultAgent };
}

export interface MergeResult {
  /**
   * Successful: "merged" (two-parent commit) or "fast_forwarded" (HEAD
   * advanced). "up_to_date" means the worktree had no new commits and
   * was cleaned up anyway. "conflicts" means the main repo is untouched
   * — user must resolve manually or pick Discard.
   */
  outcome: "merged" | "fast_forwarded" | "up_to_date" | "conflicts";
  /** Resulting HEAD commit for merged/fast_forwarded. Null on conflicts. */
  commit: string | null;
  /** Conflicting file paths when outcome === "conflicts". */
  files: string[];
}

export interface MemoryBackendStatus {
  backend: string;
  mem0Configured: boolean;
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (err) {
    throw parsePanesError(err);
  }
}

export const api = {
  // Workspaces
  listWorkspaces: async (): Promise<WorkspaceInfo[]> =>
    (await call<WorkspaceInfoWire[]>("list_workspaces")).map(decodeWorkspace),
  addWorkspace: async (path: string, name: string): Promise<WorkspaceInfo> =>
    decodeWorkspace(await call<WorkspaceInfoWire>("add_workspace", { path, name })),
  removeWorkspace: (workspaceId: string) =>
    call<void>("remove_workspace", { workspaceId }),
  // Backend IPC command kept as-is (`set_workspace_default_agent`) — same
  // reason as the column name. The frontend-facing method reflects that
  // this sets the default *adapter*, not an agent.
  setWorkspaceDefaultAdapter: (workspaceId: string, adapter: string) =>
    call<void>("set_workspace_default_agent", { workspaceId, agent: adapter }),
  setWorkspaceBudgetCap: (workspaceId: string, budgetCap: number | null) =>
    call<void>("set_workspace_budget_cap", { workspaceId, budgetCap }),

  // Threads
  startThread: (params: StartThreadParams) =>
    call<StartThreadResult>("start_thread", params as unknown as Record<string, unknown>),
  resumeThread: (params: ResumeThreadParams) =>
    call<void>("resume_thread", params as unknown as Record<string, unknown>),
  cancelThread: (threadId: string) =>
    call<void>("cancel_thread", { threadId }),
  /**
   * Switch the active model on a running thread. Errors if the thread's
   * adapter can't change models mid-session (Claude stream-json); ACP
   * adapters accept it via session/set_model.
   */
  setThreadModel: (threadId: string, model: string) =>
    call<void>("set_thread_model", { threadId, model }),
  deleteThread: (threadId: string) =>
    call<void>("delete_thread", { threadId }),
  listThreads: (workspaceId: string) =>
    call<unknown[]>("list_threads", { workspaceId }),
  listAllThreads: (limit?: number) =>
    call<unknown[]>("list_all_threads", { limit }),

  // Gates
  approveGate: (threadId: string, toolUseId: string) =>
    call<void>("approve_gate", { threadId, toolUseId }),
  rejectGate: (threadId: string, toolUseId: string, reason: string) =>
    call<void>("reject_gate", { threadId, toolUseId, reason }),

  // Git
  commitChanges: (workspacePath: string, message: string, files?: string[]) =>
    call<string>("commit_changes", { workspacePath, message, files: files ?? null }),
  revertChanges: (workspacePath: string, threadId: string) =>
    call<void>("revert_changes", { workspacePath, threadId }),
  /**
   * Merge a thread's worktree branch back into the main repo's HEAD.
   * Phase 2 only — requires the thread to have a persisted worktree.
   * Backend returns the outcome so the UI can distinguish successful
   * merges from conflicts (where the main repo is untouched and the
   * user needs to pick a resolution strategy or Discard).
   *
   * `strategy` drives the Option A conflict flow:
   * - undefined / "auto": standard merge; returns outcome=conflicts
   *   if the branches collide so the UI can prompt the user.
   * - "prefer_theirs": keep the worktree version of any conflicted
   *   file (user clicked "Use yours").
   * - "prefer_ours": keep main's version of any conflicted file
   *   (user clicked "Keep main").
   *
   * Per-file resolution + three-way diff is Option B, documented as a
   * planned follow-up.
   */
  mergeToMain: (
    threadId: string,
    message?: string,
    strategy?: "auto" | "prefer_ours" | "prefer_theirs",
  ) =>
    call<MergeResult>("merge_to_main", {
      threadId,
      message: message ?? null,
      strategy: strategy ?? null,
    }),
  worktreeHasCommits: (threadId: string) =>
    call<boolean>("worktree_has_commits", { threadId }),
  getChangedFiles: (workspacePath: string, threadId?: string) =>
    call<string[]>("get_changed_files", { workspacePath, threadId: threadId ?? null }),
  getFileDiff: (workspacePath: string, filePath: string, threadId?: string) =>
    call<string>("get_file_diff", { workspacePath, filePath, threadId: threadId ?? null }),
  getWorkspaceDiff: (workspacePath: string, files?: string[], threadId?: string) =>
    call<string>("get_workspace_diff", {
      workspacePath,
      files: files ?? null,
      threadId: threadId ?? null,
    }),
  getFilesGitStatus: (filePaths: string[]) =>
    call<RepoFileStatus[]>("get_files_git_status", { filePaths }),
  listGitRepos: (workspacePath: string) =>
    call<string[]>("list_git_repos", { workspacePath }),
  commitRepos: (commits: RepoCommitParams[]) =>
    call<string[]>("commit_repos", { commits }),
  generateCommitMessage: (workspacePath: string, diff: string) =>
    call<string>("generate_commit_message", { workspacePath, diff }),

  // Memory
  extractMemories: (workspaceId: string, threadId: string, transcript: string) =>
    call<MemoryInfo[]>("extract_memories", { workspaceId, threadId, transcript }),
  getMemories: (workspaceId: string) =>
    call<MemoryInfo[]>("get_memories", { workspaceId }),
  searchMemories: (workspaceId: string, query: string, limit?: number) =>
    call<MemoryInfo[]>("search_memories", { workspaceId, query, limit }),
  updateMemory: (memoryId: string, content: string) =>
    call<void>("update_memory", { memoryId, content }),
  deleteMemory: (memoryId: string) =>
    call<void>("delete_memory", { memoryId }),
  pinMemory: (memoryId: string, pinned: boolean) =>
    call<void>("pin_memory", { memoryId, pinned }),

  // Briefings
  getBriefing: (workspaceId: string) =>
    call<BriefingInfo | null>("get_briefing", { workspaceId }),
  setBriefing: (workspaceId: string, content: string) =>
    call<void>("set_briefing", { workspaceId, content }),
  deleteBriefing: (workspaceId: string) =>
    call<void>("delete_briefing", { workspaceId }),

  // Config
  listAdapters: () => call<string[]>("list_adapters"),
  listAgents: (adapter: string) => call<unknown[]>("list_agents", { adapter }),
  listModels: (adapter: string) => call<unknown[]>("list_models", { adapter }),

  // Cost
  getAggregateCost: () => call<number>("get_aggregate_cost"),
  getWorkspaceCost: (workspaceId: string) =>
    call<number>("get_workspace_cost", { workspaceId }),
  getCostTimeline: (days?: number, workspaceId?: string) =>
    call<{ day: string; totalUsd: number }[]>("get_cost_timeline", { days, workspaceId }),
  getWorkspaceCostBreakdown: () =>
    call<{ workspaceId: string; workspaceName: string; totalUsd: number; threadCount: number }[]>("get_workspace_cost_breakdown"),

  // Memory backend
  getMemoryBackendStatus: () => call<MemoryBackendStatus>("get_memory_backend_status"),
  setMemoryBackend: (backend: string) =>
    call<void>("set_memory_backend", { backend }),

  // Features
  getFeatures: () => call<FeatureInfo[]>("get_features"),
  setFeatureEnabled: (featureId: string, enabled: boolean) =>
    call<void>("set_feature_enabled", { featureId, enabled }),

  // Routines
  createRoutine: (params: {
    workspaceId: string;
    prompt: string;
    cronExpr: string;
    budgetCap?: number | null;
    onComplete?: string;
    onFailure?: string;
  }) => call<RoutineInfo>("create_routine", params as unknown as Record<string, unknown>),
  updateRoutine: (params: {
    routineId: string;
    prompt?: string;
    cronExpr?: string;
    budgetCap?: number;
    onComplete?: string;
    onFailure?: string;
  }) => call<void>("update_routine", params as unknown as Record<string, unknown>),
  deleteRoutine: (routineId: string) =>
    call<void>("delete_routine", { routineId }),
  listRoutines: (workspaceId?: string) =>
    call<RoutineInfo[]>("list_routines", { workspaceId }),
  toggleRoutine: (routineId: string, enabled: boolean) =>
    call<void>("toggle_routine", { routineId, enabled }),
  listRoutineExecutions: (routineId: string, limit?: number) =>
    call<RoutineExecution[]>("list_routine_executions", { routineId, limit }),
  getRoutineCost: (routineId: string) =>
    call<number>("get_routine_cost", { routineId }),

  // Output validators
  listValidatorTypes: () =>
    call<ValidatorTypeInfo[]>("list_validator_types"),
  listValidators: (workspaceId: string) =>
    call<WorkspaceValidator[]>("list_validators", { workspaceId }),
  addValidator: (params: {
    workspaceId: string;
    validatorType: string;
    configJson: string;
  }) => call<WorkspaceValidator>("add_validator", params as unknown as Record<string, unknown>),
  updateValidator: (params: {
    id: string;
    enabled?: boolean;
    configJson?: string;
  }) => call<WorkspaceValidator>("update_validator", params as unknown as Record<string, unknown>),
  removeValidator: (id: string) =>
    call<void>("remove_validator", { id }),
};

export type { PanesError };
