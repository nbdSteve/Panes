export type { AgentEvent, ThinkingEvent, TextEvent, ToolRequestEvent, ToolResultEvent, CostUpdateEvent, CompleteEvent, ErrorEvent, SubAgentSpawnedEvent, SubAgentCompleteEvent, FollowUpEvent } from "./events";
import type { AgentEvent } from "./events";
export type { PanesError, PanesErrorType } from "./errors";
export { parsePanesError, isWorkspaceOccupied, isNoGatePending, isValidationError } from "./errors";

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
  completionAction?: "committed" | "reverted" | "kept";
  queuedFollowUp?: string;
  events: AgentEvent[];
  memoryCount?: number;
  hasBriefing?: boolean;
  isRoutine?: boolean;
  routineId?: string;
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
