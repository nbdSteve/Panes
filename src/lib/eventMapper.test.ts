import { describe, it, expect } from "vitest";
import { mapBackendEvent } from "./eventMapper";

describe("mapBackendEvent", () => {
  it("returns null for missing event_type", () => {
    expect(mapBackendEvent({})).toBeNull();
    expect(mapBackendEvent({ text: "hello" })).toBeNull();
  });

  it("maps thinking event", () => {
    const result = mapBackendEvent({ event_type: "thinking", text: "Let me think..." });
    expect(result).toEqual({ event_type: "thinking", text: "Let me think..." });
  });

  it("maps text event", () => {
    const result = mapBackendEvent({ event_type: "text", text: "Hello!" });
    expect(result).toEqual({ event_type: "text", text: "Hello!" });
  });

  it("maps tool_request event", () => {
    const result = mapBackendEvent({
      event_type: "tool_request",
      id: "t1",
      tool_name: "Bash",
      description: "Run ls",
      risk_level: "low",
      needs_approval: false,
    });
    expect(result).toEqual({
      event_type: "tool_request",
      id: "t1",
      tool_name: "Bash",
      description: "Run ls",
      risk_level: "low",
      needs_approval: false,
    });
  });

  it("maps tool_result event", () => {
    const result = mapBackendEvent({
      event_type: "tool_result",
      id: "t1",
      success: true,
      output: "file.txt",
      duration_ms: 150,
    });
    expect(result).toEqual({
      event_type: "tool_result",
      id: "t1",
      success: true,
      output: "file.txt",
      duration_ms: 150,
    });
  });

  it("maps tool_result without duration_ms", () => {
    const result = mapBackendEvent({
      event_type: "tool_result",
      id: "t1",
      success: true,
      output: "done",
    });
    expect(result?.duration_ms).toBeUndefined();
  });

  it("maps cost_update event", () => {
    const result = mapBackendEvent({ event_type: "cost_update", total_usd: 0.05 });
    expect(result).toEqual({ event_type: "cost_update", total_usd: 0.05 });
  });

  it("maps complete event", () => {
    const result = mapBackendEvent({
      event_type: "complete",
      summary: "Task done",
      total_cost_usd: 0.12,
      duration_ms: 5000,
      turns: 3,
    });
    expect(result).toEqual({
      event_type: "complete",
      summary: "Task done",
      total_cost_usd: 0.12,
      duration_ms: 5000,
      turns: 3,
    });
  });

  it("maps error event", () => {
    const result = mapBackendEvent({ event_type: "error", message: "Auth failed" });
    expect(result).toEqual({ event_type: "error", message: "Auth failed" });
  });

  it("returns null for unknown event type", () => {
    expect(mapBackendEvent({ event_type: "unknown_type" })).toBeNull();
    expect(mapBackendEvent({ event_type: "sub_agent_spawned" })).toBeNull();
  });
});
