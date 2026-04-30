import { describe, it, expect, vi, afterEach } from "vitest";
import { timeAgo, formatCost, truncatePrompt, normalizeModelId } from "./utils";

describe("timeAgo", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("returns 'just now' for recent timestamps", () => {
    const now = new Date().toISOString();
    expect(timeAgo(now)).toBe("just now");
  });

  it("returns minutes for 1-59 minutes ago", () => {
    const fiveMinAgo = new Date(Date.now() - 5 * 60 * 1000).toISOString();
    expect(timeAgo(fiveMinAgo)).toBe("5m ago");
  });

  it("returns hours for 1-23 hours ago", () => {
    const threeHoursAgo = new Date(Date.now() - 3 * 60 * 60 * 1000).toISOString();
    expect(timeAgo(threeHoursAgo)).toBe("3h ago");
  });

  it("returns days for 24+ hours ago", () => {
    const twoDaysAgo = new Date(Date.now() - 48 * 60 * 60 * 1000).toISOString();
    expect(timeAgo(twoDaysAgo)).toBe("2d ago");
  });
});

describe("formatCost", () => {
  it("uses 4 decimal places for costs under $0.01", () => {
    expect(formatCost(0.0012)).toBe("$0.0012");
    expect(formatCost(0.009)).toBe("$0.0090");
  });

  it("uses 2 decimal places for costs $0.01 and above", () => {
    expect(formatCost(0.01)).toBe("$0.01");
    expect(formatCost(1.5)).toBe("$1.50");
    expect(formatCost(99.99)).toBe("$99.99");
  });

  it("handles zero", () => {
    expect(formatCost(0)).toBe("$0.0000");
  });
});

describe("truncatePrompt", () => {
  it("returns text unchanged when under limit", () => {
    expect(truncatePrompt("short text")).toBe("short text");
  });

  it("returns text unchanged when exactly at limit", () => {
    const exact = "a".repeat(80);
    expect(truncatePrompt(exact)).toBe(exact);
  });

  it("truncates and adds ellipsis when over limit", () => {
    const long = "a".repeat(100);
    const result = truncatePrompt(long);
    expect(result).toHaveLength(83);
    expect(result.endsWith("...")).toBe(true);
  });

  it("respects custom max length", () => {
    expect(truncatePrompt("hello world", 5)).toBe("hello...");
  });
});

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

  it("is case insensitive", () => {
    expect(normalizeModelId("Opus")).toBe("opus");
    expect(normalizeModelId("SONNET")).toBe("sonnet");
    expect(normalizeModelId("Claude-Opus-4.5")).toBe("opus");
  });

  it("returns unknown models as-is", () => {
    expect(normalizeModelId("gpt-4")).toBe("gpt-4");
    expect(normalizeModelId("custom-model")).toBe("custom-model");
    expect(normalizeModelId("")).toBe("");
  });
});
