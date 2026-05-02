import { useState } from "react";

interface RoutineFormProps {
  workspaceId: string;
  onSave: (data: {
    prompt: string;
    cronExpr: string;
    budgetCap: number | null;
    onComplete: string;
    onFailure: string;
  }) => void;
  onCancel: () => void;
}

const SCHEDULE_PRESETS = [
  { label: "Weekdays 9am", cron: "0 9 * * 1-5" },
  { label: "Daily midnight", cron: "0 0 * * *" },
  { label: "Weekly Mon 9am", cron: "0 9 * * 1" },
  { label: "Hourly", cron: "0 * * * *" },
];

export default function RoutineForm({ onSave, onCancel }: RoutineFormProps) {
  const [prompt, setPrompt] = useState("");
  const [cronExpr, setCronExpr] = useState("0 9 * * 1-5");
  const [customCron, setCustomCron] = useState(false);
  const [budgetCap, setBudgetCap] = useState("");
  const [onComplete, setOnComplete] = useState("notify");
  const [onFailure, setOnFailure] = useState("notify");
  const [chainPrompt, setChainPrompt] = useState("");
  const [chainFailPrompt, setChainFailPrompt] = useState("");
  const [error, setError] = useState<string | null>(null);

  const buildAction = (actionType: string, chainText: string): string => {
    if (actionType === "chain" && chainText.trim()) {
      return JSON.stringify({ action: "chain", prompt: chainText.trim(), workspace_id: null });
    }
    if (actionType === "retry_once") return JSON.stringify({ action: "retry_once" });
    return JSON.stringify({ action: "notify" });
  };

  const handleSubmit = () => {
    if (!prompt.trim()) {
      setError("Prompt is required");
      return;
    }
    if (!cronExpr.trim()) {
      setError("Schedule is required");
      return;
    }
    setError(null);
    onSave({
      prompt: prompt.trim(),
      cronExpr: cronExpr.trim(),
      budgetCap: budgetCap ? parseFloat(budgetCap) : null,
      onComplete: buildAction(onComplete, chainPrompt),
      onFailure: buildAction(onFailure, chainFailPrompt),
    });
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") onCancel();
  };

  return (
    <div className="routine-form" onKeyDown={handleKeyDown}>
      <h3>New Routine</h3>

      {error && <div className="inline-error"><span className="inline-error-icon">!</span>{error}</div>}

      <div className="form-field">
        <label>Prompt</label>
        <textarea
          className="input routine-prompt-input"
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder="Check for dependency updates..."
          rows={3}
          autoFocus
        />
      </div>

      <div className="form-field">
        <label>Schedule</label>
        {!customCron ? (
          <div className="schedule-presets">
            {SCHEDULE_PRESETS.map((p) => (
              <button
                key={p.cron}
                className={`btn btn-sm ${cronExpr === p.cron ? "btn-primary" : "btn-secondary"}`}
                onClick={() => setCronExpr(p.cron)}
              >
                {p.label}
              </button>
            ))}
            <button
              className="btn btn-sm btn-secondary"
              onClick={() => setCustomCron(true)}
            >
              Custom
            </button>
          </div>
        ) : (
          <div className="schedule-custom">
            <input
              className="input"
              type="text"
              value={cronExpr}
              onChange={(e) => setCronExpr(e.target.value)}
              placeholder="0 9 * * 1-5"
            />
            <button className="btn btn-sm btn-secondary" onClick={() => setCustomCron(false)}>
              Presets
            </button>
          </div>
        )}
        <span className="form-hint">Cron expression (min hour day month weekday)</span>
      </div>

      <div className="form-field">
        <label>Budget cap per run</label>
        <div className="budget-input">
          <span className="budget-prefix">$</span>
          <input
            className="input"
            type="number"
            step="0.50"
            min="0"
            value={budgetCap}
            onChange={(e) => setBudgetCap(e.target.value)}
            placeholder="No limit"
          />
        </div>
      </div>

      <div className="form-field">
        <label>On completion</label>
        <select className="input" value={onComplete} onChange={(e) => setOnComplete(e.target.value)}>
          <option value="notify">Notify</option>
          <option value="retry_once">Retry once</option>
          <option value="chain">Chain prompt</option>
        </select>
        {onComplete === "chain" && (
          <textarea
            className="input"
            value={chainPrompt}
            onChange={(e) => setChainPrompt(e.target.value)}
            placeholder="Follow-up prompt..."
            rows={2}
          />
        )}
      </div>

      <div className="form-field">
        <label>On failure</label>
        <select className="input" value={onFailure} onChange={(e) => setOnFailure(e.target.value)}>
          <option value="notify">Notify</option>
          <option value="retry_once">Retry once</option>
          <option value="chain">Chain prompt</option>
        </select>
        {onFailure === "chain" && (
          <textarea
            className="input"
            value={chainFailPrompt}
            onChange={(e) => setChainFailPrompt(e.target.value)}
            placeholder="Follow-up prompt on failure..."
            rows={2}
          />
        )}
      </div>

      <div className="form-actions">
        <button className="btn btn-primary" onClick={handleSubmit}>
          Create Routine
        </button>
        <button className="btn btn-secondary" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </div>
  );
}
