import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { WorkspaceInfo } from "../App";
import { timeAgo, formatCost, truncatePrompt } from "../lib/utils";
import FluidBackground from "./FluidBackground";

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
}

interface FeedViewProps {
  workspaces: WorkspaceInfo[];
  onNavigateToThread: (threadId: string, workspaceId: string) => void;
}

export default function FeedView({
  workspaces,
  onNavigateToThread,
}: FeedViewProps) {
  const [threads, setThreads] = useState<BackendThread[]>([]);
  const [totalCost, setTotalCost] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([
      invoke<BackendThread[]>("list_all_threads", { limit: 100 }),
      invoke<number>("get_aggregate_cost"),
    ])
      .then(([t, cost]) => {
        setThreads(t);
        setTotalCost(cost);
        setLoaded(true);
      })
      .catch(() => {
        setError("Failed to load activity feed");
        setLoaded(true);
      });
  }, []);

  const outcomeClass = (status: string) => {
    if (status === "completed" || status === "complete") return "success";
    if (status === "error") return "error";
    if (status === "gate") return "gate";
    if (status === "interrupted") return "interrupted";
    return "interrupted";
  };

  const workspaceName = (wsId: string) =>
    workspaces.find((w) => w.id === wsId)?.name ?? "Unknown";

  if (!loaded) return <div className="panel-loading"><span className="spinner" /></div>;

  if (error) return <div className="inline-error"><span className="inline-error-icon">!</span>{error}</div>;

  if (threads.length === 0) {
    return (
      <div className="feed-empty">
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
        <span className="feed-aggregate-cost">Total spend: {formatCost(totalCost)}</span>
      </div>

      <div className="feed-list">
        {threads.map((thread) => (
          <div
            key={thread.id}
            className="feed-item"
            onClick={() => onNavigateToThread(thread.id, thread.workspaceId)}
          >
            <span className={`feed-item-outcome ${outcomeClass(thread.status)}`} />
            <div className="feed-item-body">
              <div className="feed-item-workspace">{workspaceName(thread.workspaceId)}</div>
              <div className="feed-item-prompt">{truncatePrompt(thread.prompt)}</div>
            </div>
            <div className="feed-item-meta">
              <span className="feed-item-cost">{formatCost(thread.costUsd)}</span>
              <span>{timeAgo(thread.createdAt)}</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
