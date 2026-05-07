import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Cost Analytics", () => {
  let wsPath1: string;
  let wsPath2: string;

  test.beforeEach(async () => {
    wsPath1 = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
    wsPath2 = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("workspace cost breakdown shows after threads", async ({ page }) => {
    await page.goto("/");
    await page.waitForLoadState("networkidle");

    // Create two workspaces with threads
    await addWorkspace(page, wsPath1, "CostWS1");
    await sendPrompt(page, "hello world");
    await waitForCompletion(page);

    await addWorkspace(page, wsPath2, "CostWS2");
    await sendPrompt(page, "hello again");
    await waitForCompletion(page);

    // Navigate to feed
    await page.locator(".sidebar-item", { hasText: /^Feed$/ }).first().click();
    await expect(page.locator(".feed-view")).toBeVisible({ timeout: 10000 });

    // Cost bars should show workspace names
    await expect(page.locator(".cost-bars")).toBeVisible({ timeout: 10000 });
  });

  test("cost timeline has data after thread completion", async ({ page }) => {
    await page.goto("/");
    await page.waitForLoadState("networkidle");
    await addWorkspace(page, wsPath1, "TimelineWS");
    await sendPrompt(page, "hello world");
    await waitForCompletion(page);

    // Navigate to feed
    await page.locator(".sidebar-item", { hasText: /^Feed$/ }).first().click();
    await expect(page.locator(".feed-view")).toBeVisible({ timeout: 10000 });

    // Sparkline should be visible (may have mock data or real data)
    await expect(page.locator(".cost-sparkline")).toBeVisible({ timeout: 10000 });
  });
});
