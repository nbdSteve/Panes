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

test.describe("Gate Rejection", () => {
  test("rejecting a gate stops the thread — no completion card appears from gated work", async ({ page }) => {
    await addWorkspaceAndSend(page, "do something dangerous");

    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 2000 });

    await page.click("button:has-text('Reject')");

    // Gate should show rejected state
    await expect(page.locator("text=Rejected")).toBeVisible({ timeout: 2000 });

    // Thread should complete (rejected action ends the turn)
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // The post-gate tool_result and text events should NOT have been emitted
    // (the dangerous operation text should not appear)
    await expect(page.locator("text=The dangerous operation has been completed")).not.toBeVisible();
  });

  test("rejecting a gate allows the user to send a follow-up", async ({ page }) => {
    await addWorkspaceAndSend(page, "do something dangerous");

    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 2000 });

    await page.click("button:has-text('Reject')");
    await expect(page.locator("text=Rejected")).toBeVisible({ timeout: 2000 });

    // Wait for the turn to complete after rejection
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // User should be able to follow up
    await page.fill("textarea", "try a safer approach");
    await page.press("textarea", "Enter");

    // Follow-up divider should appear
    await expect(page.locator(".follow-up .thread-prompt-text", { hasText: "try a safer approach" })).toBeVisible({ timeout: 2000 });

    // Second completion should eventually appear
    await expect(page.locator(".completion-card")).toHaveCount(2, { timeout: 5000 });
  });
});

test.describe("Error Recovery", () => {
  test("user can send a follow-up after an error", async ({ page }) => {
    await addWorkspaceAndSend(page, "cause an error");

    await expect(page.locator(".error-card")).toBeVisible({ timeout: 2000 });

    // Textarea should be enabled for follow-up
    await expect(page.locator("textarea")).toBeEnabled();

    await page.fill("textarea", "try again please");
    await page.press("textarea", "Enter");

    // Follow-up should appear
    await expect(page.locator(".follow-up .thread-prompt-text", { hasText: "try again please" })).toBeVisible({ timeout: 2000 });

    // Should get a completion (the follow-up prompt routes to text-only scenario)
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });
  });

  test("error does not show commit/revert buttons", async ({ page }) => {
    await addWorkspaceAndSend(page, "cause an error");

    await expect(page.locator(".error-card")).toBeVisible({ timeout: 2000 });

    // No completion actions should be visible
    await expect(page.locator("button:has-text('Commit')")).not.toBeVisible();
    await expect(page.locator("button:has-text('Revert')")).not.toBeVisible();
    await expect(page.locator("button:has-text('Keep')")).not.toBeVisible();
  });
});

test.describe("Input Validation", () => {
  test("empty prompt does not send", async ({ page }) => {
    await page.goto("/");
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    // Press Enter with empty textarea
    await page.press("textarea", "Enter");

    // No thread should be created
    await expect(page.locator(".thread-prompt-display")).not.toBeVisible();
    await expect(page.locator(".thread-list-item")).toHaveCount(0);
  });

  test("whitespace-only prompt does not send", async ({ page }) => {
    await page.goto("/");
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await page.fill("textarea", "   \n  ");
    await page.press("textarea", "Enter");

    // No thread should be created
    await expect(page.locator(".thread-prompt-display")).not.toBeVisible();
    await expect(page.locator(".thread-list-item")).toHaveCount(0);
  });

  test("send button is disabled when textarea is empty", async ({ page }) => {
    await page.goto("/");
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await expect(page.locator(".btn-send")).toBeDisabled();
  });
});

test.describe("Double-Click Safety", () => {
  test("double-clicking approve only fires once", async ({ page }) => {
    await addWorkspaceAndSend(page, "do something dangerous");

    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 2000 });

    const approveBtn = page.locator("button:has-text('Approve')");

    // Rapid double-click
    await approveBtn.dblclick();

    // Should resolve to approved (not error)
    await expect(page.locator("text=Approved")).toBeVisible({ timeout: 2000 });

    // Thread should complete normally — only one completion card
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });
    await expect(page.locator(".completion-card")).toHaveCount(1);
  });

  test("double-clicking reject only fires once", async ({ page }) => {
    await addWorkspaceAndSend(page, "do something dangerous");

    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 2000 });

    const rejectBtn = page.locator("button:has-text('Reject')");

    // Rapid double-click
    await rejectBtn.dblclick();

    // Should resolve to rejected
    await expect(page.locator("text=Rejected")).toBeVisible({ timeout: 2000 });

    // Should complete — only one completion card
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });
    await expect(page.locator(".completion-card")).toHaveCount(1);
  });
});

