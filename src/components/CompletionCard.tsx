import { useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { formatCost } from "../lib/utils";

export type FileChangeAction = "created" | "modified" | "deleted" | "untracked";

export interface FileChange {
  path: string;
  action: FileChangeAction;
  absolutePath?: string;
}

export interface CompletionCardProps {
  summary: string;
  totalCost: number;
  showCost?: boolean;
  durationMs: number;
  turns: number;
  hasFileChanges: boolean;
  filesChanged?: FileChange[];
  testResults?: string;
  completionAction?: "committed" | "reverted" | "kept";
  commentCount?: number;
  feedbackSentCount?: number;
  onInspect: () => void;
  onRevert: () => void;
  onKeep: () => void;
  onFileClick?: (filePath: string) => void;
  onSendFeedback?: () => void;
}

export default function CompletionCard({
  summary,
  totalCost,
  showCost,
  durationMs,
  turns,
  hasFileChanges,
  filesChanged,
  testResults,
  completionAction,
  commentCount,
  feedbackSentCount,
  onInspect,
  onRevert,
  onKeep,
  onFileClick,
  onSendFeedback,
}: CompletionCardProps) {
  const [showFiles, setShowFiles] = useState(false);
  const [showTests, setShowTests] = useState(false);
  const durationStr =
    durationMs < 60000
      ? `${(durationMs / 1000).toFixed(1)}s`
      : `${Math.floor(durationMs / 60000)}m ${Math.round((durationMs % 60000) / 1000)}s`;

  const costStr = formatCost(totalCost);

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
          {showCost !== false && <span className="completion-stat completion-stat-cost">{costStr}</span>}
          {showCost !== false && <span className="completion-stat-sep" />}
          <span className="completion-stat">{durationStr}</span>
          <span className="completion-stat-sep" />
          <span className="completion-stat">{turns} {turns === 1 ? "turn" : "turns"}</span>
        </div>
      </div>

      {filesChanged && filesChanged.length > 0 && (
        <div className="files-changed">
          <button
            className="files-changed-summary"
            onClick={() => setShowFiles(!showFiles)}
          >
            <span className="files-changed-count">
              {filesChanged.length} file{filesChanged.length !== 1 ? "s" : ""} changed
              {(() => {
                const created = filesChanged.filter(f => f.action === "created").length;
                const modified = filesChanged.filter(f => f.action === "modified").length;
                const parts: string[] = [];
                if (created > 0) parts.push(`${created} created`);
                if (modified > 0) parts.push(`${modified} modified`);
                return parts.length > 0 ? ` (${parts.join(", ")})` : "";
              })()}
            </span>
            <span className={`files-changed-chevron ${showFiles ? "open" : ""}`}>
              <svg width="8" height="8" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3"><polyline points="9 6 15 12 9 18" /></svg>
            </span>
          </button>
          {showFiles && (
            <ul className="files-changed-list">
              {filesChanged.map((f, i) => (
                <li
                  key={i}
                  className={`files-changed-item${onFileClick ? " clickable" : ""}`}
                  onClick={onFileClick ? () => onFileClick(f.absolutePath ?? f.path) : undefined}
                >
                  <span className={`files-changed-icon ${f.action}`}>
                    {f.action === "created" || f.action === "untracked" ? "+" : f.action === "deleted" ? "-" : "~"}
                  </span>
                  <span className="files-changed-path">{f.path}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}

      {testResults && (
        <div className="test-results">
          <button
            className="test-results-summary"
            onClick={() => setShowTests(!showTests)}
          >
            <span className="test-results-label">Test results</span>
            <span className={`files-changed-chevron ${showTests ? "open" : ""}`}>
              <svg width="8" height="8" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3"><polyline points="9 6 15 12 9 18" /></svg>
            </span>
          </button>
          {showTests && (
            <pre className="test-results-output">{testResults}</pre>
          )}
        </div>
      )}

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
          <button className="btn btn-primary btn-sm" onClick={onInspect}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
              <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" /><circle cx="12" cy="12" r="3" />
            </svg>
            Inspect
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

      {feedbackSentCount != null && feedbackSentCount > 0 && (
        <div className="completion-actions">
          <span className="completion-action-badge feedback-sent">
            Feedback sent ({feedbackSentCount} comment{feedbackSentCount !== 1 ? "s" : ""})
          </span>
        </div>
      )}

      {!feedbackSentCount && onSendFeedback && commentCount != null && commentCount > 0 && (
        <div className="completion-actions">
          <button className="btn btn-primary btn-sm" onClick={onSendFeedback}>
            Send feedback ({commentCount} comment{commentCount !== 1 ? "s" : ""})
          </button>
        </div>
      )}
    </div>
  );
}
