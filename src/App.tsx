import { useState, useEffect, useCallback, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import Sidebar from "./components/Sidebar";
import ThreadList from "./components/ThreadList";
import ThreadView from "./components/ThreadView";
import MemoryPanel from "./components/MemoryPanel";
import FeedView from "./components/FeedView";
import SettingsPanel from "./components/SettingsPanel";
import { mapBackendEvent } from "./lib/eventMapper";
import { api } from "./lib/api";
import type { AgentEvent, WorkspaceInfo, AgentInfo, ModelInfo, ThreadInfo, ConfigPrefs } from "./types";

export type { AgentEvent, WorkspaceInfo, AgentInfo, ModelInfo, ThreadInfo, ConfigPrefs };

const FALLBACK_MODELS: ModelInfo[] = [
  { id: "sonnet", label: "Sonnet", description: "Fast & capable" },
  { id: "opus", label: "Opus", description: "Most capable" },
  { id: "haiku", label: "Haiku", description: "Fastest" },
];

const DEFAULT_CONFIG: ConfigPrefs = { adapter: "claude-code", agent: "", model: "sonnet" };

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
  const [activeView, setActiveView] = useState<"workspace" | "feed" | "memory" | "settings">("feed");
  const [adapters, setAdapters] = useState<string[]>([]);
  const [agents, setAgents] = useState<AgentInfo[]>([]);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const unlistenRef = useRef<UnlistenFn | null>(null);
  const wsConfigRef = useRef<Map<string, ConfigPrefs>>(new Map());
  const globalConfigRef = useRef<ConfigPrefs>(DEFAULT_CONFIG);

  const loadThreadsForWorkspace = useCallback(async (workspaceId: string) => {
    try {
      const persisted = await api.listThreads(workspaceId) as {
        id: string;
        workspaceId: string;
        prompt: string;
        status: string;
        summary: string | null;
        costUsd: number;
        durationMs: number | null;
        createdAt: string;
        events: AgentEvent[];
      }[];

      setThreads((prev) => {
        const liveIds = new Set(prev.filter((t) => t.workspaceId === workspaceId).map((t) => t.id));
        const toAdd = persisted
          .filter((p) => !liveIds.has(p.id))
          .map((p) => ({
            id: p.id,
            workspaceId: p.workspaceId,
            prompt: p.prompt,
            status: (p.status === "completed" ? "complete" : p.status) as ThreadInfo["status"],
            costUsd: p.costUsd,
            events: p.events,
            createdAt: new Date(p.createdAt).getTime(),
          }));
        return toAdd.length > 0 ? [...prev, ...toAdd] : prev;
      });
    } catch {}
  }, []);

  useEffect(() => {
    api.listWorkspaces().then((ws) => {
      setWorkspaces(ws as WorkspaceInfo[]);
      for (const w of ws) {
        loadThreadsForWorkspace(w.id);
      }
    }).catch(() => {});
    api.listAdapters().then((a) => {
      setAdapters(a);
      if (a.length > 0) {
        api.listAgents(a[0]).then((ag) => setAgents(ag as AgentInfo[])).catch(() => {});
        api.listModels(a[0])
          .then((m) => { const models = m as ModelInfo[]; setModels(models.length > 0 ? models : FALLBACK_MODELS); })
          .catch(() => setModels(FALLBACK_MODELS));
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
        await api.cancelThread(threadId);
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

    const processEvent = (threadEvent: ThreadEvent) => {
      const { thread_id, event, parent_tool_use_id } = threadEvent;
      const mapped = mapBackendEvent(event);
      if (!mapped) return null;
      if (parent_tool_use_id) {
        (mapped as unknown as Record<string, unknown>).parent_tool_use_id = parent_tool_use_id;
      }
      return { thread_id, mapped };
    };

    const applyEvents = (prev: ThreadInfo[], items: { thread_id: string; mapped: AgentEvent }[]): ThreadInfo[] => {
      const toExtract: ThreadInfo[] = [];
      let updated = [...prev];
      for (const { thread_id, mapped } of items) {
        updated = updated.map((t) => {
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
            toExtract.push(t);
          }

          if ((newStatus === "complete" || newStatus === "error") && t.queuedFollowUp) {
            pendingResumeRef.current = { threadId: t.id, prompt: t.queuedFollowUp };
          }

          const costUsd = (newStatus === "complete" && mapped.event_type === "complete")
            ? (t.costUsd ?? 0) + mapped.total_cost_usd
            : t.costUsd;

          return {
            ...t,
            status: newStatus,
            costUsd,
            events: [...t.events, mapped],
            queuedFollowUp: (newStatus === "complete" || newStatus === "error") ? undefined : t.queuedFollowUp,
          };
        });
      }
      // Fire side effects outside the pure updater via microtask
      if (toExtract.length > 0) {
        queueMicrotask(() => toExtract.forEach(extractMemoriesFromThread));
      }
      return updated;
    };

    // Listen for batched events (new format)
    const p1 = listen<ThreadEvent[]>("panes://thread-events", (ev) => {
      if (cancelled) return;
      const batch = ev.payload;
      const processed = batch.map(processEvent).filter(Boolean) as { thread_id: string; mapped: AgentEvent }[];
      if (processed.length === 0) return;
      setThreads((prev) => applyEvents(prev, processed));
    });

    // Also listen for single events (backwards compat with mock/tests)
    const p2 = listen<ThreadEvent>("panes://thread-event", (ev) => {
      if (cancelled) return;
      const result = processEvent(ev.payload);
      if (!result) return;
      setThreads((prev) => applyEvents(prev, [result]));
    });

    Promise.all([p1, p2]).then(([u1, u2]) => {
      if (cancelled) { u1(); u2(); return; }
      unlistenRef.current = () => { u1(); u2(); };
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
        const result = await api.startThread({
          workspaceId: workspace.id,
          workspacePath: workspace.path,
          workspaceName: workspace.name,
          prompt,
          agent: agent || workspace.defaultAgent || undefined,
          model: model ?? undefined,
        });

        const threadId = result.threadId;
        setThreads((prev) =>
          prev.map((t) =>
            t.id === tempId ? { ...t, id: threadId, status: "running", memoryCount: result.memoryCount, hasBriefing: result.hasBriefing } : t
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
        await api.resumeThread({
          threadId,
          workspaceId: workspace.id,
          workspacePath: workspace.path,
          workspaceName: workspace.name,
          prompt,
          agent: workspace.defaultAgent || undefined,
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

  const handleSetBudgetCap = useCallback(async (workspaceId: string, budgetCap: number | null) => {
    try {
      await api.setWorkspaceBudgetCap(workspaceId, budgetCap);
      setWorkspaces((prev) =>
        prev.map((w) => (w.id === workspaceId ? { ...w, budgetCap } : w))
      );
    } catch {}
  }, []);

  const handleRemoveWorkspace = useCallback(async (id: string) => {
    try { await api.removeWorkspace(id); } catch {}
    setWorkspaces((prev) => prev.filter((w) => w.id !== id));
    setThreads((prev) => prev.filter((t) => t.workspaceId !== id));
    if (activeWorkspace === id) {
      setActiveWorkspace(null);
      setActiveThread(null);
      setActiveView("feed");
    }
  }, [activeWorkspace]);

  const handleDeleteThread = useCallback(async (id: string) => {
    try { await api.deleteThread(id); } catch {}
    setThreads((prev) => prev.filter((t) => t.id !== id));
    if (activeThread === id) {
      setActiveThread(null);
    }
  }, [activeThread]);

  const extractMemoriesFromThread = useCallback((thread: ThreadInfo) => {
    const textEvents = thread.events
      .filter((e): e is import("./types").TextEvent => e.event_type === "text" && !!e.text)
      .map((e) => `Assistant: ${e.text}`)
      .join("\n");
    const transcript = `User: ${thread.prompt}\n${textEvents}`;
    api.extractMemories(thread.workspaceId, thread.id, transcript)
      .catch((e) => console.error("extract_memories failed:", e));
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
        onSelectSettings={() => {
          setActiveWorkspace(null);
          setActiveView("settings");
        }}
        onRemoveWorkspace={handleRemoveWorkspace}
        onAddWorkspace={async (ws) => {
          try {
            const saved = await api.addWorkspace(ws.path, ws.name) as WorkspaceInfo;
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
            key={activeThread ?? `new-${activeWs.id}`}
            workspace={activeWs}
            thread={currentThread ?? null}
            adapters={adapters}
            agents={agents}
            models={models.length > 0 ? models : FALLBACK_MODELS}
            defaultConfig={wsConfigRef.current.get(activeWs.id) ?? globalConfigRef.current}
            onConfigChange={(config) => {
              wsConfigRef.current.set(activeWs.id, config);
              globalConfigRef.current = config;
            }}
            onStartThread={(prompt, agent, model) => handleSendPrompt(activeWs, prompt, agent, model)}
            onCompletionAction={handleCompletionAction}
            onCancel={handleCancelThread}
            onQueueFollowUp={handleQueueFollowUp}
            onSetBudgetCap={handleSetBudgetCap}
          />
        )}

        {activeView === "memory" && activeWs && (
          <MemoryPanel workspaceId={activeWs.id} />
        )}

        {activeView === "settings" && (
          <SettingsPanel workspaces={workspaces} />
        )}
      </main>
    </div>
  );
}

export default App;
