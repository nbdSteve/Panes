import { useState, useCallback } from "react";
import type { RepoFileStatus, RepoCommitParams } from "../types/diff";

interface CommitDialogProps {
  repoFiles: RepoFileStatus[];
  defaultMessage: string;
  loading?: boolean;
  onCommit: (commits: RepoCommitParams[]) => void;
  onCancel: () => void;
  onViewDiff?: (absolutePath: string) => void;
  error: string | null;
}

export default function CommitDialog({ repoFiles, defaultMessage, loading, onCommit, onCancel, onViewDiff, error }: CommitDialogProps) {
  const isMultiRepo = repoFiles.length > 1;

  const [repoMessages, setRepoMessages] = useState<Record<string, string>>(() => {
    const msgs: Record<string, string> = {};
    for (const repo of repoFiles) {
      msgs[repo.repoPath] = defaultMessage;
    }
    return msgs;
  });

  const [selectedFiles, setSelectedFiles] = useState<Record<string, Set<string>>>(() => {
    const sel: Record<string, Set<string>> = {};
    for (const repo of repoFiles) {
      sel[repo.repoPath] = new Set(repo.files.map((f) => f.relativePath));
    }
    return sel;
  });

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
      return {
        ...prev,
        [repoPath]: allSelected ? new Set() : new Set(allFiles),
      };
    });
  }, []);

  const totalSelected = Object.values(selectedFiles).reduce((s, set) => s + set.size, 0);

  const handleCommit = useCallback(() => {
    const commits: RepoCommitParams[] = [];
    for (const repo of repoFiles) {
      const files = [...(selectedFiles[repo.repoPath] ?? [])];
      if (files.length === 0) continue;
      commits.push({
        repoPath: repo.repoPath,
        message: repoMessages[repo.repoPath] || defaultMessage,
        files,
      });
    }
    if (commits.length > 0) {
      onCommit(commits);
    }
  }, [repoFiles, selectedFiles, repoMessages, defaultMessage, onCommit]);

  return (
    <div className="commit-dialog">
      <div className="commit-dialog-header">
        <h4>Commit Changes</h4>
        <button className="btn-icon" onClick={onCancel}>✕</button>
      </div>

      {loading && repoFiles.length === 0 && (
        <div className="commit-loading">Loading files...</div>
      )}

      {repoFiles.map((repo) => {
        const allPaths = repo.files.map((f) => f.relativePath);
        const repoSelected = selectedFiles[repo.repoPath] ?? new Set();
        const allChecked = allPaths.every((p) => repoSelected.has(p));

        return (
          <div key={repo.repoPath} className="commit-repo-section">
            {isMultiRepo && (
              <div className="commit-repo-header">
                <span className="commit-repo-name">{repo.repoName}</span>
                <span className="commit-repo-count">{repoSelected.size}/{repo.files.length}</span>
              </div>
            )}

            {isMultiRepo && (
              <textarea
                className="commit-message-input"
                value={repoMessages[repo.repoPath] ?? ""}
                onChange={(e) => setRepoMessages((prev) => ({ ...prev, [repo.repoPath]: e.target.value }))}
                placeholder="Commit message..."
                rows={2}
              />
            )}

            <div className="commit-file-list">
              <label className="commit-file-item commit-select-all">
                <input
                  type="checkbox"
                  checked={allChecked}
                  onChange={() => handleToggleAll(repo.repoPath, allPaths)}
                />
                <span className="commit-file-label">Select all</span>
              </label>
              {repo.files.map((file) => (
                <label key={file.relativePath} className="commit-file-item">
                  <input
                    type="checkbox"
                    checked={repoSelected.has(file.relativePath)}
                    onChange={() => handleToggleFile(repo.repoPath, file.relativePath)}
                  />
                  <span className={`commit-file-status commit-status-${file.status}`}>{file.status}</span>
                  <span
                    className={`commit-file-path${onViewDiff ? " commit-file-path-clickable" : ""}`}
                    onClick={onViewDiff ? (e) => { e.preventDefault(); e.stopPropagation(); onViewDiff(file.absolutePath); } : undefined}
                  >{file.relativePath}</span>
                </label>
              ))}
            </div>
          </div>
        );
      })}

      {!isMultiRepo && (
        <textarea
          className="commit-message-input"
          value={repoMessages[repoFiles[0]?.repoPath] ?? ""}
          onChange={(e) => {
            const rp = repoFiles[0]?.repoPath;
            if (rp) setRepoMessages((prev) => ({ ...prev, [rp]: e.target.value }));
          }}
          placeholder="Commit message..."
          rows={2}
        />
      )}

      {error && <div className="commit-error">{error}</div>}

      <div className="commit-dialog-footer">
        <button className="btn-secondary" onClick={onCancel}>Cancel</button>
        <button
          className="btn-primary"
          onClick={handleCommit}
          disabled={totalSelected === 0}
        >
          Commit {totalSelected} file{totalSelected !== 1 ? "s" : ""}
        </button>
      </div>
    </div>
  );
}
