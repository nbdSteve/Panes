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

  it("returns null when all token counts are 0", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.01, input_tokens: 0, cache_read_tokens: 0, cache_creation_tokens: 0 },
    ];
    expect(calculateContextUsage(events)).toBeNull();
  });

  it("sums input + cache_read + cache_creation for total context", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.01, input_tokens: 10, cache_read_tokens: 15_000, cache_creation_tokens: 5_000, model: "claude-sonnet-4-6" },
    ];
    const result = calculateContextUsage(events);
    expect(result).not.toBeNull();
    expect(result!.inputTokens).toBe(20_010);
    expect(result!.percentage).toBeCloseTo(10.005);
    expect(result!.level).toBe("ok");
  });

  it("works with only cache_creation_tokens (first message)", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.1, input_tokens: 10, cache_creation_tokens: 20_836, model: "claude-opus-4-6" },
    ];
    const result = calculateContextUsage(events);
    expect(result!.inputTokens).toBe(20_846);
    expect(result!.level).toBe("ok");
  });

  it("returns warning level at 40%", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.05, input_tokens: 1_000, cache_read_tokens: 79_000, model: "claude-opus-4-6" },
    ];
    const result = calculateContextUsage(events);
    expect(result!.percentage).toBeCloseTo(40);
    expect(result!.level).toBe("warning");
  });

  it("returns danger level at 80%", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.1, input_tokens: 10_000, cache_read_tokens: 150_000, model: "claude-opus-4-6" },
    ];
    const result = calculateContextUsage(events);
    expect(result!.percentage).toBeCloseTo(80);
    expect(result!.level).toBe("danger");
  });

  it("uses latest cost_update for context size", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.01, input_tokens: 10, cache_creation_tokens: 10_000 },
      { event_type: "cost_update", total_usd: 0.02, input_tokens: 10, cache_read_tokens: 10_000, cache_creation_tokens: 40_000 },
    ];
    const result = calculateContextUsage(events);
    expect(result!.inputTokens).toBe(50_010);
    expect(result!.percentage).toBeCloseTo(25.005);
  });

  it("defaults to 200k limit for unknown model", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.01, input_tokens: 500, cache_read_tokens: 99_500, model: "unknown-model" },
    ];
    const result = calculateContextUsage(events);
    expect(result!.percentage).toBeCloseTo(50);
    expect(result!.level).toBe("warning");
  });

  it("handles missing cache fields gracefully", () => {
    const events: AgentEvent[] = [
      { event_type: "cost_update", total_usd: 0.01, input_tokens: 20_000 },
    ];
    const result = calculateContextUsage(events);
    expect(result!.inputTokens).toBe(20_000);
    expect(result!.percentage).toBeCloseTo(10);
  });
});
