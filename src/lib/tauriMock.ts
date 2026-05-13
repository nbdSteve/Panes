type Callback = (payload: unknown) => void;

const callbacks: Map<number, Callback> = new Map();
let nextCallbackId = 1;

interface EventListener {
  id: number;
  event: string;
  handlerCallbackId: number;
}

const eventListeners: EventListener[] = [];
let nextEventId = 1;

function emitEvent(event: string, payload: unknown) {
  for (const listener of eventListeners) {
    if (listener.event === event) {
      const cb = callbacks.get(listener.handlerCallbackId);
      if (cb) {
        cb({ id: listener.id, event, payload });
      }
    }
  }
}

function buildEvents(prompt: string): Array<Record<string, unknown>> {
  const lower = prompt.toLowerCase();
  const events: Array<Record<string, unknown>> = [];

  if (lower.includes("validate")) {
    events.push({ event_type: "thinking", text: "Generating output..." });
    events.push({
      event_type: "text",
      text: "See src/missing.rs for the bug.",
    });
    events.push({
      event_type: "validation_result",
      validator: "citation",
      target_event_index: 1,
      outcome: "fail",
      findings: [
        {
          severity: "error",
          message: "referenced path does not exist: src/missing.rs",
          span: "src/missing.rs",
          source_hint: "workspace",
        },
      ],
      duration_ms: 5,
    });
    events.push({ event_type: "__gate_pause__", id: "validator_gate" });
    events.push({
      event_type: "complete",
      summary: "See src/missing.rs for the bug.",
      total_cost_usd: 0.002,
      duration_ms: 1000,
      turns: 1,
    });
  } else if (lower.includes("error") || lower.includes("fail")) {
    events.push({ event_type: "thinking", text: "Let me try..." });
    events.push({
      event_type: "error",
      message: "Simulated error: something went wrong",
      recoverable: false,
    });
  } else if (
    lower.includes("gate") ||
    lower.includes("dangerous") ||
    lower.includes("destructive")
  ) {
    events.push({
      event_type: "thinking",
      text: "This requires a potentially risky operation.",
    });
    events.push({
      event_type: "tool_request",
      id: "gate_0",
      tool_name: "Bash",
      description: "rm -rf /tmp/test-directory",
      needs_approval: true,
      risk_level: "critical",
    });
    // GATE_PAUSE — remaining events emitted after approve/reject
    events.push({ event_type: "__gate_pause__", id: "gate_0" });
    events.push({
      event_type: "tool_result",
      id: "gate_0",
      tool_name: "Bash",
      success: true,
      output: "Command executed successfully",
      duration_ms: 250,
    });
    events.push({ event_type: "cost_update", total_usd: 0.018, input_tokens: 12000, output_tokens: 800, model: "claude-sonnet-4-6" });
    events.push({
      event_type: "text",
      text: "The dangerous operation has been completed successfully.",
    });
    events.push({
      event_type: "complete",
      summary: "The dangerous operation has been completed successfully.",
      total_cost_usd: 0.018,
      duration_ms: 12000,
      turns: 2,
    });
  } else if (
    lower.includes("edit") ||
    lower.includes("write") ||
    lower.includes("create file")
  ) {
    events.push({
      event_type: "thinking",
      text: "I'll make the requested changes.",
    });
    for (const [i, file] of ["src/main.rs", "src/lib.rs"].entries()) {
      events.push({
        event_type: "tool_request",
        id: `tool_${i}`,
        tool_name: "Edit",
        description: `Edit file: ${file}`,
        needs_approval: false,
        risk_level: "medium",
        input: { file_path: `/tmp/test-ws/${file}`, old_string: "old", new_string: "new" },
      });
      events.push({
        event_type: "tool_result",
        id: `tool_${i}`,
        tool_name: "Edit",
        success: true,
        output: "File edited successfully",
        duration_ms: 80,
      });
    }
    events.push({ event_type: "cost_update", total_usd: 0.025, input_tokens: 18000, output_tokens: 1200, model: "claude-sonnet-4-6" });
    events.push({
      event_type: "text",
      text: "I've made the requested edits to the files.",
    });
    events.push({
      event_type: "complete",
      summary: "I've made the requested edits to the files.",
      total_cost_usd: 0.025,
      duration_ms: 8000,
      turns: 3,
    });
  } else if (
    lower.includes("read") ||
    lower.includes("explain") ||
    lower.includes("analyze")
  ) {
    events.push({
      event_type: "thinking",
      text: "I'll read the relevant files first.",
    });
    for (const [i, file] of ["src/App.tsx", "src/styles.css"].entries()) {
      events.push({
        event_type: "tool_request",
        id: `tool_${i}`,
        tool_name: "Read",
        description: `Read file: ${file}`,
        needs_approval: false,
        risk_level: "low",
      });
      events.push({
        event_type: "tool_result",
        id: `tool_${i}`,
        tool_name: "Read",
        success: true,
        output: `(contents of ${file})`,
        duration_ms: 150,
      });
    }
    events.push({ event_type: "cost_update", total_usd: 0.012, input_tokens: 15000, output_tokens: 600, model: "claude-sonnet-4-6" });
    events.push({
      event_type: "text",
      text: "Based on my analysis of the files, here is what I found:\n\n- The App component manages thread state centrally\n- Styles use CSS custom properties for theming\n- The architecture follows a unidirectional data flow pattern",
    });
    events.push({
      event_type: "complete",
      summary:
        "Based on my analysis of the files, here is what I found:\n\n- The App component manages thread state centrally\n- Styles use CSS custom properties for theming\n- The architecture follows a unidirectional data flow pattern",
      total_cost_usd: 0.012,
      duration_ms: 5000,
      turns: 2,
    });
  } else if (lower.includes("subagent") || lower.includes("delegate")) {
    events.push({
      event_type: "thinking",
      text: "I'll delegate this to a sub-agent.",
    });
    events.push({
      event_type: "tool_request",
      id: "tool_0",
      tool_name: "Task",
      description: "Spawn sub-agent: Research authentication patterns",
      needs_approval: false,
      risk_level: "low",
    });
    events.push({
      event_type: "sub_agent_spawned",
      parent_tool_use_id: "tool_0",
      description: "Research authentication patterns",
    });
    events.push({
      event_type: "sub_agent_complete",
      parent_tool_use_id: "tool_0",
      summary: "Found 3 authentication patterns in the codebase",
      cost_usd: 0.008,
    });
    events.push({
      event_type: "tool_result",
      id: "tool_0",
      tool_name: "Task",
      success: true,
      output: "Found 3 authentication patterns",
      duration_ms: 4500,
    });
    events.push({ event_type: "cost_update", total_usd: 0.018, input_tokens: 12000, output_tokens: 800, model: "claude-sonnet-4-6" });
    events.push({
      event_type: "text",
      text: "Based on the sub-agent's research, I found 3 authentication patterns in the codebase.",
    });
    events.push({
      event_type: "complete",
      summary: "Based on the sub-agent's research, I found 3 authentication patterns in the codebase.",
      total_cost_usd: 0.018,
      duration_ms: 7000,
      turns: 2,
    });
  } else if (lower.includes("multi") || lower.includes("complex")) {
    events.push({
      event_type: "thinking",
      text: "I'll work through this step by step.",
    });
    const steps = [
      { tool: "Read", desc: "Read file: src/App.tsx", risk: "low", duration: 120 },
      { tool: "Edit", desc: "Edit file: src/App.tsx", risk: "medium", duration: 95 },
      { tool: "Bash", desc: "Run command: npm test", risk: "low", duration: 3200 },
    ];
    for (const [i, step] of steps.entries()) {
      events.push({
        event_type: "tool_request",
        id: `tool_${i}`,
        tool_name: step.tool,
        description: step.desc,
        needs_approval: false,
        risk_level: step.risk,
      });
      events.push({
        event_type: "tool_result",
        id: `tool_${i}`,
        tool_name: step.tool,
        success: true,
        output:
          i === 2 ? "All 42 tests passed" : i === 1 ? "File edited" : "(file contents)",
        duration_ms: step.duration,
      });
      events.push({
        event_type: "cost_update",
        total_usd: 0.005 * (i + 1),
        input_tokens: 8000 * (i + 1),
        output_tokens: 400 * (i + 1),
        model: "claude-sonnet-4-6",
      });
    }
    events.push({
      event_type: "text",
      text: "I've read the file, made edits, and verified the tests pass.",
    });
    events.push({
      event_type: "complete",
      summary: "I've read the file, made edits, and verified the tests pass.",
      total_cost_usd: 0.015,
      duration_ms: 9000,
      turns: 4,
    });
  } else {
    events.push({
      event_type: "thinking",
      text: "Let me think about this...",
    });
    events.push({ event_type: "cost_update", total_usd: 0.003, input_tokens: 5000, output_tokens: 300, model: "claude-sonnet-4-6" });
    events.push({
      event_type: "text",
      text: `I received your message: "${prompt}"\n\nThis is a **fake response** from the test adapter.`,
    });
    events.push({
      event_type: "complete",
      summary: `I received your message: "${prompt}"`,
      total_cost_usd: 0.003,
      duration_ms: 2500,
      turns: 1,
    });
  }

  return events;
}

