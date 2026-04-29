import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";

interface CompletionCardProps {
  summary: string;
  totalCost: number;
  durationMs: number;
  turns: number;
  hasFileChanges: boolean;
  completionAction?: "committed" | "reverted" | "kept";
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
  completionAction,
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
      <div className="completion-header">
        <div className="completion-label">
          <span className="completion-label-icon">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3">
              <polyline points="20 6 9 17 4 12" />
            </svg>
          </span>
          <span className="completion-label-text">Complete</span>
        </div>
        <div className="completion-stats">
          <span className="completion-stat" style={{ color: "var(--cost)" }}>{costStr}</span>
          <span className="completion-stat-sep" />
          <span className="completion-stat">{durationStr}</span>
          <span className="completion-stat-sep" />
          <span className="completion-stat">{turns} {turns === 1 ? "turn" : "turns"}</span>
        </div>
      </div>

      {summary && (
        <div className="completion-summary markdown-body">
          <Markdown
            remarkPlugins={[remarkGfm]}
            components={{
              table: ({ children }) => (
                <div className="table-wrap"><table>{children}</table></div>
              ),
            }}
          >{summary}</Markdown>
        </div>
      )}

      {hasFileChanges && completionAction && (
        <div className="completion-actions">
          <span className={`completion-action-badge ${completionAction}`}>
            {completionAction === "committed" ? "Committed" : completionAction === "reverted" ? "Reverted" : "Kept as-is"}
          </span>
        </div>
      )}

      {hasFileChanges && !completionAction && (
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
