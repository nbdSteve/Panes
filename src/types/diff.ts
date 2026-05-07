export type FileStatus = "added" | "deleted" | "modified" | "renamed";
export type DiffLineType = "add" | "delete" | "context";

export interface DiffLine {
  type: DiffLineType;
  content: string;
  oldLineNumber: number | null;
  newLineNumber: number | null;
}

export interface DiffHunk {
  header: string;
  oldStart: number;
  oldCount: number;
  newStart: number;
  newCount: number;
  lines: DiffLine[];
}

export interface DiffFile {
  oldPath: string;
  newPath: string;
  status: FileStatus;
  hunks: DiffHunk[];
  additions: number;
  deletions: number;
}

export interface ParsedDiff {
  files: DiffFile[];
  stats: { totalAdditions: number; totalDeletions: number; filesChanged: number };
}

export type CommentSide = "old" | "new";

export interface CommentThread {
  id: string;
  filePath: string;
  side: CommentSide;
  startLine: number;
  endLine: number;
  body: string;
  createdAt: string;
}

export interface LineSelection {
  filePath: string;
  side: CommentSide;
  startLine: number;
  endLine: number;
}

export interface FileGitStatus {
  absolutePath: string;
  relativePath: string;
  status: string;
}

export interface RepoFileStatus {
  repoPath: string;
  repoName: string;
  files: FileGitStatus[];
}

export interface RepoCommitParams {
  repoPath: string;
  message: string;
  files: string[];
  amend?: boolean;
}
