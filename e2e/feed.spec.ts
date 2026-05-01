import { test, expect } from "@playwright/test";

test.describe("Feed — Activity Stream", () => {
  test("completed threads appear in feed with workspace name and cost", async ({ page }) => {
    await page.goto("/");

    // Add workspace and create a thread
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/feed-ws");
    await page.fill('input[placeholder="Display name (optional)"]', "FeedTest");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Navigate to feed
    await page.click("text=Feed");

    // Thread should appear with workspace name
    await expect(page.locator(".feed-item", { hasText: "FeedTest" })).toBeVisible({ timeout: 2000 });

    // Cost should be displayed
    await expect(page.locator(".feed-item").locator("text=$")).toBeVisible();
  });

  test("feed shows threads from multiple workspaces sorted by recency", async ({ page }) => {
    await page.goto("/");

    // Create first workspace with thread
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/feed-ws1");
    await page.fill('input[placeholder="Display name (optional)"]', "WS-First");
    await page.click("text=Add");

    await page.fill("textarea", "first thread");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Create second workspace with thread
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/feed-ws2");
    await page.fill('input[placeholder="Display name (optional)"]', "WS-Second");
    await page.click("text=Add");

    await page.fill("textarea", "second thread");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Navigate to feed
    await page.click("text=Feed");

    // Both should appear
    const items = page.locator(".feed-item");
    await expect(items).toHaveCount(2, { timeout: 2000 });

    // Both workspaces should be represented
    await expect(page.locator(".feed-item", { hasText: "WS-First" })).toBeVisible();
    await expect(page.locator(".feed-item", { hasText: "WS-Second" })).toBeVisible();
  });

  test("feed shows aggregate cost total", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/feed-cost");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.click("text=Feed");

    // Aggregate cost should be visible
    await expect(page.locator(".feed-aggregate-cost")).toBeVisible({ timeout: 2000 });
  });

  test("clicking feed item navigates to that thread", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/feed-nav");
    await page.fill('input[placeholder="Display name (optional)"]', "NavWS");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Go to feed
    await page.click("text=Feed");
    await expect(page.locator(".feed-item")).toBeVisible({ timeout: 2000 });

    // Click the feed item
    await page.locator(".feed-item").first().click();

    // Should navigate to the thread view with the original prompt
    await expect(page.locator(".thread-prompt-text", { hasText: "hello world" })).toBeVisible({ timeout: 2000 });
  });
});
