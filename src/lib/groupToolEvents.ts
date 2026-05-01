import type { AgentEvent } from "../App";

export interface ToolGroup {
  type: "tool_group";
  request: AgentEvent;
  result: AgentEvent | null;
}

export type RenderItem =
  | { type: "standalone"; event: AgentEvent }
  | ToolGroup;

export function groupToolEvents(events: AgentEvent[]): RenderItem[] {
  const resultById = new Map<string, AgentEvent>();
  for (const e of events) {
    if (e.event_type === "tool_result" && e.id) {
      resultById.set(e.id, e);
    }
  }

  const items: RenderItem[] = [];
  const consumedResults = new Set<string>();

  for (const event of events) {
    if (event.event_type === "tool_result" && event.id && consumedResults.has(event.id)) {
      continue;
    }

    if (event.event_type === "tool_request" && !event.needs_approval) {
      const result = event.id ? resultById.get(event.id) ?? null : null;
      if (result && event.id) consumedResults.add(event.id);
      items.push({ type: "tool_group", request: event, result });
    } else if (event.event_type === "tool_result") {
      items.push({ type: "standalone", event });
    } else {
      items.push({ type: "standalone", event });
    }
  }

  return items;
}
