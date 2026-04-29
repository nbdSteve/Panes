import { test, expect } from "@playwright/test";

test.describe("Panes App — Core Flows", () => {
  test("shows welcome screen on launch", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("text=Welcome to Panes")).toBeVisible();
    await expect(page.locator("text=Add a workspace")).toBeVisible();
  });

  test("can add a workspace", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-workspace");
    await page.fill('input[placeholder="Display name (optional)"]', "Test Project");
    await page.click("text=Add");

    // Sidebar should show the workspace
    await expect(page.locator(".sidebar-item", { hasText: "Test Project" })).toBeVisible();
    // Thread list should appear
    await expect(page.locator(".thread-list")).toBeVisible();
    // Empty prompt should be visible
    await expect(page.locator("textarea")).toBeVisible();
  });

  test("text-only prompt shows thinking then response", async ({ page }) => {
    await page.goto("/");

    // Add workspace
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-workspace");
    await page.click("text=Add");

    // Send a text-only prompt (doesn't match any special keyword)
    await page.fill("textarea", "hello world");
    await page.press("textarea", "Enter");

    // Should see the prompt displayed
    await expect(page.locator(".thread-prompt-text", { hasText: "hello world" })).toBeVisible();

    // Should see "Starting..." indicator while waiting for events
    await expect(page.locator("text=Starting...")).toBeVisible({ timeout: 2000 });

    // Should eventually see the complete card
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });
    await expect(page.locator(".completion-label-text", { hasText: "Complete" })).toBeVisible();

    // Cost should be shown
    await expect(page.locator(".completion-stat-value").first()).toBeVisible();

    // No commit/revert buttons for text-only response
    await expect(page.locator("text=Commit")).not.toBeVisible();
  });

  test("file edit prompt shows commit/revert buttons", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-workspace");
    await page.click("text=Add");

    // "edit" keyword triggers FileEdit scenario
    await page.fill("textarea", "edit the files");
    await page.press("textarea", "Enter");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Should show commit/revert buttons
    await expect(page.locator("button", { hasText: "Commit" })).toBeVisible();
    await expect(page.locator("button", { hasText: "Revert all" })).toBeVisible();
  });

  test("gate scenario shows approval card and resolves on click", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-workspace");
    await page.click("text=Add");

    // "dangerous" keyword triggers GatedAction scenario
    await page.fill("textarea", "do something dangerous");
    await page.press("textarea", "Enter");

    // Should see the gate card
    await expect(page.locator(".gate-card")).toBeVisible({ timeout: 2000 });
    await expect(page.locator("text=Approval needed")).toBeVisible();
    await expect(page.locator(".risk-badge")).toBeVisible();

    // Click approve
    await page.click("button:has-text('Approve')");

    // Should show approved state
    await expect(page.locator("text=Approved")).toBeVisible({ timeout: 2000 });

    // Should eventually complete
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });
  });

  test("error scenario shows error card", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-workspace");
    await page.click("text=Add");

    // "error" keyword triggers Error scenario
    await page.fill("textarea", "cause an error");
    await page.press("textarea", "Enter");

    // Should see error card
    await expect(page.locator(".error-card")).toBeVisible({ timeout: 2000 });
    await expect(page.locator(".error-label-text")).toHaveText("Error");
  });

  test("multi-step scenario shows tool use cards", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-workspace");
    await page.click("text=Add");

    // "complex" keyword triggers MultiStep scenario
    await page.fill("textarea", "do something complex");
    await page.press("textarea", "Enter");

    // Should see tool cards appearing
    await expect(page.locator(".tool-name").first()).toBeVisible({ timeout: 3000 });

    // Should eventually complete
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 5000 });
  });

  test("multiple threads in one workspace", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-workspace");
    await page.click("text=Add");

    // Send first prompt
    await page.fill("textarea", "first message");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Thread list should show 1 thread
    await expect(page.locator(".thread-list-item")).toHaveCount(1);

    // Start new thread
    await page.click(".thread-list-new");
    await expect(page.locator(".thread-empty")).toBeVisible();

    // Send second prompt
    await page.fill("textarea", "second message");
    await page.press("textarea", "Enter");
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Thread list should show 2 threads
    await expect(page.locator(".thread-list-item")).toHaveCount(2);

    // Click first thread — should see its content
    await page.click(".thread-list-item:last-child");
    await expect(page.locator(".thread-prompt-text", { hasText: "first message" })).toBeVisible();
  });

  test("sidebar shows workspace status", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-workspace");
    await page.fill('input[placeholder="Display name (optional)"]', "Status Test");
    await page.click("text=Add");

    // Before sending: idle dot
    const dot = page.locator(".sidebar-item", { hasText: "Status Test" }).locator(".status-dot");
    await expect(dot).toBeVisible();

    // Send a prompt — status should change to working
    await page.fill("textarea", "hello");
    await page.press("textarea", "Enter");

    // After completion — should have a thread count badge
    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });
    await expect(page.locator(".thread-count", { hasText: "1" })).toBeVisible();
  });

  test("feed view and workspace switching", async ({ page }) => {
    await page.goto("/");

    // Feed should be visible initially
    await expect(page.locator("text=Welcome to Panes")).toBeVisible();

    // Add workspace
    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-workspace");
    await page.fill('input[placeholder="Display name (optional)"]', "Switch Test");
    await page.click("text=Add");

    // Should be in workspace view
    await expect(page.locator(".thread-list")).toBeVisible();

    // Click Feed
    await page.click(".sidebar-item:has-text('Feed')");
    await expect(page.locator("text=Welcome to Panes")).toBeVisible();
    await expect(page.locator(".thread-list")).not.toBeVisible();

    // Click workspace again
    await page.click(".sidebar-item:has-text('Switch Test')");
    await expect(page.locator(".thread-list")).toBeVisible();
  });

  test("markdown renders in responses", async ({ page }) => {
    await page.goto("/");

    await page.click("text=Add workspace");
    await page.fill('input[placeholder="/path/to/project"]', "/tmp/test-workspace");
    await page.click("text=Add");

    // "read" triggers ReadAndRespond with markdown bullets
    await page.fill("textarea", "read and explain the code");
    await page.press("textarea", "Enter");

    await expect(page.locator(".completion-card")).toBeVisible({ timeout: 3000 });

    // Markdown should render list items
    await expect(page.locator(".markdown-body li").first()).toBeVisible();
  });
});
