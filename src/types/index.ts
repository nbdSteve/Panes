export type { AgentEvent, ThinkingEvent, TextEvent, ToolRequestEvent, ToolResultEvent, CostUpdateEvent, CompleteEvent, ErrorEvent, SubAgentSpawnedEvent, SubAgentCompleteEvent, FollowUpEvent, ValidationResultEvent, ContextUsageEvent, ValidationFinding, ValidationOutcome, FindingSeverity } from "./events";
import type { AgentEvent } from "./events";
export type { PanesError, PanesErrorType } from "./errors";
export { parsePanesError, isWorkspaceOccupied, isNoGatePending, isValidationError } from "./errors";
export type { CommentThread, CommentSide, LineSelection, FileGitStatus, RepoFileStatus, RepoCommitParams, ParsedDiff, DiffFile, DiffHunk, DiffLine, DiffLineType, FileStatus } from "./diff";

export interface WorkspaceInfo {
  id: string;
  path: string;
  name: string;
  defaultAgent?: string;
  budgetCap?: number | null;
}

export interface AgentInfo {
  name: string;
  model: string | null;
  description: string | null;
}

export interface ModelInfo {
  id: string;
  label: string;
  description: string;
}

export interface ThreadInfo {
  id: string;
  workspaceId: string;
  prompt: string;
  status: "starting" | "running" | "gate" | "complete" | "error" | "interrupted";
  costUsd?: number;
  completionActions?: Record<number, "committed" | "reverted" | "kept">;
  diffComments?: Record<number, import("./diff").CommentThread[]>;
  feedbackSent?: Record<number, number>;
  activeDiffView?: { completionIdx: number; activeFile?: string };
  queuedFollowUp?: string;
  events: AgentEvent[];
  memoryCount?: number;
  hasBriefing?: boolean;
  isRoutine?: boolean;
  routineId?: string;
  /**
   * Which version tracker the backend is using for this thread's file edits.
   * "git" for git-backed workspaces, "shadow" for Panes-managed snapshots in
   * non-git workspaces. Drives whether the UI shows git-only actions like
   * commit vs. revert-only flows.
   */
  trackerKind?: "git" | "shadow";
  createdAt: number;
}

export interface ConfigPrefs {
  adapter: string;
  agent: string;
  model: string;
}

export interface FeatureInfo {
  id: string;
  enabled: boolean;
  label: string;
  description: string;
}

export type ScheduleAction =
  | { action: "notify" }
  | { action: "retry_once" }
  | { action: "chain"; prompt: string; workspace_id?: string | null };

export interface RoutineInfo {
  id: string;
  workspaceId: string;
  prompt: string;
  cronExpr: string;
  budgetCap: number | null;
  onComplete: ScheduleAction;
  onFailure: ScheduleAction;
  enabled: boolean;
  lastRunAt: string | null;
  createdAt: string;
}

export interface RoutineExecution {
  id: string;
  routineId: string;
  threadId: string | null;
  status: string;
  costUsd: number;
  startedAt: string;
  completedAt: string | null;
  errorMessage: string | null;
}

export interface WorkspaceValidator {
  id: string;
  workspaceId: string;
  validatorType: string;
  enabled: boolean;
  configJson: string;
  createdAt: string;
  updatedAt: string;
}

export interface ValidatorTypeInfo {
  typeId: string;
  label: string;
  description: string;
  defaultConfig: unknown;
  correctable: boolean;
}
