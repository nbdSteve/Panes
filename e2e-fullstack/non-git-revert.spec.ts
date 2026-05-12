import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { createNonGitWorkspace, fileExists } from "./fixtures/nongit-workspace";

test.describe("Full-Stack: Non-Git Revert (Shadow Tracker)", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = createNonGitWorkspace();
  });

  test("file edit -> inspect shows diff -> revert deletes created files", async ({
    page,
  }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "edit some files");

    await waitForCompletion(page);

    // Fake adapter writes the files in a background task — poll until both
    // actually land on disk before we trigger revert, otherwise the shadow
    // tracker's revert would run before a write has happened and leave the
    // later-written file behind.
    await expect
      .poll(() => fileExists(wsPath, "src/main.rs") && fileExists(wsPath, "src/lib.rs"), {
        timeout: 5000,
      })
      .toBe(true);

    // Inspect should open the diff modal with non-empty content — proving
    // the shadow tracker produced a usable unified diff even though there's
    // no git repo backing the workspace.
    await page.click("button:has-text('Inspect')");
    const diffModal = page.locator(".diff-modal");
    await diffModal.waitFor({ timeout: 5000 });
    await expect(diffModal).toContainText("main.rs");

    // Close and revert.
    await page.keyboard.press("Escape");
    await page.click("button:has-text('Revert all')");
    const confirmBtn = page.locator(".revert-confirm button:has-text('Revert')");
    await confirmBtn.waitFor({ timeout: 3000 });
    await confirmBtn.click();

    await expect(page.locator("text=Reverted")).toBeVisible({ timeout: 5000 });

    // The shadow tracker records tombstones for created files; revert must
    // delete them from disk.
    expect(fileExists(wsPath, "src/main.rs")).toBe(false);
    expect(fileExists(wsPath, "src/lib.rs")).toBe(false);
  });
});
