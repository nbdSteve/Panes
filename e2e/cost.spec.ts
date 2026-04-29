import { test, expect } from "@playwright/test";

test.describe("Cost Visibility", () => {
  test("running cost updates during active thread", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    // Multi-step has multiple cost_update events
    await page.fill("textarea", "do something complex");
    await page.press("textarea", "Enter");

    // Cost badge should appear and update during execution
    await expect(page.locator(".cost-badge")).toBeVisible({ timeout: 3000 });

    // Wait for completion
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });

    // Final cost should be shown in completion card
    await expect(page.locator(".completion-stat-value").first()).toBeVisible();
  });

  test("completion card shows cost, duration, and turns", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // All three stats should be present
    await expect(page.locator(".completion-stat-label:has-text('Cost')")).toBeVisible();
    await expect(page.locator(".completion-stat-label:has-text('Duration')")).toBeVisible();
    await expect(page.locator(".completion-stat-label:has-text('Turns')")).toBeVisible();
  });

  test("workspace cost shown in sidebar", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.fill('input[placeholder="Display name (optional)"]', "Cost Test");
    await page.click("text=Add");

    await page.fill("textarea", "hello");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Sidebar workspace item should show cumulative cost
    const wsItem = page.locator(".sidebar-item", { hasText: "Cost Test" });
    await expect(wsItem.locator(".workspace-cost")).toBeVisible();
  });
});
