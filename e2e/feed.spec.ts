import { test, expect } from "@playwright/test";

test.describe("Feed — Activity Stream", () => {
  test("completed threads appear in feed with workspace name and cost", async ({ page }) => {
    await page.goto("/");

    // Add workspace and complete a thread
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.fill('input[placeholder="Display name (optional)"]', "Backend");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Switch to Feed
    await page.click(".sidebar-item:has-text('Feed')");

    // Feed should show the completed thread
    await expect(page.locator(".feed-item")).toHaveCount(1);
    await expect(page.locator(".feed-item").first()).toContainText("Backend");
    await expect(page.locator(".feed-item .feed-item-cost")).toBeVisible();
  });

  test("feed shows threads from multiple workspaces sorted by recency", async ({ page }) => {
    await page.goto("/");

    // Add first workspace and send
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/ws-one");
    await page.fill('input[placeholder="Display name (optional)"]', "Backend");
    await page.click("text=Add");

    await page.fill("textarea", "first task");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Add second workspace and send
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/ws-two");
    await page.fill('input[placeholder="Display name (optional)"]', "Frontend");
    await page.click("text=Add");

    await page.fill("textarea", "second task");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Switch to Feed
    await page.click(".sidebar-item:has-text('Feed')");

    // Feed should show both, most recent first
    const items = page.locator(".feed-item");
    await expect(items).toHaveCount(2);
    await expect(items.first()).toContainText("Frontend");
    await expect(items.last()).toContainText("Backend");
  });

  test("feed shows aggregate cost total", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await page.fill("textarea", "hello");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click(".sidebar-item:has-text('Feed')");

    // Feed should show total cost
    await expect(page.locator(".feed-total-cost")).toBeVisible();
  });

  test("clicking feed item navigates to that thread", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.fill('input[placeholder="Display name (optional)"]', "Backend");
    await page.click("text=Add");

    await page.fill("textarea", "hello from feed");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click(".sidebar-item:has-text('Feed')");
    await page.click(".feed-item");

    // Should navigate to the workspace and thread
    await expect(page.locator(".thread-prompt-text", { hasText: "hello from feed" })).toBeVisible();
  });
});
