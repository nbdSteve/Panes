import type { AgentEvent, ToolRequestEvent, ToolResultEvent, SubAgentSpawnedEvent, SubAgentCompleteEvent } from "../types";

export interface ToolGroup {
  type: "tool_group";
  request: ToolRequestEvent;
  result: ToolResultEvent | null;
  subAgentSpawned: SubAgentSpawnedEvent | null;
  subAgentComplete: SubAgentCompleteEvent | null;
}

export type RenderItem =
  | { type: "standalone"; event: AgentEvent }
  | ToolGroup;

export function groupToolEvents(events: AgentEvent[]): RenderItem[] {
  const resultById = new Map<string, ToolResultEvent>();
  const spawnedByParent = new Map<string, SubAgentSpawnedEvent>();
  const completeByParent = new Map<string, SubAgentCompleteEvent>();
  for (const e of events) {
    if (e.event_type === "tool_result" && e.id) {
      resultById.set(e.id, e);
    }
    if (e.event_type === "sub_agent_spawned" && e.parent_tool_use_id) {
      spawnedByParent.set(e.parent_tool_use_id, e);
    }
    if (e.event_type === "sub_agent_complete" && e.parent_tool_use_id) {
      completeByParent.set(e.parent_tool_use_id, e);
    }
  }

  const items: RenderItem[] = [];
  const consumedResults = new Set<string>();
  const consumedSubAgents = new Set<AgentEvent>();

  for (const event of events) {
    if (event.event_type === "tool_result" && event.id && consumedResults.has(event.id)) {
      continue;
    }
    if (consumedSubAgents.has(event)) {
      continue;
    }

    if (event.event_type === "tool_request" && !event.needs_approval) {
      const result = event.id ? resultById.get(event.id) ?? null : null;
      if (result && event.id) consumedResults.add(event.id);
      const spawned = event.id ? spawnedByParent.get(event.id) ?? null : null;
      const complete = event.id ? completeByParent.get(event.id) ?? null : null;
      if (spawned) consumedSubAgents.add(spawned);
      if (complete) consumedSubAgents.add(complete);
      items.push({ type: "tool_group", request: event, result, subAgentSpawned: spawned, subAgentComplete: complete });
    } else if (event.event_type === "tool_result") {
      items.push({ type: "standalone", event });
    } else {
      items.push({ type: "standalone", event });
    }
  }

  return items;
}
