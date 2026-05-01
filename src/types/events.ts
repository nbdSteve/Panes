export type AgentEvent =
  | ThinkingEvent
  | TextEvent
  | ToolRequestEvent
  | ToolResultEvent
  | CostUpdateEvent
  | CompleteEvent
  | ErrorEvent
  | SubAgentSpawnedEvent
  | SubAgentCompleteEvent
  | FollowUpEvent;

export interface ThinkingEvent {
  event_type: "thinking";
  text: string;
}

export interface TextEvent {
  event_type: "text";
  text: string;
}

export interface ToolRequestEvent {
  event_type: "tool_request";
  id: string;
  tool_name: string;
  description: string;
  risk_level: string;
  needs_approval: boolean;
  input?: Record<string, unknown>;
}

export interface ToolResultEvent {
  event_type: "tool_result";
  id: string;
  tool_name?: string;
  success: boolean;
  output: string;
  raw_output?: string;
  duration_ms?: number;
}

export interface CostUpdateEvent {
  event_type: "cost_update";
  total_usd: number;
  input_tokens?: number;
  output_tokens?: number;
  cache_read_tokens?: number;
  cache_creation_tokens?: number;
  model?: string;
}

export interface CompleteEvent {
  event_type: "complete";
  summary: string;
  total_cost_usd: number;
  duration_ms: number;
  turns: number;
}

export interface ErrorEvent {
  event_type: "error";
  message: string;
  recoverable?: boolean;
}

export interface SubAgentSpawnedEvent {
  event_type: "sub_agent_spawned";
  parent_tool_use_id: string;
  description: string;
}

export interface SubAgentCompleteEvent {
  event_type: "sub_agent_complete";
  parent_tool_use_id: string;
  summary: string;
  cost_usd: number;
}

export interface FollowUpEvent {
  event_type: "follow_up";
  text: string;
}
