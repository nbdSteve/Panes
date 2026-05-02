import { useState, useEffect, useCallback } from "react";
import { api } from "../lib/api";
import type { RoutineInfo, RoutineExecution } from "../types";
import { formatCost } from "../lib/utils";
import RoutineForm from "./RoutineForm";

interface RoutinePanelProps {
  workspaceId: string;
}

function cronToHuman(cron: string): string {
  const parts = cron.split(/\s+/);
  if (parts.length < 5) return cron;
  const [min, hour, , , dow] = parts;

  const hourStr = hour === "*" ? "" : `${hour}:${min.padStart(2, "0")}`;
  if (dow === "1-5" && hourStr) return `Weekdays at ${hourStr}`;
  if (dow === "*" && hour === "*") return "Every hour";
  if (dow === "*" && hourStr) return `Daily at ${hourStr}`;
  if (dow === "1" && hourStr) return `Weekly Mon at ${hourStr}`;
  return cron;
}

function timeAgo(dateStr: string): string {
  const diff = Date.now() - new Date(dateStr).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

export default function RoutinePanel({ workspaceId }: RoutinePanelProps) {
  const [routines, setRoutines] = useState<RoutineInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [showForm, setShowForm] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [executions, setExecutions] = useState<Record<string, RoutineExecution[]>>({});
  const [costs, setCosts] = useState<Record<string, number>>({});
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  useEffect(() => {
    if (!confirmDeleteId) return;
    const timer = setTimeout(() => setConfirmDeleteId(null), 3000);
    return () => clearTimeout(timer);
  }, [confirmDeleteId]);

  const loadRoutines = useCallback(async () => {
    try {
      const data = await api.listRoutines(workspaceId);
      setRoutines(data);
    } catch {
      // routines feature may not be enabled yet
    } finally {
      setLoading(false);
    }
  }, [workspaceId]);

  useEffect(() => {
    loadRoutines();
  }, [loadRoutines]);

  const handleCreate = async (data: {
    prompt: string;
    cronExpr: string;
    budgetCap: number | null;
    onComplete: string;
    onFailure: string;
  }) => {
    try {
      await api.createRoutine({
        workspaceId,
        prompt: data.prompt,
        cronExpr: data.cronExpr,
        budgetCap: data.budgetCap,
        onComplete: data.onComplete,
        onFailure: data.onFailure,
      });
      setShowForm(false);
      await loadRoutines();
    } catch (err: unknown) {
      console.error("Failed to create routine:", err);
    }
  };

  const handleToggle = async (routineId: string, enabled: boolean) => {
    try {
      await api.toggleRoutine(routineId, enabled);
      setRoutines((prev) =>
        prev.map((r) => (r.id === routineId ? { ...r, enabled } : r))
      );
    } catch (err: unknown) {
      console.error("Failed to toggle routine:", err);
    }
  };

  const handleDelete = async (routineId: string) => {
    if (confirmDeleteId !== routineId) {
      setConfirmDeleteId(routineId);
      return;
    }
    setConfirmDeleteId(null);
    try {
      await api.deleteRoutine(routineId);
      setRoutines((prev) => prev.filter((r) => r.id !== routineId));
      if (expandedId === routineId) setExpandedId(null);
      setExecutions((prev) => { const next = { ...prev }; delete next[routineId]; return next; });
      setCosts((prev) => { const next = { ...prev }; delete next[routineId]; return next; });
    } catch (err: unknown) {
      console.error("Failed to delete routine:", err);
    }
  };

  const toggleExpand = async (routineId: string) => {
    if (expandedId === routineId) {
      setExpandedId(null);
      return;
    }
    setExpandedId(routineId);
    try {
      const [execs, cost] = await Promise.all([
        api.listRoutineExecutions(routineId, 20),
        api.getRoutineCost(routineId),
      ]);
      setExecutions((prev) => ({ ...prev, [routineId]: execs }));
      setCosts((prev) => ({ ...prev, [routineId]: cost }));
    } catch {
      // ignore
    }
  };

  if (loading) return <div className="panel-loading"><span className="spinner" /></div>;

  if (showForm) {
    return (
      <div className="routine-panel">
        <RoutineForm
          workspaceId={workspaceId}
          onSave={handleCreate}
          onCancel={() => setShowForm(false)}
        />
      </div>
    );
  }

  return (
    <div className="routine-panel">
      <div className="routine-panel-header">
        <h2>Routines</h2>
        <button className="btn btn-primary btn-sm" onClick={() => setShowForm(true)}>
          New Routine
        </button>
      </div>

      {routines.length === 0 ? (
        <div className="routine-empty">
          <p>No routines yet. Create one to schedule recurring agent tasks.</p>
        </div>
      ) : (
        <div className="routine-list">
          {routines.map((routine) => (
            <div key={routine.id} className={`routine-item ${!routine.enabled ? "disabled" : ""}`}>
              <div className="routine-item-header" onClick={() => toggleExpand(routine.id)}>
                <div className="routine-item-main">
                  <span className="routine-prompt">{routine.prompt.length > 80 ? routine.prompt.slice(0, 80) + "..." : routine.prompt}</span>
                  <div className="routine-meta">
                    <span className="routine-schedule">{cronToHuman(routine.cronExpr)}</span>
                    {routine.lastRunAt && (
                      <span className="routine-last-run">Last: {timeAgo(routine.lastRunAt)}</span>
                    )}
                    {routine.budgetCap && (
                      <span className="routine-budget">Cap: {formatCost(routine.budgetCap)}</span>
                    )}
                    {costs[routine.id] != null && costs[routine.id] > 0 && (
                      <span className="routine-cost">Spent: {formatCost(costs[routine.id])}</span>
                    )}
                  </div>
                </div>
                <div className="routine-item-actions">
                  <label className="toggle" onClick={(e) => e.stopPropagation()}>
                    <input
                      type="checkbox"
                      checked={routine.enabled}
                      onChange={(e) => handleToggle(routine.id, e.target.checked)}
                    />
                    <span className="toggle-slider" />
                  </label>
                  {confirmDeleteId === routine.id ? (
                    <button
                      className="btn btn-sm btn-danger"
                      onClick={(e) => { e.stopPropagation(); handleDelete(routine.id); }}
                    >
                      Confirm?
                    </button>
                  ) : (
                    <button
                      className="btn-icon btn-delete-inline"
                      onClick={(e) => { e.stopPropagation(); handleDelete(routine.id); }}
                      title="Delete routine"
                    >
                      <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <path d="M3 6h18"/><path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/><path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"/>
                      </svg>
                    </button>
                  )}
                </div>
              </div>

              {expandedId === routine.id && (
                <div className="routine-executions">
                  <h4>Execution History</h4>
                  {(executions[routine.id] ?? []).length === 0 ? (
                    <p className="muted">No executions yet</p>
                  ) : (
                    <div className="execution-list">
                      {(executions[routine.id] ?? []).map((exec) => (
                        <div key={exec.id} className={`execution-item status-${exec.status}`}>
                          <span className="execution-status">{exec.status.replace(/_/g, " ")}</span>
                          <span className="execution-time">{timeAgo(exec.startedAt)}</span>
                          {exec.costUsd > 0 && <span className="execution-cost">{formatCost(exec.costUsd)}</span>}
                          {exec.errorMessage && <span className="execution-error">{exec.errorMessage}</span>}
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
