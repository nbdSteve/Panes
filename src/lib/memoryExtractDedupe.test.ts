import { describe, it, expect } from "vitest";
import { shouldExtractOnComplete } from "./memoryExtractDedupe";
import type { AgentEvent } from "../types";

const text = (t: string): AgentEvent => ({ event_type: "text", text: t });
const done = (): AgentEvent => ({
  event_type: "complete",
  summary: "ok",
  total_cost_usd: 0,
  duration_ms: 0,
  turns: 1,
});

describe("shouldExtractOnComplete", () => {
  it("first complete on a brand-new thread extracts", () => {
    const r = shouldExtractOnComplete([text("hi")], 0, false);
    expect(r.extract).toBe(true);
    expect(r.newExtractedCount).toBe(1);
  });

  it("follow-up complete on a persisted thread DOES extract", () => {
    // User resumed a thread loaded from SQLite (already has a complete
    // and extractedMemories). The stream now has a second complete
    // incoming. The persisted baseline covers complete #1 so extraction
    // must fire for complete #2.
    const priorEvents: AgentEvent[] = [
      text("first answer"),
      done(),
      { event_type: "follow_up", text: "and this" },
      text("second answer"),
    ];
    const r = shouldExtractOnComplete(priorEvents, 0, true);
    expect(r.extract).toBe(true);
    expect(r.newExtractedCount).toBe(2);
  });

  it("second live complete (same session) re-extracts", () => {
    // Session started fresh, complete #1 already extracted this session.
    // User sent follow-up and complete #2 just arrived.
    const priorEvents: AgentEvent[] = [
      text("first answer"),
      done(),
      { event_type: "follow_up", text: "more" },
      text("second answer"),
    ];
    const r = shouldExtractOnComplete(priorEvents, 1, false);
    expect(r.extract).toBe(true);
    expect(r.newExtractedCount).toBe(2);
  });

  it("same complete arriving twice in one batch is idempotent", () => {
    // Protects against intra-batch double-processing: the ref was already
    // bumped to 1 by a prior item in the same batch, and the second item
    // reports the same prior event stream (no extra complete added yet).
    const priorEvents: AgentEvent[] = [text("first")];
    const r = shouldExtractOnComplete(priorEvents, 1, false);
    expect(r.extract).toBe(false);
    expect(r.newExtractedCount).toBe(1);
  });

  it("persisted baseline only applies when nothing has been extracted this session yet", () => {
    // Edge case: session-scoped extraction count is nonzero (live run
    // completed once already) AND the thread also has persisted
    // extraction. The persisted baseline shouldn't double-count —
    // session count is authoritative once it's nonzero.
    const priorEvents: AgentEvent[] = [text("first"), done()];
    const r = shouldExtractOnComplete(priorEvents, 1, true);
    expect(r.extract).toBe(true);
    expect(r.newExtractedCount).toBe(2);
  });
});