interface MockWorkspace {
  id: string;
  path: string;
  name: string;
  defaultAgent: string | null;
}

const mockWorkspaces: MockWorkspace[] = [];

interface MockMemory {
  id: string;
  workspaceId: string | null;
  memoryType: string;
  content: string;
  sourceThreadId: string;
  pinned: boolean;
  createdAt: string;
}

const mockMemories: MockMemory[] = [];
const mockBriefings = new Map<string, { workspaceId: string; content: string }>();

interface MockThread {
  id: string;
  workspaceId: string;
  prompt: string;
  status: string;
  summary: string;
  costUsd: number;
  durationMs: number;
  createdAt: string;
  events: Array<Record<string, unknown>>;
}

const mockThreads: MockThread[] = [];
const activeThreadMeta = new Map<string, { workspaceId: string; prompt: string; events: Array<Record<string, unknown>>; adapter?: string }>();
// Test-only: record the adapter of the most recent start_thread / resume_thread
// call so E2E tests can verify the frontend routed a prompt through the expected
// backend. `lastResumeThreadAdapter` captures the *effective* adapter the mock
// used (stored-adapter > frontend-hint > default), mirroring the production
// backend's DB lookup in resume_thread.
let lastStartThreadAdapter: string | null = null;
let lastResumeThreadAdapter: string | null = null;
const workspacePathsWithEdits = new Set<string>();

