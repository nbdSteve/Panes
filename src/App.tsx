import { useState, useEffect, useCallback, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import Sidebar from "./components/Sidebar";
import ThreadList from "./components/ThreadList";
import ThreadView from "./components/ThreadView";
import MemoryPanel from "./components/MemoryPanel";

export interface WorkspaceInfo {
  id: string;
  path: string;
  name: string;
  defaultAgent?: string;
}

export interface AgentEvent {
  event_type: string;
  text?: string;
  id?: string;
  tool_name?: string;
  description?: string;
  risk_level?: string;
  needs_approval?: boolean;
  summary?: string;
  total_cost_usd?: number;
  duration_ms?: number;
  turns?: number;
  total_usd?: number;
  success?: boolean;
  output?: string;
  message?: string;
}

export interface ThreadInfo {
  id: string;
  workspaceId: string;
  prompt: string;
  status: "starting" | "running" | "gate" | "complete" | "error" | "interrupted";
  completionAction?: "committed" | "reverted" | "kept";
  queuedFollowUp?: string;
  events: AgentEvent[];
  createdAt: number;
}

interface ThreadEvent {
  thread_id: string;
  timestamp: string;
  event: Record<string, unknown>;
  parent_tool_use_id: string | null;
}

function mapBackendEvent(raw: Record<string, unknown>): AgentEvent | null {
  const eventType = raw.event_type as string | undefined;
  if (!eventType) return null;

  switch (eventType) {
    case "thinking":
      return { event_type: "thinking", text: raw.text as string };
    case "text":
      return { event_type: "text", text: raw.text as string };
    case "tool_request":
      return {
        event_type: "tool_request",
        id: raw.id as string,
        tool_name: raw.tool_name as string,
        description: raw.description as string,
        risk_level: raw.risk_level as string,
        needs_approval: raw.needs_approval as boolean,
      };
    case "tool_result":
      return {
        event_type: "tool_result",
        id: raw.id as string,
        success: raw.success as boolean,
        output: raw.output as string,
      };
    case "cost_update":
      return { event_type: "cost_update", total_usd: raw.total_usd as number };
    case "complete":
      return {
        event_type: "complete",
        summary: raw.summary as string,
        total_cost_usd: raw.total_cost_usd as number,
        duration_ms: raw.duration_ms as number,
        turns: raw.turns as number,
      };
    case "error":
      return { event_type: "error", message: raw.message as string };
    default:
      return null;
  }
}

function App() {
  const [workspaces, setWorkspaces] = useState<WorkspaceInfo[]>([]);
  const [threads, setThreads] = useState<ThreadInfo[]>([]);
  const [activeWorkspace, setActiveWorkspace] = useState<string | null>(null);
  const [activeThread, setActiveThread] = useState<string | null>(null);
  const [activeView, setActiveView] = useState<"workspace" | "feed" | "memory">("feed");
  const unlistenRef = useRef<UnlistenFn | null>(null);

  useEffect(() => {
    invoke<WorkspaceInfo[]>("list_workspaces").then(setWorkspaces).catch(() => {});
  }, []);

  const handleCompletionAction = useCallback(
    (threadId: string, action: "committed" | "reverted" | "kept") => {
      setThreads((prev) =>
        prev.map((t) =>
          t.id === threadId ? { ...t, completionAction: action } : t
        )
      );
    },
    []
  );

  const handleCancelThread = useCallback(
    async (threadId: string) => {
      try {
        await invoke("cancel_thread", { threadId });
      } catch {}
      setThreads((prev) =>
        prev.map((t) =>
          t.id === threadId
            ? { ...t, status: "interrupted" as const, queuedFollowUp: undefined }
            : t
        )
      );
    },
    []
  );

  const handleQueueFollowUp = useCallback(
    (threadId: string, prompt: string) => {
      setThreads((prev) =>
        prev.map((t) =>
          t.id === threadId ? { ...t, queuedFollowUp: prompt || undefined } : t
        )
      );
    },
    []
  );

  const pendingResumeRef = useRef<{ threadId: string; prompt: string } | null>(null);

  useEffect(() => {
    let cancelled = false;
    listen<ThreadEvent>("panes://thread-event", (ev) => {
      if (cancelled) return;
      const { thread_id, event } = ev.payload;
      const mapped = mapBackendEvent(event);
      if (!mapped) return;

      setThreads((prev) => {
        const updated = prev.map((t) => {
          if (t.id !== thread_id) return t;
          const newStatus =
            mapped.event_type === "complete"
              ? "complete" as const
              : mapped.event_type === "error"
                ? "error" as const
                : mapped.event_type === "tool_request" && mapped.needs_approval
                  ? "gate" as const
                  : "running" as const;

          if (newStatus === "complete" && mapped.event_type === "complete") {
            extractMemoriesFromThread(t);
          }

          if ((newStatus === "complete" || newStatus === "error") && t.queuedFollowUp) {
            pendingResumeRef.current = { threadId: t.id, prompt: t.queuedFollowUp };
          }

          return {
            ...t,
            status: newStatus,
            events: [...t.events, mapped],
            queuedFollowUp: (newStatus === "complete" || newStatus === "error") ? undefined : t.queuedFollowUp,
          };
        });
        return updated;
      });
    }).then((unlisten) => {
      if (cancelled) { unlisten(); return; }
      unlistenRef.current = unlisten;
    });
    return () => {
      cancelled = true;
      unlistenRef.current?.();
    };
  }, []);

  const handleStartThread = useCallback(
    async (workspace: WorkspaceInfo, prompt: string) => {
      const tempId = crypto.randomUUID();

      setThreads((prev) => [
        ...prev,
        {
          id: tempId,
          workspaceId: workspace.id,
          prompt,
          status: "starting",
          events: [],
          createdAt: Date.now(),
        },
      ]);
      setActiveThread(tempId);

      try {
        const threadId = await invoke<string>("start_thread", {
          workspaceId: workspace.id,
          workspacePath: workspace.path,
          workspaceName: workspace.name,
          prompt,
          agent: workspace.defaultAgent ?? null,
        });

        setThreads((prev) =>
          prev.map((t) =>
            t.id === tempId ? { ...t, id: threadId, status: "running" } : t
          )
        );
        setActiveThread(threadId);
      } catch (e) {
        setThreads((prev) =>
          prev.map((t) =>
            t.id === tempId
              ? { ...t, status: "error", events: [{ event_type: "error", message: String(e) }] }
              : t
          )
        );
      }
    },
    []
  );

  const handleResumeThread = useCallback(
    async (workspace: WorkspaceInfo, threadId: string, prompt: string) => {
      setThreads((prev) =>
        prev.map((t) =>
          t.id === threadId
            ? {
                ...t,
                status: "running" as const,
                events: [
                  ...t.events,
                  { event_type: "follow_up", text: prompt },
                ],
              }
            : t
        )
      );

      try {
        await invoke("resume_thread", {
          threadId,
          workspaceId: workspace.id,
          workspacePath: workspace.path,
          workspaceName: workspace.name,
          prompt,
          agent: workspace.defaultAgent ?? null,
        });
      } catch (e) {
        setThreads((prev) =>
          prev.map((t) =>
            t.id === threadId
              ? {
                  ...t,
                  status: "error" as const,
                  events: [...t.events, { event_type: "error", message: String(e) }],
                }
              : t
          )
        );
      }
    },
    []
  );

  useEffect(() => {
    if (!pendingResumeRef.current) return;
    const { threadId, prompt } = pendingResumeRef.current;
    pendingResumeRef.current = null;
    const thread = threads.find((t) => t.id === threadId);
    if (!thread) return;
    const ws = workspaces.find((w) => w.id === thread.workspaceId);
    if (ws) {
      handleResumeThread(ws, threadId, prompt);
    }
  });

  const handleSendPrompt = useCallback(
    (workspace: WorkspaceInfo, prompt: string) => {
      const thread = threads.find((t) => t.id === activeThread);
      if (thread && (thread.status === "complete" || thread.status === "error" || thread.status === "interrupted")) {
        handleResumeThread(workspace, thread.id, prompt);
      } else if (!thread) {
        handleStartThread(workspace, prompt);
      }
    },
    [activeThread, threads, handleStartThread, handleResumeThread]
  );

  const extractMemoriesFromThread = useCallback((thread: ThreadInfo) => {
    const textEvents = thread.events
      .filter((e) => e.event_type === "text" && e.text)
      .map((e) => `Assistant: ${e.text}`)
      .join("\n");
    const transcript = `User: ${thread.prompt}\n${textEvents}`;
    invoke("extract_memories", {
      workspaceId: thread.workspaceId,
      threadId: thread.id,
      transcript,
    }).catch(() => {});
  }, []);

  const activeWs = workspaces.find((w) => w.id === activeWorkspace);
  const wsThreads = threads.filter((t) => t.workspaceId === activeWorkspace);
  const currentThread = threads.find((t) => t.id === activeThread);

  return (
    <div className="app">
      <Sidebar
        workspaces={workspaces}
        threads={threads}
        activeWorkspace={activeWorkspace}
        activeView={activeView}
        onSelectWorkspace={(id) => {
          setActiveWorkspace(id);
          setActiveView("workspace");
          const lastThread = threads
            .filter((t) => t.workspaceId === id)
            .sort((a, b) => b.createdAt - a.createdAt)[0];
          setActiveThread(lastThread?.id ?? null);
        }}
        onSelectFeed={() => {
          setActiveWorkspace(null);
          setActiveView("feed");
        }}
        onSelectMemory={(wsId) => {
          setActiveWorkspace(wsId);
          setActiveView("memory");
        }}
        onAddWorkspace={async (ws) => {
          try {
            const saved = await invoke<WorkspaceInfo>("add_workspace", {
              path: ws.path,
              name: ws.name,
            });
            setWorkspaces((prev) => [...prev, saved]);
            setActiveWorkspace(saved.id);
          } catch {
            setWorkspaces((prev) => [...prev, ws]);
            setActiveWorkspace(ws.id);
          }
          setActiveView("workspace");
          setActiveThread(null);
        }}
      />

      {activeView === "workspace" && activeWs && (
        <ThreadList
          threads={wsThreads}
          activeThread={activeThread}
          onSelectThread={setActiveThread}
          onNewThread={() => setActiveThread(null)}
        />
      )}

      <main className="main-panel">
        {activeView === "feed" && (
          <div className="feed-placeholder">
            <div className="feed-icon">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M12 3v18" /><path d="M8 7l4-4 4 4" />
              </svg>
            </div>
            <h2>Welcome to Panes</h2>
            <p>
              Add a workspace to start sending tasks to your AI agent. Activity
              from all workspaces will appear here.
            </p>
          </div>
        )}

        {activeView === "workspace" && activeWs && (
          <ThreadView
            key={activeThread ?? "new"}
            workspace={activeWs}
            thread={currentThread ?? null}
            onStartThread={(prompt) => handleSendPrompt(activeWs, prompt)}
            onCompletionAction={handleCompletionAction}
            onCancel={handleCancelThread}
            onQueueFollowUp={handleQueueFollowUp}
          />
        )}

        {activeView === "memory" && activeWs && (
          <MemoryPanel workspaceId={activeWs.id} />
        )}
      </main>
    </div>
  );
}

export default App;
