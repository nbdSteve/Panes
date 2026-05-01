import { describe, it, expect } from "vitest";
import { groupToolEvents, type ToolGroup } from "./groupToolEvents";
import type { AgentEvent } from "../App";

describe("groupToolEvents", () => {
  it("returns empty array for empty input", () => {
    expect(groupToolEvents([])).toEqual([]);
  });

  it("passes standalone events through", () => {
    const events: AgentEvent[] = [
      { event_type: "thinking", text: "hmm" },
      { event_type: "text", text: "hello" },
      { event_type: "error", message: "oops" },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(3);
    expect(result.every((r) => r.type === "standalone")).toBe(true);
  });

  it("pairs tool_request with matching tool_result", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "t1", tool_name: "Bash", description: "ls", risk_level: "low", needs_approval: false },
      { event_type: "tool_result", id: "t1", success: true, output: "file.txt", duration_ms: 100 },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(1);
    expect(result[0].type).toBe("tool_group");
    const group = result[0] as ToolGroup;
    expect(group.request.tool_name).toBe("Bash");
    expect(group.result?.success).toBe(true);
    expect(group.result?.duration_ms).toBe(100);
  });

  it("handles unmatched tool_request (in-progress)", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "t1", tool_name: "Read", description: "reading", risk_level: "low", needs_approval: false },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(1);
    const group = result[0] as ToolGroup;
    expect(group.type).toBe("tool_group");
    expect(group.result).toBeNull();
  });

  it("handles multiple tool pairs interleaved with standalone events", () => {
    const events: AgentEvent[] = [
      { event_type: "thinking", text: "planning" },
      { event_type: "tool_request", id: "t1", tool_name: "Read", description: "read main.rs", risk_level: "low", needs_approval: false },
      { event_type: "tool_result", id: "t1", success: true, output: "fn main()", duration_ms: 50 },
      { event_type: "text", text: "I see" },
      { event_type: "tool_request", id: "t2", tool_name: "Edit", description: "edit main.rs", risk_level: "medium", needs_approval: false },
      { event_type: "tool_result", id: "t2", success: true, output: "edited", duration_ms: 200 },
      { event_type: "complete", summary: "done", total_cost_usd: 0.01, duration_ms: 1000, turns: 1 },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(5);
    expect(result[0].type).toBe("standalone"); // thinking
    expect(result[1].type).toBe("tool_group"); // Read + result
    expect(result[2].type).toBe("standalone"); // text
    expect(result[3].type).toBe("tool_group"); // Edit + result
    expect(result[4].type).toBe("standalone"); // complete
  });

  it("does not group gated tool_requests (needs_approval=true)", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "t1", tool_name: "Bash", description: "rm -rf", risk_level: "critical", needs_approval: true },
      { event_type: "tool_result", id: "t1", success: true, output: "done", duration_ms: 10 },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(2);
    expect(result[0].type).toBe("standalone");
    expect(result[1].type).toBe("standalone");
  });

  it("handles follow_up and sub_agent events as standalone", () => {
    const events: AgentEvent[] = [
      { event_type: "follow_up", text: "next question" },
      { event_type: "sub_agent_spawned", parent_tool_use_id: "t1", description: "research" },
      { event_type: "sub_agent_complete", parent_tool_use_id: "t1", summary: "found it", cost_usd: 0.02 },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(3);
    expect(result.every((r) => r.type === "standalone")).toBe(true);
  });
});
