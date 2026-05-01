import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt, waitForGate } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Gate Reject", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("gate request -> abort -> thread stops", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "do something dangerous");

    await waitForGate(page);
    await page.waitForTimeout(500);
    await page.click("button:has-text('Abort')");
    await expect(page.locator("text=Aborted")).toBeVisible({ timeout: 5000 });
  });
});
