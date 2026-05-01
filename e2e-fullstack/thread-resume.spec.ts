import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForCompletion } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Thread Resume", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("complete thread -> send follow-up -> second completion", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "hello first");

    await waitForCompletion(page);
    await expect(page.locator(".completion-label-text").first()).toHaveText("Complete");

    await sendPrompt(page, "hello follow-up");

    await page.waitForTimeout(5000);
    const completionCards = await page.locator(".completion-card").count();
    expect(completionCards).toBeGreaterThanOrEqual(2);
  });
});
