import Markdown from "react-markdown";

interface CompletionCardProps {
  summary: string;
  totalCost: number;
  durationMs: number;
  turns: number;
  hasFileChanges: boolean;
  onCommit: () => void;
  onRevert: () => void;
  onKeep: () => void;
}

export default function CompletionCard({
  summary,
  totalCost,
  durationMs,
  turns,
  hasFileChanges,
  onCommit,
  onRevert,
  onKeep,
}: CompletionCardProps) {
  const durationStr =
    durationMs < 60000
      ? `${(durationMs / 1000).toFixed(1)}s`
      : `${Math.floor(durationMs / 60000)}m ${Math.round((durationMs % 60000) / 1000)}s`;

  const costStr = totalCost < 0.01 ? `$${totalCost.toFixed(4)}` : `$${totalCost.toFixed(2)}`;

  return (
    <div className="card completion-card">
      <div className="completion-label">
        <span className="completion-label-icon">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
            <polyline points="20 6 9 17 4 12" />
          </svg>
        </span>
        <span className="completion-label-text">Complete</span>
      </div>

      {summary && <div className="completion-summary markdown-body"><Markdown>{summary}</Markdown></div>}

      <div className="completion-stats">
        <div className="completion-stat">
          <span className="completion-stat-label">Cost</span>
          <span className="completion-stat-value" style={{ color: "var(--cost)" }}>{costStr}</span>
        </div>
        <div className="completion-stat">
          <span className="completion-stat-label">Duration</span>
          <span className="completion-stat-value">{durationStr}</span>
        </div>
        <div className="completion-stat">
          <span className="completion-stat-label">Turns</span>
          <span className="completion-stat-value">{turns}</span>
        </div>
      </div>

      {hasFileChanges && (
        <div className="completion-actions">
          <button className="btn btn-success btn-sm" onClick={onCommit}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
              <circle cx="12" cy="12" r="4" /><line x1="1.05" y1="12" x2="7" y2="12" /><line x1="17.01" y1="12" x2="22.96" y2="12" />
            </svg>
            Commit
          </button>
          <button className="btn btn-danger btn-sm" onClick={onRevert}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
              <polyline points="1 4 1 10 7 10" /><path d="M3.51 15a9 9 0 1 0 2.13-9.36L1 10" />
            </svg>
            Revert all
          </button>
          <button className="btn btn-secondary btn-sm" onClick={onKeep}>
            Keep as-is
          </button>
        </div>
      )}
    </div>
  );
}
