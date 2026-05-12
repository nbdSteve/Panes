import { useState, useCallback, useEffect, useRef } from "react";
import type { ParsedDiff, DiffHunk, DiffLine, CommentThread, CommentSide, LineSelection, RepoFileStatus, RepoCommitParams } from "../types/diff";

type ViewMode = "diff" | "commit";

interface DiffViewProps {
  diff: ParsedDiff;
  selectedFile?: string;
  comments: CommentThread[];
  onAddComment: (filePath: string, side: CommentSide, startLine: number, endLine: number, body: string) => void;
  onActiveFileChange?: (filePath: string) => void;
  onClose: () => void;
  repoFiles?: RepoFileStatus[];
  onCommit?: (commits: RepoCommitParams[]) => Promise<void>;
  commitError?: string | null;
  suggestedMessage?: string;
  onGenerateMessage?: () => void;
  generatingMessage?: boolean;
  onSendFeedback?: () => void;
  /**
   * Which version tracker owns the thread's edits. "git" enables the
   * Commit flow (makes sense in git-backed workspaces only). "shadow"
   * always hides it — there's no repo to commit into. Optional for
   * backward compat; missing is treated as "git".
   */
  trackerKind?: "git" | "shadow";
}

export default function DiffView({ diff, selectedFile, comments, onAddComment, onActiveFileChange, onClose, repoFiles, onCommit, commitError, suggestedMessage, onGenerateMessage, generatingMessage, onSendFeedback, trackerKind }: DiffViewProps) {
  const [pendingSelection, setPendingSelection] = useState<LineSelection | null>(null);
  const matchedFile = selectedFile ? diff.files.find((f) => f.newPath === selectedFile || selectedFile.endsWith("/" + f.newPath)) : undefined;
  const [activeFile, setActiveFileLocal] = useState<string>(matchedFile?.newPath ?? diff.files[0]?.newPath ?? "");
  const setActiveFile = useCallback((path: string) => {
    setActiveFileLocal(path);
    onActiveFileChange?.(path);
  }, [onActiveFileChange]);
  const contentRef = useRef<HTMLDivElement>(null);
  const [viewMode, setViewMode] = useState<ViewMode>("diff");

  // Commit state
  const [commitMessage, setCommitMessage] = useState(suggestedMessage ?? "");
  const [amend, setAmend] = useState(false);
  const [selectedFiles, setSelectedFiles] = useState<Record<string, Set<string>>>(() => {
    if (!repoFiles) return {};
    const sel: Record<string, Set<string>> = {};
    for (const repo of repoFiles) {
      sel[repo.repoPath] = new Set(repo.files.map((f) => f.relativePath));
    }
    return sel;
  });

  useEffect(() => {
    if (suggestedMessage && !commitMessage) {
      setCommitMessage(suggestedMessage);
    }
  }, [suggestedMessage]);

  useEffect(() => {
    if (repoFiles && Object.keys(selectedFiles).length === 0) {
      const sel: Record<string, Set<string>> = {};
      for (const repo of repoFiles) {
        sel[repo.repoPath] = new Set(repo.files.map((f) => f.relativePath));
      }
      setSelectedFiles(sel);
    }
  }, [repoFiles]);

  useEffect(() => {
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (viewMode === "commit") {
          setViewMode("diff");
        } else {
          onClose();
        }
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onClose, viewMode]);

  useEffect(() => {
    if (contentRef.current) contentRef.current.scrollTop = 0;
  }, [activeFile]);

  const currentFile = diff.files.find((f) => f.newPath === activeFile);
  const fileComments = comments.filter((c) => c.filePath === activeFile);
  const commentCountByFile = (path: string) => comments.filter((c) => c.filePath === path).length;
  const totalComments = comments.length;

  const currentIdx = diff.files.findIndex((f) => f.newPath === activeFile);
  const goNext = () => { if (currentIdx < diff.files.length - 1) setActiveFile(diff.files[currentIdx + 1].newPath); };
  const goPrev = () => { if (currentIdx > 0) setActiveFile(diff.files[currentIdx - 1].newPath); };

  const handleToggleFile = useCallback((repoPath: string, filePath: string) => {
    setSelectedFiles((prev) => {
      const repoSet = new Set(prev[repoPath] ?? []);
      if (repoSet.has(filePath)) {
        repoSet.delete(filePath);
      } else {
        repoSet.add(filePath);
      }
      return { ...prev, [repoPath]: repoSet };
    });
  }, []);

  const handleToggleAll = useCallback((repoPath: string, allFiles: string[]) => {
    setSelectedFiles((prev) => {
      const repoSet = prev[repoPath] ?? new Set();
      const allSelected = allFiles.every((f) => repoSet.has(f));
      return { ...prev, [repoPath]: allSelected ? new Set() : new Set(allFiles) };
    });
  }, []);

  const totalSelected = Object.values(selectedFiles).reduce((s, set) => s + set.size, 0);

  const handleCommit = useCallback(async () => {
    if (!onCommit || !repoFiles) return;
    const commits: RepoCommitParams[] = [];
    for (const repo of repoFiles) {
      const files = [...(selectedFiles[repo.repoPath] ?? [])];
      if (files.length === 0) continue;
      commits.push({
        repoPath: repo.repoPath,
        message: commitMessage || "Update files",
        files,
        amend,
      });
    }
    if (commits.length > 0) {
      await onCommit(commits);
    }
  }, [onCommit, repoFiles, selectedFiles, commitMessage, amend]);

  // Commit is git-only: a "shadow" tracker workspace has no repo to
  // commit into. Default to git when trackerKind is absent (legacy threads
  // pre-dating the tracker abstraction) — the `repoFiles` check still
  // gates on actual git-tracked content being present.
  const isGitTracker = trackerKind !== "shadow";
  const hasCommitSupport = isGitTracker && !!onCommit && !!repoFiles && repoFiles.length > 0;

  if (viewMode === "commit" && hasCommitSupport) {
    return (
      <div className="diff-overlay">
        <div className="diff-modal">
          <div className="diff-commit-view">
            <div className="diff-commit-view-header">
              <button className="diff-nav-btn" onClick={() => setViewMode("diff")} title="Back to diff">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round"><polyline points="15 18 9 12 15 6" /></svg>
              </button>
              <h3 className="diff-commit-view-title">Commit Changes</h3>
              <button className="diff-close-btn" onClick={onClose} title="Close (Esc)">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round">
                  <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
                </svg>
              </button>
            </div>

            <div className="diff-commit-view-body">
              <div className="diff-commit-message-section">
                <div className="diff-commit-message-header">
                  <label className="diff-commit-label">Commit message</label>
                  {onGenerateMessage && (
                    <button
                      className="btn btn-xs btn-secondary"
                      onClick={onGenerateMessage}
                      disabled={generatingMessage}
                      title="Generate message with AI"
                    >
                      {generatingMessage ? (
                        <span className="spinner-xs" />
                      ) : (
                        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                          <path d="M12 2v4M12 18v4M4.93 4.93l2.83 2.83M16.24 16.24l2.83 2.83M2 12h4M18 12h4M4.93 19.07l2.83-2.83M16.24 7.76l2.83-2.83" />
                        </svg>
                      )}
                      Generate
                    </button>
                  )}
                </div>
                <textarea
                  className="diff-commit-input"
                  value={commitMessage}
                  onChange={(e) => setCommitMessage(e.target.value)}
                  placeholder="Describe your changes..."
                  rows={4}
                  autoFocus
                />
              </div>

              <div className="diff-commit-options">
                <label className="diff-commit-option">
                  <input
                    type="checkbox"
                    checked={amend}
                    onChange={(e) => setAmend(e.target.checked)}
                  />
                  <span>Amend last commit</span>
                </label>
              </div>

              <div className="diff-commit-files-section">
                {repoFiles!.map((repo) => {
                  const allPaths = repo.files.map((f) => f.relativePath);
                  const repoSelected = selectedFiles[repo.repoPath] ?? new Set();
                  const allChecked = allPaths.length > 0 && allPaths.every((p) => repoSelected.has(p));

                  return (
                    <div key={repo.repoPath} className="diff-commit-repo">
                      <div className="diff-commit-repo-header">
                        <span className="diff-commit-repo-name">{repo.repoName}</span>
                        <span className="diff-commit-repo-count">{repoSelected.size}/{repo.files.length} files</span>
                      </div>
                      <div className="diff-commit-file-list">
                        <label className="diff-commit-file diff-commit-select-all">
                          <input
                            type="checkbox"
                            checked={allChecked}
                            onChange={() => handleToggleAll(repo.repoPath, allPaths)}
                          />
                          <span className="diff-commit-file-label">Select all</span>
                        </label>
                        {repo.files.map((file) => (
                          <label key={file.relativePath} className="diff-commit-file">
                            <input
                              type="checkbox"
                              checked={repoSelected.has(file.relativePath)}
                              onChange={() => handleToggleFile(repo.repoPath, file.relativePath)}
                            />
                            <span className={`diff-commit-file-status status-${file.status}`}>
                              {file.status}
                            </span>
                            <span className="diff-commit-file-path">{file.relativePath}</span>
                          </label>
                        ))}
                      </div>
                    </div>
                  );
                })}
              </div>

              {commitError && <div className="diff-commit-error">{commitError}</div>}
            </div>

            <div className="diff-commit-view-footer">
              <button className="btn btn-secondary btn-sm" onClick={() => setViewMode("diff")}>
                Back
              </button>
              <button
                className="btn btn-primary btn-sm diff-commit-btn"
                onClick={handleCommit}
                disabled={totalSelected === 0 || !commitMessage.trim()}
              >
                {amend ? "Amend" : "Commit"} {totalSelected} file{totalSelected !== 1 ? "s" : ""}
              </button>
            </div>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="diff-overlay">
      <div className="diff-modal">
        {/* File sidebar */}
        <div className="diff-sidebar">
          <div className="diff-sidebar-head">
            <span className="diff-sidebar-title">Changed Files</span>
            <span className="diff-sidebar-count">{diff.stats.filesChanged}</span>
          </div>
          <div className="diff-sidebar-list">
            {diff.files.map((file) => {
              const cc = commentCountByFile(file.newPath);
              const isActive = file.newPath === activeFile;
              return (
                <button
                  key={file.newPath}
                  className={`diff-sidebar-file${isActive ? " active" : ""}`}
                  onClick={() => setActiveFile(file.newPath)}
                >
                  <span className={`diff-sidebar-status diff-sidebar-status-${file.status}`}>
                    {file.status === "added" ? "A" : file.status === "deleted" ? "D" : file.status === "renamed" ? "R" : "M"}
                  </span>
                  <span className="diff-sidebar-name">
                    <span className="diff-sidebar-filename">{file.newPath.split("/").pop()}</span>
                    <span className="diff-sidebar-dir">{file.newPath.split("/").slice(0, -1).join("/")}</span>
                  </span>
                  <span className="diff-sidebar-meta">
                    {cc > 0 && <span className="diff-sidebar-badge">{cc}</span>}
                    <span className="diff-sidebar-delta">
                      <span className="diff-stat-add">+{file.additions}</span>
                      <span className="diff-stat-del">-{file.deletions}</span>
                    </span>
                  </span>
                </button>
              );
            })}
          </div>
          {totalComments > 0 && (
            <div className="diff-sidebar-footer">
              {totalComments} comment{totalComments !== 1 ? "s" : ""} total
            </div>
          )}
        </div>

        {/* Main diff area */}
        <div className="diff-main">
          {/* Toolbar */}
          <div className="diff-toolbar">
            <div className="diff-toolbar-left">
              <button className="diff-nav-btn" onClick={goPrev} disabled={currentIdx <= 0} title="Previous file">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round"><polyline points="15 18 9 12 15 6" /></svg>
              </button>
              <button className="diff-nav-btn" onClick={goNext} disabled={currentIdx >= diff.files.length - 1} title="Next file">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round"><polyline points="9 6 15 12 9 18" /></svg>
              </button>
              <span className="diff-toolbar-fileinfo">
                {currentFile && (
                  <>
                    {currentFile.status !== "modified" && (
                      <span className={`diff-file-badge diff-file-badge-${currentFile.status}`}>{currentFile.status}</span>
                    )}
                    <span className="diff-toolbar-path">{currentFile.newPath}</span>
                  </>
                )}
              </span>
            </div>
            <div className="diff-toolbar-right">
              <span className="diff-toolbar-hint">Click a line number to comment</span>
              <button className="diff-close-btn" onClick={onClose} title="Close (Esc)">
                <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round">
                  <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
                </svg>
              </button>
            </div>
          </div>

          {/* Diff content */}
          <div className="diff-content" ref={contentRef}>
            {currentFile && currentFile.hunks.map((hunk, hi) => (
              <DiffHunkView
                key={hi}
                hunk={hunk}
                filePath={currentFile.newPath}
                comments={fileComments}
                pendingSelection={pendingSelection?.filePath === currentFile.newPath ? pendingSelection : null}
                onLineClick={(side, line) => {
                  setPendingSelection({ filePath: currentFile.newPath, side, startLine: line, endLine: line });
                }}
                onSubmitComment={(body) => {
                  if (pendingSelection) {
                    onAddComment(pendingSelection.filePath, pendingSelection.side, pendingSelection.startLine, pendingSelection.endLine, body);
                    setPendingSelection(null);
                  }
                }}
                onCancelComment={() => setPendingSelection(null)}
              />
            ))}
            {currentFile && currentFile.hunks.length === 0 && (
              <div className="diff-empty">No changes in this file</div>
            )}
            {!currentFile && (
              <div className="diff-empty">Select a file to view changes</div>
            )}
          </div>

          {/* Bottom action bar */}
          {(hasCommitSupport || (onSendFeedback && totalComments > 0)) && (
            <div className="diff-action-bar">
              {onSendFeedback && totalComments > 0 && (
                <button className="btn btn-primary btn-sm" onClick={onSendFeedback}>
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                    <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
                  </svg>
                  Send feedback ({totalComments} comment{totalComments !== 1 ? "s" : ""})
                </button>
              )}
              {hasCommitSupport && (
                <button className="btn btn-success btn-sm diff-commit-trigger" onClick={() => setViewMode("commit")}>
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
                    <circle cx="12" cy="12" r="4" />
                    <line x1="1.05" y1="12" x2="7" y2="12" />
                    <line x1="17" y1="12" x2="22.95" y2="12" />
                  </svg>
                  Commit
                </button>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

interface DiffHunkViewProps {
  hunk: DiffHunk;
  filePath: string;
  comments: CommentThread[];
  pendingSelection: LineSelection | null;
  onLineClick: (side: CommentSide, line: number) => void;
  onSubmitComment: (body: string) => void;
  onCancelComment: () => void;
}

function DiffHunkView({ hunk, comments, pendingSelection, onLineClick, onSubmitComment, onCancelComment }: DiffHunkViewProps) {
  return (
    <div className="diff-hunk">
      <div className="diff-hunk-header">
        @@ -{hunk.oldStart},{hunk.oldCount} +{hunk.newStart},{hunk.newCount} @@{hunk.header ? ` ${hunk.header}` : ""}
      </div>
      {hunk.lines.map((line, li) => {
        const lineNum = line.type === "delete" ? line.oldLineNumber : line.newLineNumber;
        const side: CommentSide = line.type === "delete" ? "old" : "new";
        const isSelected = pendingSelection && pendingSelection.side === side && lineNum !== null &&
          lineNum >= pendingSelection.startLine && lineNum <= pendingSelection.endLine;
        const lineComments = comments.filter((c) =>
          c.side === side && lineNum !== null && lineNum >= c.startLine && lineNum <= c.endLine
        );

        return (
          <div key={li}>
            <DiffLineRow
              line={line}
              isSelected={!!isSelected}
              onGutterClick={() => {
                if (lineNum !== null) onLineClick(side, lineNum);
              }}
            />
            {lineComments.map((c) => (
              <div key={c.id} className="diff-inline-comment">
                <div className="diff-comment-body">{c.body}</div>
              </div>
            ))}
            {isSelected && lineNum === pendingSelection!.endLine && (
              <CommentForm
                onSubmit={onSubmitComment}
                onCancel={onCancelComment}
              />
            )}
          </div>
        );
      })}
    </div>
  );
}

interface DiffLineRowProps {
  line: DiffLine;
  isSelected: boolean;
  onGutterClick: () => void;
}

function DiffLineRow({ line, isSelected, onGutterClick }: DiffLineRowProps) {
  const prefix = line.type === "add" ? "+" : line.type === "delete" ? "-" : " ";
  const className = `diff-line diff-line-${line.type}${isSelected ? " diff-line-selected" : ""}`;

  return (
    <div className={className}>
      <span className="diff-gutter diff-gutter-old" onClick={onGutterClick}>
        {line.oldLineNumber ?? ""}
      </span>
      <span className="diff-gutter diff-gutter-new" onClick={onGutterClick}>
        {line.newLineNumber ?? ""}
      </span>
      <span className="diff-line-prefix">{prefix}</span>
      <span className="diff-line-content">{line.content}</span>
    </div>
  );
}

interface CommentFormProps {
  onSubmit: (body: string) => void;
  onCancel: () => void;
}

function CommentForm({ onSubmit, onCancel }: CommentFormProps) {
  const [body, setBody] = useState("");

  const handleSubmit = useCallback(() => {
    if (body.trim()) {
      onSubmit(body.trim());
      setBody("");
    }
  }, [body, onSubmit]);

  return (
    <div className="diff-comment-form">
      <textarea
        className="diff-comment-input"
        placeholder="Leave feedback for the agent..."
        value={body}
        onChange={(e) => setBody(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            handleSubmit();
          }
          if (e.key === "Escape") {
            onCancel();
          }
        }}
        autoFocus
      />
      <div className="diff-comment-actions">
        <button className="btn btn-secondary btn-sm" onClick={onCancel}>Cancel</button>
        <button className="btn btn-primary btn-sm" onClick={handleSubmit} disabled={!body.trim()}>Comment</button>
      </div>
    </div>
  );
}
