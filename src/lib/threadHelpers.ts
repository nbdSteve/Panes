import type { AgentEvent, ToolRequestEvent } from "../types";

export const FILE_WRITE_TOOLS = new Set(["Write", "Edit", "NotebookEdit"]);

export const TEST_CMD_PATTERN = /\b(test|spec|jest|vitest|pytest|cargo\s+test|npm\s+test|yarn\s+test|npx\s+vitest|npx\s+jest)\b/i;

export function collectTestResults(events: AgentEvent[]): string | undefined {
  const results: string[] = [];
  const requestById = new Map<string, ToolRequestEvent>();

  for (const e of events) {
    if (e.event_type === "tool_request" && e.id) {
      requestById.set(e.id, e);
    }
  }

  for (const e of events) {
    if (e.event_type !== "tool_result" || !e.id || !e.output) continue;
    const req = requestById.get(e.id);
    if (!req) continue;
    const desc = req.description || "";
    if (TEST_CMD_PATTERN.test(desc)) {
      results.push(e.output);
    }
  }

  return results.length > 0 ? results.join("\n---\n") : undefined;
}

export function parseGitStatus(lines: string[]): { path: string; action: "created" | "modified" | "deleted" | "untracked" }[] {
  return lines.map((line) => {
    const status = line.substring(0, 2);
    const path = line.substring(3).trim();
    if (status === "??") return { path, action: "untracked" as const };
    if (status.includes("D")) return { path, action: "deleted" as const };
    if (status.includes("A")) return { path, action: "created" as const };
    return { path, action: "modified" as const };
  });
}

export function collectFilesChanged(events: AgentEvent[]): { path: string; action: "created" | "modified" }[] {
  const files: { path: string; action: "created" | "modified" }[] = [];
  const seen = new Set<string>();

  for (const e of events) {
    if (e.event_type !== "tool_request") continue;
    const toolName = e.tool_name ?? "";
    if (!["Write", "Edit", "NotebookEdit"].includes(toolName)) continue;

    const desc = e.description ?? "";
    const match = desc.match(/(?:Edit|Write|Create|Modify)\s+(?:file:\s*|to\s+)?(.+)/i);
    const path = match ? match[1].trim() : desc;

    if (path && !seen.has(path)) {
      seen.add(path);
      files.push({
        path,
        action: toolName === "Write" ? "created" : "modified",
      });
    }
  }
  return files;
}
