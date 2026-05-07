import { useState, useCallback } from "react";
import type { ParsedDiff, DiffFile, DiffHunk, DiffLine, CommentThread, CommentSide, LineSelection } from "../types/diff";

interface DiffViewProps {
  diff: ParsedDiff;
  selectedFile?: string;
  comments: CommentThread[];
  onAddComment: (filePath: string, side: CommentSide, startLine: number, endLine: number, body: string) => void;
  onClose: () => void;
}

export default function DiffView({ diff, selectedFile, comments, onAddComment, onClose }: DiffViewProps) {
  const [pendingSelection, setPendingSelection] = useState<LineSelection | null>(null);
  const [expandedFile, setExpandedFile] = useState<string | null>(selectedFile ?? diff.files[0]?.newPath ?? null);

  return (
    <div className="diff-overlay" onClick={onClose}>
      <div className="diff-panel" onClick={(e) => e.stopPropagation()}>
        <div className="diff-header">
          <h3>Changes</h3>
          <span className="diff-stats">
            +{diff.stats.totalAdditions} −{diff.stats.totalDeletions} in {diff.stats.filesChanged} file{diff.stats.filesChanged !== 1 ? "s" : ""}
          </span>
          <button className="diff-close-btn" onClick={onClose}>✕</button>
        </div>
        <div className="diff-body">
          {diff.files.map((file) => (
            <DiffFileSection
              key={file.newPath}
              file={file}
              expanded={expandedFile === file.newPath}
              onToggle={() => setExpandedFile(expandedFile === file.newPath ? null : file.newPath)}
              comments={comments.filter((c) => c.filePath === file.newPath)}
              pendingSelection={pendingSelection?.filePath === file.newPath ? pendingSelection : null}
              onLineClick={(side, line) => {
                setPendingSelection({ filePath: file.newPath, side, startLine: line, endLine: line });
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
        </div>
      </div>
    </div>
  );
}

interface DiffFileSectionProps {
  file: DiffFile;
  expanded: boolean;
  onToggle: () => void;
  comments: CommentThread[];
  pendingSelection: LineSelection | null;
  onLineClick: (side: CommentSide, line: number) => void;
  onSubmitComment: (body: string) => void;
  onCancelComment: () => void;
}

function DiffFileSection({ file, expanded, onToggle, comments, pendingSelection, onLineClick, onSubmitComment, onCancelComment }: DiffFileSectionProps) {
  const statusLabel = file.status === "added" ? "new" : file.status === "deleted" ? "del" : file.status === "renamed" ? "ren" : "";

  return (
    <div className="diff-file">
      <div className="diff-file-header" onClick={onToggle}>
        <span className="diff-file-toggle">{expanded ? "▾" : "▸"}</span>
        {statusLabel && <span className={`diff-file-badge diff-file-badge-${file.status}`}>{statusLabel}</span>}
        <span className="diff-file-path">{file.newPath}</span>
        <span className="diff-file-stats">
          <span className="diff-stat-add">+{file.additions}</span>
          <span className="diff-stat-del">−{file.deletions}</span>
        </span>
      </div>
      {expanded && (
        <div className="diff-file-content">
          {file.hunks.map((hunk, hi) => (
            <DiffHunkView
              key={hi}
              hunk={hunk}
              filePath={file.newPath}
              comments={comments}
              pendingSelection={pendingSelection}
              onLineClick={onLineClick}
              onSubmitComment={onSubmitComment}
              onCancelComment={onCancelComment}
            />
          ))}
        </div>
      )}
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
        @@ -{hunk.oldStart},{hunk.oldCount} +{hunk.newStart},{hunk.newCount} @@ {hunk.header}
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
        placeholder="Add feedback for the agent..."
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
        <button className="btn-secondary" onClick={onCancel}>Cancel</button>
        <button className="btn-primary" onClick={handleSubmit} disabled={!body.trim()}>
          Comment (⌘↵)
        </button>
      </div>
    </div>
  );
}
