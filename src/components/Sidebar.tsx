import { useState, useMemo, useEffect } from "react";
import type { WorkspaceInfo, ThreadInfo } from "../App";
import { formatCost } from "../lib/utils";
import { workspaceDisplayCost } from "../lib/cost";
import appIcon from "../assets/icon.png";

interface SidebarProps {
  workspaces: WorkspaceInfo[];
  threads: ThreadInfo[];
  activeWorkspace: string | null;
  activeView: "workspace" | "feed" | "memory" | "settings";
  onSelectWorkspace: (id: string) => void;
  onSelectFeed: () => void;
  onSelectMemory: (workspaceId: string) => void;
  onSelectSettings: () => void;
  onAddWorkspace: (ws: WorkspaceInfo) => void;
  onRemoveWorkspace: (id: string) => void;
}

export default function Sidebar({
  workspaces,
  threads,
  activeWorkspace,
  activeView,
  onSelectWorkspace,
  onSelectFeed,
  onSelectMemory,
  onSelectSettings,
  onAddWorkspace,
  onRemoveWorkspace,
}: SidebarProps) {
  const [showAdd, setShowAdd] = useState(false);
  const [addPath, setAddPath] = useState("");
  const [addName, setAddName] = useState("");
  const [confirmRemoveId, setConfirmRemoveId] = useState<string | null>(null);

  useEffect(() => {
    if (!confirmRemoveId) return;
    const timer = setTimeout(() => setConfirmRemoveId(null), 3000);
    return () => clearTimeout(timer);
  }, [confirmRemoveId]);

  const handleRemove = (id: string) => {
    if (confirmRemoveId === id) {
      setConfirmRemoveId(null);
      onRemoveWorkspace(id);
    } else {
      setConfirmRemoveId(id);
    }
  };
  const workspaceCosts = useMemo(() => {
    const costs: Record<string, number> = {};
    for (const ws of workspaces) {
      const wsThreads = threads.filter((t) => t.workspaceId === ws.id);
      costs[ws.id] = workspaceDisplayCost(wsThreads);
    }
    return costs;
  }, [workspaces, threads]);

  const handleAdd = () => {
    if (!addPath.trim()) return;
    const name = addName.trim() || addPath.split("/").pop() || "workspace";
    onAddWorkspace({
      id: crypto.randomUUID(),
      path: addPath.trim(),
      name,
      defaultAgent: "claude-code",
    });
    setAddPath("");
    setAddName("");
    setShowAdd(false);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      handleAdd();
    }
    if (e.key === "Escape") {
      setShowAdd(false);
    }
  };

  return (
    <nav className="sidebar">
      <div className="sidebar-header">
        <img src={appIcon} alt="" className="sidebar-icon" />
        Panes
        <button
          className={`btn-icon sidebar-header-action ${activeView === "settings" ? "active" : ""}`}
          onClick={onSelectSettings}
          title="Settings"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
        </button>
      </div>

      <div className="sidebar-section">
        <div
          className={`sidebar-item ${activeView === "feed" ? "active" : ""}`}
          onClick={onSelectFeed}
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <path d="M4 11a9 9 0 0 1 9 9" /><path d="M4 4a16 16 0 0 1 16 16" /><circle cx="5" cy="19" r="1" />
          </svg>
          Feed
        </div>
      </div>

      {workspaces.length > 0 && (
        <div className="sidebar-section">
          <div className="sidebar-section-label">Workspaces</div>
          {workspaces.map((ws) => {
            const wsThreads = threads.filter((t) => t.workspaceId === ws.id);
            const hasGate = wsThreads.some((t) => t.status === "gate");
            const hasRunning = wsThreads.some((t) => t.status === "running" || t.status === "starting");
            const hasError = wsThreads.some((t) => t.status === "error");
            const dotClass = hasGate ? "gate" : hasRunning ? "working" : hasError ? "error" : wsThreads.length > 0 ? "complete" : "idle";
            const cost = workspaceCosts[ws.id] ?? 0;
            return (
              <div
                key={ws.id}
                className={`sidebar-item ${activeWorkspace === ws.id ? "active" : ""}`}
                onClick={() => onSelectWorkspace(ws.id)}
              >
                <span className={`status-dot ${dotClass}`} />
                <span>{ws.name}</span>
                {cost > 0 && (
                  <span className="workspace-cost">{formatCost(cost)}</span>
                )}
                {wsThreads.length > 0 && (
                  <span className="thread-count">{wsThreads.length}</span>
                )}
                {confirmRemoveId === ws.id ? (
                  <button
                    className="btn btn-sm btn-danger"
                    onClick={(e) => { e.stopPropagation(); handleRemove(ws.id); }}
                  >
                    Confirm?
                  </button>
                ) : (
                  <button
                    className="btn-icon btn-delete-inline"
                    onClick={(e) => { e.stopPropagation(); handleRemove(ws.id); }}
                    title="Remove workspace"
                  >
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/>
                    </svg>
                  </button>
                )}
              </div>
            );
          })}
        </div>
      )}

      {activeWorkspace && (
        <div className="sidebar-section">
          <div
            className={`sidebar-item ${activeView === "memory" ? "active" : ""}`}
            onClick={() => onSelectMemory(activeWorkspace)}
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z" />
              <path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z" />
            </svg>
            Memory
          </div>
        </div>
      )}

      <div className="sidebar-footer">
        {showAdd ? (
          <div className="sidebar-add-form">
            <input
              className="input"
              type="text"
              placeholder="/path/to/project"
              value={addPath}
              onChange={(e) => setAddPath(e.target.value)}
              onKeyDown={handleKeyDown}
              autoFocus
            />
            <input
              className="input"
              type="text"
              placeholder="Display name (optional)"
              value={addName}
              onChange={(e) => setAddName(e.target.value)}
              onKeyDown={handleKeyDown}
            />
            <div className="sidebar-add-actions">
              <button className="btn btn-primary btn-sm" onClick={handleAdd}>
                Add
              </button>
              <button
                className="btn btn-secondary btn-sm"
                onClick={() => setShowAdd(false)}
              >
                Cancel
              </button>
            </div>
          </div>
        ) : (
          <button
            className="btn btn-secondary btn-block"
            onClick={() => setShowAdd(true)}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round">
              <path d="M12 5v14" /><path d="M5 12h14" />
            </svg>
            Add workspace
          </button>
        )}
      </div>
    </nav>
  );
}
