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

  if (lower.includes("error") || lower.includes("fail")) {
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
    });
    events.push({ event_type: "cost_update", total_usd: 0.018 });
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
      });
      events.push({
        event_type: "tool_result",
        id: `tool_${i}`,
        tool_name: "Edit",
        success: true,
        output: "File edited successfully",
      });
    }
    events.push({ event_type: "cost_update", total_usd: 0.025 });
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
      });
    }
    events.push({ event_type: "cost_update", total_usd: 0.012 });
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
  } else if (lower.includes("multi") || lower.includes("complex")) {
    events.push({
      event_type: "thinking",
      text: "I'll work through this step by step.",
    });
    const steps = [
      { tool: "Read", desc: "Read file: src/App.tsx", risk: "low" },
      { tool: "Edit", desc: "Edit file: src/App.tsx", risk: "medium" },
      { tool: "Bash", desc: "Run command: npm test", risk: "low" },
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
      });
      events.push({
        event_type: "cost_update",
        total_usd: 0.005 * (i + 1),
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
    events.push({ event_type: "cost_update", total_usd: 0.003 });
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

    emitEvent("panes://thread-event", {
      thread_id: threadId,
      timestamp: new Date().toISOString(),
      event,
      parent_tool_use_id: null,
    });
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
      const threadId = crypto.randomUUID();
      const events = buildEvents(prompt);
      setTimeout(() => emitThreadEvents(threadId, events), 300);
      return threadId;
    }

    case "resume_thread": {
      const threadId = args?.threadId as string;
      const prompt = args?.prompt as string;
      const events = buildEvents(prompt);
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
      pausedThreads.delete(threadId);
      setTimeout(() => {
        emitEvent("panes://thread-event", {
          thread_id: threadId,
          timestamp: new Date().toISOString(),
          event: {
            event_type: "complete",
            summary: "Action was rejected by the user.",
            total_cost_usd: 0.005,
            duration_ms: 3000,
            turns: 1,
          },
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
      return null;
    }

    case "commit_changes":
      return "mock-commit-hash";

    case "revert_changes":
      return null;

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
    case "get_workspaces":
      return [...mockWorkspaces];

    case "remove_workspace": {
      const wsId = args?.workspace_id as string;
      const idx = mockWorkspaces.findIndex((w) => w.id === wsId);
      if (idx >= 0) mockWorkspaces.splice(idx, 1);
      return null;
    }

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

    default:
      console.warn(`[tauriMock] unhandled invoke: ${cmd}`, args);
      return null;
  }
}

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
