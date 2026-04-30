import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { WorkspaceInfo } from "../App";

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
      .catch(() => setLoaded(true));
  }, []);

  const timeAgo = (iso: string) => {
    const seconds = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
    if (seconds < 60) return "just now";
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    return `${Math.floor(hours / 24)}d ago`;
  };

  const formatCost = (cost: number) =>
    cost < 0.01 ? `$${cost.toFixed(4)}` : `$${cost.toFixed(2)}`;

  const truncatePrompt = (text: string, max = 80) =>
    text.length > max ? text.substring(0, max) + "..." : text;

  const outcomeClass = (status: string) => {
    if (status === "completed" || status === "complete") return "success";
    if (status === "error") return "error";
    if (status === "gate") return "gate";
    if (status === "interrupted") return "interrupted";
    return "interrupted";
  };

  const workspaceName = (wsId: string) =>
    workspaces.find((w) => w.id === wsId)?.name ?? "Unknown";

  if (!loaded) return null;

  if (threads.length === 0) {
    return (
      <div className="feed-empty">
        <h2>No activity yet</h2>
        <p>
          Add a workspace and send a task to your AI agent. Completed threads
          from all workspaces will appear here.
        </p>
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
