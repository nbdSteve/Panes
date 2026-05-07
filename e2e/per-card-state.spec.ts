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

test.describe("Per-Card State — Independent File Tracking", () => {
  test("first completion card has its own files from tool events", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Files changed should appear (extracted from Edit tool events' input.file_path)
    await expect(page.locator(".files-changed")).toBeVisible();
    await expect(page.locator(".files-changed-count")).toContainText("file");
  });

  test("follow-up completion card has different files", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Keep the first card's changes
    await page.click("button:has-text('Keep')");
    await expect(page.locator("text=Kept as-is")).toBeVisible();

    // Send a follow-up that does NOT edit files (text-only response)
    await page.fill("textarea", "explain the code");
    await page.press("textarea", "Enter");

    // Wait for second completion card
    await expect(page.locator(".completion-card").nth(1)).toBeVisible({ timeout: 5000 });

    // Second card should not have inspect/revert buttons (no file changes)
    const secondCard = page.locator(".completion-card").nth(1);
    await expect(secondCard.locator("button:has-text('Inspect')")).not.toBeVisible();
    await expect(secondCard.locator("button:has-text('Revert')")).not.toBeVisible();
  });

  test("committing card 1 does not affect card 2", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Keep first card (don't commit yet)
    await page.click("button:has-text('Keep')");

    // Follow up with another edit
    await page.fill("textarea", "write more code");
    await page.press("textarea", "Enter");

    await expect(page.locator(".completion-card").nth(1)).toBeVisible({ timeout: 5000 });

    // Second card should have its own action buttons
    const secondCard = page.locator(".completion-card").nth(1);
    await expect(secondCard.locator("button:has-text('Inspect')")).toBeVisible();
    await expect(secondCard.locator("button:has-text('Revert')")).toBeVisible();
  });

  test("committed badge only on committed card", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Commit first card via inspect modal
    await page.click("button:has-text('Inspect')");
    await expect(page.locator(".diff-overlay")).toBeVisible();
    await page.click(".diff-commit-trigger");
    await page.fill(".diff-commit-input", "feat: initial commit");
    await page.click(".diff-commit-btn");
    await expect(page.locator("text=Committed")).toBeVisible({ timeout: 2000 });

    // Follow up with another edit
    await page.fill("textarea", "write more code");
    await page.press("textarea", "Enter");

    await expect(page.locator(".completion-card").nth(1)).toBeVisible({ timeout: 5000 });

    // First card shows "Committed" badge
    const firstCard = page.locator(".completion-card").nth(0);
    await expect(firstCard.locator("text=Committed")).toBeVisible();

    // Second card should have action buttons, not a badge
    const secondCard = page.locator(".completion-card").nth(1);
    await expect(secondCard.locator("button:has-text('Inspect')")).toBeVisible();
    await expect(secondCard.locator("text=Committed")).not.toBeVisible();
  });
});
