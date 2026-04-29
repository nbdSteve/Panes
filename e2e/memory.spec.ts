import { test, expect } from "@playwright/test";

test.describe("Memory & Briefings", () => {
  test("context indicator shows injected memories and briefing", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    // Send a prompt — context injection indicator should appear
    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Start a second thread — should show context indicator if memories were extracted
    await page.click(".thread-list-new");
    await page.fill("textarea", "follow up task");
    await page.press("textarea", "Enter");

    // Context indicator should show what was injected
    await expect(page.locator(".context-indicator")).toBeVisible({ timeout: 3000 });
    await expect(page.locator(".context-indicator")).toContainText("memor");
  });

  test("context indicator is expandable to show details", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await page.fill("textarea", "first task");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click(".thread-list-new");
    await page.fill("textarea", "second task");
    await page.press("textarea", "Enter");

    await expect(page.locator(".context-indicator")).toBeVisible({ timeout: 3000 });

    // Click to expand
    await page.click(".context-indicator");
    await expect(page.locator(".context-detail")).toBeVisible();
  });

  test("briefing editor is accessible from workspace", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.fill('input[placeholder="Display name (optional)"]', "Briefing Test");
    await page.click("text=Add");

    // Should be able to access briefing editor
    await page.click("text=Briefing");

    await expect(page.locator(".briefing-editor")).toBeVisible();
    await expect(page.locator(".briefing-editor textarea")).toBeVisible();
  });

  test("briefing can be saved and persists", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await page.click("text=Briefing");
    await page.fill(".briefing-editor textarea", "Always use TypeScript, never JavaScript");
    await page.click(".briefing-editor button:has-text('Save')");

    // Should show saved confirmation
    await expect(page.locator("text=Saved")).toBeVisible({ timeout: 2000 });

    // Reload and verify
    await page.reload();
    await page.click(".sidebar-item:has-text('test-ws')");
    await page.click("text=Briefing");
    await expect(page.locator(".briefing-editor textarea")).toHaveValue("Always use TypeScript, never JavaScript");
  });

  test("memory panel shows extracted memories", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    // Complete a thread so memories can be extracted
    await page.fill("textarea", "Use Zod for validation, not Joi");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open memory panel
    await page.click("text=Memory");

    await expect(page.locator(".memory-panel")).toBeVisible();
    await expect(page.locator(".memory-item")).toHaveCount(1, { timeout: 3000 });
  });

  test("memory can be pinned, edited, and deleted", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await page.fill("textarea", "Always prefer composition over inheritance");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("text=Memory");
    await expect(page.locator(".memory-item")).toHaveCount(1, { timeout: 3000 });

    // Pin
    await page.click(".memory-item .pin-button");
    await expect(page.locator(".memory-item.pinned")).toBeVisible();

    // Edit
    await page.click(".memory-item .edit-button");
    await page.fill(".memory-item textarea", "Updated memory content");
    await page.click(".memory-item button:has-text('Save')");
    await expect(page.locator(".memory-item")).toContainText("Updated memory content");

    // Delete
    await page.click(".memory-item .delete-button");
    await expect(page.locator(".memory-item")).toHaveCount(0);
  });
});
