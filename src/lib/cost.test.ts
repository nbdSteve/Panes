import { describe, it, expect } from "vitest";
import { calculateRunningCost, type ThreadEvent } from "./cost";

describe("calculateRunningCost", () => {
  it("single turn: uses last cost_update value", () => {
    const events: ThreadEvent[] = [
      { event_type: "thinking" },
      { event_type: "cost_update", total_usd: 0.003 },
      { event_type: "text" },
      { event_type: "complete" },
    ];
    expect(calculateRunningCost(events)).toBeCloseTo(0.003);
  });

  it("single turn with multiple cost updates: uses last value per turn", () => {
    const events: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.001 },
      { event_type: "cost_update", total_usd: 0.005 },
      { event_type: "complete" },
    ];
    // Last cost_update in the turn is 0.005
    expect(calculateRunningCost(events)).toBeCloseTo(0.005);
  });

  it("multi-turn: accumulates across follow-ups", () => {
    const events: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.01 },
      { event_type: "complete" },
      { event_type: "follow_up" },
      { event_type: "cost_update", total_usd: 0.02 },
      { event_type: "complete" },
    ];
    expect(calculateRunningCost(events)).toBeCloseTo(0.03);
  });

  it("mid-stream: includes cost from in-progress turn", () => {
    const events: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.01 },
      { event_type: "complete" },
      { event_type: "follow_up" },
      { event_type: "cost_update", total_usd: 0.02 },
      // No complete yet — still running
    ];
    expect(calculateRunningCost(events)).toBeCloseTo(0.03);
  });

  it("no cost events: returns zero", () => {
    const events: ThreadEvent[] = [
      { event_type: "thinking" },
      { event_type: "text" },
      { event_type: "complete" },
    ];
    expect(calculateRunningCost(events)).toBe(0);
  });

  it("empty events: returns zero", () => {
    expect(calculateRunningCost([])).toBe(0);
  });

  it("cost_update with missing total_usd: treated as zero", () => {
    const events: ThreadEvent[] = [
      { event_type: "cost_update" },
      { event_type: "complete" },
    ];
    expect(calculateRunningCost(events)).toBe(0);
  });

  it("matches backend Complete event total", () => {
    // Simulate what the backend CostTracker produces:
    // Backend accumulates all cost_update.total_usd values then Complete overrides.
    // Frontend takes last cost_update per turn, accumulates across turns.
    // For single cost_update per turn, they agree.
    const events: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.012 },
      { event_type: "complete" },
      { event_type: "follow_up" },
      { event_type: "cost_update", total_usd: 0.018 },
      { event_type: "complete" },
    ];
    const frontendTotal = calculateRunningCost(events);
    const backendTotal = 0.012 + 0.018; // backend accumulates
    expect(frontendTotal).toBeCloseTo(backendTotal);
  });
});
