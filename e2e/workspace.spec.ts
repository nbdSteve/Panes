import { test, expect } from "@playwright/test";

test.describe("Workspace Management", () => {
  test("remove workspace from sidebar", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.fill('input[placeholder="Display name (optional)"]', "Removable");
    await page.click("text=Add");

    await expect(page.locator(".sidebar-item", { hasText: "Removable" })).toBeVisible();

    // Right-click or menu to remove
    await page.locator(".sidebar-item", { hasText: "Removable" }).click({ button: "right" });
    await page.click("text=Remove workspace");

    // Workspace should be gone
    await expect(page.locator(".sidebar-item", { hasText: "Removable" })).not.toBeVisible();

    // Should return to feed
    await expect(page.locator("text=Welcome to Panes")).toBeVisible();
  });

  test("one active thread per workspace — blocks second send", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    // Start a long-running thread (multi-step takes longer)
    await page.fill("textarea", "do something complex");
    await page.press("textarea", "Enter");

    // While running, start a new thread
    await page.click(".thread-list-new");

    // Prompt bar should indicate workspace is busy
    await expect(page.locator("textarea")).toBeDisabled();
    await expect(page.locator("text=Workspace has an active thread")).toBeVisible();
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
