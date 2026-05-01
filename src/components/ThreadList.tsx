import { useState, useEffect } from "react";
import type { ThreadInfo } from "../App";

interface ThreadListProps {
  threads: ThreadInfo[];
  activeThread: string | null;
  onSelectThread: (id: string) => void;
  onNewThread: () => void;
  onDeleteThread: (id: string) => void;
}

export default function ThreadList({
  threads,
  activeThread,
  onSelectThread,
  onNewThread,
  onDeleteThread,
}: ThreadListProps) {
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  useEffect(() => {
    if (!confirmDeleteId) return;
    const timer = setTimeout(() => setConfirmDeleteId(null), 3000);
    return () => clearTimeout(timer);
  }, [confirmDeleteId]);

  const handleDelete = (id: string) => {
    if (confirmDeleteId === id) {
      setConfirmDeleteId(null);
      onDeleteThread(id);
    } else {
      setConfirmDeleteId(id);
    }
  };

  const sorted = [...threads].sort((a, b) => b.createdAt - a.createdAt);

  const statusDot = (status: ThreadInfo["status"]) => {
    switch (status) {
      case "starting":
      case "running":
        return "working";
      case "error":
        return "error";
      case "complete":
        return "complete";
      default:
        return "idle";
    }
  };

  const truncatePrompt = (text: string, max = 50) =>
    text.length > max ? text.substring(0, max) + "..." : text;

  const timeAgo = (ts: number) => {
    const seconds = Math.floor((Date.now() - ts) / 1000);
    if (seconds < 60) return "just now";
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m ago`;
    const hours = Math.floor(minutes / 60);
    if (hours < 24) return `${hours}h ago`;
    return `${Math.floor(hours / 24)}d ago`;
  };

  return (
    <div className="thread-list">
      <div className="thread-list-header">
        <span className="thread-list-title">Threads</span>
        <button className="thread-list-new" onClick={onNewThread} title="New thread">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round">
            <path d="M12 5v14" /><path d="M5 12h14" />
          </svg>
        </button>
      </div>

      <div className="thread-list-items">
        {sorted.length === 0 && (
          <div className="thread-list-empty">No threads yet</div>
        )}

        {sorted.map((thread) => (
          <div
            key={thread.id}
            className={`thread-list-item ${activeThread === thread.id ? "active" : ""}`}
            onClick={() => onSelectThread(thread.id)}
          >
            <span className={`thread-dot ${statusDot(thread.status)}`} />
            <div className="thread-list-item-content">
              <span className="thread-list-item-prompt">
                {truncatePrompt(thread.prompt)}
              </span>
              <span className="thread-list-item-meta">
                {timeAgo(thread.createdAt)}
              </span>
            </div>
            {confirmDeleteId === thread.id ? (
              <button
                className="btn btn-sm btn-danger"
                onClick={(e) => { e.stopPropagation(); handleDelete(thread.id); }}
              >
                Confirm?
              </button>
            ) : (
              <button
                className="btn-icon btn-delete-inline"
                onClick={(e) => { e.stopPropagation(); handleDelete(thread.id); }}
                title="Delete thread"
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/>
                </svg>
              </button>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
