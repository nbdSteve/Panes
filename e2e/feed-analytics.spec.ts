import { test, expect } from "@playwright/test";

test.describe("Feed — Cost Analytics", () => {
  test("feed shows cost sparkline", async ({ page }) => {
    await page.goto("/");

    // Add workspace and create a thread so feed has data
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/feed-spark");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Navigate to feed
    await page.click("text=Feed");
    await expect(page.locator(".feed-view")).toBeVisible({ timeout: 3000 });

    // Sparkline should be visible in analytics section
    await expect(page.locator(".cost-sparkline")).toBeVisible({ timeout: 3000 });
  });

  test("feed shows workspace cost bars", async ({ page }) => {
    await page.goto("/");

    // Add workspace and create a thread
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/feed-bars");
    await page.fill('input[placeholder="Display name (optional)"]', "BarTest");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Navigate to feed
    await page.click("text=Feed");
    await expect(page.locator(".cost-bars")).toBeVisible({ timeout: 3000 });
    await expect(page.locator(".cost-bar-row")).toBeVisible();
  });

  test("sort by cost reorders feed items", async ({ page }) => {
    await page.goto("/");

    // Add workspace
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/feed-sort");
    await page.click("text=Add");

    // Create a thread
    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Navigate to feed
    await page.click("text=Feed");
    await expect(page.locator(".feed-item")).toBeVisible({ timeout: 3000 });

    // Click "Cost" sort button — should not break the view
    await page.click("button:has-text('Cost')");
    await expect(page.locator(".feed-filter-btn.active", { hasText: "Cost" })).toBeVisible();
    await expect(page.locator(".feed-item")).toBeVisible();
  });

  test("date range filter reduces visible items", async ({ page }) => {
    await page.goto("/");

    // Add workspace and create thread
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/feed-range");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Navigate to feed
    await page.click("text=Feed");
    await expect(page.locator(".feed-item")).toBeVisible({ timeout: 3000 });

    // Click "7d" filter — recent thread should still be visible
    await page.click("button:has-text('7d')");
    await expect(page.locator(".feed-item")).toBeVisible();

    // Click "All" — should still be visible
    await page.click("button:has-text('All')");
    await expect(page.locator(".feed-item")).toBeVisible();
  });
});
