import { useState, useEffect, useCallback, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import Sidebar from "./components/Sidebar";
import ThreadList from "./components/ThreadList";
import ThreadView from "./components/ThreadView";
import MemoryPanel from "./components/MemoryPanel";
import FeedView from "./components/FeedView";
import { mapBackendEvent } from "./lib/eventMapper";

export interface WorkspaceInfo {
  id: string;
  path: string;
  name: string;
  defaultAgent?: string;
}

export interface AgentInfo {
  name: string;
  model: string | null;
  description: string | null;
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

function App() {
  const [workspaces, setWorkspaces] = useState<WorkspaceInfo[]>([]);
  const [threads, setThreads] = useState<ThreadInfo[]>([]);
  const [activeWorkspace, setActiveWorkspace] = useState<string | null>(null);
  const [activeThread, setActiveThread] = useState<string | null>(null);
  const [activeView, setActiveView] = useState<"workspace" | "feed" | "memory">("feed");
  const [adapters, setAdapters] = useState<string[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const unlistenRef = useRef<UnlistenFn | null>(null);

  const loadThreadsForWorkspace = useCallback(async (workspaceId: string) => {
    try {
      const persisted = await invoke<{
        id: string;
        workspaceId: string;
        prompt: string;
        status: string;
        summary: string | null;
        costUsd: number;
        durationMs: number | null;
        createdAt: string;
        events: AgentEvent[];
      }[]>("list_threads", { workspaceId });

      setThreads((prev) => {
        const liveIds = new Set(prev.filter((t) => t.workspaceId === workspaceId).map((t) => t.id));
        const toAdd = persisted
          .filter((p) => !liveIds.has(p.id))
          .map((p) => ({
            id: p.id,
            workspaceId: p.workspaceId,
            prompt: p.prompt,
            status: (p.status === "completed" ? "complete" : p.status) as ThreadInfo["status"],
            events: p.events,
            createdAt: new Date(p.createdAt).getTime(),
          }));
        return toAdd.length > 0 ? [...prev, ...toAdd] : prev;
      });
    } catch {}
  }, []);

  useEffect(() => {
    invoke<WorkspaceInfo[]>("list_workspaces").then((ws) => {
      setWorkspaces(ws);
      for (const w of ws) {
        loadThreadsForWorkspace(w.id);
      }
    }).catch(() => {});
    invoke<string[]>("list_adapters").then((a) => {
      setAdapters(a);
      if (a.length > 0) {
        invoke<AgentInfo[]>("list_agents", { adapter: a[0] }).then(setAgents).catch(() => {});
      }
    }).catch(() => {});
  }, [loadThreadsForWorkspace]);

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
    async (workspace: WorkspaceInfo, prompt: string, agent?: string, model?: string) => {
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
          agent: agent ?? workspace.defaultAgent ?? null,
          model: model ?? null,
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
    (workspace: WorkspaceInfo, prompt: string, agent?: string, model?: string) => {
      const thread = threads.find((t) => t.id === activeThread);
      if (thread && (thread.status === "complete" || thread.status === "error" || thread.status === "interrupted")) {
        handleResumeThread(workspace, thread.id, prompt);
      } else if (!thread) {
        handleStartThread(workspace, prompt, agent, model);
      }
    },
    [activeThread, threads, handleStartThread, handleResumeThread]
  );

  const handleRemoveWorkspace = useCallback(async (id: string) => {
    try { await invoke("remove_workspace", { workspaceId: id }); } catch {}
    setWorkspaces((prev) => prev.filter((w) => w.id !== id));
    setThreads((prev) => prev.filter((t) => t.workspaceId !== id));
    if (activeWorkspace === id) {
      setActiveWorkspace(null);
      setActiveThread(null);
      setActiveView("feed");
    }
  }, [activeWorkspace]);

  const handleDeleteThread = useCallback(async (id: string) => {
    try { await invoke("delete_thread", { threadId: id }); } catch {}
    setThreads((prev) => prev.filter((t) => t.id !== id));
    if (activeThread === id) {
      setActiveThread(null);
    }
  }, [activeThread]);

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
          loadThreadsForWorkspace(id);
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
        onRemoveWorkspace={handleRemoveWorkspace}
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
          onDeleteThread={handleDeleteThread}
        />
      )}

      <main className="main-panel">
        {activeView === "feed" && (
          <FeedView
            workspaces={workspaces}
            onNavigateToThread={(threadId, workspaceId) => {
              setActiveWorkspace(workspaceId);
              setActiveView("workspace");
              loadThreadsForWorkspace(workspaceId);
              setActiveThread(threadId);
            }}
          />
        )}

        {activeView === "workspace" && activeWs && (
          <ThreadView
            key={activeThread ?? "new"}
            workspace={activeWs}
            thread={currentThread ?? null}
            adapters={adapters}
            agents={agents}
            onStartThread={(prompt, agent, model) => handleSendPrompt(activeWs, prompt, agent, model)}
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
