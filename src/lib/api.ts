import { invoke } from "@tauri-apps/api/core";
import { parsePanesError, type PanesError } from "../types/errors";

export interface StartThreadParams {
  workspaceId: string;
  workspacePath: string;
  workspaceName: string;
  prompt: string;
  agent?: string;
  model?: string;
}

export interface StartThreadResult {
  threadId: string;
  memoryCount: number;
  hasBriefing: boolean;
}

export interface ResumeThreadParams {
  threadId: string;
  workspaceId: string;
  workspacePath: string;
  workspaceName: string;
  prompt: string;
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
  defaultAgent?: string;
  budgetCap?: number | null;
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
  listWorkspaces: () => call<WorkspaceInfo[]>("list_workspaces"),
  addWorkspace: (path: string, name: string) =>
    call<WorkspaceInfo>("add_workspace", { path, name }),
  removeWorkspace: (workspaceId: string) =>
    call<void>("remove_workspace", { workspaceId }),
  setWorkspaceDefaultAgent: (workspaceId: string, agent: string) =>
    call<void>("set_workspace_default_agent", { workspaceId, agent }),
  setWorkspaceBudgetCap: (workspaceId: string, budgetCap: number | null) =>
    call<void>("set_workspace_budget_cap", { workspaceId, budgetCap }),

  // Threads
  startThread: (params: StartThreadParams) =>
    call<StartThreadResult>("start_thread", params as unknown as Record<string, unknown>),
  resumeThread: (params: ResumeThreadParams) =>
    call<void>("resume_thread", params as unknown as Record<string, unknown>),
  cancelThread: (threadId: string) =>
    call<void>("cancel_thread", { threadId }),
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
  commitChanges: (workspacePath: string, message: string) =>
    call<string>("commit_changes", { workspacePath, message }),
  revertChanges: (workspacePath: string, threadId: string) =>
    call<void>("revert_changes", { workspacePath, threadId }),
  getChangedFiles: (workspacePath: string) =>
    call<string[]>("get_changed_files", { workspacePath }),

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

  // Memory backend
  getMemoryBackendStatus: () => call<MemoryBackendStatus>("get_memory_backend_status"),
  setMemoryBackend: (backend: string) =>
    call<void>("set_memory_backend", { backend }),
};

export type { PanesError };
