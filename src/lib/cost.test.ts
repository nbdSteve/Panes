import { describe, it, expect } from "vitest";
import {
  calculateRunningCost,
  threadDisplayCost,
  workspaceDisplayCost,
  type ThreadEvent,
  type CostThread,
} from "./cost";

describe("calculateRunningCost", () => {
  it("single turn: sums cost_update values", () => {
    const events: ThreadEvent[] = [
      { event_type: "thinking" },
      { event_type: "cost_update", total_usd: 0.003 },
      { event_type: "text" },
      { event_type: "complete" },
    ];
    expect(calculateRunningCost(events)).toBeCloseTo(0.003);
  });

  it("single turn with multiple cost updates: sums all", () => {
    const events: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.001 },
      { event_type: "cost_update", total_usd: 0.005 },
      { event_type: "complete" },
    ];
    expect(calculateRunningCost(events)).toBeCloseTo(0.006);
  });

  it("complete with total_cost_usd overrides estimates", () => {
    const events: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.001 },
      { event_type: "cost_update", total_usd: 0.005 },
      { event_type: "complete", total_cost_usd: 0.008 },
    ];
    expect(calculateRunningCost(events)).toBeCloseTo(0.008);
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
      { event_type: "cost_update", total_usd: 0.008 },
      { event_type: "cost_update", total_usd: 0.012 },
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

  it("multi-turn with authoritative complete totals", () => {
    const events: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.005 },
      { event_type: "cost_update", total_usd: 0.007 },
      { event_type: "complete", total_cost_usd: 0.015 },
      { event_type: "follow_up" },
      { event_type: "cost_update", total_usd: 0.010 },
      { event_type: "cost_update", total_usd: 0.008 },
      { event_type: "complete", total_cost_usd: 0.020 },
    ];
    expect(calculateRunningCost(events)).toBeCloseTo(0.035);
  });
});

describe("cost display consistency", () => {
  // Simulates the accumulation App.tsx does when complete events arrive:
  // costUsd += complete.total_cost_usd
  function simulateAppCostAccumulation(events: ThreadEvent[]): number {
    let costUsd = 0;
    for (const e of events) {
      if (e.event_type === "complete" && e.total_cost_usd != null) {
        costUsd += e.total_cost_usd;
      }
    }
    return costUsd;
  }

  // Simulates what the DB would return: SUM of all complete.total_cost_usd
  // (same as simulateAppCostAccumulation — this is the point)
  function simulateDbCost(events: ThreadEvent[]): number {
    return simulateAppCostAccumulation(events);
  }

  it("single turn: all display points agree", () => {
    const events: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.005 },
      { event_type: "cost_update", total_usd: 0.007 },
      { event_type: "complete", total_cost_usd: 0.015 },
    ];
    const costUsd = simulateAppCostAccumulation(events);
    const thread: CostThread = { status: "complete", costUsd, events };

    const threadView = threadDisplayCost(thread);
    const sidebar = workspaceDisplayCost([thread]);
    const dbTotal = simulateDbCost(events);

    expect(threadView).toBeCloseTo(0.015);
    expect(sidebar).toBeCloseTo(0.015);
    expect(dbTotal).toBeCloseTo(0.015);
    expect(threadView).toBeCloseTo(sidebar);
    expect(sidebar).toBeCloseTo(dbTotal);
  });

  it("multi-turn: all display points agree", () => {
    const events: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.005 },
      { event_type: "complete", total_cost_usd: 0.015 },
      { event_type: "follow_up" },
      { event_type: "cost_update", total_usd: 0.010 },
      { event_type: "cost_update", total_usd: 0.008 },
      { event_type: "complete", total_cost_usd: 0.020 },
    ];
    const costUsd = simulateAppCostAccumulation(events);
    const thread: CostThread = { status: "complete", costUsd, events };

    const threadView = threadDisplayCost(thread);
    const sidebar = workspaceDisplayCost([thread]);
    const dbTotal = simulateDbCost(events);

    expect(threadView).toBeCloseTo(0.035);
    expect(sidebar).toBeCloseTo(0.035);
    expect(dbTotal).toBeCloseTo(0.035);
    expect(threadView).toBeCloseTo(sidebar);
    expect(sidebar).toBeCloseTo(dbTotal);
  });

  it("in-progress turn: sidebar and threadView use live estimate", () => {
    const events: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.005 },
      { event_type: "complete", total_cost_usd: 0.015 },
      { event_type: "follow_up" },
      { event_type: "cost_update", total_usd: 0.010 },
      { event_type: "cost_update", total_usd: 0.008 },
      // No complete yet — second turn still running
    ];
    const costUsd = simulateAppCostAccumulation(events);
    const thread: CostThread = { status: "running", costUsd, events };

    const threadView = threadDisplayCost(thread);
    const sidebar = workspaceDisplayCost([thread]);

    // Running thread uses calculateRunningCost: turn1 authoritative (0.015) + turn2 estimate (0.018)
    expect(threadView).toBeCloseTo(0.033);
    expect(sidebar).toBeCloseTo(threadView);
  });

  it("multiple threads in workspace: sidebar sums correctly", () => {
    const t1Events: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.01 },
      { event_type: "complete", total_cost_usd: 0.015 },
    ];
    const t2Events: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.02 },
      { event_type: "complete", total_cost_usd: 0.025 },
    ];
    const t1: CostThread = { status: "complete", costUsd: simulateAppCostAccumulation(t1Events), events: t1Events };
    const t2: CostThread = { status: "complete", costUsd: simulateAppCostAccumulation(t2Events), events: t2Events };

    const sidebar = workspaceDisplayCost([t1, t2]);
    const dbTotal = simulateDbCost(t1Events) + simulateDbCost(t2Events);

    expect(sidebar).toBeCloseTo(0.040);
    expect(sidebar).toBeCloseTo(dbTotal);
  });

  it("mixed running and complete threads: sidebar handles both", () => {
    const doneEvents: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.01 },
      { event_type: "complete", total_cost_usd: 0.015 },
    ];
    const liveEvents: ThreadEvent[] = [
      { event_type: "cost_update", total_usd: 0.008 },
      { event_type: "cost_update", total_usd: 0.004 },
    ];
    const done: CostThread = { status: "complete", costUsd: 0.015, events: doneEvents };
    const live: CostThread = { status: "running", costUsd: undefined, events: liveEvents };

    const sidebar = workspaceDisplayCost([done, live]);

    expect(threadDisplayCost(done)).toBeCloseTo(0.015);
    expect(threadDisplayCost(live)).toBeCloseTo(0.012);
    expect(sidebar).toBeCloseTo(0.027);
  });
});
