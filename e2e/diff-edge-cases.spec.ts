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

test.describe("Diff Viewer Edge Cases", () => {
  test("escape key closes the diff modal", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await expect(page.locator(".diff-modal")).toBeVisible();

    // Press Escape
    await page.keyboard.press("Escape");
    await expect(page.locator(".diff-overlay")).not.toBeVisible();
  });

  test("escape in commit view returns to diff view, not close", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await expect(page.locator(".diff-modal")).toBeVisible();

    // Switch to commit view
    await page.click(".diff-action-bar button:has-text('Commit')");
    await expect(page.locator("text=Commit Changes")).toBeVisible();

    // Press Escape — should go back to diff, not close
    await page.keyboard.press("Escape");
    await expect(page.locator("text=Commit Changes")).not.toBeVisible();
    await expect(page.locator(".diff-modal")).toBeVisible();
  });

  test("commit button disabled when no message entered", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view and go to commit view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await page.click(".diff-action-bar button:has-text('Commit')");
    await expect(page.locator("text=Commit Changes")).toBeVisible();

    // Commit button should be disabled with empty message
    await expect(page.locator(".diff-commit-btn")).toBeDisabled();
  });

  test("commit button enabled after entering message", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view and go to commit view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await page.click(".diff-action-bar button:has-text('Commit')");

    // Type a message
    await page.fill('textarea[placeholder="Describe your changes..."]', "feat: add feature");
    await expect(page.locator(".diff-commit-btn")).not.toBeDisabled();
  });

  test("deselecting all files disables commit even with message", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view and go to commit view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await page.click(".diff-action-bar button:has-text('Commit')");

    // Type a message
    await page.fill('textarea[placeholder="Describe your changes..."]', "some message");

    // Deselect all via the select-all checkbox
    await page.click(".diff-commit-select-all input[type='checkbox']");

    // Commit button should be disabled
    await expect(page.locator(".diff-commit-btn")).toBeDisabled();
  });

  test("back button in commit view returns to diff", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view and go to commit view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await page.click(".diff-action-bar button:has-text('Commit')");
    await expect(page.locator("text=Commit Changes")).toBeVisible();

    // Click back
    await page.click("text=Back");

    // Should return to diff view
    await expect(page.locator("text=Commit Changes")).not.toBeVisible();
    await expect(page.locator(".diff-sidebar")).toBeVisible();
  });

  test("generate button calls message generation", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view and go to commit view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await page.click(".diff-action-bar button:has-text('Commit')");

    // Click generate
    await page.click("text=Generate");

    // The suggested message should populate (mock returns a canned message)
    await expect(page.locator('textarea[placeholder="Describe your changes..."]')).not.toHaveValue("");
  });

  test("amend checkbox is available in commit view", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view and go to commit view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await page.click(".diff-action-bar button:has-text('Commit')");

    // Amend checkbox should be present
    await expect(page.locator("text=Amend last commit")).toBeVisible();
  });

  test("comment form cancel button dismisses form", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await expect(page.locator(".diff-hunk")).toBeVisible();

    // Click gutter to open comment form
    await page.locator(".diff-gutter-new").first().click();
    await expect(page.locator(".diff-comment-form")).toBeVisible();

    // Cancel
    await page.click(".diff-comment-actions button:has-text('Cancel')");
    await expect(page.locator(".diff-comment-form")).not.toBeVisible();
  });

  test("comment submit button disabled with empty text", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await expect(page.locator(".diff-hunk")).toBeVisible();

    // Click gutter to open comment form
    await page.locator(".diff-gutter-new").first().click();
    await expect(page.locator(".diff-comment-form")).toBeVisible();

    // Submit button should be disabled
    await expect(page.locator(".diff-comment-actions button.btn-primary")).toBeDisabled();
  });

  test("prev/next file buttons navigate correctly", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await expect(page.locator(".diff-modal")).toBeVisible();

    // Prev should be disabled on first file
    await expect(page.locator(".diff-nav-btn >> nth=0")).toBeDisabled();

    // Next should be enabled
    await expect(page.locator(".diff-nav-btn >> nth=1")).not.toBeDisabled();

    // Click next
    await page.click(".diff-nav-btn >> nth=1");

    // Now prev enabled, next disabled (only 2 files)
    await expect(page.locator(".diff-nav-btn >> nth=0")).not.toBeDisabled();
    await expect(page.locator(".diff-nav-btn >> nth=1")).toBeDisabled();
  });

  test("opening inspect from completion card without file changes shows empty state", async ({ page }) => {
    await addWorkspaceAndSend(page, "just say hello");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Text-only scenario should not show Inspect button
    await expect(page.locator("button:has-text('Inspect')")).not.toBeVisible();
  });

  test("multiple comments increment the action bar counter", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await expect(page.locator(".diff-hunk")).toBeVisible();

    // Add first comment
    const gutters = page.locator(".diff-gutter-new");
    await gutters.nth(0).click();
    await page.fill(".diff-comment-input", "First comment");
    await page.click(".diff-comment-actions button.btn-primary");

    await expect(page.locator(".diff-action-bar button:has-text('1 comment')")).toBeVisible();

    // Add second comment on a different line
    await gutters.nth(1).click();
    await page.fill(".diff-comment-input", "Second comment");
    await page.click(".diff-comment-actions button.btn-primary");

    await expect(page.locator(".diff-action-bar button:has-text('2 comments')")).toBeVisible();
  });
});
