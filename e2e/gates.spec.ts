import { test, expect } from "@playwright/test";

function addWorkspaceAndSend(page: any, prompt: string) {
  return (async () => {
    await page.goto("/");
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");
    await page.fill("textarea", prompt);
    await page.press("textarea", "Enter");
  })();
}

test.describe("Gates — Continue & Abort", () => {
  test("gate card appears for dangerous prompt", async ({ page }) => {
    await addWorkspaceAndSend(page, "do something dangerous");

    const gateCard = page.locator(".gate-card");
    await expect(gateCard).toBeVisible({ timeout: 3000 });

    // Should show approval-needed state with Continue and Abort buttons
    await expect(page.locator("text=Approval needed")).toBeVisible();
    await expect(page.locator("button:has-text('Continue')")).toBeVisible();
    await expect(page.locator("button:has-text('Abort')")).toBeVisible();
  });

  test("gate card shows risk badge and tool name", async ({ page }) => {
    await addWorkspaceAndSend(page, "do something dangerous");

    const gateCard = page.locator(".gate-card");
    await expect(gateCard).toBeVisible({ timeout: 3000 });

    // Should display risk level and tool name
    await expect(gateCard.locator(".risk-badge")).toBeVisible();
    await expect(gateCard.locator("text=Bash")).toBeVisible();
  });

  test("continue resolves gate and thread completes", async ({ page }) => {
    await addWorkspaceAndSend(page, "do something dangerous");

    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Continue')");

    // Gate should resolve to "Continued" state
    await expect(page.locator("text=Continued")).toBeVisible({ timeout: 3000 });

    // Thread should eventually complete
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });
  });

  test("abort resolves gate and stops thread", async ({ page }) => {
    await addWorkspaceAndSend(page, "do something dangerous");

    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 3000 });

    await page.click("button:has-text('Abort')");

    // Gate should resolve to "Aborted" state
    await expect(page.locator("text=Aborted")).toBeVisible({ timeout: 3000 });
  });
});
