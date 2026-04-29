import { test, expect } from "@playwright/test";

test.describe("Persistence — Survives Reload", () => {
  test("workspaces persist across page reload", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/persist-ws");
    await page.fill('input[placeholder="Display name (optional)"]', "Persistent");
    await page.click("text=Add");

    await expect(page.locator(".sidebar-item", { hasText: "Persistent" })).toBeVisible();

    // Reload the page
    await page.reload();

    // Workspace should still be there
    await expect(page.locator(".sidebar-item", { hasText: "Persistent" })).toBeVisible();
  });

  test("completed threads persist across page reload", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/persist-ws");
    await page.fill('input[placeholder="Display name (optional)"]', "Persistent");
    await page.click("text=Add");

    await page.fill("textarea", "persistent message");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Reload
    await page.reload();

    // Click workspace
    await page.click(".sidebar-item:has-text('Persistent')");

    // Thread should still be there
    await expect(page.locator(".thread-list-item")).toHaveCount(1);
    await page.click(".thread-list-item");
    await expect(page.locator(".thread-prompt-text", { hasText: "persistent message" })).toBeVisible();
    await expect(page.locator(".completion-card")).toBeVisible();
  });

  test("thread count badge persists across reload", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/persist-ws");
    await page.fill('input[placeholder="Display name (optional)"]', "Badge WS");
    await page.click("text=Add");

    await page.fill("textarea", "message one");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.reload();

    // Badge should still show correct count
    const wsItem = page.locator(".sidebar-item", { hasText: "Badge WS" });
    await expect(wsItem.locator(".thread-count", { hasText: "1" })).toBeVisible();
  });
});
