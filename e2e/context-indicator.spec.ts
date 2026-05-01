import { test, expect } from "@playwright/test";

test.describe("Context & Cost Indicators", () => {
  test("context usage percentage appears during thread", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/ctx-ws");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Context usage badge should show a percentage
    await expect(page.locator(".context-usage")).toBeVisible();
    const text = await page.locator(".context-usage").textContent();
    expect(text).toMatch(/\d+%/);
  });

  test("cost badge appears in thread header", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/cost-badge-ws");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Cost badge in header
    await expect(page.locator(".thread-header-right .cost-badge")).toBeVisible();
    const text = await page.locator(".thread-header-right .cost-badge").textContent();
    expect(text).toMatch(/\$/);
  });

  test("sidebar shows workspace cost", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/sidebar-cost-ws");
    await page.fill('input[placeholder="Display name (optional)"]', "CostWS");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Sidebar workspace item should show cost
    const wsItem = page.locator(".sidebar-item", { hasText: "CostWS" });
    await expect(wsItem.locator(".workspace-cost")).toBeVisible();
    const costText = await wsItem.locator(".workspace-cost").textContent();
    expect(costText).toMatch(/\$/);
  });
});
