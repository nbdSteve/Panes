import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { createGitWorkspace, getHeadHash, isClean } from "./fixtures/git-workspace";

test.describe("Full-Stack: Git Revert", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = createGitWorkspace();
  });

  test("file edit -> revert all -> reverted badge", async ({ page }) => {
    const hashBefore = getHeadHash(wsPath);

    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "edit some files");

    await waitForCompletion(page);
    // Fake adapter writes files in a background task that may land after
    // Complete fires. Poll until the workspace is actually dirty before
    // asserting the revert restores it.
    await expect.poll(() => isClean(wsPath), { timeout: 5000 }).toBe(false);

    await page.click("button:has-text('Revert all')");
    const confirmBtn = page.locator(".revert-confirm button:has-text('Revert')");
    await confirmBtn.waitFor({ timeout: 3000 });
    await confirmBtn.click();

    await expect(page.locator("text=Reverted")).toBeVisible({ timeout: 5000 });
    expect(isClean(wsPath)).toBe(true);
    expect(getHeadHash(wsPath)).toBe(hashBefore);
  });
});
