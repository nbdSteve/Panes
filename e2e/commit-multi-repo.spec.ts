import { test, expect } from "@playwright/test";

function addWorkspaceAndSend(page: any, prompt: string) {
  return (async () => {
    await page.goto("/");
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");
    await page.fill("textarea", prompt);
    await page.press("textarea", "Enter");
  })();
}

test.describe("Commit via Inspect Modal", () => {
  test("inspect opens diff modal with commit button in action bar", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Inspect')");
    await expect(page.locator(".diff-overlay")).toBeVisible();

    // Should have a commit button in the bottom action bar
    await expect(page.locator(".diff-action-bar .diff-commit-trigger")).toBeVisible();
  });

  test("commit button opens commit view with file checkboxes", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Inspect')");
    await expect(page.locator(".diff-overlay")).toBeVisible();

    // Click commit in action bar
    await page.click(".diff-commit-trigger");

    // Should show commit view
    await expect(page.locator(".diff-commit-view")).toBeVisible();
    await expect(page.locator(".diff-commit-input")).toBeVisible();
    await expect(page.locator(".diff-commit-file")).toHaveCount(3, { timeout: 2000 }); // select-all + 2 files
  });

  test("file checkboxes toggle selection", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Inspect')");
    await page.click(".diff-commit-trigger");
    await expect(page.locator(".diff-commit-view")).toBeVisible({ timeout: 2000 });

    // All files checked by default
    const fileCheckboxes = page.locator(".diff-commit-file:not(.diff-commit-select-all) input[type='checkbox']");
    const count = await fileCheckboxes.count();
    for (let i = 0; i < count; i++) {
      await expect(fileCheckboxes.nth(i)).toBeChecked();
    }

    // Uncheck first file
    await fileCheckboxes.nth(0).uncheck();
    await expect(fileCheckboxes.nth(0)).not.toBeChecked();

    // Button text should update
    await expect(page.locator(".diff-commit-btn")).toContainText(`${count - 1} file`);
  });

  test("select all / deselect all works", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Inspect')");
    await page.click(".diff-commit-trigger");
    await expect(page.locator(".diff-commit-view")).toBeVisible({ timeout: 2000 });

    // Click "Select all" to deselect all (since all are already checked)
    const selectAll = page.locator(".diff-commit-select-all input[type='checkbox']");
    await selectAll.uncheck();

    // All file checkboxes should be unchecked
    const fileCheckboxes = page.locator(".diff-commit-file:not(.diff-commit-select-all) input[type='checkbox']");
    const count = await fileCheckboxes.count();
    for (let i = 0; i < count; i++) {
      await expect(fileCheckboxes.nth(i)).not.toBeChecked();
    }

    // Commit button should be disabled
    await expect(page.locator(".diff-commit-btn")).toBeDisabled();

    // Re-check select all
    await selectAll.check();
    for (let i = 0; i < count; i++) {
      await expect(fileCheckboxes.nth(i)).toBeChecked();
    }
  });

  test("commit button disabled when no files selected", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Inspect')");
    await page.click(".diff-commit-trigger");
    await expect(page.locator(".diff-commit-view")).toBeVisible({ timeout: 2000 });

    // Deselect all files
    const selectAll = page.locator(".diff-commit-select-all input[type='checkbox']");
    await selectAll.uncheck();

    // Button should be disabled
    await expect(page.locator(".diff-commit-btn")).toBeDisabled();
  });

  test("successful commit shows committed badge", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Inspect')");
    await page.click(".diff-commit-trigger");
    await expect(page.locator(".diff-commit-input")).toBeVisible({ timeout: 2000 });

    // Enter commit message and confirm
    await page.fill(".diff-commit-input", "feat: add new feature");
    await page.click(".diff-commit-btn");

    // Should show committed state (modal closes, badge appears)
    await expect(page.locator(".diff-overlay")).not.toBeVisible({ timeout: 2000 });
    await expect(page.locator("text=Committed")).toBeVisible({ timeout: 2000 });
  });
});
