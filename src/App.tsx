import { useState, useEffect, useCallback, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import Sidebar from "./components/Sidebar";
import ThreadList from "./components/ThreadList";
import ThreadView from "./components/ThreadView";
import MemoryPanel from "./components/MemoryPanel";
import FeedView from "./components/FeedView";
import DashboardView from "./components/DashboardView";
import SettingsPanel from "./components/SettingsPanel";
import RoutinePanel from "./components/RoutinePanel";
import WorkspaceValidatorsPanel from "./components/WorkspaceValidatorsPanel";
import { mapBackendEvent } from "./lib/eventMapper";
import { shouldFirePendingResume } from "./lib/pendingResume";
import { shouldExtractOnComplete } from "./lib/memoryExtractDedupe";
import { api } from "./lib/api";
import { AdapterCache, type AdapterCacheMap } from "./lib/adapterCache";
import { deriveConfig } from "./lib/deriveConfig";
import type { AgentEvent, WorkspaceInfo, AgentInfo, ModelInfo, ThreadInfo, ConfigPrefs, FeatureInfo, RoutineInfo, ValidatorTypeInfo } from "./types";

export type { AgentEvent, WorkspaceInfo, AgentInfo, ModelInfo, ThreadInfo, ConfigPrefs, FeatureInfo, RoutineInfo };

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
  const [activeView, setActiveView] = useState<"workspace" | "feed" | "memory" | "settings" | "routines" | "dashboard" | "validators">("dashboard");
  // Memory id to scroll to + highlight when the Memory panel opens.
  // Cleared once the MemoryPanel consumes it.
  const [memoryHighlightId, setMemoryHighlightId] = useState<string | null>(null);
  const [adapters, setAdapters] = useState<string[]>([]);
  // Per-adapter cache of agent + model lists. Keyed by adapter name so
  // switching workspaces or changing the Settings default adapter picks up
  // the right list without refetching (and, crucially, without showing the
  // previous adapter's list until the user pokes the ThreadView dropdown).
  const [adapterLists, setAdapterLists] = useState<AdapterCacheMap>(new Map());
  // Adapter currently being probed. Used for the "..." picker loading state.
  const [loadingAdapter, setLoadingAdapter] = useState<string | null>(null);
  const [features, setFeatures] = useState<FeatureInfo[]>([]);
  const [routines, setRoutines] = useState<RoutineInfo[]>([]);
  const [validatorTypes, setValidatorTypes] = useState<ValidatorTypeInfo[]>([]);
  const unlistenRef = useRef<UnlistenFn | null>(null);
  // In-session ThreadView picks, keyed by workspace id. State rather than
  // a ref: when ThreadView broadcasts onConfigChange, we need App to
  // re-render so derivedConfig picks up the new adapter. With a ref,
  // dropdown switches silently mutated memory and the app kept rendering
  // from the stale value — which is why switching back to an already-
  // cached adapter appeared to do nothing.
  const [wsConfig, setWsConfig] = useState<Map<string, ConfigPrefs>>(new Map());
  const globalConfigRef = useRef<ConfigPrefs>(DEFAULT_CONFIG);

  // Instantiate the cache once — it owns the cached/in-flight sets. React
  // state lives in setAdapterLists/setLoadingAdapter; the cache just drives
  // them. useRef + a lazy initializer would also work, but useState with
  // an initializer closes over the setters cleanly at construction time.
  const [adapterCache] = useState(() => new AdapterCache(
    {
      listAgents: (name) => api.listAgents(name) as Promise<AgentInfo[]>,
      listModels: (name) => api.listModels(name) as Promise<ModelInfo[]>,
    },
    {
      setLists: (updater) => setAdapterLists(updater),
      onLoadingStart: (adapter) => setLoadingAdapter(adapter),
      onLoadingEnd: (adapter) => setLoadingAdapter((cur) => (cur === adapter ? null : cur)),
    },
    FALLBACK_MODELS,
  ));

  const ensureAdapterLists = useCallback((adapter: string) => {
    adapterCache.ensure(adapter);
  }, [adapterCache]);

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
        isRoutine?: boolean;
        routineId?: string;
        injectedMemories?: import("./lib/api").MemoryInfo[];
        injectedBriefing?: string | null;
        extractedMemories?: import("./lib/api").MemoryInfo[];
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
            isRoutine: p.isRoutine,
            routineId: p.routineId,
            injectedMemories: p.injectedMemories,
            injectedBriefing: p.injectedBriefing ?? null,
            extractedMemories: p.extractedMemories,
            createdAt: new Date(p.createdAt).getTime(),
          }));
        return toAdd.length > 0 ? [...prev, ...toAdd] : prev;
      });
    } catch {}
  }, []);

  useEffect(() => {
    api.listWorkspaces().then((ws) => {
      setWorkspaces(ws);
      for (const w of ws) {
        loadThreadsForWorkspace(w.id);
      }
    }).catch(() => {});
    // Note: we don't pre-fetch lists for adapters[0] here — the
    // derivedAdapter effect below will fetch lists for the adapter of
    // whichever workspace is actually active. If the active workspace's
    // adapter is not adapters[0] (common for kiro-cli workspaces), the
    // old pre-fetch was pure waste.
    api.listAdapters().then(setAdapters).catch(() => {});
    api.getFeatures().then(setFeatures).catch(() => {});
    api.listValidatorTypes().then(setValidatorTypes).catch(() => {});
  }, [loadThreadsForWorkspace]);

  const handleCompletionAction = useCallback(
    (threadId: string, action: "committed" | "reverted" | "kept") => {
      setThreads((prev) =>
        prev.map((t) => {
          if (t.id !== threadId) return t;
          const idx = t.events.filter((e) => e.event_type === "complete").length - 1;
          const completionActions = { ...t.completionActions, [idx]: action };
          return { ...t, completionActions };
        })
      );
    },
    []
  );

  const handleAddDiffComment = useCallback(
    (threadId: string, completionIdx: number, comment: import("./types/diff").CommentThread) => {
      setThreads((prev) =>
        prev.map((t) => {
          if (t.id !== threadId) return t;
          const existing = t.diffComments?.[completionIdx] ?? [];
          return { ...t, diffComments: { ...t.diffComments, [completionIdx]: [...existing, comment] } };
        })
      );
    },
    []
  );

  const handleDiffViewChange = useCallback(
    (threadId: string, view: { completionIdx: number; activeFile?: string } | null) => {
      setThreads((prev) =>
        prev.map((t) =>
          t.id === threadId ? { ...t, activeDiffView: view ?? undefined } : t
        )
      );
    },
    []
  );

  const handleMarkFeedbackSent = useCallback(
    (threadId: string, completionIdx: number, commentCount: number) => {
      setThreads((prev) =>
        prev.map((t) =>
          t.id === threadId ? { ...t, feedbackSent: { ...t.feedbackSent, [completionIdx]: commentCount } } : t
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
  // Per-thread count of `complete` events we've already scheduled extraction
  // for in this session. Each new `complete` (e.g. a follow-up) bumps the
  // thread's count and triggers a fresh extraction that overwrites the
  // previous result. Reopening a completed thread from SQLite does NOT add
  // an entry here — we gate on whether the *next* complete goes beyond the
  // count of completes already present in the persisted event stream.
  const extractedCompletesRef = useRef<Map<string, number>>(new Map());

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
                  : mapped.event_type === "validation_result" && mapped.outcome === "fail"
                    ? "gate" as const
                    : "running" as const;

          if (newStatus === "complete" && mapped.event_type === "complete") {
            const decision = shouldExtractOnComplete(
              t.events,
              extractedCompletesRef.current.get(t.id) ?? 0,
              t.extractedMemories !== undefined,
            );
            if (decision.extract) {
              extractedCompletesRef.current.set(t.id, decision.newExtractedCount);
              toExtract.push({
                ...t,
                events: [...t.events, mapped],
              });
            }
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
    async (workspace: WorkspaceInfo, prompt: string, adapter?: string, agent?: string, model?: string) => {
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
          adapter: adapter || workspace.defaultAdapter || undefined,
          agent: agent || undefined,
          model: model ?? undefined,
        });

        const threadId = result.threadId;
        setThreads((prev) =>
          prev.map((t) =>
            t.id === tempId ? {
              ...t,
              id: threadId,
              status: "running",
              injectedMemories: result.injectedMemories,
              injectedBriefing: result.briefingPreview,
            } : t
          )
        );
        setActiveThread(threadId);
      } catch (e) {
        setThreads((prev) =>
          prev.map((t) =>
            t.id === tempId
              ? { ...t, status: "error", events: [{ event_type: "error", message: e instanceof Error ? e.message : typeof e === "string" ? e : (e as { message?: string })?.message ?? JSON.stringify(e) }] }
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
        // No `adapter`/`agent` here: resume_thread reads the adapter from
        // the DB row (thread.agent_type) — that's the adapter the session
        // was originally spawned with. Forwarding workspace.defaultAdapter
        // as `agent` was a long-standing mistake (different concept, wrong
        // field), and the backend's stored_agent fallback already masks it.
        // Send only what the resume actually needs.
        await api.resumeThread({
          threadId,
          workspaceId: workspace.id,
          workspacePath: workspace.path,
          workspaceName: workspace.name,
          prompt,
        });
      } catch (e) {
        setThreads((prev) =>
          prev.map((t) =>
            t.id === threadId
              ? {
                  ...t,
                  status: "error" as const,
                  events: [...t.events, { event_type: "error", message: e instanceof Error ? e.message : typeof e === "string" ? e : (e as { message?: string })?.message ?? JSON.stringify(e) }],
                }
              : t
          )
        );
      }
    },
    []
  );

  useEffect(() => {
    const target = shouldFirePendingResume(
      pendingResumeRef.current,
      threads,
      workspaces,
    );
    if (!target) return;
    pendingResumeRef.current = null;
    handleResumeThread(target.workspace, target.threadId, target.prompt);
  }, [threads, workspaces, handleResumeThread]);

  const handleSendPrompt = useCallback(
    (workspace: WorkspaceInfo, prompt: string, adapter?: string, agent?: string, model?: string) => {
      const thread = threads.find((t) => t.id === activeThread);
      if (thread && (thread.status === "complete" || thread.status === "error" || thread.status === "interrupted")) {
        handleResumeThread(workspace, thread.id, prompt);
      } else if (!thread) {
        handleStartThread(workspace, prompt, adapter, agent, model);
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

  const handleConfigChange = useCallback((workspaceId: string, config: ConfigPrefs) => {
    // Only commit when something actually changed to avoid a setState loop
    // with ThreadView's broadcast effect (which fires on every local
    // selection change — including the ones that arrive here).
    setWsConfig((prev) => {
      const existing = prev.get(workspaceId);
      if (
        existing &&
        existing.adapter === config.adapter &&
        existing.agent === config.agent &&
        existing.model === config.model
      ) {
        return prev;
      }
      const next = new Map(prev);
      next.set(workspaceId, config);
      return next;
    });
    globalConfigRef.current = config;
    if (config.adapter) {
      adapterCache.ensure(config.adapter);
    }
  }, [adapterCache]);

  const handleSetDefaultAdapter = useCallback(async (workspaceId: string, adapter: string) => {
    // Optimistic local update so the dropdown reflects the choice immediately;
    // revert on failure so the UI doesn't drift from persisted state.
    const prev = workspaces.find((w) => w.id === workspaceId)?.defaultAdapter;
    setWorkspaces((w) =>
      w.map((ws) => (ws.id === workspaceId ? { ...ws, defaultAdapter: adapter } : ws))
    );
    // Drop any in-session ThreadView pick for this workspace so the Settings
    // change — not the stale dropdown value — wins when we re-derive the
    // config next render. Without this, ThreadView would keep showing the
    // previous adapter's agents/models until the user clicked the dropdown.
    setWsConfig((prev) => {
      if (!prev.has(workspaceId)) return prev;
      const next = new Map(prev);
      next.delete(workspaceId);
      return next;
    });
    try {
      await api.setWorkspaceDefaultAdapter(workspaceId, adapter);
    } catch {
      setWorkspaces((w) =>
        w.map((ws) => (ws.id === workspaceId ? { ...ws, defaultAdapter: prev } : ws))
      );
    }
  }, [workspaces]);

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
    extractedCompletesRef.current.delete(id);
    if (activeThread === id) {
      setActiveThread(null);
    }
  }, [activeThread]);

  const extractMemoriesFromThread = useCallback((thread: ThreadInfo) => {
    // Build a transcript that interleaves user turns (initial prompt +
    // follow-ups) with assistant text in chronological order. This gives
    // the extractor on a multi-turn thread the context of what the user
    // actually asked in later turns, not just the opening prompt.
    const lines: string[] = [`User: ${thread.prompt}`];
    for (const e of thread.events) {
      if (e.event_type === "text" && e.text) {
        lines.push(`Assistant: ${e.text}`);
      } else if (e.event_type === "follow_up" && e.text) {
        lines.push(`User: ${e.text}`);
      }
    }
    const transcript = lines.join("\n");
    const threadId = thread.id;
    api.extractMemories(thread.workspaceId, threadId, transcript)
      .then((extracted) => {
        setThreads((prev) =>
          prev.map((t) =>
            t.id === threadId
              ? { ...t, extractedMemories: extracted, extractedMemoriesError: undefined }
              : t
          )
        );
      })
      .catch((e) => {
        const message = e instanceof Error ? e.message : String(e);
        console.error("extract_memories failed:", e);
        setThreads((prev) =>
          prev.map((t) =>
            t.id === threadId
              ? { ...t, extractedMemories: undefined, extractedMemoriesError: message }
              : t
          )
        );
      });
  }, []);

  const handleToggleFeature = useCallback(async (featureId: string, enabled: boolean) => {
    try {
      await api.setFeatureEnabled(featureId, enabled);
      setFeatures((prev) =>
        prev.map((f) => (f.id === featureId ? { ...f, enabled } : f))
      );
    } catch {}
  }, []);

  const routinesEnabled = features.some((f) => f.id === "routines" && f.enabled);
  const validatorsEnabled = features.some((f) => f.id === "validators" && f.enabled);
  const costTrackingEnabled = features.some((f) => f.id === "cost_tracking" && f.enabled);
  const routineCount = routines.filter((r) => r.enabled).length;

  useEffect(() => {
    if (!routinesEnabled || workspaces.length === 0) {
      setRoutines([]);
      return;
    }
    if (activeWorkspace) {
      api.listRoutines(activeWorkspace).then(setRoutines).catch(() => {});
    }
  }, [routinesEnabled, activeWorkspace, workspaces.length]);

  useEffect(() => {
    const unlisten = listen<{ title: string; body: string }>("panes://routine-notification", (ev) => {
      console.info("[routine notification]", ev.payload.title, ev.payload.body);
      if (activeWorkspace && routinesEnabled) {
        api.listRoutines(activeWorkspace).then(setRoutines).catch(() => {});
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [activeWorkspace, routinesEnabled]);

  const activeWs = workspaces.find((w) => w.id === activeWorkspace);
  const wsThreads = threads.filter((t) => t.workspaceId === activeWorkspace);
  const currentThread = threads.find((t) => t.id === activeThread);

  // Derive the adapter/config the active workspace should render with.
  // Precedence (see deriveConfig): ThreadView's in-session pick (wsConfig)
  // > persisted workspace default > global fallback. handleSetDefaultAdapter
  // clears wsConfig[workspaceId] so a Settings-driven change flows straight
  // through here without requiring a dropdown click.
  const wsPick = activeWs ? wsConfig.get(activeWs.id) : undefined;
  const derivedConfig: ConfigPrefs = deriveConfig(
    wsPick,
    activeWs?.defaultAdapter,
    globalConfigRef.current,
  );
  const derivedAdapter = derivedConfig.adapter;
  const cachedLists = adapterLists.get(derivedAdapter);
  // "..." in both pickers while this specific adapter is being probed
  // AND we have no cached list for it yet.
  const viewListsLoading = loadingAdapter === derivedAdapter && !cachedLists;
  const viewAgents = cachedLists?.agents ?? [];
  // During the loading window we pass [] so the model picker renders its
  // "Discovering models…" placeholder instead of briefly showing claude's
  // sonnet/opus/haiku under a kiro-cli adapter. Fallback still kicks in if
  // discovery settles with no result (backend error, adapter doesn't
  // expose a model list) so the picker is never permanently empty.
  const viewModels = cachedLists?.models ?? (viewListsLoading ? [] : FALLBACK_MODELS);

  useEffect(() => {
    if (derivedAdapter) ensureAdapterLists(derivedAdapter);
  }, [derivedAdapter, ensureAdapterLists]);

  return (
    <div className="app">
      <Sidebar
        workspaces={workspaces}
        threads={threads}
        activeWorkspace={activeWorkspace}
        activeView={activeView}
        routinesEnabled={routinesEnabled}
        validatorsEnabled={validatorsEnabled}
        routineCount={routineCount}
        showCost={costTrackingEnabled}
        onSelectWorkspace={(id) => {
          setActiveWorkspace(id);
          setActiveView("workspace");
          loadThreadsForWorkspace(id);
          if (routinesEnabled) {
            api.listRoutines(id).then(setRoutines).catch(() => {});
          }
          const lastThread = threads
            .filter((t) => t.workspaceId === id)
            .sort((a, b) => b.createdAt - a.createdAt)[0];
          setActiveThread(lastThread?.id ?? null);
        }}
        onSelectDashboard={() => {
          setActiveWorkspace(null);
          setActiveView("dashboard");
        }}
        onSelectFeed={() => {
          setActiveWorkspace(null);
          setActiveView("feed");
        }}
        onSelectMemory={(wsId) => {
          setActiveWorkspace(wsId);
          setActiveView("memory");
        }}
        onSelectRoutines={(wsId) => {
          setActiveWorkspace(wsId);
          setActiveView("routines");
        }}
        onSelectValidators={(wsId) => {
          setActiveWorkspace(wsId);
          setActiveView("validators");
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
        {activeView === "dashboard" && (
          <DashboardView
            workspaces={workspaces}
            threads={threads}
            showCost={costTrackingEnabled}
            onNavigateToWorkspace={(wsId) => {
              setActiveWorkspace(wsId);
              setActiveView("workspace");
              loadThreadsForWorkspace(wsId);
              const lastThread = threads
                .filter((t) => t.workspaceId === wsId)
                .sort((a, b) => b.createdAt - a.createdAt)[0];
              setActiveThread(lastThread?.id ?? null);
            }}
            onApproveGate={(threadId, toolUseId) => {
              api.approveGate(threadId, toolUseId).catch(console.error);
            }}
            onRejectGate={(threadId, toolUseId) => {
              api.rejectGate(threadId, toolUseId, "Rejected from dashboard").catch(console.error);
            }}
          />
        )}

        {activeView === "feed" && (
          <FeedView
            workspaces={workspaces}
            showCost={costTrackingEnabled}
            refreshKey={threads.filter(t => t.status === "complete" || t.status === "error" || t.status === "interrupted").length}
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
            agents={viewAgents}
            listsLoading={viewListsLoading}
            models={viewModels}
            validatorTypes={validatorTypes}
            defaultConfig={derivedConfig}
            onConfigChange={(config) => handleConfigChange(activeWs.id, config)}
            onStartThread={(prompt, adapter, agent, model) => handleSendPrompt(activeWs, prompt, adapter, agent, model)}
            onCompletionAction={handleCompletionAction}
            onCancel={handleCancelThread}
            onQueueFollowUp={handleQueueFollowUp}
            onResumeThread={(threadId, prompt) => handleResumeThread(activeWs, threadId, prompt)}
            onAddDiffComment={handleAddDiffComment}
            onDiffViewChange={handleDiffViewChange}
            onMarkFeedbackSent={handleMarkFeedbackSent}
            onSetBudgetCap={handleSetBudgetCap}
            onViewMemories={(memoryId) => {
              setMemoryHighlightId(memoryId ?? null);
              setActiveView("memory");
            }}
            showCost={costTrackingEnabled}
          />
        )}

        {activeView === "memory" && activeWs && (
          <MemoryPanel
            workspaceId={activeWs.id}
            highlightMemoryId={memoryHighlightId}
            onHighlightConsumed={() => setMemoryHighlightId(null)}
          />
        )}

        {activeView === "routines" && activeWs && (
          <RoutinePanel workspaceId={activeWs.id} onRoutinesChanged={setRoutines} />
        )}

        {activeView === "validators" && activeWs && (
          <WorkspaceValidatorsPanel
            workspaceId={activeWs.id}
            workspaceName={activeWs.name}
          />
        )}

        {activeView === "settings" && (
          <SettingsPanel
            workspaces={workspaces}
            features={features}
            onToggleFeature={handleToggleFeature}
            onSetDefaultAdapter={handleSetDefaultAdapter}
          />
        )}
      </main>
    </div>
  );
}

export default App;
