import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { createGitWorkspace, isClean, lastCommitMessage } from "./fixtures/git-workspace";

test.describe("Full-Stack: Git Commit", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = createGitWorkspace();
  });

  test("file edit -> commit -> committed badge", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "edit some files");

    await waitForCompletion(page);
    await expect.poll(() => isClean(wsPath), { timeout: 5000 }).toBe(false);

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
    expect(isClean(wsPath)).toBe(true);
  });
});
