import { describe, it, expect } from "vitest";
import { groupToolEvents, type ToolGroup } from "./groupToolEvents";
import type { AgentEvent } from "../types";

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

  it("merges adjacent text events into one rendered card", () => {
    // Regression: kiro-cli streams tokens every ~100ms. Without merging, the
    // UI renders one step-card per chunk and the output looks jammed together
    // because the card padding hides the natural text flow.
    const events: AgentEvent[] = [
      { event_type: "text", text: "This is a **Brazil workspace" },
      { event_type: "text", text: "** containing the" },
      { event_type: "text", text: " **GroceryPackUWC**" },
      { event_type: "text", text: " fleet." },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(1);
    const merged = (result[0] as { type: "standalone"; event: AgentEvent }).event;
    expect(merged.event_type).toBe("text");
    if (merged.event_type === "text") {
      expect(merged.text).toBe("This is a **Brazil workspace** containing the **GroceryPackUWC** fleet.");
    }
  });

  it("does not merge text across non-text events", () => {
    // A tool call between two text chunks must preserve the boundary so the
    // narrative reads "...before..." / tool / "...after..." in the timeline.
    const events: AgentEvent[] = [
      { event_type: "text", text: "About to " },
      { event_type: "text", text: "read the file." },
      { event_type: "tool_request", id: "t1", tool_name: "Read", description: "reading", risk_level: "low", needs_approval: false },
      { event_type: "tool_result", id: "t1", success: true, output: "contents", duration_ms: 10 },
      { event_type: "text", text: "Now I understand " },
      { event_type: "text", text: "the issue." },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(3);
    // First: merged text
    const first = (result[0] as { type: "standalone"; event: AgentEvent }).event;
    expect(first.event_type === "text" && first.text).toBe("About to read the file.");
    // Second: tool group
    expect(result[1].type).toBe("tool_group");
    // Third: merged text after the tool
    const third = (result[2] as { type: "standalone"; event: AgentEvent }).event;
    expect(third.event_type === "text" && third.text).toBe("Now I understand the issue.");
  });

  it("does not merge text with thinking", () => {
    // Thinking and text are semantically different — the UI renders them with
    // different styling. Merging would hide thinking inside a text card.
    const events: AgentEvent[] = [
      { event_type: "thinking", text: "considering options" },
      { event_type: "text", text: "here is the answer" },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(2);
  });

  it("collapses a 30-chunk streaming response to a single card", () => {
    // Regression for "ACP adapter renders text all jank": kiro-cli streams
    // tokens every ~100ms so the translator can emit dozens of Text events
    // per response. Without merging, the UI renders 30+ step-cards in a
    // column, each with its own padding. End-to-end assertion: however many
    // chunks the translator emits, the UI-facing output is one text item.
    const chunks = [
      "This ", "is ", "a ", "**Brazil ", "workspace** ",
      "containing ", "the ", "**GroceryPackUWC** ", "fleet ",
      "— ", "a ", "UWC ", "(Universal ", "Workc", "ell) ",
      "project ", "for ", "grocery ", "automation. ",
      "It ", "has ", "3 ", "packages:\n\n",
      "1. ", "**GroceryPackUWCCDK** ", "— ", "CDK ",
      "infrastructure ", "(Type", "Script).",
    ];
    const events: AgentEvent[] = chunks.map((text) => ({
      event_type: "text" as const,
      text,
    }));
    const result = groupToolEvents(events);
    expect(result).toHaveLength(1);
    const merged = (result[0] as { type: "standalone"; event: AgentEvent }).event;
    expect(merged.event_type).toBe("text");
    if (merged.event_type === "text") {
      expect(merged.text).toBe(chunks.join(""));
    }
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
    expect(group.subAgentSpawned).toBeNull();
    expect(group.subAgentComplete).toBeNull();
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

  it("orphaned sub_agent events remain standalone", () => {
    const events: AgentEvent[] = [
      { event_type: "follow_up", text: "next question" },
      { event_type: "sub_agent_spawned", parent_tool_use_id: "t99", description: "research" },
      { event_type: "sub_agent_complete", parent_tool_use_id: "t99", summary: "found it", cost_usd: 0.02 },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(3);
    expect(result.every((r) => r.type === "standalone")).toBe(true);
  });

  it("nests sub-agent events into matching ToolGroup", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "t0", tool_name: "Task", description: "delegate work", risk_level: "low", needs_approval: false },
      { event_type: "sub_agent_spawned", parent_tool_use_id: "t0", description: "researching docs" },
      { event_type: "sub_agent_complete", parent_tool_use_id: "t0", summary: "found answer", cost_usd: 0.012 },
      { event_type: "tool_result", id: "t0", success: true, output: "delegated", duration_ms: 5000 },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(1);
    const group = result[0] as ToolGroup;
    expect(group.type).toBe("tool_group");
    expect(group.request.tool_name).toBe("Task");
    expect(group.result?.success).toBe(true);
    expect(group.subAgentSpawned?.description).toBe("researching docs");
    expect(group.subAgentComplete?.summary).toBe("found answer");
    expect(group.subAgentComplete?.cost_usd).toBe(0.012);
  });

  it("multiple sub-agent groups stay independent", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "t0", tool_name: "Task", description: "job A", risk_level: "low", needs_approval: false },
      { event_type: "sub_agent_spawned", parent_tool_use_id: "t0", description: "agent A" },
      { event_type: "sub_agent_complete", parent_tool_use_id: "t0", summary: "A done", cost_usd: 0.01 },
      { event_type: "tool_result", id: "t0", success: true, output: "ok", duration_ms: 1000 },
      { event_type: "tool_request", id: "t1", tool_name: "Task", description: "job B", risk_level: "low", needs_approval: false },
      { event_type: "sub_agent_spawned", parent_tool_use_id: "t1", description: "agent B" },
      { event_type: "sub_agent_complete", parent_tool_use_id: "t1", summary: "B done", cost_usd: 0.02 },
      { event_type: "tool_result", id: "t1", success: true, output: "ok", duration_ms: 2000 },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(2);
    const g0 = result[0] as ToolGroup;
    const g1 = result[1] as ToolGroup;
    expect(g0.subAgentSpawned?.description).toBe("agent A");
    expect(g0.subAgentComplete?.cost_usd).toBe(0.01);
    expect(g1.subAgentSpawned?.description).toBe("agent B");
    expect(g1.subAgentComplete?.cost_usd).toBe(0.02);
  });

  it("in-progress sub-agent (no complete yet)", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "t0", tool_name: "Task", description: "delegate", risk_level: "low", needs_approval: false },
      { event_type: "sub_agent_spawned", parent_tool_use_id: "t0", description: "working on it" },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(1);
    const group = result[0] as ToolGroup;
    expect(group.subAgentSpawned?.description).toBe("working on it");
    expect(group.subAgentComplete).toBeNull();
    expect(group.result).toBeNull();
  });

  it("mixed: one tool with sub-agent, one without", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "t1", tool_name: "Read", description: "read file", risk_level: "low", needs_approval: false },
      { event_type: "tool_result", id: "t1", success: true, output: "contents", duration_ms: 50 },
      { event_type: "tool_request", id: "t2", tool_name: "Task", description: "delegate", risk_level: "low", needs_approval: false },
      { event_type: "sub_agent_spawned", parent_tool_use_id: "t2", description: "sub work" },
      { event_type: "sub_agent_complete", parent_tool_use_id: "t2", summary: "sub done", cost_usd: 0.005 },
      { event_type: "tool_result", id: "t2", success: true, output: "delegated", duration_ms: 3000 },
    ];
    const result = groupToolEvents(events);
    expect(result).toHaveLength(2);
    const g1 = result[0] as ToolGroup;
    const g2 = result[1] as ToolGroup;
    expect(g1.request.tool_name).toBe("Read");
    expect(g1.subAgentSpawned).toBeNull();
    expect(g1.subAgentComplete).toBeNull();
    expect(g2.request.tool_name).toBe("Task");
    expect(g2.subAgentSpawned?.description).toBe("sub work");
    expect(g2.subAgentComplete?.cost_usd).toBe(0.005);
  });
});
