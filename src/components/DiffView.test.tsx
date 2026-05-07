import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import DiffView from "./DiffView";
import type { ParsedDiff, CommentThread, RepoFileStatus } from "../types/diff";

const simpleDiff: ParsedDiff = {
  files: [
    {
      oldPath: "src/main.rs",
      newPath: "src/main.rs",
      status: "modified",
      additions: 2,
      deletions: 1,
      hunks: [{
        header: "fn main()",
        oldStart: 1,
        oldCount: 3,
        newStart: 1,
        newCount: 4,
        lines: [
          { type: "context", content: "use std::io;", oldLineNumber: 1, newLineNumber: 1 },
          { type: "delete", content: 'println!("old");', oldLineNumber: 2, newLineNumber: null },
          { type: "add", content: 'println!("new");', oldLineNumber: null, newLineNumber: 2 },
          { type: "add", content: 'println!("extra");', oldLineNumber: null, newLineNumber: 3 },
          { type: "context", content: "}", oldLineNumber: 3, newLineNumber: 4 },
        ],
      }],
    },
    {
      oldPath: "src/lib.rs",
      newPath: "src/lib.rs",
      status: "modified",
      additions: 1,
      deletions: 0,
      hunks: [{
        header: "",
        oldStart: 1,
        oldCount: 2,
        newStart: 1,
        newCount: 3,
        lines: [
          { type: "context", content: "pub fn add(a: i32, b: i32) -> i32 {", oldLineNumber: 1, newLineNumber: 1 },
          { type: "add", content: "    // fast path", oldLineNumber: null, newLineNumber: 2 },
          { type: "context", content: "    a + b", oldLineNumber: 2, newLineNumber: 3 },
        ],
      }],
    },
  ],
  stats: { totalAdditions: 3, totalDeletions: 1, filesChanged: 2 },
};

const emptyDiff: ParsedDiff = {
  files: [],
  stats: { totalAdditions: 0, totalDeletions: 0, filesChanged: 0 },
};

const baseProps = {
  diff: simpleDiff,
  comments: [] as CommentThread[],
  onAddComment: vi.fn(),
  onClose: vi.fn(),
};

