import type { DiffFile, DiffHunk, DiffLine, DiffLineType, ParsedDiff } from "../types/diff";

const DIFF_HEADER = /^diff --git a\/(.*) b\/(.*)$/;
const HUNK_HEADER = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(.*)$/;
const NEW_FILE = /^new file mode/;
const DELETED_FILE = /^deleted file mode/;
const RENAME_FROM = /^rename from (.*)$/;
const RENAME_TO = /^rename to (.*)$/;

export function parseDiff(raw: string): ParsedDiff {
  const lines = raw.split("\n");
  const files: DiffFile[] = [];
  let currentFile: DiffFile | null = null;
  let currentHunk: DiffHunk | null = null;
  let oldLine = 0;
  let newLine = 0;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    const headerMatch = line.match(DIFF_HEADER);
    if (headerMatch) {
      if (currentFile) files.push(currentFile);
      currentFile = {
        oldPath: headerMatch[1],
        newPath: headerMatch[2],
        status: "modified",
        hunks: [],
        additions: 0,
        deletions: 0,
      };
      currentHunk = null;
      continue;
    }

    if (!currentFile) continue;

    if (NEW_FILE.test(line)) {
      currentFile.status = "added";
      continue;
    }
    if (DELETED_FILE.test(line)) {
      currentFile.status = "deleted";
      continue;
    }
    const renameFromMatch = line.match(RENAME_FROM);
    if (renameFromMatch) {
      currentFile.oldPath = renameFromMatch[1];
      currentFile.status = "renamed";
      continue;
    }
    const renameToMatch = line.match(RENAME_TO);
    if (renameToMatch) {
      currentFile.newPath = renameToMatch[1];
      continue;
    }

    if (line.startsWith("---") || line.startsWith("+++")) continue;
    if (line.startsWith("index ") || line.startsWith("Binary ")) continue;

    const hunkMatch = line.match(HUNK_HEADER);
    if (hunkMatch) {
      const oldStart = parseInt(hunkMatch[1], 10);
      const oldCount = hunkMatch[2] !== undefined ? parseInt(hunkMatch[2], 10) : 1;
      const newStart = parseInt(hunkMatch[3], 10);
      const newCount = hunkMatch[4] !== undefined ? parseInt(hunkMatch[4], 10) : 1;
      currentHunk = {
        header: hunkMatch[5]?.trim() || "",
        oldStart,
        oldCount,
        newStart,
        newCount,
        lines: [],
      };
      currentFile.hunks.push(currentHunk);
      oldLine = oldStart;
      newLine = newStart;
      continue;
    }

    if (!currentHunk) continue;

    if (line.startsWith("+")) {
      const diffLine: DiffLine = {
        type: "add" as DiffLineType,
        content: line.slice(1),
        oldLineNumber: null,
        newLineNumber: newLine++,
      };
      currentHunk.lines.push(diffLine);
      currentFile.additions++;
    } else if (line.startsWith("-")) {
      const diffLine: DiffLine = {
        type: "delete" as DiffLineType,
        content: line.slice(1),
        oldLineNumber: oldLine++,
        newLineNumber: null,
      };
      currentHunk.lines.push(diffLine);
      currentFile.deletions++;
    } else if (line.startsWith(" ") || line === "") {
      const diffLine: DiffLine = {
        type: "context" as DiffLineType,
        content: line.startsWith(" ") ? line.slice(1) : "",
        oldLineNumber: oldLine++,
        newLineNumber: newLine++,
      };
      currentHunk.lines.push(diffLine);
    } else if (line.startsWith("\\")) {
      // "\ No newline at end of file" — skip
    }
  }

  if (currentFile) files.push(currentFile);

  // Also detect added status from --- /dev/null pattern
  for (const file of files) {
    if (file.oldPath === "/dev/null" || file.oldPath === "dev/null") {
      file.status = "added";
      file.oldPath = file.newPath;
    }
    if (file.newPath === "/dev/null" || file.newPath === "dev/null") {
      file.status = "deleted";
      file.newPath = file.oldPath;
    }
  }

  const totalAdditions = files.reduce((s, f) => s + f.additions, 0);
  const totalDeletions = files.reduce((s, f) => s + f.deletions, 0);

  return {
    files,
    stats: { totalAdditions, totalDeletions, filesChanged: files.length },
  };
}
