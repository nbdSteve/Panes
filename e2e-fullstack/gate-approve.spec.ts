import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForGate, waitForCompletion } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Gate Approve", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("gate request -> approve -> completion", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "do something dangerous");

    await waitForGate(page);
    await expect(page.locator(".gate-card")).toBeVisible();

    // Small delay to let backend set up gate channel
    await page.waitForTimeout(500);
    await page.click("button:has-text('Continue')");
    await expect(page.locator("text=Continued")).toBeVisible({ timeout: 5000 });
    await waitForCompletion(page);
    await expect(page.locator(".completion-label-text")).toHaveText("Complete");
  });
});
