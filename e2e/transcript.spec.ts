import { test, expect } from "@playwright/test";

test.describe("Transcript View", () => {
  test("transcript toggle switches between timeline and transcript", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/transcript-ws");
    await page.click("text=Add");

    await page.fill("textarea", "read the codebase");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });

    // Toggle should be visible
    const toggle = page.locator(".transcript-toggle");
    await expect(toggle).toBeVisible();

    // Click to enter transcript mode
    await toggle.click();
    await expect(page.locator(".transcript-view")).toBeVisible();

    // Prompt should appear as "You" message
    await expect(page.locator(".transcript-user .transcript-role", { hasText: "You" })).toBeVisible();

    // Tool calls should appear
    await expect(page.locator(".transcript-role", { hasText: "Tool call: Read" }).first()).toBeVisible();

    // Click again to go back to timeline
    await toggle.click();
    await expect(page.locator(".transcript-view")).not.toBeVisible();
    // Completion card should be back in the timeline
    await expect(page.locator(".completion-card")).toBeVisible();
  });

  test("transcript shows thinking events", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/transcript-think");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.locator(".transcript-toggle").click();
    await expect(page.locator(".transcript-thinking")).toBeVisible();
    await expect(page.locator(".transcript-thinking .transcript-role")).toHaveText("Thinking");
  });

  test("transcript shows completion with cost", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/transcript-complete");
    await page.click("text=Add");

    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    await page.locator(".transcript-toggle").click();

    // Session complete message should appear with cost
    await expect(page.locator(".transcript-role", { hasText: "Session complete" })).toBeVisible();
    await expect(page.locator(".transcript-message", { hasText: "Session complete" })).toContainText("$");
  });
});
