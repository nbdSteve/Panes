import { describe, it, expect } from "vitest";
import { shouldFirePendingResume } from "./pendingResume";
import type { ThreadInfo, WorkspaceInfo } from "../types";

function makeThread(overrides: Partial<ThreadInfo> = {}): ThreadInfo {
  return {
    id: "t1",
    workspaceId: "w1",
    prompt: "hi",
    status: "interrupted",
    events: [],
    createdAt: Date.now(),
    ...overrides,
  };
}

const ws: WorkspaceInfo = {
  id: "w1",
  path: "/tmp/w1",
  name: "w1",
};

describe("shouldFirePendingResume", () => {
  it("returns null when no pending ref", () => {
    expect(shouldFirePendingResume(null, [makeThread()], [ws])).toBeNull();
  });

  it("returns null when thread is not found", () => {
    const result = shouldFirePendingResume(
      { threadId: "missing", prompt: "x" },
      [makeThread()],
      [ws],
    );
    expect(result).toBeNull();
  });

  it("returns null when thread is still running", () => {
    const result = shouldFirePendingResume(
      { threadId: "t1", prompt: "x" },
      [makeThread({ status: "running" })],
      [ws],
    );
    expect(result).toBeNull();
  });

  it("returns null when thread is in gate state", () => {
    const result = shouldFirePendingResume(
      { threadId: "t1", prompt: "x" },
      [makeThread({ status: "gate" })],
      [ws],
    );
    expect(result).toBeNull();
  });

  it("returns null when workspace is missing", () => {
    const result = shouldFirePendingResume(
      { threadId: "t1", prompt: "x" },
      [makeThread({ status: "interrupted" })],
      [],
    );
    expect(result).toBeNull();
  });

  it.each(["interrupted", "error", "complete"] as const)(
    "returns target when thread is %s",
    (status) => {
      const result = shouldFirePendingResume(
        { threadId: "t1", prompt: "hello" },
        [makeThread({ status })],
        [ws],
      );
      expect(result).toEqual({
        workspace: ws,
        threadId: "t1",
        prompt: "hello",
      });
    },
  );
});
