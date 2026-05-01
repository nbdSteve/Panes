import { useState, useEffect, useCallback } from "react";
import { api, type MemoryBackendStatus } from "../lib/api";
import type { WorkspaceInfo } from "../App";
import { formatCost } from "../lib/utils";

interface SettingsPanelProps {
  workspaces: WorkspaceInfo[];
}

export default function SettingsPanel({ workspaces }: SettingsPanelProps) {
  const [backendStatus, setBackendStatus] = useState<MemoryBackendStatus | null>(null);
  const [totalCost, setTotalCost] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [switching, setSwitching] = useState(false);

  const loadStatus = useCallback(async () => {
    try {
      const status = await api.getMemoryBackendStatus();
      setBackendStatus(status);
    } catch {
      setError("Failed to load settings");
    }
  }, []);

  useEffect(() => {
    Promise.all([
      loadStatus(),
      api.getAggregateCost().then(setTotalCost).catch(() => {}),
    ]).finally(() => setLoading(false));
  }, [loadStatus]);

  const handleToggleBackend = async (backend: string) => {
    setSwitching(true);
    try {
      await api.setMemoryBackend(backend);
      await loadStatus();
    } catch {}
    setSwitching(false);
  };

  if (loading) return <div className="panel-loading"><span className="spinner" /></div>;

  if (error) return <div className="inline-error"><span className="inline-error-icon">!</span>{error}</div>;

  return (
    <div className="settings-panel">
      <h2 className="settings-title">
        Settings
      </h2>

      <div className="settings-section">
        <h3>Memory Backend</h3>
        {backendStatus ? (
          <>
            <div className="settings-row">
              <span className="settings-label">Active backend</span>
              <span className="settings-value">{backendStatus.backend === "mem0" ? "Mem0" : "SQLite"}</span>
            </div>
            {backendStatus.mem0Configured ? (
              <div className="settings-row">
                <span className="settings-label">Switch backend</span>
                <div className="backend-toggle">
                  <button
                    className={`btn btn-sm ${backendStatus.backend === "sqlite" ? "btn-primary" : "btn-secondary"}`}
                    onClick={() => handleToggleBackend("sqlite")}
                    disabled={switching || backendStatus.backend === "sqlite"}
                    title={switching ? "Switching backend..." : backendStatus.backend === "sqlite" ? "Already active" : undefined}
                  >
                    SQLite
                  </button>
                  <button
                    className={`btn btn-sm ${backendStatus.backend === "mem0" ? "btn-primary" : "btn-secondary"}`}
                    onClick={() => handleToggleBackend("mem0")}
                    disabled={switching || backendStatus.backend === "mem0"}
                    title={switching ? "Switching backend..." : backendStatus.backend === "mem0" ? "Already active" : undefined}
                  >
                    Mem0
                  </button>
                </div>
              </div>
            ) : (
              <div className="settings-row">
                <span className="settings-label settings-value muted">
                  Mem0 not configured. Set PANES_MEM0_PYTHON to enable.
                </span>
              </div>
            )}
          </>
        ) : (
          <div className="settings-row">
            <span className="settings-label settings-value muted">Unable to load backend status</span>
          </div>
        )}
      </div>

      {workspaces.length > 0 && (
        <div className="settings-section">
          <h3>Workspace Defaults</h3>
          {workspaces.map((ws) => (
            <div key={ws.id} className="settings-row">
              <span className="settings-label">{ws.name}</span>
              <div className="settings-ws-details">
                <span className="settings-value">
                  {ws.defaultAgent ?? "claude-code"}
                </span>
                <span className={`settings-value ${ws.budgetCap ? "" : "muted"}`}>
                  {ws.budgetCap ? `Cap: ${formatCost(ws.budgetCap)}` : "No cap"}
                </span>
              </div>
            </div>
          ))}
        </div>
      )}

      <div className="settings-section">
        <h3>About</h3>
        <div className="settings-row">
          <span className="settings-label">Total spend</span>
          <span className="settings-value">{formatCost(totalCost)}</span>
        </div>
        <div className="settings-row">
          <span className="settings-label">Workspaces</span>
          <span className="settings-value">{workspaces.length}</span>
        </div>
      </div>
    </div>
  );
}
