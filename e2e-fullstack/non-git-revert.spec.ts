import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { createNonGitWorkspace } from "./fixtures/nongit-workspace";
import { existsSync } from "fs";
import { resolve } from "path";

test.describe("Full-Stack: Auto-Init Git", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = createNonGitWorkspace();
  });

  test("non-git workspace is auto-initialized and thread completes with file changes", async ({
    page,
  }) => {
    // Workspace starts without git.
    expect(existsSync(resolve(wsPath, ".git"))).toBe(false);

    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "edit some files");

    await waitForCompletion(page);

    // After thread completion, git should have been auto-initialized.
    expect(existsSync(resolve(wsPath, ".git"))).toBe(true);

    // The completion card should show file changes were detected.
    await expect(page.locator("text=files changed")).toBeVisible({ timeout: 5000 });
  });
});
