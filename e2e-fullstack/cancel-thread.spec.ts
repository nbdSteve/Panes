import { test, expect } from "@playwright/test";
import { addWorkspace, sendPrompt } from "./helpers";
import { mkdtempSync } from "fs";
import { tmpdir } from "os";
import { resolve } from "path";

test.describe("Full-Stack: Cancel Thread", () => {
  let wsPath: string;

  test.beforeEach(async () => {
    wsPath = mkdtempSync(resolve(tmpdir(), "panes-ws-"));
  });

  test("slow task -> stop -> thread cancelled", async ({ page }) => {
    await page.goto("/");
    await addWorkspace(page, wsPath);
    await sendPrompt(page, "slow task please");

    // Wait until the thread is mid-run (tool events flowing) before
    // clicking stop — auto-init may add startup latency.
    await page.locator(".tool-group").first().waitFor({ timeout: 15_000 });
    await page.locator(".btn-stop").waitFor({ timeout: 10_000 });
    await page.click(".btn-stop");
    await expect(page.locator("text=Cancelled")).toBeVisible({ timeout: 10_000 });
  });
});
