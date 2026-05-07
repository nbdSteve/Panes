import { describe, it, expect } from "vitest";
import { extractFilePaths, parseGitStatus, collectTestResults } from "./threadHelpers";
import type { AgentEvent } from "../types";

describe("extractFilePaths", () => {
  it("extracts file_path from Write tool events", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "1", tool_name: "Write", description: "Write file", risk_level: "low", needs_approval: false, input: { file_path: "/Users/test/project/src/main.ts", content: "hello" } },
      { event_type: "tool_request", id: "2", tool_name: "Edit", description: "Edit file", risk_level: "low", needs_approval: false, input: { file_path: "/Users/test/project/src/utils.ts", old_string: "a", new_string: "b" } },
    ];
    const paths = extractFilePaths(events);
    expect(paths).toEqual(["/Users/test/project/src/main.ts", "/Users/test/project/src/utils.ts"]);
  });

  it("extracts from NotebookEdit events", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "1", tool_name: "NotebookEdit", description: "Edit notebook", risk_level: "low", needs_approval: false, input: { file_path: "/tmp/notebook.ipynb", cell_number: 1 } },
    ];
    expect(extractFilePaths(events)).toEqual(["/tmp/notebook.ipynb"]);
  });

  it("ignores non-write tools", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "1", tool_name: "Read", description: "Read file", risk_level: "low", needs_approval: false, input: { file_path: "/Users/test/readme.md" } },
      { event_type: "tool_request", id: "2", tool_name: "Bash", description: "Run command", risk_level: "low", needs_approval: false, input: { command: "ls" } },
    ];
    expect(extractFilePaths(events)).toEqual([]);
  });

  it("deduplicates repeated edits to same file", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "1", tool_name: "Edit", description: "Edit", risk_level: "low", needs_approval: false, input: { file_path: "/src/app.ts" } },
      { event_type: "tool_request", id: "2", tool_name: "Edit", description: "Edit", risk_level: "low", needs_approval: false, input: { file_path: "/src/app.ts" } },
      { event_type: "tool_request", id: "3", tool_name: "Write", description: "Write", risk_level: "low", needs_approval: false, input: { file_path: "/src/new.ts" } },
    ];
    expect(extractFilePaths(events)).toEqual(["/src/app.ts", "/src/new.ts"]);
  });

  it("handles missing input gracefully", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "1", tool_name: "Write", description: "Write", risk_level: "low", needs_approval: false },
      { event_type: "tool_request", id: "2", tool_name: "Edit", description: "Edit", risk_level: "low", needs_approval: false, input: {} },
      { event_type: "tool_request", id: "3", tool_name: "Edit", description: "Edit", risk_level: "low", needs_approval: false, input: { file_path: 42 } },
    ];
    expect(extractFilePaths(events)).toEqual([]);
  });

  it("skips non-tool-request events", () => {
    const events: AgentEvent[] = [
      { event_type: "text", text: "hello" },
      { event_type: "thinking", text: "thinking..." },
      { event_type: "tool_result", id: "1", success: true, output: "ok" },
      { event_type: "tool_request", id: "4", tool_name: "Write", description: "Write", risk_level: "low", needs_approval: false, input: { file_path: "/real/path.ts" } },
    ];
    expect(extractFilePaths(events)).toEqual(["/real/path.ts"]);
  });
});

describe("parseGitStatus", () => {
  it("parses modified files", () => {
    expect(parseGitStatus([" M src/app.ts"])).toEqual([{ path: "src/app.ts", action: "modified" }]);
  });

  it("parses added files", () => {
    expect(parseGitStatus(["A  src/new.ts"])).toEqual([{ path: "src/new.ts", action: "created" }]);
  });

  it("parses deleted files", () => {
    expect(parseGitStatus([" D old.ts"])).toEqual([{ path: "old.ts", action: "deleted" }]);
  });

  it("parses untracked files", () => {
    expect(parseGitStatus(["?? untracked.ts"])).toEqual([{ path: "untracked.ts", action: "untracked" }]);
  });
});

describe("collectTestResults", () => {
  it("returns undefined when no test commands present", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "1", tool_name: "Bash", description: "ls src/", risk_level: "low", needs_approval: false },
      { event_type: "tool_result", id: "1", success: true, output: "app.ts" },
    ];
    expect(collectTestResults(events)).toBeUndefined();
  });

  it("captures test output", () => {
    const events: AgentEvent[] = [
      { event_type: "tool_request", id: "1", tool_name: "Bash", description: "npx vitest run", risk_level: "low", needs_approval: false },
      { event_type: "tool_result", id: "1", success: true, output: "Tests: 5 passed" },
    ];
    expect(collectTestResults(events)).toBe("Tests: 5 passed");
  });
});
