import { test, expect } from "@playwright/test";

test.describe("Thread Lifecycle", () => {
  test("follow-up continues same thread with divider", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    // First message
    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Follow-up
    await page.fill("textarea", "tell me more");
    await page.press("textarea", "Enter");

    // Should show follow-up divider
    await expect(page.locator(".follow-up .thread-prompt-text", { hasText: "tell me more" })).toBeVisible({ timeout: 2000 });

    // Should complete with a second completion card
    await expect(page.locator(".completion-card")).toHaveCount(2, { timeout: 3000 });

    // Thread list should still show 1 thread (not 2)
    await expect(page.locator(".thread-list-item")).toHaveCount(1);
  });

  test("step cards are collapsible", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    // Multi-step generates multiple tool cards
    await page.fill("textarea", "do something complex");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });

    // Step cards should be visible
    const steps = page.locator(".step-card");
    const count = await steps.count();
    expect(count).toBeGreaterThan(2);

    // Click a step card to collapse it
    await steps.first().click();
    await expect(steps.first().locator(".step-detail")).not.toBeVisible();
  });

  test.skip("thread shows elapsed time per step", async ({ page }) => {
    // Not yet implemented: .step-elapsed UI element on tool result cards
  });

  test("thread view scrolls to bottom as new events arrive", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await page.fill("textarea", "do something complex");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });

    // The completion card (last element) should be in the viewport
    await expect(page.locator(".completion-card")).toBeInViewport();
  });

  test("queued follow-up sends automatically after current turn completes", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    // Start a thread
    await page.fill("textarea", "do something complex");
    await page.press("textarea", "Enter");

    // While running, type a follow-up and send
    await expect(page.locator(".btn-stop")).toBeVisible({ timeout: 2000 });
    await page.fill("textarea", "now do more");
    await page.press("textarea", "Enter");

    // Queued indicator should appear
    await expect(page.locator(".queued-follow-up")).toBeVisible();
    await expect(page.locator(".queued-follow-up-text")).toHaveText("now do more");

    // Wait for first turn to complete and follow-up to auto-fire
    await expect(page.locator(".follow-up .thread-prompt-text", { hasText: "now do more" })).toBeVisible({ timeout: 5000 });

    // Should eventually get a second completion
    await expect(page.locator(".completion-card")).toHaveCount(2, { timeout: 5000 });
  });

  test("queued follow-up can be cancelled", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await page.fill("textarea", "do something complex");
    await page.press("textarea", "Enter");

    // Queue a follow-up
    await expect(page.locator(".btn-stop")).toBeVisible({ timeout: 2000 });
    await page.fill("textarea", "follow up");
    await page.press("textarea", "Enter");
    await expect(page.locator(".queued-follow-up")).toBeVisible();

    // Cancel it
    await page.click(".queued-follow-up-cancel");
    await expect(page.locator(".queued-follow-up")).not.toBeVisible();

    // Only one completion should appear
    await expect(page.locator(".completion-card")).toHaveCount(1, { timeout: 5000 });
  });

  test("stop button interrupts running thread", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await page.fill("textarea", "do something complex");
    await page.press("textarea", "Enter");

    await expect(page.locator(".btn-stop")).toBeVisible({ timeout: 2000 });
    await page.click(".btn-stop");

    // Should show interrupted indicator
    await expect(page.locator(".interrupted-card")).toBeVisible({ timeout: 2000 });
    await expect(page.locator("text=Cancelled")).toBeVisible();

    // No completion card should appear
    await expect(page.locator(".completion-card")).not.toBeVisible();

    // Textarea should accept input for follow-up
    await page.fill("textarea", "try again");
    await expect(page.locator("textarea")).toHaveValue("try again");
  });

  test("switching threads preserves scroll position and content", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    // Create first thread
    await page.fill("textarea", "first message");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Create second thread
    await page.click(".thread-list-new");
    await page.fill("textarea", "second message");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Switch back to first
    await page.click(".thread-list-item:last-child");
    await expect(page.locator(".thread-prompt-text", { hasText: "first message" })).toBeVisible();
    await expect(page.locator(".completion-card")).toBeVisible();

    // Switch back to second
    await page.click(".thread-list-item:first-child");
    await expect(page.locator(".thread-prompt-text", { hasText: "second message" })).toBeVisible();
  });
});
