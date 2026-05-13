import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt } from "./helpers";
import { createGitWorkspace } from "./fixtures/git-workspace";
import { getDataDir } from "./fixtures/tauri-app";
import { readdirSync, existsSync } from "fs";
import { join } from "path";

/**
 * Phase 2 git worktrees — fullstack invariant.
 *
 * The unit tests in crates/panes-core prove the worktree primitives
 * (create / remove / merge) work on a real git repo. This test proves
 * the bigger promise: two agent threads in the same git-backed
 * workspace can actually run concurrently end-to-end, each in its own
 * worktree directory under $PANES_DATA_DIR/worktrees/.
 *
 * Mirrors workspace-guard.spec.ts's serial pattern (second prompt
 * starts while first is running), but against a git-initialized
 * workspace — which now lifts the one-thread guard.
 */
test.describe("Full-Stack: Git Worktrees", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = createGitWorkspace();
  });

  test("two concurrent threads in a git workspace each get their own worktree", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);

    // Start the first (slow) thread.
    await sendPrompt(page, "slow task please");

    // Wait until the first thread is mid-run — at least one tool group
    // rendered means the adapter has started emitting events, which
    // means the thread is in SessionManager's active map.
    await page.locator(".tool-group").first().waitFor({ timeout: 15_000 });

    // Start a second thread via "New thread" so the textarea resets
    // and the frontend fires a fresh start_thread call. Pre-Phase-2
    // this would throw WorkspaceOccupied on the backend.
    await page.click(".thread-list-new");
    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");

    // Two threads should exist in the sidebar — the second didn't
    // bounce with a WorkspaceOccupied error. `.thread-list-item` is
    // rendered once per thread in the workspace's thread column.
    await expect
      .poll(async () => page.locator(".thread-list-item").count(), { timeout: 15_000 })
      .toBeGreaterThanOrEqual(2);

    // The core invariant of Phase 2: each thread got its own isolated
    // worktree. Poll the data dir because thread #2 starts spawning
    // async after the UI send. Worktrees stick around post-completion
    // until the user picks Merge/Discard, so the dir count is stable.
    const worktreesRoot = join(getDataDir(), "worktrees");
    await expect
      .poll(
        () => (existsSync(worktreesRoot) ? readdirSync(worktreesRoot).length : 0),
        { timeout: 30_000 },
      )
      .toBeGreaterThanOrEqual(2);

    // Each worktree dir is named by thread id and must contain the
    // checked-out working tree (README.md from createGitWorkspace).
    const worktrees = readdirSync(worktreesRoot);
    for (const wt of worktrees) {
      expect(
        existsSync(join(worktreesRoot, wt, "README.md")),
        `worktree ${wt} missing checked-out files`,
      ).toBe(true);
    }
  });
});
