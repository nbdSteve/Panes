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

test.describe("Gates — Steer & Advanced Approval", () => {
  test("steer button opens text input for feedback", async ({ page }) => {
    await addWorkspaceAndSend(page, "do something dangerous");

    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 2000 });

    await page.click("button:has-text('Steer')");

    // Should open a text input for steering feedback
    await expect(page.locator(".gate-steer-input textarea")).toBeVisible();
    await expect(page.locator(".gate-steer-input textarea")).toBeFocused();
  });

  test("steer sends feedback and agent continues", async ({ page }) => {
    await addWorkspaceAndSend(page, "do something dangerous");

    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 2000 });

    await page.click("button:has-text('Steer')");
    await page.fill(".gate-steer-input textarea", "Use a safer approach instead");
    await page.click(".gate-steer-input button:has-text('Send')");

    // Gate should resolve with steered state
    await expect(page.locator("text=Steered")).toBeVisible({ timeout: 2000 });

    // Thread should eventually complete
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });
  });

  test("reject shows reason and stops the thread", async ({ page }) => {
    await addWorkspaceAndSend(page, "do something dangerous");

    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 2000 });

    await page.click("button:has-text('Reject')");

    // Should show rejected state with reason
    await expect(page.locator("text=Rejected")).toBeVisible({ timeout: 2000 });
  });

  test("approve-for-thread auto-approves subsequent similar gates", async ({ page }) => {
    await addWorkspaceAndSend(page, "do something dangerous");

    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 2000 });

    // Should have an "Approve for this thread" option
    await expect(page.locator("text=Approve for this thread")).toBeVisible();

    await page.click("text=Approve for this thread");

    // Gate should resolve
    await expect(page.locator("text=Approved")).toBeVisible({ timeout: 2000 });
  });
});
