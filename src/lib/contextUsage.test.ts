import { describe, it, expect } from "vitest";
import { calculateContextUsage } from "./contextUsage";
import type { AgentEvent } from "../App";

describe("calculateContextUsage", () => {
  it("returns null when no cost_update events", () => {
    const events: AgentEvent[] = [
      { event_type: "thinking", text: "hmm" },
      { event_type: "text", text: "hello" },
    ];
    expect(calculateContextUsage(events)).toBeNull();
  });

  it("returns null when input_tokens is 0", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.01, input_tokens: 0 },
    ];
    expect(calculateContextUsage(events)).toBeNull();
  });

  it("computes percentage and green level for low usage", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.01, input_tokens: 20_000, model: "claude-sonnet-4-6" },
    ];
    const result = calculateContextUsage(events);
    expect(result).not.toBeNull();
    expect(result!.inputTokens).toBe(20_000);
    expect(result!.percentage).toBeCloseTo(10);
    expect(result!.level).toBe("ok");
  });

  it("returns warning level at 40%", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.05, input_tokens: 80_000, model: "claude-opus-4-6" },
    ];
    const result = calculateContextUsage(events);
    expect(result!.percentage).toBeCloseTo(40);
    expect(result!.level).toBe("warning");
  });

  it("returns danger level at 80%", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.1, input_tokens: 160_000, model: "claude-opus-4-6" },
    ];
    const result = calculateContextUsage(events);
    expect(result!.percentage).toBeCloseTo(80);
    expect(result!.level).toBe("danger");
  });

  it("uses latest cost_update input_tokens", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.01, input_tokens: 10_000 },
      { event_type: "cost_update", total_usd: 0.02, input_tokens: 50_000 },
    ];
    const result = calculateContextUsage(events);
    expect(result!.inputTokens).toBe(50_000);
    expect(result!.percentage).toBeCloseTo(25);
  });

  it("defaults to 200k limit for unknown model", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.01, input_tokens: 100_000, model: "unknown-model" },
    ];
    const result = calculateContextUsage(events);
    expect(result!.percentage).toBeCloseTo(50);
    expect(result!.level).toBe("warning");
  });
});
