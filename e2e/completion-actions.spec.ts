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

test.describe("Completion Actions — Inspect, Revert, Keep", () => {
  test("inspect opens diff modal with commit button at bottom", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Inspect')");

    // Should open the diff modal
    await expect(page.locator(".diff-overlay")).toBeVisible();

    // Should have commit button in action bar
    await expect(page.locator(".diff-action-bar .diff-commit-trigger")).toBeVisible();
  });

  test("commit from inspect modal shows committed badge", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Inspect')");
    await expect(page.locator(".diff-overlay")).toBeVisible();

    // Click commit button, enter message, confirm
    await page.click(".diff-commit-trigger");
    await expect(page.locator(".diff-commit-view")).toBeVisible();
    await page.fill(".diff-commit-input", "Custom commit message");
    await page.click(".diff-commit-btn");

    // Should show committed state
    await expect(page.locator("text=Committed")).toBeVisible({ timeout: 2000 });

    // Inspect/revert buttons should be replaced with badge
    await expect(page.locator("button:has-text('Inspect')")).not.toBeVisible();
    await expect(page.locator("button:has-text('Revert')")).not.toBeVisible();
  });

  test("revert restores pre-thread state with confirmation", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Revert all')");

    // Should show confirmation dialog
    await expect(page.locator(".revert-confirm")).toBeVisible();
    await expect(page.locator("text=Undo all changes")).toBeVisible();

    await page.click(".revert-confirm button:has-text('Revert')");

    // Should show reverted state
    await expect(page.locator("text=Reverted")).toBeVisible({ timeout: 2000 });

    // Inspect/revert buttons should be gone
    await expect(page.locator("button:has-text('Inspect')")).not.toBeVisible();
  });

  test("keep dismisses action buttons", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Keep')");

    // Buttons should be dismissed
    await expect(page.locator("button:has-text('Inspect')")).not.toBeVisible();
    await expect(page.locator("button:has-text('Revert')")).not.toBeVisible();
    await expect(page.locator("button:has-text('Keep')")).not.toBeVisible();
  });

  test.skip("completion card shows files changed summary", async ({ page }) => {
    // Not yet implemented: .files-changed UI element
  });
});