interface PausedThread {
  threadId: string;
  remainingEvents: Array<Record<string, unknown>>;
}

const pausedThreads = new Map<string, PausedThread>();
const activeIntervals = new Map<string, ReturnType<typeof setInterval>>();

function emitThreadEvents(threadId: string, events: Array<Record<string, unknown>>) {
  let i = 0;
  const interval = setInterval(() => {
    if (i >= events.length) {
      clearInterval(interval);
      activeIntervals.delete(threadId);
      return;
    }
    const event = events[i];

    if (event.event_type === "__gate_pause__") {
      clearInterval(interval);
      activeIntervals.delete(threadId);
      pausedThreads.set(threadId, {
        threadId,
        remainingEvents: events.slice(i + 1),
      });
      return;
    }

    const meta = activeThreadMeta.get(threadId);
    if (meta) {
      meta.events.push(event);
    }

    emitEvent("panes://thread-event", {
      thread_id: threadId,
      timestamp: new Date().toISOString(),
      event,
      parent_tool_use_id: null,
    });

    if (meta && event.event_type === "complete") {
      mockThreads.push({
        id: threadId,
        workspaceId: meta.workspaceId,
        prompt: meta.prompt,
        status: "completed",
        summary: (event.summary as string) || "",
        costUsd: (event.total_cost_usd as number) || 0,
        durationMs: (event.duration_ms as number) || 0,
        createdAt: new Date().toISOString(),
        events: [...meta.events],
      });
    }

    i++;
  }, 200);
  activeIntervals.set(threadId, interval);
}

function resumeAfterGate(threadId: string) {
  const paused = pausedThreads.get(threadId);
  if (!paused) return;
  pausedThreads.delete(threadId);
  setTimeout(() => emitThreadEvents(threadId, paused.remainingEvents), 100);
}

