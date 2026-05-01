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
    expect(isClean(wsPath)).toBe(false);

    await page.click("button:has-text('Commit')");
    const confirmBtn = page.locator("button:has-text('Confirm')");
    await confirmBtn.waitFor({ timeout: 3000 });
    await confirmBtn.click();

    await expect(page.locator("text=Committed")).toBeVisible({ timeout: 5000 });
    expect(isClean(wsPath)).toBe(true);
  });
});
