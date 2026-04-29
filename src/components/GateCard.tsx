import { useState } from "react";
import CostBadge from "./CostBadge";

interface GateCardProps {
  description: string;
  riskLevel: string;
  toolUseId: string;
  toolName: string;
  runningCost: number;
  onApprove: () => void;
  onReject: () => void;
  onSteer: (feedback: string) => void;
}

type GateState = "pending" | "approved" | "rejected";

export default function GateCard({
  description,
  riskLevel,
  toolName,
  runningCost,
  onApprove,
  onReject,
}: GateCardProps) {
  const [state, setState] = useState<GateState>("pending");

  const handleApprove = () => {
    setState("approved");
    onApprove();
  };

  const handleReject = () => {
    setState("rejected");
    onReject();
  };

  if (state === "approved") {
    return (
      <div className="card gate-card gate-resolved gate-approved">
        <div className="gate-resolved-row">
          <span className="gate-resolved-icon approved">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
              <polyline points="20 6 9 17 4 12" />
            </svg>
          </span>
          <span className="gate-resolved-text">
            <span className="gate-resolved-label">Approved</span>
            <span className="gate-resolved-desc">
              {toolName && <span className="gate-resolved-tool">{toolName}</span>}
              {description}
            </span>
          </span>
        </div>
      </div>
    );
  }

  if (state === "rejected") {
    return (
      <div className="card gate-card gate-resolved gate-rejected">
        <div className="gate-resolved-row">
          <span className="gate-resolved-icon rejected">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
              <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </span>
          <span className="gate-resolved-text">
            <span className="gate-resolved-label">Rejected</span>
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
        <button className="btn btn-success btn-sm" onClick={handleApprove}>
          Approve
        </button>
        <button className="btn btn-danger btn-sm" onClick={handleReject}>
          Reject
        </button>
      </div>
    </div>
  );
}