async function mockInvoke(cmd: string, args?: Record<string, unknown>): Promise<unknown> {
  switch (cmd) {
    case "plugin:event|listen": {
      const eventName = args?.event as string;
      const handlerCbId = args?.handler as number;
      const id = nextEventId++;
      eventListeners.push({ id, event: eventName, handlerCallbackId: handlerCbId });
      return id;
    }

    case "plugin:event|unlisten": {
      const eventId = args?.eventId as number;
      const idx = eventListeners.findIndex((l) => l.id === eventId);
      if (idx >= 0) eventListeners.splice(idx, 1);
      return null;
    }

    case "start_thread": {
      const prompt = args?.prompt as string;
      // `adapter` is the adapter selector; `agent` is the per-adapter
      // sub-agent/mode. Backward-compat: if adapter isn't set, accept
      // `agent` as the adapter name (matches the pre-kiro-cli IPC shape).
      const adapter = (args?.adapter as string)
        || (args?.agent as string)
        || "claude-code";
      if (adapter !== "claude-code" && adapter !== "kiro-cli") {
        throw new Error(`unknown adapter: ${adapter}`);
      }
      const threadId = crypto.randomUUID();
      const events = buildEvents(prompt);
      const workspaceId = args?.workspaceId as string;
      const workspacePath = args?.workspacePath as string;
      const lower = prompt.toLowerCase();
      if (lower.includes("edit") || lower.includes("write") || lower.includes("create file")) {
        workspacePathsWithEdits.add(workspacePath);
      }
      activeThreadMeta.set(threadId, {
        workspaceId,
        prompt,
        events: [],
        adapter,
      });
      lastStartThreadAdapter = adapter;
      setTimeout(() => emitThreadEvents(threadId, events), 300);
      // Simulate git-workspace worktree isolation when the workspace
      // path opts in via a `/tmp/git-` prefix. Tests flip this to
      // exercise the Merge / Discard UI without a real git repo.
      const isIsolated = typeof workspacePath === "string" && workspacePath.startsWith("/tmp/git-");
      return {
        threadId,
        injectedMemories: [],
        briefingPreview: null,
        worktreeStatus: isIsolated ? "isolated" : undefined,
      };
    }

    case "resume_thread": {
      const threadId = args?.threadId as string;
      const prompt = args?.prompt as string;
      // Mirror the production backend: the adapter the thread was spawned
      // with (stored in activeThreadMeta, analogous to threads.agent_type
      // on the server) wins. The frontend-supplied hint is only a fallback
      // for the case where the row is missing. Without this, kiro-cli
      // follow-ups get routed to claude-code and fail.
      const storedAdapter = activeThreadMeta.get(threadId)?.adapter;
      const adapter = storedAdapter
        || (args?.adapter as string)
        || (args?.agent as string)
        || "claude-code";
      if (adapter !== "claude-code" && adapter !== "kiro-cli") {
        throw new Error(`unknown adapter: ${adapter}`);
      }
      lastResumeThreadAdapter = adapter;
      const events = buildEvents(prompt);
      activeThreadMeta.set(threadId, {
        workspaceId: args?.workspaceId as string,
        prompt,
        events: [],
        adapter,
      });
      setTimeout(() => emitThreadEvents(threadId, events), 300);
      return null;
    }

    case "approve_gate": {
      const threadId = args?.threadId as string;
      resumeAfterGate(threadId);
      return null;
    }

    case "reject_gate": {
      const threadId = args?.threadId as string;
      if (!pausedThreads.has(threadId)) return null;
      pausedThreads.delete(threadId);
      setTimeout(() => {
        // Mirror the backend: rejecting a gate produces a non-recoverable Error
        // that marks the thread as 'interrupted'. The thread remains resumable
        // because the backend preserves session_id through interruption.
        const errorEvent = {
          event_type: "error",
          message: "Gate rejected by user",
          recoverable: false,
        };
        const meta = activeThreadMeta.get(threadId);
        if (meta) {
          meta.events.push(errorEvent);
          mockThreads.push({
            id: threadId,
            workspaceId: meta.workspaceId,
            prompt: meta.prompt,
            status: "interrupted",
            summary: "",
            costUsd: 0,
            durationMs: 0,
            createdAt: new Date().toISOString(),
            events: [...meta.events],
          });
        }
        emitEvent("panes://thread-event", {
          thread_id: threadId,
          timestamp: new Date().toISOString(),
          event: errorEvent,
          parent_tool_use_id: null,
        });
      }, 100);
      return null;
    }

    case "cancel_thread": {
      const cancelId = args?.threadId as string;
      const activeInterval = activeIntervals.get(cancelId);
      if (activeInterval) {
        clearInterval(activeInterval);
        activeIntervals.delete(cancelId);
      }
      pausedThreads.delete(cancelId);
      const meta = activeThreadMeta.get(cancelId);
      if (meta) {
        mockThreads.push({
          id: cancelId,
          workspaceId: meta.workspaceId,
          prompt: meta.prompt,
          status: "interrupted",
          summary: "Cancelled by user",
          costUsd: 0,
          durationMs: 0,
          createdAt: new Date().toISOString(),
          events: [...meta.events],
        });
      }
      return null;
    }

    case "set_thread_model":
      // Mock accepts any model switch. Real behavior differs per adapter.
      return null;

    case "commit_changes":
      return "mock-commit-hash";

    case "revert_changes":
      return null;

    case "merge_to_main":
      // Default success outcome; tests that need a conflict can shadow
      // this case via their own mock setup before invoke.
      return { outcome: "fast_forwarded", commit: "mock-merge-commit", files: [] };

    case "add_workspace": {
      const ws: MockWorkspace = {
        id: crypto.randomUUID(),
        path: args?.path as string,
        name: args?.name as string || (args?.path as string).split("/").pop() || "workspace",
        defaultAgent: "claude-code",
      };
      mockWorkspaces.push(ws);
      return ws;
    }

    case "list_workspaces":
      return [...mockWorkspaces];

    case "remove_workspace": {
      const wsId = args?.workspaceId as string ?? args?.workspace_id as string;
      const idx = mockWorkspaces.findIndex((w) => w.id === wsId);
      if (idx >= 0) mockWorkspaces.splice(idx, 1);
      return null;
    }

    case "list_threads": {
      const wsId = args?.workspaceId as string;
      return mockThreads.filter(t => t.workspaceId === wsId);
    }

    case "list_all_threads": {
      const limit = (args?.limit as number) || 100;
      return mockThreads.slice(0, limit);
    }

    case "delete_thread":
      return null;

    case "get_aggregate_cost":
      return mockThreads.reduce((sum, t) => sum + (t.costUsd || 0), 0);

    case "get_cost_timeline": {
      const days = (args?.days as number) || 30;
      const data: { day: string; totalUsd: number }[] = [];
      for (let i = days - 1; i >= 0; i--) {
        const d = new Date(Date.now() - i * 86400000);
        data.push({
          day: d.toISOString().slice(0, 10),
          totalUsd: Math.round((Math.random() * 0.4 + 0.02) * 1000) / 1000,
        });
      }
      return data;
    }

    case "get_workspace_cost_breakdown":
      return mockWorkspaces.map(ws => {
        const wsThreads = mockThreads.filter(t => t.workspaceId === ws.id);
        return {
          workspaceId: ws.id,
          workspaceName: ws.name,
          totalUsd: wsThreads.reduce((sum, t) => sum + (t.costUsd || 0), 0),
          threadCount: wsThreads.length,
        };
      });

    case "get_memories":
      return [...mockMemories.filter((m) => m.workspaceId === args?.workspaceId)];

    case "search_memories":
      return [...mockMemories.filter((m) =>
        m.workspaceId === args?.workspaceId &&
        m.content.toLowerCase().includes((args?.query as string || "").toLowerCase())
      )];

    case "extract_memories": {
      const mem = {
        id: crypto.randomUUID(),
        workspaceId: args?.workspaceId as string,
        memoryType: "pattern",
        content: `Extracted from thread: ${(args?.transcript as string || "").slice(0, 80)}`,
        sourceThreadId: args?.threadId as string,
        pinned: false,
        createdAt: new Date().toISOString(),
      };
      mockMemories.push(mem);
      return [mem];
    }

    case "update_memory": {
      const mi = mockMemories.findIndex((m) => m.id === args?.memoryId);
      if (mi >= 0) mockMemories[mi].content = args?.content as string;
      return null;
    }

    case "delete_memory": {
      const di = mockMemories.findIndex((m) => m.id === args?.memoryId);
      if (di >= 0) mockMemories.splice(di, 1);
      return null;
    }

    case "pin_memory": {
      const pi = mockMemories.findIndex((m) => m.id === args?.memoryId);
      if (pi >= 0) mockMemories[pi].pinned = args?.pinned as boolean;
      return null;
    }

    case "get_briefing":
      return mockBriefings.get(args?.workspaceId as string) ?? null;

    case "set_briefing":
      mockBriefings.set(args?.workspaceId as string, {
        workspaceId: args?.workspaceId as string,
        content: args?.content as string,
      });
      return null;

    case "delete_briefing":
      mockBriefings.delete(args?.workspaceId as string);
      return null;

    case "get_changed_files": {
      const wsPath = args?.workspacePath as string;
      if (workspacePathsWithEdits.has(wsPath)) {
        return ["M  src/main.ts", "M  src/utils.ts", "?? src/new-file.ts"];
      }
      return [];
    }

    case "get_file_diff":
      return `diff --git a/src/main.ts b/src/main.ts
--- a/src/main.ts
+++ b/src/main.ts
@@ -1,5 +1,6 @@
 import { app } from './app';
+import { logger } from './logger';

 function main() {
-  app.start();
+  logger.info('starting');
+  app.start({ verbose: true });
 }`;

    case "get_workspace_diff":
      return `diff --git a/src/main.rs b/src/main.rs
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,7 @@
 use std::io;
+use std::fmt;

 fn main() {
-    println!("Hello, world!");
+    let name = "Panes";
+    println!("Hello, {}!", name);
 }
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,8 @@
+pub mod utils;
+
 pub fn add(a: i32, b: i32) -> i32 {
     a + b
 }
+
+pub fn subtract(a: i32, b: i32) -> i32 {
+    a - b
+}`;

    case "get_files_git_status": {
      const filePaths = (args?.filePaths as string[]) ?? [];
      if (filePaths.length === 0) return [];
      return [{
        repoPath: "/mock/workspace",
        repoName: "workspace",
        files: filePaths.map((p: string) => ({
          absolutePath: p,
          relativePath: p.split("/").slice(-2).join("/"),
          status: "M",
        })),
      }];
    }

    case "list_git_repos":
      return [""];

    case "commit_repos":
      return ["mock-commit-hash-123"];

    case "generate_commit_message":
      return "feat: add formatting utilities and refactor main entry point\n\nIntroduce fmt import for display formatting, rename greeting target, and add subtract utility function to lib module.";

    case "get_workspace_cost":
      return 0;

    case "set_workspace_budget_cap":
      return null;

    case "get_memory_backend_status":
      return { active: "sqlite", mem0Available: false };

    case "set_memory_backend":
      return null;

    case "list_adapters":
      return ["claude-code", "kiro-cli"];

    // Test-only: returns the adapter name of the most recent start_thread
    // call. Lets E2E tests verify the frontend actually routes prompts to
    // the adapter the user picked — the production code doesn't need this.
    case "__test_last_start_thread_adapter":
      return lastStartThreadAdapter;

    // Test-only: returns the effective adapter used by the most recent
    // resume_thread call. Covers the regression where kiro-cli follow-ups
    // were routed through claude-code because the frontend didn't supply
    // a hint and the backend defaulted to claude-code.
    case "__test_last_resume_thread_adapter":
      return lastResumeThreadAdapter;

    case "list_agents":
      if ((args?.adapter as string) === "claude-code") {
        return [
          { name: "codebase-analyzer", model: "sonnet", description: "Analyze code implementation details with precise file:line references..." },
          { name: "codebase-locator", model: "sonnet", description: "Locate files and components relevant to a task..." },
          { name: "codebase-pattern-finder", model: "opus", description: "Find similar implementations and usage patterns..." },
          { name: "context-doc-generator", model: "opus", description: "Create CONTEXT.md files for AI-friendly codebase documentation..." },
          { name: "karen", model: null, description: "Assess project completion state and create realistic plans..." },
          { name: "load-test-planner", model: "opus", description: "Plan load tests before implementation begins..." },
          { name: "thoughts-analyzer", model: "opus", description: "Deep dive on research topics by analyzing thought documents..." },
          { name: "thoughts-locator", model: "sonnet", description: "Discover relevant documents in the thoughts/ directory..." },
        ];
      }
      if ((args?.adapter as string) === "kiro-cli") {
        // Neutral fake modes — the real list comes from the backend via
        // discovery. These two are just enough to exercise the picker in
        // E2E tests.
        return [
          { name: "mode-a", model: null, description: "Fake mode A" },
          { name: "mode-b", model: null, description: "Fake mode B" },
        ];
      }
      return [];

    case "list_models":
      return [
        { id: "sonnet", label: "Sonnet", description: "Fast & capable" },
        { id: "opus", label: "Opus", description: "Most capable" },
        { id: "haiku", label: "Haiku", description: "Fastest" },
      ];

    case "set_workspace_default_agent": {
      const wsId = args?.workspaceId as string ?? args?.workspace_id as string;
      const agent = args?.agent as string;
      const ws = mockWorkspaces.find(w => w.id === wsId);
      if (ws) ws.defaultAgent = agent;
      return null;
    }

    case "get_features":
      return [
        { id: "routines", enabled: false, label: "Routines", description: "Scheduled agent tasks" },
        { id: "cost_tracking", enabled: true, label: "Cost Tracking", description: "Track API costs" },
      ];

    case "set_feature_enabled":
      return null;

    case "create_routine":
      return {
        id: crypto.randomUUID(),
        workspaceId: args?.workspaceId,
        prompt: args?.prompt,
        cronExpr: args?.cronExpr,
        budgetCap: args?.budgetCap ?? null,
        onComplete: { action: "notify" },
        onFailure: { action: "notify" },
        enabled: true,
        lastRunAt: null,
        createdAt: new Date().toISOString(),
      };

    case "list_routines":
      return [];

    case "toggle_routine":
    case "delete_routine":
    case "update_routine":
      return null;

    case "list_routine_executions":
      return [];

    case "get_routine_cost":
      return 0;

    case "list_validator_types":
      return [
        {
          typeId: "citation",
          label: "Citation Check",
          description: "Verifies file path citations resolve in the workspace.",
          defaultConfig: { check_line_refs: true },
          correctable: true,
        },
        {
          typeId: "secret_scan",
          label: "Secret Scan",
          description: "Flags output containing secret-like strings.",
          defaultConfig: { custom_patterns: [] },
          correctable: false,
        },
      ];

    case "list_validators":
      return mockValidators.filter((v) => v.workspaceId === args?.workspaceId);

    case "add_validator": {
      const now = new Date().toISOString();
      const v = {
        id: crypto.randomUUID(),
        workspaceId: args?.workspaceId as string,
        validatorType: args?.validatorType as string,
        enabled: true,
        configJson: args?.configJson as string,
        createdAt: now,
        updatedAt: now,
      };
      mockValidators.push(v);
      return v;
    }

    case "update_validator": {
      const idx = mockValidators.findIndex((v) => v.id === args?.id);
      if (idx >= 0) {
        const next = { ...mockValidators[idx] };
        if (args?.enabled !== undefined) next.enabled = args.enabled as boolean;
        if (args?.configJson !== undefined)
          next.configJson = args.configJson as string;
        next.updatedAt = new Date().toISOString();
        mockValidators[idx] = next;
        return next;
      }
      return null;
    }

    case "remove_validator": {
      const idx = mockValidators.findIndex((v) => v.id === args?.id);
      if (idx >= 0) mockValidators.splice(idx, 1);
      return null;
    }

    default:
      console.warn(`[tauriMock] unhandled invoke: ${cmd}`, args);
      return null;
  }
}

const mockValidators: Array<{
  id: string;
  workspaceId: string;
  validatorType: string;
  enabled: boolean;
  configJson: string;
  createdAt: string;
  updatedAt: string;
}> = [];

export function installTauriMock() {
  if ((window as any).__TAURI_INTERNALS__) return;

  (window as any).__TAURI_INTERNALS__ = {
    invoke: mockInvoke,
    transformCallback(callback: Callback, _once?: boolean): number {
      const id = nextCallbackId++;
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback(id: number) {
      callbacks.delete(id);
    },
    convertFileSrc(path: string) {
      return path;
    },
  };

  (window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(_event: string, _eventId: number) {},
  };
}
