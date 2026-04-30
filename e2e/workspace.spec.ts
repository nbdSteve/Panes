import { test, expect } from "@playwright/test";

test.describe("Workspace Management", () => {
  test.skip("remove workspace from sidebar", async ({ page }) => {
    // Not yet implemented: right-click context menu for workspace removal
  });

  test.skip("one active thread per workspace — blocks second send", async ({ page }) => {
    // Not yet implemented: "Workspace has an active thread" message
  });

  test("cancel running thread", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await page.fill("textarea", "do something complex");
    await page.press("textarea", "Enter");

    // Stop button should appear in prompt bar while running
    await expect(page.locator(".btn-stop")).toBeVisible({ timeout: 2000 });

    await page.click(".btn-stop");

    // Thread should be marked as interrupted
    await expect(page.locator("text=Cancelled")).toBeVisible({ timeout: 2000 });

    // Textarea should remain enabled for follow-up
    await expect(page.locator("textarea")).toBeEnabled();
  });

  test("sidebar shows gate status for workspace with pending gate", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.fill('input[placeholder="Display name (optional)"]', "Gate WS");
    await page.click("text=Add");

    await page.fill("textarea", "do something dangerous");
    await page.press("textarea", "Enter");

    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 2000 });

    // Sidebar should show gate status
    const wsItem = page.locator(".sidebar-item", { hasText: "Gate WS" });
    await expect(wsItem.locator(".status-dot.gate")).toBeVisible();
  });

  test("sidebar shows error status for workspace with errored thread", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.fill('input[placeholder="Display name (optional)"]', "Err WS");
    await page.click("text=Add");

    await page.fill("textarea", "cause an error");
    await page.press("textarea", "Enter");

    await expect(page.locator(".error-card")).toBeVisible({ timeout: 2000 });

    // Sidebar should show error status
    const wsItem = page.locator(".sidebar-item", { hasText: "Err WS" });
    await expect(wsItem.locator(".status-dot.error")).toBeVisible();
  });
});
