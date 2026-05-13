import { useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { formatCost } from "../lib/utils";
import { copyTextToClipboard } from "../lib/clipboard";

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
  /**
   * When "isolated", the thread ran in its own git worktree. The card
   * swaps the Revert label to "Discard worktree" and renders a "Merge
   * to main" action that fires `onMerge`. Any other value (or absent)
   * keeps the Phase 1 Commit/Revert/Keep UI.
   */
  worktreeStatus?: "isolated" | "main";
  /** Transient error shown inline when a merge attempt conflicted. */
  mergeError?: { message: string; files: string[] } | null;
  onInspect: () => void;
  onRevert: () => void;
  onKeep: () => void;
  onMerge?: () => void;
  /**
   * Option A conflict resolver: user picks a whole-merge strategy
   * after seeing the conflict file list. Wiring is optional so
   * non-worktree callers don't have to pass a no-op.
   */
  onResolveMerge?: (strategy: "prefer_ours" | "prefer_theirs") => void;
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
  worktreeStatus,
  mergeError,
  onInspect,
  onRevert,
  onKeep,
  onMerge,
  onResolveMerge,
  onFileClick,
  onSendFeedback,
}: CompletionCardProps) {
  const isIsolated = worktreeStatus === "isolated";
  const revertLabel = isIsolated ? "Discard worktree" : "Revert all";
  const [showFiles, setShowFiles] = useState(false);
  const [showTests, setShowTests] = useState(false);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");

  const handleCopy = async () => {
    if (!summary) return;
    const ok = await copyTextToClipboard(summary);
    setCopyState(ok ? "copied" : "failed");
    setTimeout(() => setCopyState("idle"), 1500);
  };
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
          {summary && (
            <>
              <span className="completion-stat-sep" />
              <button
                type="button"
                className={`completion-copy-btn${copyState === "copied" ? " copied" : ""}${copyState === "failed" ? " failed" : ""}`}
                onClick={handleCopy}
                title={copyState === "copied" ? "Copied" : copyState === "failed" ? "Copy failed" : "Copy response"}
                aria-label="Copy response to clipboard"
              >
                {copyState === "copied" ? (
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
                    <polyline points="20 6 9 17 4 12" />
                  </svg>
                ) : (
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <rect x="9" y="9" width="13" height="13" rx="2" ry="2" />
                    <path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" />
                  </svg>
                )}
              </button>
            </>
          )}
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
          {isIsolated && onMerge && (
            <button className="btn btn-primary btn-sm" onClick={onMerge}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="18" cy="18" r="3" /><circle cx="6" cy="6" r="3" />
                <path d="M6 21V9a9 9 0 0 0 9 9" />
              </svg>
              Merge to main
            </button>
          )}
          <button className="btn btn-danger btn-sm" onClick={onRevert}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
              <polyline points="1 4 1 10 7 10" /><path d="M3.51 15a9 9 0 1 0 2.13-9.36L1 10" />
            </svg>
            {revertLabel}
          </button>
          <button className="btn btn-secondary btn-sm" onClick={onKeep}>
            Keep as-is
          </button>
        </div>
      )}

      {/* Worktree-only action bar for threads that produced no file
          changes. Without this, a text-only agent turn in a worktree
          leaves the worktree orphaned until startup recovery because
          the main action bar (above) requires hasFileChanges. */}
      {isIsolated && !hasFileChanges && !completionAction && onMerge && (
        <div className="completion-actions">
          <button className="btn btn-primary btn-sm" onClick={onMerge}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <circle cx="18" cy="18" r="3" /><circle cx="6" cy="6" r="3" />
              <path d="M6 21V9a9 9 0 0 0 9 9" />
            </svg>
            Merge to main
          </button>
          <button className="btn btn-danger btn-sm" onClick={onRevert}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
              <polyline points="1 4 1 10 7 10" /><path d="M3.51 15a9 9 0 1 0 2.13-9.36L1 10" />
            </svg>
            Discard worktree
          </button>
        </div>
      )}

      {mergeError && mergeError.files.length > 0 && (
        <div className="merge-conflict">
          <div className="merge-conflict-title">
            Merge couldn't complete — {mergeError.files.length} conflicting file{mergeError.files.length === 1 ? "" : "s"}:
          </div>
          <ul className="merge-conflict-files">
            {mergeError.files.map((f) => (
              <li key={f}>{f}</li>
            ))}
          </ul>
          {onResolveMerge ? (
            <>
              <div className="merge-conflict-hint">
                Pick which side to keep for every conflicting file, or discard the worktree.
              </div>
              <div className="merge-conflict-actions">
                <button
                  type="button"
                  className="btn btn-primary btn-sm"
                  onClick={() => onResolveMerge("prefer_theirs")}
                  title="Keep the worktree's version of every conflicting file and complete the merge"
                >
                  Use yours
                </button>
                <button
                  type="button"
                  className="btn btn-secondary btn-sm"
                  onClick={() => onResolveMerge("prefer_ours")}
                  title="Keep main's version of every conflicting file and complete the merge"
                >
                  Keep main
                </button>
              </div>
            </>
          ) : (
            <div className="merge-conflict-hint">
              Discard this worktree or resolve the conflicts manually before merging again.
            </div>
          )}
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
