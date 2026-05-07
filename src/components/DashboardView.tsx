import { useState, useEffect, useRef } from "react";
import type { WorkspaceInfo, ThreadInfo, ToolRequestEvent } from "../types";
import { threadDisplayCost } from "../lib/cost";
import { formatCost } from "../lib/utils";
import CostBadge from "./CostBadge";
import { api } from "../lib/api";

interface DashboardViewProps {
  workspaces: WorkspaceInfo[];
  threads: ThreadInfo[];
  showCost: boolean;
  onNavigateToWorkspace: (workspaceId: string) => void;
  onApproveGate: (threadId: string, toolUseId: string) => void;
  onRejectGate: (threadId: string, toolUseId: string) => void;
}

type WorkspaceStatus = "idle" | "working" | "gate" | "error";

function deriveStatus(wsThreads: ThreadInfo[]): WorkspaceStatus {
  if (wsThreads.some(t => t.status === "gate")) return "gate";
  if (wsThreads.some(t => t.status === "running" || t.status === "starting")) return "working";
  const mostRecent = wsThreads.sort((a, b) => b.createdAt - a.createdAt)[0];
  if (mostRecent?.status === "error") return "error";
  return "idle";
}

function formatDuration(createdAt: number): string {
  const seconds = Math.floor((Date.now() - createdAt) / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${seconds % 60}s`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
}

function getGateToolUseId(thread: ThreadInfo): string | null {
  for (let i = thread.events.length - 1; i >= 0; i--) {
    const ev = thread.events[i];
    if (ev.event_type === "tool_request" && (ev as ToolRequestEvent).needs_approval) {
      return (ev as ToolRequestEvent).id;
    }
  }
  return null;
}

export default function DashboardView({
  workspaces,
  threads,
  showCost,
  onNavigateToWorkspace,
  onApproveGate,
  onRejectGate,
}: DashboardViewProps) {
  const [aggregateCost, setAggregateCost] = useState(0);
  const [, setTick] = useState(0);
  const hasActive = threads.some(t => t.status === "running" || t.status === "starting" || t.status === "gate");
  const tickRef = useRef<ReturnType<typeof setInterval> | null>(null);

  useEffect(() => {
    if (hasActive) {
      tickRef.current = setInterval(() => setTick(t => t + 1), 1000);
    }
    return () => { if (tickRef.current) clearInterval(tickRef.current); };
  }, [hasActive]);

  const completedCount = threads.filter(t => t.status === "complete" || t.status === "error" || t.status === "interrupted").length;
  useEffect(() => {
    api.getAggregateCost().then(setAggregateCost).catch(() => {});
  }, [completedCount]);

  const activeThreads = threads.filter(
    t => t.status === "running" || t.status === "starting" || t.status === "gate"
  );
  const needsAttention = workspaces.filter(ws => {
    const wsThreads = threads.filter(t => t.workspaceId === ws.id);
    const status = deriveStatus(wsThreads);
    return status === "gate" || status === "error";
  });

  return (
    <div className="dashboard-view">
      <div className="dashboard-summary">
        <span className="dashboard-stat">
          <strong>{activeThreads.length}</strong> active
        </span>
        {needsAttention.length > 0 && (
          <span className="dashboard-stat dashboard-stat-attention">
            <strong>{needsAttention.length}</strong> needs attention
          </span>
        )}
        {showCost && (
          <span className="dashboard-stat">
            Total: <strong>{formatCost(aggregateCost)}</strong>
          </span>
        )}
      </div>

      {workspaces.length === 0 ? (
        <div className="dashboard-empty">
          Add a workspace to get started.
        </div>
      ) : (
        <div className="dashboard-grid">
          {workspaces.map(ws => {
            const wsThreads = threads.filter(t => t.workspaceId === ws.id);
            const status = deriveStatus(wsThreads);
            const activeThread = wsThreads.find(
              t => t.status === "running" || t.status === "starting" || t.status === "gate"
            );

            return (
              <div
                key={ws.id}
                className={`dashboard-card dashboard-card-${status}`}
                onClick={() => onNavigateToWorkspace(ws.id)}
              >
                <div className="dashboard-card-header">
                  <span className={`status-dot ${status}`} />
                  <span className="dashboard-card-name">{ws.name}</span>
                  <span className="dashboard-card-status">{status}</span>
                </div>

                {activeThread ? (
                  <div className="dashboard-card-active">
                    <div className="dashboard-card-prompt">
                      {activeThread.prompt.slice(0, 60)}
                      {activeThread.prompt.length > 60 ? "..." : ""}
                    </div>
                    <div className="dashboard-card-meta">
                      <span>{formatDuration(activeThread.createdAt)}</span>
                      {showCost && (
                        <CostBadge
                          cost={threadDisplayCost(activeThread)}
                          budgetCap={ws.budgetCap ?? undefined}
                        />
                      )}
                    </div>
                    {activeThread.status === "gate" && (
                      <div className="dashboard-gate-actions" onClick={e => e.stopPropagation()}>
                        <button
                          className="btn btn-sm btn-approve"
                          onClick={() => {
                            const toolUseId = getGateToolUseId(activeThread);
                            if (toolUseId) onApproveGate(activeThread.id, toolUseId);
                          }}
                        >
                          Continue
                        </button>
                        <button
                          className="btn btn-sm btn-reject"
                          onClick={() => {
                            const toolUseId = getGateToolUseId(activeThread);
                            if (toolUseId) onRejectGate(activeThread.id, toolUseId);
                          }}
                        >
                          Abort
                        </button>
                      </div>
                    )}
                  </div>
                ) : (
                  <div className="dashboard-idle-text">Idle</div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
