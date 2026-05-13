import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { createGitWorkspace, getHeadHash, isClean } from "./fixtures/git-workspace";
import { getDataDir } from "./fixtures/tauri-app";
import { readdirSync, existsSync } from "fs";
import { join } from "path";

test.describe("Full-Stack: Git Revert", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = createGitWorkspace();
  });

  test("file edit -> discard worktree -> reverted badge", async ({ page }) => {
    // Phase 2: git-backed workspaces run each thread in an isolated
    // worktree, so the main checkout stays clean regardless of what the
    // agent does. "Revert all" is relabeled to "Discard worktree" and
    // removes the per-thread checkout under $PANES_DATA_DIR/worktrees.
    const hashBefore = getHeadHash(wsPath);

    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "edit some files");

    await waitForCompletion(page);

    // Main checkout should be untouched throughout — the agent operated
    // inside its worktree, not here.
    expect(isClean(wsPath)).toBe(true);
    expect(getHeadHash(wsPath)).toBe(hashBefore);

    // A worktree directory should exist for this thread.
    const worktreesRoot = join(getDataDir(), "worktrees");
    await expect
      .poll(() => (existsSync(worktreesRoot) ? readdirSync(worktreesRoot).length : 0), { timeout: 5000 })
      .toBeGreaterThanOrEqual(1);
    const worktreeCountBefore = readdirSync(worktreesRoot).length;

    await page.click("button:has-text('Discard worktree')");
    const confirmBtn = page.locator(".revert-confirm button:has-text('Revert')");
    await confirmBtn.waitFor({ timeout: 3000 });
    await confirmBtn.click();

    await expect(page.locator("text=Reverted")).toBeVisible({ timeout: 5000 });
    // Discard removes the worktree dir.
    await expect
      .poll(() => readdirSync(worktreesRoot).length, { timeout: 5000 })
      .toBeLessThan(worktreeCountBefore);
    // Main checkout still clean at the same commit.
    expect(isClean(wsPath)).toBe(true);
    expect(getHeadHash(wsPath)).toBe(hashBefore);
  });
});
