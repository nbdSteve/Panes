import type { AgentEvent } from "../App";

export function mapBackendEvent(
  raw: Record<string, unknown>
): AgentEvent | null {
  const eventType = raw.event_type as string | undefined;
  if (!eventType) return null;

  switch (eventType) {
    case "thinking":
      return { event_type: "thinking", text: raw.text as string };
    case "text":
      return { event_type: "text", text: raw.text as string };
    case "tool_request":
      return {
        event_type: "tool_request",
        id: raw.id as string,
        tool_name: raw.tool_name as string,
        description: raw.description as string,
        risk_level: raw.risk_level as string,
        needs_approval: raw.needs_approval as boolean,
      };
    case "tool_result":
      return {
        event_type: "tool_result",
        id: raw.id as string,
        success: raw.success as boolean,
        output: raw.output as string,
        duration_ms: raw.duration_ms as number | undefined,
      };
    case "cost_update":
      return { event_type: "cost_update", total_usd: raw.total_usd as number };
    case "complete":
      return {
        event_type: "complete",
        summary: raw.summary as string,
        total_cost_usd: raw.total_cost_usd as number,
        duration_ms: raw.duration_ms as number,
        turns: raw.turns as number,
      };
    case "error":
      return { event_type: "error", message: raw.message as string };
    default:
      return null;
  }
}
