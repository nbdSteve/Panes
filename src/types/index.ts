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
  createdAt: number;
}

export interface ConfigPrefs {
  adapter: string;
  agent: string;
  model: string;
}
