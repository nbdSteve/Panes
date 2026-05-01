import { useState } from "react";
import CostBadge from "./CostBadge";

interface GateCardProps {
  description: string;
  riskLevel: string;
  toolUseId: string;
  toolName: string;
  runningCost: number;
  resolved?: "approved" | "rejected" | "steered";
  onApprove: () => void;
  onReject: () => void;
  onSteer?: (text: string) => void;
}

export default function GateCard({
  description,
  riskLevel,
  toolName,
  runningCost,
  resolved,
  onApprove,
  onReject,
  onSteer,
}: GateCardProps) {
  const [steerMode, setSteerMode] = useState(false);
  const [steerText, setSteerText] = useState("");
  if (resolved === "approved") {
    return (
      <div className="card gate-card gate-resolved gate-approved">
        <div className="gate-resolved-row">
          <span className="gate-resolved-icon approved">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
              <polyline points="20 6 9 17 4 12" />
            </svg>
          </span>
          <span className="gate-resolved-text">
            <span className="gate-resolved-label">Continued</span>
            <span className="gate-resolved-desc">
              {toolName && <span className="gate-resolved-tool">{toolName}</span>}
              {description}
            </span>
          </span>
        </div>
      </div>
    );
  }

  if (resolved === "rejected") {
    return (
      <div className="card gate-card gate-resolved gate-rejected">
        <div className="gate-resolved-row">
          <span className="gate-resolved-icon rejected">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
              <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </span>
          <span className="gate-resolved-text">
            <span className="gate-resolved-label">Aborted</span>
            <span className="gate-resolved-desc">
              {toolName && <span className="gate-resolved-tool">{toolName}</span>}
              {description}
            </span>
          </span>
        </div>
      </div>
    );
  }

  if (resolved === "steered") {
    return (
      <div className="card gate-card gate-resolved gate-steered">
        <div className="gate-resolved-row">
          <span className="gate-resolved-icon steered">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
              <polyline points="15 10 20 15 15 20" /><path d="M4 4v7a4 4 0 0 0 4 4h12" />
            </svg>
          </span>
          <span className="gate-resolved-text">
            <span className="gate-resolved-label">Steered</span>
            <span className="gate-resolved-desc">
              {toolName && <span className="gate-resolved-tool">{toolName}</span>}
              {description}
            </span>
          </span>
        </div>
      </div>
    );
  }

  return (
    <div className="card gate-card">
      <div className="gate-label">
        <span className="gate-label-icon">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
            <path d="M12 9v4" /><path d="M12 17h.01" />
            <path d="M10.29 3.86L1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0z" />
          </svg>
        </span>
        <span className="gate-label-text">Approval needed</span>
      </div>

      <div className="gate-description">
        {toolName && (
          <span style={{ fontFamily: "'SF Mono', 'Fira Code', monospace", fontSize: 12, color: "var(--warning)", marginRight: 6 }}>
            {toolName}
          </span>
        )}
        {description}
      </div>

      <div className="gate-meta">
        <span className={`risk-badge ${riskLevel}`}>{riskLevel}</span>
        <CostBadge cost={runningCost} label="So far" />
      </div>

      <div className="gate-actions">
        <button className="btn btn-success btn-sm" onClick={onApprove}>
          Continue
        </button>
        {onSteer && (
          <button className="btn btn-secondary btn-sm" onClick={() => setSteerMode(!steerMode)}>
            Steer
          </button>
        )}
        <button className="btn btn-danger btn-sm" onClick={onReject}>
          Abort
        </button>
      </div>

      {steerMode && onSteer && (
        <div className="gate-steer-input">
          <textarea
            placeholder="Redirect the agent..."
            value={steerText}
            onChange={(e) => setSteerText(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && steerText.trim()) {
                e.preventDefault();
                onSteer(steerText.trim());
                setSteerMode(false);
                setSteerText("");
              }
              if (e.key === "Escape") {
                setSteerMode(false);
              }
            }}
            rows={2}
            autoFocus
          />
          <button
            className="btn btn-sm btn-steer-submit"
            disabled={!steerText.trim()}
            onClick={() => {
              if (steerText.trim()) {
                onSteer(steerText.trim());
                setSteerMode(false);
                setSteerText("");
              }
            }}
          >
            Send
          </button>
        </div>
      )}
    </div>
  );
}
