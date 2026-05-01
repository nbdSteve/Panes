import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Memory Extraction", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-mem-"));
  });

  test("completing a thread extracts memories visible in memory panel", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "hello world");
    await waitForCompletion(page);

    // Give memory extraction a moment to complete (fire-and-forget from frontend)
    await page.waitForTimeout(1000);

    // Navigate to memory panel via sidebar
    await page.locator(".sidebar-item:has-text('Memory')").click();
    await expect(page.locator(".memory-panel")).toBeVisible({ timeout: 5000 });

    // Verify at least one memory was extracted
    await expect(page.locator(".memory-card").first()).toBeVisible({ timeout: 5000 });
    const count = await page.locator(".memory-card").count();
    expect(count).toBeGreaterThanOrEqual(1);
  });
});
