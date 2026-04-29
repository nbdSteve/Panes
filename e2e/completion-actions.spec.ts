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

test.describe("Completion Actions — Commit, Revert, Keep", () => {
  test("commit opens dialog with auto-generated message", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Commit')");

    // Should open a commit dialog
    await expect(page.locator(".commit-dialog")).toBeVisible();

    // Should pre-fill with thread summary as commit message
    const messageInput = page.locator(".commit-dialog textarea");
    await expect(messageInput).toBeVisible();
    const value = await messageInput.inputValue();
    expect(value.length).toBeGreaterThan(0);
  });

  test("commit dialog allows editing message before confirming", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Commit')");
    await expect(page.locator(".commit-dialog")).toBeVisible();

    // Edit the message
    await page.fill(".commit-dialog textarea", "Custom commit message");
    await page.click(".commit-dialog button:has-text('Confirm')");

    // Should show committed state
    await expect(page.locator("text=Committed")).toBeVisible({ timeout: 2000 });

    // Commit/revert buttons should be replaced
    await expect(page.locator("button:has-text('Commit')")).not.toBeVisible();
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

    // Commit/revert buttons should be gone
    await expect(page.locator("button:has-text('Commit')")).not.toBeVisible();
  });

  test("keep dismisses action buttons", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Keep')");

    // Buttons should be dismissed
    await expect(page.locator("button:has-text('Commit')")).not.toBeVisible();
    await expect(page.locator("button:has-text('Revert')")).not.toBeVisible();
    await expect(page.locator("button:has-text('Keep')")).not.toBeVisible();
  });

  test("completion card shows files changed summary", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Should show file change summary
    await expect(page.locator(".files-changed")).toBeVisible();
    await expect(page.locator(".files-changed")).toContainText("file");
  });
});
