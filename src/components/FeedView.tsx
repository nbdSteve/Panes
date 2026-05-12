import { useState, useEffect, useMemo } from "react";
import { api } from "../lib/api";
import type { WorkspaceInfo } from "../App";
import { timeAgo, formatCost, truncatePrompt } from "../lib/utils";
import FluidBackground from "./FluidBackground";
import RoutineBadge from "./RoutineBadge";
import CostSparkline from "./CostSparkline";
import WorkspaceCostBars from "./WorkspaceCostBars";

interface BackendThread {
  id: string;
  workspaceId: string;
  prompt: string;
  status: string;
  summary: string | null;
  costUsd: number;
  durationMs: number | null;
  createdAt: string;
  events: unknown[];
  isRoutine?: boolean;
  routineId?: string;
}

interface FeedViewProps {
  workspaces: WorkspaceInfo[];
  showCost?: boolean;
  refreshKey?: number;
  onNavigateToThread: (threadId: string, workspaceId: string) => void;
}

type SortBy = "date" | "cost" | "workspace";
type DateRange = 7 | 30 | 90 | null;

export default function FeedView({
  workspaces,
  showCost,
  refreshKey,
  onNavigateToThread,
}: FeedViewProps) {
  const [threads, setThreads] = useState<BackendThread[]>([]);
  const [totalCost, setTotalCost] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sortBy, setSortBy] = useState<SortBy>("date");
  const [dateRange, setDateRange] = useState<DateRange>(null);
  const [analyticsOpen, setAnalyticsOpen] = useState(true);
  const [timeline, setTimeline] = useState<{ day: string; totalUsd: number }[]>([]);
  const [breakdown, setBreakdown] = useState<{ workspaceName: string; totalUsd: number }[]>([]);

  useEffect(() => {
    Promise.all([
      api.listAllThreads(100) as Promise<BackendThread[]>,
      api.getAggregateCost(),
      api.getCostTimeline(30),
      api.getWorkspaceCostBreakdown(),
    ])
      .then(([t, cost, tl, bd]) => {
        setThreads(t);
        setTotalCost(cost);
        setTimeline(tl);
        setBreakdown(bd.map(b => ({ workspaceName: b.workspaceName, totalUsd: b.totalUsd })));
        setLoaded(true);
      })
      .catch(() => {
        setError("Failed to load activity feed");
        setLoaded(true);
      });
  }, [refreshKey]);

  const filteredThreads = useMemo(() => {
    let result = [...threads];
    if (dateRange) {
      const cutoff = Date.now() - dateRange * 86400000;
      result = result.filter(t => new Date(t.createdAt).getTime() >= cutoff);
    }
    switch (sortBy) {
      case "cost":
        result.sort((a, b) => b.costUsd - a.costUsd);
        break;
      case "workspace":
        result.sort((a, b) => {
          const nameA = workspaces.find(w => w.id === a.workspaceId)?.name ?? "";
          const nameB = workspaces.find(w => w.id === b.workspaceId)?.name ?? "";
          return nameA.localeCompare(nameB);
        });
        break;
      default:
        result.sort((a, b) => new Date(b.createdAt).getTime() - new Date(a.createdAt).getTime());
    }
    return result;
  }, [threads, sortBy, dateRange, workspaces]);

  const outcomeClass = (status: string) => {
    if (status === "completed" || status === "complete") return "success";
    if (status === "error") return "error";
    if (status === "gate") return "gate";
    return "interrupted";
  };

  const workspaceName = (wsId: string) =>
    workspaces.find((w) => w.id === wsId)?.name ?? "Unknown";

  if (!loaded) return <div className="panel-loading"><span className="spinner" /></div>;

  if (error) return <div className="inline-error"><span className="inline-error-icon">!</span>{error}</div>;

  // Empty state is still wrapped in `.feed-view` so callers (and E2E
  // tests) that assert on the panel's presence don't have to special-
  // case the empty-list branch. `.feed-empty` remains as a modifier so
  // the existing styling keeps working.
  if (threads.length === 0) {
    return (
      <div className="feed-view feed-empty">
        <FluidBackground />
        <div className="feed-empty-content">
          <h2>No activity yet</h2>
          <p>
            Add a workspace and send a task to your AI agent. Completed threads
            from all workspaces will appear here.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="feed-view">
      <div className="feed-aggregate">
        <span>Activity across {workspaces.length} workspace{workspaces.length !== 1 ? "s" : ""}</span>
        {showCost !== false && <span className="feed-aggregate-cost">Total spend: {formatCost(totalCost)}</span>}
      </div>

      {showCost !== false && (
        <div className="feed-analytics">
          <button
            className="feed-analytics-toggle"
            onClick={() => setAnalyticsOpen(!analyticsOpen)}
          >
            {analyticsOpen ? "Hide" : "Show"} Analytics
          </button>
          {analyticsOpen && (
            <div className="feed-analytics-content">
              <div className="feed-analytics-section">
                <div className="feed-analytics-label">Daily Spend (30d)</div>
                <CostSparkline data={timeline} />
              </div>
              {breakdown.length > 0 && (
                <div className="feed-analytics-section">
                  <div className="feed-analytics-label">By Workspace</div>
                  <WorkspaceCostBars data={breakdown} />
                </div>
              )}
            </div>
          )}
        </div>
      )}

      <div className="feed-controls">
        <div className="feed-sort">
          {(["date", "cost", "workspace"] as SortBy[]).map(s => (
            <button
              key={s}
              className={`feed-filter-btn ${sortBy === s ? "active" : ""}`}
              onClick={() => setSortBy(s)}
            >
              {s.charAt(0).toUpperCase() + s.slice(1)}
            </button>
          ))}
        </div>
        <div className="feed-date-range">
          {([7, 30, 90, null] as DateRange[]).map(d => (
            <button
              key={d ?? "all"}
              className={`feed-filter-btn ${dateRange === d ? "active" : ""}`}
              onClick={() => setDateRange(d)}
            >
              {d ? `${d}d` : "All"}
            </button>
          ))}
        </div>
      </div>

      <div className="feed-list">
        {filteredThreads.map((thread) => (
          <div
            key={thread.id}
            className="feed-item"
            onClick={() => onNavigateToThread(thread.id, thread.workspaceId)}
          >
            <span className={`feed-item-outcome ${outcomeClass(thread.status)}`} />
            <div className="feed-item-body">
              <div className="feed-item-workspace">
                {workspaceName(thread.workspaceId)}
                {thread.isRoutine && <RoutineBadge />}
              </div>
              <div className="feed-item-prompt">{truncatePrompt(thread.prompt)}</div>
            </div>
            <div className="feed-item-meta">
              {showCost !== false && <span className="feed-item-cost">{formatCost(thread.costUsd)}</span>}
              <span>{timeAgo(thread.createdAt)}</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
