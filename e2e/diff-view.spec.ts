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

test.describe("Diff Viewer", () => {
  test("clicking file in completion card opens diff viewer", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Expand files list
    await page.click(".files-changed-summary");
    await expect(page.locator(".files-changed-list")).toBeVisible();

    // Click a file
    await page.click(".files-changed-item.clickable >> nth=0");

    // Diff modal should appear
    await expect(page.locator(".diff-overlay")).toBeVisible();
    await expect(page.locator(".diff-modal")).toBeVisible();
  });

  test("diff viewer shows sidebar with files and diff content", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open file list and click first file
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");

    await expect(page.locator(".diff-modal")).toBeVisible();

    // Should have sidebar with file list
    await expect(page.locator(".diff-sidebar")).toBeVisible();
    await expect(page.locator(".diff-sidebar-file")).toHaveCount(2);

    // Should have toolbar with file path
    await expect(page.locator(".diff-toolbar")).toBeVisible();

    // Should have diff content with hunks
    await expect(page.locator(".diff-hunk")).toBeVisible();
  });

  test("diff viewer close button returns to thread", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await expect(page.locator(".diff-modal")).toBeVisible();

    // Close via button
    await page.click(".diff-close-btn");
    await expect(page.locator(".diff-overlay")).not.toBeVisible();
  });

  test("can navigate between files in sidebar", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await expect(page.locator(".diff-modal")).toBeVisible();

    // First file should be active
    await expect(page.locator(".diff-sidebar-file.active")).toHaveCount(1);

    // Click second file in sidebar
    await page.click(".diff-sidebar-file >> nth=1");

    // Second file should now be active
    const secondFile = page.locator(".diff-sidebar-file >> nth=1");
    await expect(secondFile).toHaveClass(/active/);
  });

  test("can add comment via line gutter click", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await expect(page.locator(".diff-modal")).toBeVisible();
    await expect(page.locator(".diff-hunk")).toBeVisible();

    // Click a gutter line number to start comment
    const gutterCell = page.locator(".diff-gutter-new").first();
    await gutterCell.click();

    // Comment form should appear
    await expect(page.locator(".diff-comment-form")).toBeVisible();

    // Type and submit
    await page.fill(".diff-comment-input", "This line needs fixing");
    await page.click(".diff-comment-actions button.btn-primary");

    // Comment should appear inline
    await expect(page.locator(".diff-inline-comment")).toBeVisible();
    await expect(page.locator(".diff-comment-body")).toContainText("This line needs fixing");
  });

  test("send feedback button appears in modal when comments exist", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await expect(page.locator(".diff-modal")).toBeVisible();
    await expect(page.locator(".diff-hunk")).toBeVisible();

    // No send feedback button yet
    await expect(page.locator(".diff-action-bar button:has-text('Send feedback')")).not.toBeVisible();

    // Click a gutter to add comment
    const gutterCell = page.locator(".diff-gutter-new").first();
    await gutterCell.click();
    await page.fill(".diff-comment-input", "Fix this");
    await page.click(".diff-comment-actions button.btn-primary");

    // Send feedback button should now appear in the action bar
    await expect(page.locator(".diff-action-bar button:has-text('Send feedback')")).toBeVisible();
    await expect(page.locator(".diff-action-bar button:has-text('1 comment')")).toBeVisible();
  });

  test("send feedback button also shows on completion card", async ({ page }) => {
    await addWorkspaceAndSend(page, "edit the files");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Open diff view and add a comment
    await page.click(".files-changed-summary");
    await page.click(".files-changed-item.clickable >> nth=0");
    await expect(page.locator(".diff-hunk")).toBeVisible();

    const gutterCell = page.locator(".diff-gutter-new").first();
    await gutterCell.click();
    await page.fill(".diff-comment-input", "Fix this");
    await page.click(".diff-comment-actions button.btn-primary");

    // Close diff view
    await page.click(".diff-close-btn");

    // Completion card should also show "Send feedback" button
    await expect(page.locator(".completion-card button:has-text('Send feedback')")).toBeVisible();
  });
});
