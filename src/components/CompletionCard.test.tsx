import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import CompletionCard from "./CompletionCard";
import type { CompletionCardProps } from "./CompletionCard";
import { copyTextToClipboard } from "../lib/clipboard";

vi.mock("../lib/clipboard", () => ({
  copyTextToClipboard: vi.fn(),
}));

const mockedCopy = vi.mocked(copyTextToClipboard);

const baseProps: CompletionCardProps = {
  summary: "Implemented feature X",
  totalCost: 0.0234,
  showCost: true,
  durationMs: 45000,
  turns: 3,
  hasFileChanges: true,
  filesChanged: [
    { path: "src/main.rs", action: "modified" },
    { path: "src/new.rs", action: "created" },
  ],
  onInspect: vi.fn(),
  onRevert: vi.fn(),
  onKeep: vi.fn(),
};

describe("CompletionCard", () => {
  describe("header and stats", () => {
    it("renders complete label", () => {
      render(<CompletionCard {...baseProps} />);
      expect(screen.getByText("Complete")).toBeInTheDocument();
    });

    it("formats duration under a minute as seconds", () => {
      render(<CompletionCard {...baseProps} durationMs={12345} />);
      expect(screen.getByText("12.3s")).toBeInTheDocument();
    });

    it("formats duration over a minute as Xm Ys", () => {
      render(<CompletionCard {...baseProps} durationMs={125000} />);
      expect(screen.getByText("2m 5s")).toBeInTheDocument();
    });

    it("shows turn count with singular form", () => {
      render(<CompletionCard {...baseProps} turns={1} />);
      expect(screen.getByText("1 turn")).toBeInTheDocument();
    });

    it("shows turn count with plural form", () => {
      render(<CompletionCard {...baseProps} turns={5} />);
      expect(screen.getByText("5 turns")).toBeInTheDocument();
    });

    it("shows cost when showCost is true", () => {
      render(<CompletionCard {...baseProps} showCost={true} totalCost={0.0234} />);
      expect(screen.getByText("$0.02")).toBeInTheDocument();
    });

    it("hides cost when showCost is false", () => {
      render(<CompletionCard {...baseProps} showCost={false} />);
      expect(screen.queryByText(/\$/)).not.toBeInTheDocument();
    });
  });

  describe("files changed", () => {
    it("shows file count summary", () => {
      render(<CompletionCard {...baseProps} />);
      expect(screen.getByText(/2 files changed/)).toBeInTheDocument();
    });

    it("shows breakdown of created and modified", () => {
      render(<CompletionCard {...baseProps} />);
      expect(screen.getByText(/1 created, 1 modified/)).toBeInTheDocument();
    });

    it("does not show files section when filesChanged is empty", () => {
      render(<CompletionCard {...baseProps} filesChanged={[]} />);
      expect(screen.queryByText(/files changed/)).not.toBeInTheDocument();
    });

    it("does not show files section when filesChanged is undefined", () => {
      render(<CompletionCard {...baseProps} filesChanged={undefined} />);
      expect(screen.queryByText(/files changed/)).not.toBeInTheDocument();
    });

    it("expands file list on click", async () => {
      const user = userEvent.setup();
      render(<CompletionCard {...baseProps} />);

      await user.click(screen.getByText(/2 files changed/));
      expect(screen.getByText("src/main.rs")).toBeInTheDocument();
      expect(screen.getByText("src/new.rs")).toBeInTheDocument();
    });

    it("collapses file list on second click", async () => {
      const user = userEvent.setup();
      render(<CompletionCard {...baseProps} />);

      await user.click(screen.getByText(/2 files changed/));
      expect(screen.getByText("src/main.rs")).toBeInTheDocument();

      await user.click(screen.getByText(/2 files changed/));
      expect(screen.queryByText("src/main.rs")).not.toBeInTheDocument();
    });

    it("clicking a file calls onFileClick with absolutePath", async () => {
      const user = userEvent.setup();
      const onFileClick = vi.fn();
      render(<CompletionCard {...baseProps} filesChanged={[
        { path: "src/main.rs", action: "modified", absolutePath: "/ws/src/main.rs" },
      ]} onFileClick={onFileClick} />);

      await user.click(screen.getByText(/1 file changed/));
      await user.click(screen.getByText("src/main.rs"));
      expect(onFileClick).toHaveBeenCalledWith("/ws/src/main.rs");
    });

    it("clicking a file without absolutePath falls back to path", async () => {
      const user = userEvent.setup();
      const onFileClick = vi.fn();
      render(<CompletionCard {...baseProps} filesChanged={[
        { path: "src/main.rs", action: "modified" },
      ]} onFileClick={onFileClick} />);

      await user.click(screen.getByText(/1 file changed/));
      await user.click(screen.getByText("src/main.rs"));
      expect(onFileClick).toHaveBeenCalledWith("src/main.rs");
    });

    it("file list items are not clickable when onFileClick is not provided", async () => {
      const user = userEvent.setup();
      render(<CompletionCard {...baseProps} />);

      await user.click(screen.getByText(/2 files changed/));
      const item = document.querySelector(".files-changed-item");
      expect(item).not.toBeNull();
      expect(item!.classList.contains("clickable")).toBe(false);
    });
  });

  describe("test results", () => {
    it("shows test results section when provided", () => {
      render(<CompletionCard {...baseProps} testResults="3 passed, 1 failed" />);
      expect(screen.getByText("Test results")).toBeInTheDocument();
    });

    it("does not show test results when undefined", () => {
      render(<CompletionCard {...baseProps} />);
      expect(screen.queryByText("Test results")).not.toBeInTheDocument();
    });

    it("expands test output on click", async () => {
      const user = userEvent.setup();
      render(<CompletionCard {...baseProps} testResults="PASS src/test.ts" />);

      await user.click(screen.getByText("Test results"));
      expect(screen.getByText("PASS src/test.ts")).toBeInTheDocument();
    });
  });

  describe("summary", () => {
    it("renders markdown summary", () => {
      render(<CompletionCard {...baseProps} summary="**Bold** text" />);
      const bold = document.querySelector(".completion-summary strong");
      expect(bold).not.toBeNull();
      expect(bold!.textContent).toBe("Bold");
    });

    it("does not render summary section when empty", () => {
      render(<CompletionCard {...baseProps} summary="" />);
      expect(document.querySelector(".completion-summary")).toBeNull();
    });
  });

  describe("completion actions (no action taken yet)", () => {
    it("shows inspect, revert, keep buttons when hasFileChanges and no completionAction", () => {
      render(<CompletionCard {...baseProps} />);
      expect(screen.getByText("Inspect")).toBeInTheDocument();
      expect(screen.getByText("Revert all")).toBeInTheDocument();
      expect(screen.getByText("Keep as-is")).toBeInTheDocument();
    });

    it("does not show action buttons when hasFileChanges is false", () => {
      render(<CompletionCard {...baseProps} hasFileChanges={false} />);
      expect(screen.queryByText("Inspect")).not.toBeInTheDocument();
      expect(screen.queryByText("Revert all")).not.toBeInTheDocument();
    });

    it("clicking inspect calls onInspect", async () => {
      const user = userEvent.setup();
      const onInspect = vi.fn();
      render(<CompletionCard {...baseProps} onInspect={onInspect} />);

      await user.click(screen.getByText("Inspect"));
      expect(onInspect).toHaveBeenCalled();
    });

    it("clicking revert calls onRevert", async () => {
      const user = userEvent.setup();
      const onRevert = vi.fn();
      render(<CompletionCard {...baseProps} onRevert={onRevert} />);

      await user.click(screen.getByText("Revert all"));
      expect(onRevert).toHaveBeenCalled();
    });

    it("clicking keep calls onKeep", async () => {
      const user = userEvent.setup();
      const onKeep = vi.fn();
      render(<CompletionCard {...baseProps} onKeep={onKeep} />);

      await user.click(screen.getByText("Keep as-is"));
      expect(onKeep).toHaveBeenCalled();
    });
  });

  describe("completion action badges", () => {
    it("shows Committed badge when completionAction is committed", () => {
      render(<CompletionCard {...baseProps} completionAction="committed" />);
      expect(screen.getByText("Committed")).toBeInTheDocument();
      expect(screen.queryByText("Inspect")).not.toBeInTheDocument();
    });

    it("shows Reverted badge when completionAction is reverted", () => {
      render(<CompletionCard {...baseProps} completionAction="reverted" />);
      expect(screen.getByText("Reverted")).toBeInTheDocument();
      expect(screen.queryByText("Inspect")).not.toBeInTheDocument();
    });

    it("shows Kept as-is badge when completionAction is kept", () => {
      render(<CompletionCard {...baseProps} completionAction="kept" />);
      expect(screen.getByText("Kept as-is")).toBeInTheDocument();
      // Should not show the button version
      expect(screen.queryByText("Inspect")).not.toBeInTheDocument();
      expect(screen.queryByText("Revert all")).not.toBeInTheDocument();
    });

    it("does not show badge when hasFileChanges is false even with completionAction", () => {
      render(<CompletionCard {...baseProps} hasFileChanges={false} completionAction="committed" />);
      expect(screen.queryByText("Committed")).not.toBeInTheDocument();
    });
  });

  describe("feedback", () => {
    it("shows send feedback button when onSendFeedback provided and comments > 0", () => {
      const onSendFeedback = vi.fn();
      render(<CompletionCard {...baseProps} onSendFeedback={onSendFeedback} commentCount={3} />);
      expect(screen.getByText("Send feedback (3 comments)")).toBeInTheDocument();
    });

    it("does not show send feedback when commentCount is 0", () => {
      const onSendFeedback = vi.fn();
      render(<CompletionCard {...baseProps} onSendFeedback={onSendFeedback} commentCount={0} />);
      expect(screen.queryByText(/Send feedback/)).not.toBeInTheDocument();
    });

    it("does not show send feedback when onSendFeedback is undefined", () => {
      render(<CompletionCard {...baseProps} commentCount={3} />);
      expect(screen.queryByText(/Send feedback/)).not.toBeInTheDocument();
    });

    it("clicking send feedback calls handler", async () => {
      const user = userEvent.setup();
      const onSendFeedback = vi.fn();
      render(<CompletionCard {...baseProps} onSendFeedback={onSendFeedback} commentCount={2} />);

      await user.click(screen.getByText("Send feedback (2 comments)"));
      expect(onSendFeedback).toHaveBeenCalled();
    });

    it("shows singular 'comment' for count of 1", () => {
      render(<CompletionCard {...baseProps} onSendFeedback={vi.fn()} commentCount={1} />);
      expect(screen.getByText("Send feedback (1 comment)")).toBeInTheDocument();
    });

    it("shows feedback sent badge when feedbackSentCount > 0", () => {
      render(<CompletionCard {...baseProps} feedbackSentCount={4} />);
      expect(screen.getByText("Feedback sent (4 comments)")).toBeInTheDocument();
    });

    it("shows singular in feedback sent badge for count of 1", () => {
      render(<CompletionCard {...baseProps} feedbackSentCount={1} />);
      expect(screen.getByText("Feedback sent (1 comment)")).toBeInTheDocument();
    });

    it("feedback sent badge takes priority over send feedback button", () => {
      render(<CompletionCard {...baseProps} feedbackSentCount={2} onSendFeedback={vi.fn()} commentCount={2} />);
      expect(screen.getByText("Feedback sent (2 comments)")).toBeInTheDocument();
      expect(screen.queryByText(/Send feedback/)).not.toBeInTheDocument();
    });

    it("does not show feedback sent badge when feedbackSentCount is 0", () => {
      render(<CompletionCard {...baseProps} feedbackSentCount={0} />);
      expect(screen.queryByText(/Feedback sent/)).not.toBeInTheDocument();
    });
  });

  describe("copy button", () => {
    it("renders a copy button when summary is present", () => {
      render(<CompletionCard {...baseProps} />);
      expect(
        screen.getByRole("button", { name: /copy response to clipboard/i }),
      ).toBeInTheDocument();
    });

    it("does not render copy button when summary is empty", () => {
      render(<CompletionCard {...baseProps} summary="" />);
      expect(
        screen.queryByRole("button", { name: /copy response to clipboard/i }),
      ).not.toBeInTheDocument();
    });

    it("copies the summary via the clipboard helper and shows a copied state", async () => {
      mockedCopy.mockResolvedValue(true);
      const user = userEvent.setup();

      render(<CompletionCard {...baseProps} summary="the full response body" />);
      const btn = screen.getByRole("button", { name: /copy response to clipboard/i });
      await user.click(btn);

      await waitFor(() => {
        expect(mockedCopy).toHaveBeenCalledWith("the full response body");
        expect(btn).toHaveAttribute("title", "Copied");
      });
      expect(btn.className).toContain("copied");
      mockedCopy.mockReset();
    });

    it("surfaces a failed state when the clipboard helper returns false", async () => {
      mockedCopy.mockResolvedValue(false);
      const user = userEvent.setup();

      render(<CompletionCard {...baseProps} summary="nope" />);
      const btn = screen.getByRole("button", { name: /copy response to clipboard/i });
      await user.click(btn);

      await waitFor(() => {
        expect(btn).toHaveAttribute("title", "Copy failed");
      });
      expect(btn.className).toContain("failed");
      mockedCopy.mockReset();
    });
  });

  describe("worktree mode", () => {
    it("renders 'Merge to main' only when worktreeStatus === 'isolated' and onMerge is supplied", () => {
      const onMerge = vi.fn();
      const { rerender } = render(
        <CompletionCard {...baseProps} />,
      );
      expect(screen.queryByText(/Merge to main/i)).not.toBeInTheDocument();

      rerender(
        <CompletionCard
          {...baseProps}
          worktreeStatus="isolated"
          onMerge={onMerge}
        />,
      );
      expect(screen.getByText(/Merge to main/i)).toBeInTheDocument();
    });

    it("doesn't render Merge when worktreeStatus is isolated but no onMerge is supplied", () => {
      render(<CompletionCard {...baseProps} worktreeStatus="isolated" />);
      expect(screen.queryByText(/Merge to main/i)).not.toBeInTheDocument();
    });

    it("relabels 'Revert all' to 'Discard worktree' when isolated", () => {
      render(
        <CompletionCard {...baseProps} worktreeStatus="isolated" onMerge={vi.fn()} />,
      );
      expect(screen.getByText(/Discard worktree/i)).toBeInTheDocument();
      expect(screen.queryByText(/Revert all/i)).not.toBeInTheDocument();
    });

    it("clicking Merge to main fires onMerge", async () => {
      const onMerge = vi.fn();
      const user = userEvent.setup();
      render(
        <CompletionCard {...baseProps} worktreeStatus="isolated" onMerge={onMerge} />,
      );
      await user.click(screen.getByText(/Merge to main/i));
      expect(onMerge).toHaveBeenCalledOnce();
    });

    it("renders merge conflict list when mergeError.files is non-empty", () => {
      render(
        <CompletionCard
          {...baseProps}
          worktreeStatus="isolated"
          onMerge={vi.fn()}
          mergeError={{
            message: "Merge couldn't complete because of conflicts.",
            files: ["src/main.rs", "README.md"],
          }}
        />,
      );
      expect(screen.getByText(/2 conflicting files/i)).toBeInTheDocument();
      expect(screen.getByText("src/main.rs")).toBeInTheDocument();
      expect(screen.getByText("README.md")).toBeInTheDocument();
    });

    it("does not render merge conflict block when mergeError is null", () => {
      const { container } = render(
        <CompletionCard {...baseProps} worktreeStatus="isolated" onMerge={vi.fn()} />,
      );
      expect(container.querySelector(".merge-conflict")).toBeNull();
    });

    it("renders a worktree-only action bar when isolated thread has no file changes", () => {
      // Phase 2 gap fix: text-only worktree turns still need Merge/
      // Discard buttons or the worktree orphans until startup recovery.
      const onMerge = vi.fn();
      const onRevert = vi.fn();
      render(
        <CompletionCard
          {...baseProps}
          hasFileChanges={false}
          filesChanged={[]}
          worktreeStatus="isolated"
          onMerge={onMerge}
          onRevert={onRevert}
        />,
      );
      expect(screen.getByText(/Merge to main/i)).toBeInTheDocument();
      expect(screen.getByText(/Discard worktree/i)).toBeInTheDocument();
    });

    it("does not render worktree-only bar for non-isolated threads with no file changes", () => {
      render(<CompletionCard {...baseProps} hasFileChanges={false} filesChanged={[]} />);
      expect(screen.queryByText(/Merge to main/i)).not.toBeInTheDocument();
      expect(screen.queryByText(/Discard worktree/i)).not.toBeInTheDocument();
    });

    it("renders 'Use yours' / 'Keep main' resolution buttons when onResolveMerge is wired", async () => {
      const onResolve = vi.fn();
      const user = userEvent.setup();
      render(
        <CompletionCard
          {...baseProps}
          worktreeStatus="isolated"
          onMerge={vi.fn()}
          onResolveMerge={onResolve}
          mergeError={{ message: "x", files: ["src/main.rs"] }}
        />,
      );

      const useYours = screen.getByRole("button", { name: /Use yours/i });
      const keepMain = screen.getByRole("button", { name: /Keep main/i });
      expect(useYours).toBeInTheDocument();
      expect(keepMain).toBeInTheDocument();

      await user.click(useYours);
      expect(onResolve).toHaveBeenLastCalledWith("prefer_theirs");

      await user.click(keepMain);
      expect(onResolve).toHaveBeenLastCalledWith("prefer_ours");
    });

    it("falls back to plain Discard hint when onResolveMerge is not wired", () => {
      render(
        <CompletionCard
          {...baseProps}
          worktreeStatus="isolated"
          onMerge={vi.fn()}
          mergeError={{ message: "x", files: ["README.md"] }}
        />,
      );
      expect(screen.queryByRole("button", { name: /Use yours/i })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: /Keep main/i })).not.toBeInTheDocument();
      expect(screen.getByText(/Discard this worktree/i)).toBeInTheDocument();
    });

    it("singular conflict wording when exactly one file conflicts", () => {
      render(
        <CompletionCard
          {...baseProps}
          worktreeStatus="isolated"
          onMerge={vi.fn()}
          mergeError={{ message: "x", files: ["README.md"] }}
        />,
      );
      expect(screen.getByText(/1 conflicting file/i)).toBeInTheDocument();
      // Should NOT match "files" (plural)
      expect(screen.queryByText(/conflicting files/i)).not.toBeInTheDocument();
    });
  });
});
