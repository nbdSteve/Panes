import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Feed View", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("completed thread appears in feed", async ({ page }) => {
    await page.goto("/");
    await page.waitForLoadState("networkidle");
    await addWorkspace(page, wsPath, "Test Feed WS");
    await sendPrompt(page, "hello feed test");

    await waitForCompletion(page);

    await page.locator(".sidebar-item", { hasText: /^Feed$/ }).first().click();
    await expect(page.locator(".feed-item").first()).toBeVisible({ timeout: 10000 });
  });
});
