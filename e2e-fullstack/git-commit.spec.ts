import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { createGitWorkspace, isClean, getHeadHash } from "./fixtures/git-workspace";
import { getDataDir } from "./fixtures/tauri-app";
import { readdirSync, existsSync } from "fs";
import { execSync } from "child_process";
import { join } from "path";

test.describe("Full-Stack: Git Commit", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = createGitWorkspace();
  });

  test("file edit -> commit -> committed badge", async ({ page }) => {
    // Phase 2: the agent edits files inside a per-thread worktree. The
    // main checkout (wsPath) stays clean throughout. The Commit flow
    // from the completion card commits INSIDE the worktree, so we
    // verify the commit landed on the worktree's branch, not on the
    // main repo HEAD.
    const mainHeadBefore = getHeadHash(wsPath);

    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "edit some files");

    await waitForCompletion(page);

    // Wait for the worktree to appear on disk and pick it up.
    const worktreesRoot = join(getDataDir(), "worktrees");
    await expect
      .poll(() => (existsSync(worktreesRoot) ? readdirSync(worktreesRoot).length : 0), { timeout: 5000 })
      .toBeGreaterThanOrEqual(1);
    const worktreeDir = join(worktreesRoot, readdirSync(worktreesRoot)[0]);

    // Fake adapter writes happen async after Complete — wait for the
    // worktree to actually have pending edits before opening the diff.
    await expect.poll(() => isClean(worktreeDir), { timeout: 5000 }).toBe(false);

    // Open the diff viewer via Inspect
    await page.click("button:has-text('Inspect')");
    await page.locator(".diff-modal").waitFor({ timeout: 5000 });

    // Switch to commit view via action bar
    await page.click(".diff-action-bar button:has-text('Commit')");
    await page.locator("text=Commit Changes").waitFor({ timeout: 3000 });

    // Fill commit message and submit
    await page.fill('textarea[placeholder="Describe your changes..."]', "test: fullstack commit");
    await page.click(".diff-commit-btn");

    await expect(page.locator("text=Committed")).toBeVisible({ timeout: 5000 });

    // Commit landed inside the worktree — it's clean now with an
    // advanced HEAD. Main repo untouched until the user merges.
    expect(isClean(worktreeDir)).toBe(true);
    const worktreeHead = execSync("git rev-parse HEAD", { cwd: worktreeDir })
      .toString()
      .trim();
    expect(worktreeHead).not.toBe(mainHeadBefore);
    expect(getHeadHash(wsPath)).toBe(mainHeadBefore);
  });
});
