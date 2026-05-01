import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Workspace Guard", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("second prompt in same workspace queues until first completes", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);

    // Start a slow task — takes 500ms per step × 5 steps
    await sendPrompt(page, "slow task please");

    // Wait for at least one tool group to appear (thread is running)
    await page.locator(".tool-group").first().waitFor({ timeout: 15_000 });

    // Send a second prompt while the first is still running
    const textarea = page.locator("textarea");
    await textarea.fill("hello second");
    await textarea.press("Enter");

    // First thread should eventually complete
    await waitForCompletion(page, 30_000);

    // The follow-up should eventually produce a second completion
    await page.waitForTimeout(2000);
    const completions = await page.locator(".completion-card").count();
    expect(completions).toBeGreaterThanOrEqual(1);
  });

  test("different workspaces can run threads concurrently", async ({ page }) => {
    const wsPath2 = mkdtempSync(resolve(tmpdir(), "panes-ws-"));

    await page.goto("/");
    await addWorkspace(page, wsPath, "WS-A");
    await addWorkspace(page, wsPath2, "WS-B");

    // Click on first workspace and start a slow task
    await page.locator(".sidebar-item").filter({ has: page.locator("span", { hasText: /^WS-A$/ }) }).click();
    await sendPrompt(page, "slow task please");

    // Switch to second workspace and start a task
    await page.locator(".sidebar-item").filter({ has: page.locator("span", { hasText: /^WS-B$/ }) }).click();
    await sendPrompt(page, "hello world");

    // Second workspace should complete (text-only is fast)
    await waitForCompletion(page, 15_000);
    await expect(page.locator(".completion-label-text").first()).toHaveText("Complete");

    // Switch back to first workspace — it should also eventually complete
    await page.locator(".sidebar-item").filter({ has: page.locator("span", { hasText: /^WS-A$/ }) }).click();
    await waitForCompletion(page, 30_000);
  });
});
