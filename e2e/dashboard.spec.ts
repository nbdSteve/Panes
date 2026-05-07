import { test, expect } from "@playwright/test";

test.describe("Dashboard — Multi-Workspace Overview", () => {
  test("dashboard is default view on launch", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".dashboard-view")).toBeVisible({ timeout: 3000 });
  });

  test("workspace cards show correct status", async ({ page }) => {
    await page.goto("/");

    // Add a workspace
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/dash-ws");
    await page.fill('input[placeholder="Display name (optional)"]', "DashTest");
    await page.click("text=Add");

    // Navigate back to dashboard
    await page.click("text=Dashboard");
    await expect(page.locator(".dashboard-card")).toBeVisible({ timeout: 3000 });
    await expect(page.locator(".dashboard-card", { hasText: "DashTest" })).toBeVisible();
  });

  test("gate card on dashboard allows approval", async ({ page }) => {
    await page.goto("/");

    // Add workspace
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/dash-gate");
    await page.fill('input[placeholder="Display name (optional)"]', "GateWS");
    await page.click("text=Add");

    // Send a gated prompt
    await page.fill("textarea", "do something dangerous");
    await page.press("textarea", "Enter");

    // Wait for gate in thread view
    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 3000 });

    // Navigate to dashboard
    await page.click("text=Dashboard");

    // Dashboard should show the gate with Continue button
    await expect(page.locator(".dashboard-card", { hasText: "GateWS" })).toBeVisible({ timeout: 3000 });
    const card = page.locator(".dashboard-card", { hasText: "GateWS" });
    await expect(card.locator(".dashboard-card-status", { hasText: "gate" })).toBeVisible();
    await expect(card.locator("button:has-text('Continue')")).toBeVisible();

    // Approve from dashboard
    await card.locator("button:has-text('Continue')").click();

    // Status should change from gate
    await expect(card.locator(".dashboard-card-status", { hasText: "gate" })).not.toBeVisible({ timeout: 5000 });
  });

  test("clicking workspace card navigates to detail", async ({ page }) => {
    await page.goto("/");

    // Add workspace
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/dash-nav");
    await page.fill('input[placeholder="Display name (optional)"]', "NavWS");
    await page.click("text=Add");

    // Go to dashboard
    await page.click("text=Dashboard");
    await expect(page.locator(".dashboard-card", { hasText: "NavWS" })).toBeVisible({ timeout: 3000 });

    // Click the card
    await page.locator(".dashboard-card", { hasText: "NavWS" }).click();

    // Should navigate to workspace thread view (textarea visible)
    await expect(page.locator("textarea")).toBeVisible({ timeout: 3000 });
  });

  test("dashboard shows aggregate cost", async ({ page }) => {
    await page.goto("/");

    // Add workspace and run a thread
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/dash-cost");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Navigate to dashboard
    await page.click("text=Dashboard");
    await expect(page.locator(".dashboard-summary")).toBeVisible({ timeout: 3000 });
    // Total cost should be shown
    await expect(page.locator(".dashboard-summary", { hasText: "$" })).toBeVisible();
  });
});
