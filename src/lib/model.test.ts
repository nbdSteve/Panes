import { describe, it, expect } from "vitest";

// Inline the function since it's not exported from a module
function normalizeModelId(raw: string): string {
  const lower = raw.toLowerCase();
  if (lower.includes("opus")) return "opus";
  if (lower.includes("sonnet")) return "sonnet";
  if (lower.includes("haiku")) return "haiku";
  return raw;
}

describe("normalizeModelId", () => {
  it("short names pass through", () => {
    expect(normalizeModelId("opus")).toBe("opus");
    expect(normalizeModelId("sonnet")).toBe("sonnet");
    expect(normalizeModelId("haiku")).toBe("haiku");
  });

  it("full claude model IDs normalize to short names", () => {
    expect(normalizeModelId("claude-opus-4.6")).toBe("opus");
    expect(normalizeModelId("claude-sonnet-4.6")).toBe("sonnet");
    expect(normalizeModelId("claude-haiku-4-5-20251001")).toBe("haiku");
  });

  it("case insensitive", () => {
    expect(normalizeModelId("Opus")).toBe("opus");
    expect(normalizeModelId("SONNET")).toBe("sonnet");
    expect(normalizeModelId("Claude-Opus-4.5")).toBe("opus");
  });

  it("unknown model returns as-is", () => {
    expect(normalizeModelId("gpt-4")).toBe("gpt-4");
    expect(normalizeModelId("custom-model")).toBe("custom-model");
    expect(normalizeModelId("")).toBe("");
  });
});