describe("DiffView", () => {
  describe("rendering", () => {
    it("renders file sidebar with correct file count", () => {
      render(<DiffView {...baseProps} />);
      const countBadge = document.querySelector(".diff-sidebar-count");
      expect(countBadge).not.toBeNull();
      expect(countBadge!.textContent).toBe("2");
      expect(screen.getByText("Changed Files")).toBeInTheDocument();
    });

    it("renders sidebar file entries with status and stats", () => {
      render(<DiffView {...baseProps} />);
      expect(screen.getAllByText("M")).toHaveLength(2);
      expect(screen.getByText("+2")).toBeInTheDocument();
      expect(screen.getByText("-1")).toBeInTheDocument();
    });

    it("renders hunk header", () => {
      render(<DiffView {...baseProps} />);
      expect(screen.getByText(/@@ -1,3 \+1,4 @@/)).toBeInTheDocument();
    });

    it("renders add lines with + prefix", () => {
      render(<DiffView {...baseProps} />);
      const addLines = document.querySelectorAll(".diff-line-add");
      expect(addLines.length).toBe(2);
    });

    it("renders delete lines with - prefix", () => {
      render(<DiffView {...baseProps} />);
      const delLines = document.querySelectorAll(".diff-line-delete");
      expect(delLines.length).toBe(1);
    });

    it("renders context lines", () => {
      render(<DiffView {...baseProps} />);
      const ctxLines = document.querySelectorAll(".diff-line-context");
      expect(ctxLines.length).toBe(2);
    });

    it("shows empty state when diff has no files", () => {
      render(<DiffView {...baseProps} diff={emptyDiff} />);
      expect(screen.getByText("Select a file to view changes")).toBeInTheDocument();
    });

    it("renders toolbar with file path", () => {
      render(<DiffView {...baseProps} />);
      expect(screen.getByText("src/main.rs")).toBeInTheDocument();
    });

    it("shows hint text about commenting", () => {
      render(<DiffView {...baseProps} />);
      expect(screen.getByText("Click a line number to comment")).toBeInTheDocument();
    });
  });

  describe("file navigation", () => {
    it("first file is active by default", () => {
      render(<DiffView {...baseProps} />);
      const activeFile = document.querySelector(".diff-sidebar-file.active");
      expect(activeFile).not.toBeNull();
      expect(activeFile!.textContent).toContain("main.rs");
    });

    it("clicking sidebar file switches active file", async () => {
      const user = userEvent.setup();
      render(<DiffView {...baseProps} />);

      const files = document.querySelectorAll(".diff-sidebar-file");
      await user.click(files[1]);

      const activeFile = document.querySelector(".diff-sidebar-file.active");
      expect(activeFile!.textContent).toContain("lib.rs");
    });

    it("calls onActiveFileChange when switching files", async () => {
      const user = userEvent.setup();
      const onActiveFileChange = vi.fn();
      render(<DiffView {...baseProps} onActiveFileChange={onActiveFileChange} />);

      const files = document.querySelectorAll(".diff-sidebar-file");
      await user.click(files[1]);

      expect(onActiveFileChange).toHaveBeenCalledWith("src/lib.rs");
    });

    it("prev/next buttons navigate files", async () => {
      const user = userEvent.setup();
      render(<DiffView {...baseProps} />);

      const prevBtn = document.querySelectorAll(".diff-nav-btn")[0];
      const nextBtn = document.querySelectorAll(".diff-nav-btn")[1];

      // prev should be disabled on first file
      expect(prevBtn).toBeDisabled();
      expect(nextBtn).not.toBeDisabled();

      await user.click(nextBtn);
      const activeFile = document.querySelector(".diff-sidebar-file.active");
      expect(activeFile!.textContent).toContain("lib.rs");

      // now next should be disabled, prev enabled
      expect(nextBtn).toBeDisabled();
      expect(prevBtn).not.toBeDisabled();
    });

    it("selectedFile prop sets initial active file via suffix match", () => {
      render(<DiffView {...baseProps} selectedFile="/abs/path/to/src/lib.rs" />);
      const activeFile = document.querySelector(".diff-sidebar-file.active");
      expect(activeFile!.textContent).toContain("lib.rs");
    });
  });

  describe("comments", () => {
    it("clicking gutter shows comment form", async () => {
      const user = userEvent.setup();
      render(<DiffView {...baseProps} />);

      const gutter = document.querySelector(".diff-gutter-new")!;
      await user.click(gutter);

      expect(screen.getByPlaceholderText("Leave feedback for the agent...")).toBeInTheDocument();
    });

    it("submitting comment calls onAddComment", async () => {
      const user = userEvent.setup();
      const onAddComment = vi.fn();
      render(<DiffView {...baseProps} onAddComment={onAddComment} />);

      const gutter = document.querySelector(".diff-gutter-new")!;
      await user.click(gutter);

      const textarea = screen.getByPlaceholderText("Leave feedback for the agent...");
      await user.type(textarea, "needs refactoring");
      await user.click(screen.getByText("Comment"));

      expect(onAddComment).toHaveBeenCalledWith(
        "src/main.rs",
        "new",
        expect.any(Number),
        expect.any(Number),
        "needs refactoring"
      );
    });

    it("cancel button dismisses comment form", async () => {
      const user = userEvent.setup();
      render(<DiffView {...baseProps} />);

      const gutter = document.querySelector(".diff-gutter-new")!;
      await user.click(gutter);
      expect(screen.getByPlaceholderText("Leave feedback for the agent...")).toBeInTheDocument();

      await user.click(screen.getByText("Cancel"));
      expect(screen.queryByPlaceholderText("Leave feedback for the agent...")).not.toBeInTheDocument();
    });

    it("comment button is disabled when textarea is empty", async () => {
      const user = userEvent.setup();
      render(<DiffView {...baseProps} />);

      const gutter = document.querySelector(".diff-gutter-new")!;
      await user.click(gutter);

      expect(screen.getByText("Comment")).toBeDisabled();
    });

    it("renders existing comments inline", () => {
      const comments: CommentThread[] = [{
        id: "c1",
        filePath: "src/main.rs",
        side: "new",
        startLine: 2,
        endLine: 2,
        body: "This should be a constant",
        createdAt: new Date().toISOString(),
      }];
      render(<DiffView {...baseProps} comments={comments} />);

      expect(screen.getByText("This should be a constant")).toBeInTheDocument();
    });

    it("sidebar shows comment count badge per file", () => {
      const comments: CommentThread[] = [
        { id: "c1", filePath: "src/main.rs", side: "new", startLine: 2, endLine: 2, body: "fix", createdAt: "" },
        { id: "c2", filePath: "src/main.rs", side: "new", startLine: 3, endLine: 3, body: "fix2", createdAt: "" },
      ];
      render(<DiffView {...baseProps} comments={comments} />);

      const badges = document.querySelectorAll(".diff-sidebar-badge");
      expect(badges.length).toBeGreaterThanOrEqual(1);
      expect(badges[0].textContent).toBe("2");
    });

    it("sidebar footer shows total comment count", () => {
      const comments: CommentThread[] = [
        { id: "c1", filePath: "src/main.rs", side: "new", startLine: 2, endLine: 2, body: "fix", createdAt: "" },
      ];
      render(<DiffView {...baseProps} comments={comments} />);
      expect(screen.getByText("1 comment total")).toBeInTheDocument();
    });
  });

  describe("send feedback button", () => {
    it("not shown when no comments", () => {
      const onSendFeedback = vi.fn();
      render(<DiffView {...baseProps} onSendFeedback={onSendFeedback} />);
      expect(screen.queryByText(/Send feedback/)).not.toBeInTheDocument();
    });

    it("shown in action bar when comments exist", () => {
      const comments: CommentThread[] = [
        { id: "c1", filePath: "src/main.rs", side: "new", startLine: 2, endLine: 2, body: "fix", createdAt: "" },
      ];
      const onSendFeedback = vi.fn();
      render(<DiffView {...baseProps} comments={comments} onSendFeedback={onSendFeedback} />);
      expect(screen.getByText(/Send feedback \(1 comment\)/)).toBeInTheDocument();
    });

    it("clicking send feedback calls handler", async () => {
      const user = userEvent.setup();
      const comments: CommentThread[] = [
        { id: "c1", filePath: "src/main.rs", side: "new", startLine: 2, endLine: 2, body: "fix", createdAt: "" },
        { id: "c2", filePath: "src/lib.rs", side: "new", startLine: 1, endLine: 1, body: "fix2", createdAt: "" },
      ];
      const onSendFeedback = vi.fn();
      render(<DiffView {...baseProps} comments={comments} onSendFeedback={onSendFeedback} />);

      await user.click(screen.getByText(/Send feedback \(2 comments\)/));
      expect(onSendFeedback).toHaveBeenCalled();
    });

    it("not shown when onSendFeedback is undefined even with comments", () => {
      const comments: CommentThread[] = [
        { id: "c1", filePath: "src/main.rs", side: "new", startLine: 2, endLine: 2, body: "fix", createdAt: "" },
      ];
      render(<DiffView {...baseProps} comments={comments} />);
      expect(screen.queryByText(/Send feedback/)).not.toBeInTheDocument();
    });
  });

  describe("commit flow", () => {
    const repoFiles: RepoFileStatus[] = [{
      repoPath: "/workspace",
      repoName: "workspace",
      files: [
        { absolutePath: "/workspace/src/main.rs", relativePath: "src/main.rs", status: "M" },
        { absolutePath: "/workspace/src/lib.rs", relativePath: "src/lib.rs", status: "M" },
      ],
    }];

    it("shows commit button in action bar when onCommit provided", () => {
      render(<DiffView {...baseProps} repoFiles={repoFiles} onCommit={vi.fn()} />);
      expect(screen.getByText("Commit")).toBeInTheDocument();
    });

    it("no commit button when onCommit is undefined", () => {
      render(<DiffView {...baseProps} repoFiles={repoFiles} />);
      expect(screen.queryByText("Commit")).not.toBeInTheDocument();
    });

    it("no commit button when repoFiles is empty", () => {
      render(<DiffView {...baseProps} repoFiles={[]} onCommit={vi.fn()} />);
      expect(screen.queryByText("Commit")).not.toBeInTheDocument();
    });

    it("clicking commit button switches to commit view", async () => {
      const user = userEvent.setup();
      render(<DiffView {...baseProps} repoFiles={repoFiles} onCommit={vi.fn()} />);

      await user.click(screen.getByText("Commit"));

      expect(screen.getByText("Commit Changes")).toBeInTheDocument();
      expect(screen.getByPlaceholderText("Describe your changes...")).toBeInTheDocument();
    });

    it("commit view shows files with checkboxes all selected by default", async () => {
      const user = userEvent.setup();
      render(<DiffView {...baseProps} repoFiles={repoFiles} onCommit={vi.fn()} />);

      await user.click(screen.getByText("Commit"));

      const checkboxes = document.querySelectorAll(".diff-commit-file:not(.diff-commit-select-all) input[type='checkbox']") as NodeListOf<HTMLInputElement>;
      expect(checkboxes.length).toBe(2);
      expect(checkboxes[0].checked).toBe(true);
      expect(checkboxes[1].checked).toBe(true);
    });

    it("unchecking file updates commit button text", async () => {
      const user = userEvent.setup();
      render(<DiffView {...baseProps} repoFiles={repoFiles} onCommit={vi.fn()} />);

      await user.click(screen.getByText("Commit"));

      const checkboxes = document.querySelectorAll(".diff-commit-file:not(.diff-commit-select-all) input[type='checkbox']");
      await user.click(checkboxes[0]);

      expect(screen.getByText(/1 file$/)).toBeInTheDocument();
    });

    it("commit button disabled when no message", async () => {
      const user = userEvent.setup();
      render(<DiffView {...baseProps} repoFiles={repoFiles} onCommit={vi.fn()} />);

      await user.click(screen.getByText("Commit"));

      const commitBtn = document.querySelector(".diff-commit-btn") as HTMLButtonElement;
      expect(commitBtn.disabled).toBe(true);
    });

    it("commit button disabled when no files selected", async () => {
      const user = userEvent.setup();
      render(<DiffView {...baseProps} repoFiles={repoFiles} onCommit={vi.fn()} />);

      await user.click(screen.getByText("Commit"));

      // Type a message
      await user.type(screen.getByPlaceholderText("Describe your changes..."), "msg");

      // Deselect all
      const selectAll = document.querySelector(".diff-commit-select-all input[type='checkbox']")!;
      await user.click(selectAll);

      const commitBtn = document.querySelector(".diff-commit-btn") as HTMLButtonElement;
      expect(commitBtn.disabled).toBe(true);
    });

    it("submitting commit calls onCommit with correct params", async () => {
      const user = userEvent.setup();
      const onCommit = vi.fn().mockResolvedValue(undefined);
      render(<DiffView {...baseProps} repoFiles={repoFiles} onCommit={onCommit} />);

      await user.click(screen.getByText("Commit"));
      await user.type(screen.getByPlaceholderText("Describe your changes..."), "feat: update");
      await user.click(document.querySelector(".diff-commit-btn")!);

      expect(onCommit).toHaveBeenCalledWith([{
        repoPath: "/workspace",
        message: "feat: update",
        files: ["src/main.rs", "src/lib.rs"],
        amend: false,
      }]);
    });

    it("amend checkbox changes commit params", async () => {
      const user = userEvent.setup();
      const onCommit = vi.fn().mockResolvedValue(undefined);
      render(<DiffView {...baseProps} repoFiles={repoFiles} onCommit={onCommit} />);

      await user.click(screen.getByText("Commit"));
      await user.type(screen.getByPlaceholderText("Describe your changes..."), "amend msg");

      const amendCheckbox = screen.getByLabelText("Amend last commit");
      await user.click(amendCheckbox);
      await user.click(document.querySelector(".diff-commit-btn")!);

      expect(onCommit).toHaveBeenCalledWith([expect.objectContaining({ amend: true })]);
    });

    it("back button returns to diff view", async () => {
      const user = userEvent.setup();
      render(<DiffView {...baseProps} repoFiles={repoFiles} onCommit={vi.fn()} />);

      await user.click(screen.getByText("Commit"));
      expect(screen.getByText("Commit Changes")).toBeInTheDocument();

      await user.click(screen.getByText("Back"));
      expect(screen.queryByText("Commit Changes")).not.toBeInTheDocument();
      expect(screen.getByText("Changed Files")).toBeInTheDocument();
    });

    it("shows commit error when provided", async () => {
      const user = userEvent.setup();
      render(<DiffView {...baseProps} repoFiles={repoFiles} onCommit={vi.fn()} commitError="nothing to commit" />);

      await user.click(screen.getByText("Commit"));
      expect(screen.getByText("nothing to commit")).toBeInTheDocument();
    });

    it("generate button calls onGenerateMessage", async () => {
      const user = userEvent.setup();
      const onGenerate = vi.fn();
      render(<DiffView {...baseProps} repoFiles={repoFiles} onCommit={vi.fn()} onGenerateMessage={onGenerate} />);

      await user.click(screen.getByText("Commit"));
      await user.click(screen.getByText("Generate"));

      expect(onGenerate).toHaveBeenCalled();
    });

    it("suggested message populates textarea", async () => {
      const user = userEvent.setup();
      render(<DiffView {...baseProps} repoFiles={repoFiles} onCommit={vi.fn()} suggestedMessage="feat: auto msg" />);

      await user.click(screen.getByText("Commit"));
      const textarea = screen.getByPlaceholderText("Describe your changes...") as HTMLTextAreaElement;
      expect(textarea.value).toBe("feat: auto msg");
    });

    it("multi-repo shows repo headers", async () => {
      const user = userEvent.setup();
      const multiRepoFiles: RepoFileStatus[] = [
        { repoPath: "/ws/frontend", repoName: "frontend", files: [{ absolutePath: "/ws/frontend/app.tsx", relativePath: "app.tsx", status: "M" }] },
        { repoPath: "/ws/backend", repoName: "backend", files: [{ absolutePath: "/ws/backend/main.rs", relativePath: "main.rs", status: "M" }] },
      ];
      render(<DiffView {...baseProps} repoFiles={multiRepoFiles} onCommit={vi.fn()} />);

      await user.click(screen.getByText("Commit"));
      expect(screen.getByText("frontend")).toBeInTheDocument();
      expect(screen.getByText("backend")).toBeInTheDocument();
    });
  });

  describe("close behavior", () => {
    it("close button calls onClose", async () => {
      const user = userEvent.setup();
      const onClose = vi.fn();
      render(<DiffView {...baseProps} onClose={onClose} />);

      await user.click(document.querySelector(".diff-close-btn")!);
      expect(onClose).toHaveBeenCalled();
    });

    it("escape in commit view goes back to diff (not close)", async () => {
      const user = userEvent.setup();
      const onClose = vi.fn();
      const repoFiles: RepoFileStatus[] = [{
        repoPath: "/ws", repoName: "ws",
        files: [{ absolutePath: "/ws/a.ts", relativePath: "a.ts", status: "M" }],
      }];
      render(<DiffView {...baseProps} onClose={onClose} repoFiles={repoFiles} onCommit={vi.fn()} />);

      await user.click(screen.getByText("Commit"));
      expect(screen.getByText("Commit Changes")).toBeInTheDocument();

      await user.keyboard("{Escape}");
      expect(screen.queryByText("Commit Changes")).not.toBeInTheDocument();
      expect(onClose).not.toHaveBeenCalled();
    });

    it("escape in diff view closes", async () => {
      const user = userEvent.setup();
      const onClose = vi.fn();
      render(<DiffView {...baseProps} onClose={onClose} />);

      await user.keyboard("{Escape}");
      expect(onClose).toHaveBeenCalled();
    });
  });
});
