import { describe, it, expect } from "vitest";
import { collectTestResults, parseGitStatus } from "../lib/threadHelpers";
import type { AgentEvent } from "../types";

describe("collectTestResults", () => {
  it("returns undefined when no test commands ran", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "t1", tool_name: "Read", description: "Read file: main.rs", risk_level: "low", needs_approval: false },
      { event_type: "tool_result", id: "t1", success: true, output: "fn main() {}" },
    ];
    expect(collectTestResults(events)).toBeUndefined();
  });

  it("extracts test output from matching tool results", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "t1", tool_name: "Bash", description: "Run command: cargo test", risk_level: "low", needs_approval: false },
      { event_type: "tool_result", id: "t1", success: true, output: "test result: ok. 5 passed" },
    ];
    expect(collectTestResults(events)).toBe("test result: ok. 5 passed");
  });

  it("matches various test command patterns", () => {
    const patterns = ["npm test", "npx vitest", "npx jest", "yarn test", "pytest", "cargo test"];
    for (const cmd of patterns) {
      const events: AgentEvent[] = [
        { event_type: "tool_request", id: "t1", tool_name: "Bash", description: `Run command: ${cmd}`, risk_level: "low", needs_approval: false },
        { event_type: "tool_result", id: "t1", success: true, output: `${cmd} output` },
      ];
      expect(collectTestResults(events)).toBe(`${cmd} output`);
    }
  });

  it("ignores non-test tool results", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "t1", tool_name: "Bash", description: "Run command: ls", risk_level: "low", needs_approval: false },
      { event_type: "tool_result", id: "t1", success: true, output: "main.rs lib.rs" },
      { event_type: "tool_request", id: "t2", tool_name: "Bash", description: "Run command: cargo test", risk_level: "low", needs_approval: false },
      { event_type: "tool_result", id: "t2", success: true, output: "all 10 tests passed" },
    ];
    expect(collectTestResults(events)).toBe("all 10 tests passed");
  });

  it("joins multiple test results", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "t1", tool_name: "Bash", description: "Run: npx vitest", risk_level: "low", needs_approval: false },
      { event_type: "tool_result", id: "t1", success: true, output: "5 passed" },
      { event_type: "tool_request", id: "t2", tool_name: "Bash", description: "Run: cargo test", risk_level: "low", needs_approval: false },
      { event_type: "tool_result", id: "t2", success: true, output: "10 passed" },
    ];
    expect(collectTestResults(events)).toBe("5 passed\n---\n10 passed");
  });
});

describe("parseGitStatus", () => {
  it("parses modified files", () => {
    expect(parseGitStatus([" M src/main.rs"])).toEqual([
      { path: "src/main.rs", action: "modified" },
    ]);
  });

  it("parses added files", () => {
    expect(parseGitStatus(["A  src/new.rs"])).toEqual([
      { path: "src/new.rs", action: "created" },
    ]);
  });

  it("parses deleted files", () => {
    expect(parseGitStatus([" D src/old.rs"])).toEqual([
      { path: "src/old.rs", action: "deleted" },
    ]);
  });

  it("parses untracked files", () => {
    expect(parseGitStatus(["?? src/tmp.rs"])).toEqual([
      { path: "src/tmp.rs", action: "untracked" },
    ]);
  });

  it("parses mixed statuses", () => {
    const lines = [
      " M src/lib.rs",
      "A  src/new.rs",
      " D src/old.rs",
      "?? src/tmp.rs",
    ];
    expect(parseGitStatus(lines)).toEqual([
      { path: "src/lib.rs", action: "modified" },
      { path: "src/new.rs", action: "created" },
      { path: "src/old.rs", action: "deleted" },
      { path: "src/tmp.rs", action: "untracked" },
    ]);
  });
});