test.describe("Dialog Cancellation", () => {
  test("cancelling commit dialog does not commit — buttons remain", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open commit dialog
    await page.click("button:has-text('Commit')");
    await expect(page.locator(".commit-dialog")).toBeVisible();

    // Cancel the dialog
    await page.click(".commit-dialog button:has-text('Cancel')");

    // Dialog should close
    await expect(page.locator(".commit-dialog")).not.toBeVisible();

    // Commit/Revert/Keep buttons should still be present
    await expect(page.locator("button:has-text('Commit')")).toBeVisible();
    await expect(page.locator("button:has-text('Revert all')")).toBeVisible();
    await expect(page.locator("button:has-text('Keep')")).toBeVisible();
  });

  test("cancelling revert confirmation does not revert — buttons remain", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open revert confirm
    await page.click("button:has-text('Revert all')");
    await expect(page.locator(".revert-confirm")).toBeVisible();

    // Cancel the confirmation
    await page.click(".revert-confirm button:has-text('Cancel')");

    // Dialog should close
    await expect(page.locator(".revert-confirm")).not.toBeVisible();

    // Buttons should still be present
    await expect(page.locator("button:has-text('Commit')")).toBeVisible();
    await expect(page.locator("button:has-text('Revert all')")).toBeVisible();
  });
});

test.describe("Queue Edge Cases", () => {
  test("second queued message replaces the first — only latest is sent", async ({ page }) => {
    await page.goto("/");
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await page.fill("textarea", "do something complex");
    await page.press("textarea", "Enter");

    // Wait for running state
    await expect(page.locator(".btn-stop")).toBeVisible({ timeout: 2000 });

    // Queue first follow-up
    await page.fill("textarea", "first follow up");
    await page.press("textarea", "Enter");
    await expect(page.locator(".queued-follow-up-text")).toHaveText("first follow up");

    // Queue second follow-up (replaces first)
    await page.fill("textarea", "second follow up");
    await page.press("textarea", "Enter");
    await expect(page.locator(".queued-follow-up-text")).toHaveText("second follow up");

    // Wait for first turn to complete and follow-up to fire
    await expect(page.locator(".follow-up .thread-prompt-text", { hasText: "second follow up" })).toBeVisible({ timeout: 5000 });

    // "first follow up" should never appear as a follow-up divider
    await expect(page.locator(".follow-up .thread-prompt-text", { hasText: "first follow up" })).not.toBeVisible();
  });

  test("stopping a thread clears any queued follow-up", async ({ page }) => {
    await page.goto("/");
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await page.fill("textarea", "do something complex");
    await page.press("textarea", "Enter");

    await expect(page.locator(".btn-stop")).toBeVisible({ timeout: 2000 });

    // Queue a follow-up
    await page.fill("textarea", "queued message");
    await page.press("textarea", "Enter");
    await expect(page.locator(".queued-follow-up")).toBeVisible();

    // Stop the thread
    await page.click(".btn-stop");

    // Queued indicator should be gone
    await expect(page.locator(".queued-follow-up")).not.toBeVisible();

    // Interrupted card should appear
    await expect(page.locator("text=Cancelled")).toBeVisible({ timeout: 2000 });

    // The queued message should never be sent — no follow-up divider
    await expect(page.locator(".follow-up .thread-prompt-text", { hasText: "queued message" })).not.toBeVisible();
  });
});

test.describe("Interrupted Thread Recovery", () => {
  test("user can resume after stopping a thread", async ({ page }) => {
    await page.goto("/");
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-ws");
    await page.click("text=Add");

    await page.fill("textarea", "do something complex");
    await page.press("textarea", "Enter");

    await expect(page.locator(".btn-stop")).toBeVisible({ timeout: 2000 });
    await page.click(".btn-stop");
    await expect(page.locator("text=Cancelled")).toBeVisible({ timeout: 2000 });

    // Should be able to send a follow-up
    await page.fill("textarea", "continue from where you stopped");
    await page.press("textarea", "Enter");

    await expect(page.locator(".follow-up .thread-prompt-text", { hasText: "continue from where you stopped" })).toBeVisible({ timeout: 2000 });
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });
  });
});
