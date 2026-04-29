import { useState } from "react";
import type { WorkspaceInfo, ThreadInfo } from "../App";

interface SidebarProps {
  workspaces: WorkspaceInfo[];
  threads: ThreadInfo[];
  activeWorkspace: string | null;
  onSelectWorkspace: (id: string) => void;
  onSelectFeed: () => void;
  onAddWorkspace: (ws: WorkspaceInfo) => void;
}

export default function Sidebar({
  workspaces,
  threads,
  activeWorkspace,
  onSelectWorkspace,
  onSelectFeed,
  onAddWorkspace,
}: SidebarProps) {
  const [showAdd, setShowAdd] = useState(false);
  const [addPath, setAddPath] = useState("");
  const [addName, setAddName] = useState("");

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
      <div className="sidebar-header">Panes</div>

      <div className="sidebar-section">
        <div
          className={`sidebar-item ${activeWorkspace === null ? "active" : ""}`}
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
            return (
              <div
                key={ws.id}
                className={`sidebar-item ${activeWorkspace === ws.id ? "active" : ""}`}
                onClick={() => onSelectWorkspace(ws.id)}
              >
                <span className={`status-dot ${dotClass}`} />
                <span>{ws.name}</span>
                {wsThreads.length > 0 && (
                  <span className="thread-count">{wsThreads.length}</span>
                )}
              </div>
            );
          })}
        </div>
      )}

      <div className="sidebar-footer">
        {showAdd ? (
          <div style={{ display: "flex", flexDirection: "column", gap: "8px" }}>
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
            <div style={{ display: "flex", gap: "6px" }}>
              <button className="btn btn-primary btn-sm" style={{ flex: 1 }} onClick={handleAdd}>
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
